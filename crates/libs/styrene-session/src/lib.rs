//! Frontend session profiles over the shared Styrene IPC client.
//!
//! A session owns one negotiated connection to a daemon contract and the
//! lifecycle of whatever it started to reach it. The three profiles never
//! fall back to one another:
//!
//! - **Connected** reaches an existing daemon endpoint that something else
//!   owns. A failure is a typed, recoverable connection error; no runtime is
//!   started.
//! - **Quick**, **Local**, and **Portable** open a managed operator profile,
//!   start its daemon in this process, and reach it over the profile's
//!   host-private socket, so every operation takes the same typed client
//!   path as Connected. Closing the session shuts the daemon down and
//!   releases what the profile owns.
//! - **Fixture** answers the client from a deterministic script over an
//!   in-process stream pair. It opens no daemon, socket file, or network
//!   interface, and exposes exactly the operations the script supports.
//!
//! Live is an observed runtime condition, not a profile. `live` and
//! `embedded` remain as constructors for Connected and Quick sessions.
//!
//! Sessions return canonical `styrene_ipc` records through
//! [`styrene_ipc_client::Client`]; they define no second record set.

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use rmpv::Value;
use serde::Serialize;
use styrene_ipc::IpcError;
use styrene_ipc::types::{ActiveCapabilitiesInfo, DaemonStatusInfo, ProfileInfo};
use styrene_ipc_wire::{self as wire, MessageType, REQUEST_ID_SIZE};
use thiserror::Error;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc};

pub use styrene_ipc_client::{
    Client, ClientError, CompatibilityEvent, CompatibilityWatch, ConnectionGeneration,
    DEFAULT_DEADLINE, EventFrame, EventTopic, Negotiation,
};
pub use styrened::operator_profile::{
    MediaCapability, MediaInspector, PortableSelector, StaticMediaInspector,
};
pub use styrened::profile_manager::ProfileRoots;

/// Which lifecycle a session owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionProfile {
    /// A temporary managed profile, removed when the session closes.
    Quick,
    /// A persistent managed profile.
    Local,
    /// A managed profile on encrypted removable media.
    Portable,
    /// An existing daemon endpoint owned elsewhere; nothing is started.
    Connected,
    /// A scripted responder over an in-process stream pair.
    Fixture,
}

impl SessionProfile {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Quick => "quick",
            Self::Local => "local",
            Self::Portable => "portable",
            Self::Connected => "connected",
            Self::Fixture => "fixture",
        }
    }

    /// Whether the session starts and owns a daemon.
    #[must_use]
    pub const fn owns_daemon(self) -> bool {
        matches!(self, Self::Quick | Self::Local | Self::Portable)
    }
}

impl fmt::Display for SessionProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Frontend session generation: assigned once per opened session, distinct
/// from the IPC connection generation and the daemon runtime generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SessionGeneration(pub u64);

static NEXT_SESSION_GENERATION: AtomicU64 = AtomicU64::new(1);
static NEXT_CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(1);

fn next_session_generation() -> SessionGeneration {
    SessionGeneration(NEXT_SESSION_GENERATION.fetch_add(1, Ordering::Relaxed))
}

fn next_connection_generation() -> ConnectionGeneration {
    ConnectionGeneration(NEXT_CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed))
}

/// What a session learned when it opened.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct SessionMetadata {
    pub profile: SessionProfile,
    pub generation: SessionGeneration,
    /// The endpoint the session reached: a socket path for Live and Embedded,
    /// a placeholder for Fixture.
    pub endpoint: PathBuf,
    /// The daemon's connection generation at negotiation.
    pub daemon_generation: u64,
    pub capabilities: Option<ActiveCapabilitiesInfo>,
    pub status: DaemonStatusInfo,
    /// The backend's description of the profile the daemon runs from, when
    /// the daemon manages profiles. Frontends read profile truth here, not
    /// from their own mode names.
    pub profile_info: Option<ProfileInfo>,
}

/// Why a session could not open.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SessionError {
    /// The endpoint could not be opened or negotiated. Recoverable: the
    /// frontend may retry or select another profile explicitly.
    #[error("{profile} session could not reach {}: {source}", endpoint.display())]
    Connect {
        profile: SessionProfile,
        endpoint: PathBuf,
        #[source]
        source: ClientError,
    },
    /// The owned runtime or its profile failed to start or open.
    #[error("managed profile failed: {message}")]
    Runtime { message: String },
    /// The fixture transport could not be created.
    #[error("fixture transport failed: {message}")]
    Fixture { message: String },
    /// The session's private temporary directory could not be created.
    #[error("session temporary resources unavailable: {source}")]
    Resources {
        #[source]
        source: std::io::Error,
    },
}

impl SessionError {
    /// True when retrying or reselecting the profile can succeed without
    /// changing the environment.
    #[must_use]
    pub fn is_recoverable(&self) -> bool {
        matches!(self, Self::Connect { .. })
    }
}

/// Runtime inputs for an Embedded session. The socket path is always private
/// to the session.
#[derive(Clone, Debug, Default)]
pub struct EmbeddedConfig {
    /// Message store path; the daemon default when absent.
    pub db: Option<PathBuf>,
    /// Daemon configuration file; daemon defaults when absent.
    pub config: Option<PathBuf>,
    /// Identity file; ignored when `ephemeral` is set.
    pub identity: Option<PathBuf>,
    /// Use a random in-memory identity with no persistence.
    pub ephemeral: bool,
}

enum Owned {
    None,
    Managed {
        running: Option<Box<styrened::operator_profile::RunningManagedProfile>>,
        _temp: Option<tempfile::TempDir>,
    },
    Fixture {
        responder: Option<tokio::task::JoinHandle<()>>,
    },
}

/// A managed profile to open or create.
pub enum ManagedTarget<'a> {
    /// Create a Quick profile under these roots.
    Quick { roots: ProfileRoots, display_name: &'a str },
    /// Open an existing Local profile root, or create it with `display_name`
    /// when it does not exist yet.
    Local { root: PathBuf, runtime_parent: PathBuf, display_name: Option<&'a str> },
    /// Open an existing Portable profile by selector on any offered mount.
    Portable {
        selector: PortableSelector,
        mounts: Vec<PathBuf>,
        runtime_parent: PathBuf,
        inspector: &'a dyn MediaInspector,
    },
}

/// One open frontend session.
pub struct Session {
    metadata: SessionMetadata,
    negotiation: Negotiation,
    client: Client,
    owned: Owned,
    closed: bool,
}

impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("profile", &self.metadata.profile)
            .field("generation", &self.metadata.generation)
            .field("endpoint", &self.metadata.endpoint)
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl Session {
    /// Open a Connected session to a daemon endpoint owned elsewhere, with
    /// the default negotiation deadline. Never starts a runtime.
    pub async fn connected(endpoint: &Path) -> Result<Self, SessionError> {
        Self::connected_with_deadline(endpoint, DEFAULT_DEADLINE).await
    }

    /// `Session::connected` under its older name.
    pub async fn live(endpoint: &Path) -> Result<Self, SessionError> {
        Self::connected(endpoint).await
    }

    /// `Session::connected_with_deadline` under its older name.
    pub async fn live_with_deadline(
        endpoint: &Path,
        deadline: Duration,
    ) -> Result<Self, SessionError> {
        Self::connected_with_deadline(endpoint, deadline).await
    }

    /// Open a Connected session, bounding negotiation by `deadline`.
    pub async fn connected_with_deadline(
        endpoint: &Path,
        deadline: Duration,
    ) -> Result<Self, SessionError> {
        let (client, negotiation) =
            Client::connect_unix(endpoint, next_connection_generation(), deadline).await.map_err(
                |source| SessionError::Connect {
                    profile: SessionProfile::Connected,
                    endpoint: endpoint.to_path_buf(),
                    source,
                },
            )?;
        let mut session = Self::open(
            SessionProfile::Connected,
            endpoint.to_path_buf(),
            client,
            negotiation,
            Owned::None,
        );
        session.load_profile_info().await;
        Ok(session)
    }

    /// Open a managed profile session: create a Quick profile, open or
    /// create a Local one, or resolve a Portable one, then start its daemon
    /// and connect over the profile's host-private socket. The daemon is
    /// shut down before any error returns.
    pub async fn managed(target: ManagedTarget<'_>) -> Result<Self, SessionError> {
        use styrened::operator_profile::StoppedManagedProfile;
        let runtime = |error: styrened::operator_profile::ProfileError| SessionError::Runtime {
            message: error.to_string(),
        };
        let (profile, stopped) = match target {
            ManagedTarget::Quick { roots, display_name } => (
                SessionProfile::Quick,
                StoppedManagedProfile::create_quick(
                    &roots.profiles_parent,
                    &roots.runtime_parent,
                    display_name,
                )
                .map_err(runtime)?,
            ),
            ManagedTarget::Local { root, runtime_parent, display_name } => {
                let stopped = if root.join("manifest.toml").is_file() {
                    StoppedManagedProfile::open(&root, &runtime_parent).map_err(runtime)?
                } else {
                    let name = display_name.ok_or_else(|| SessionError::Runtime {
                        message: format!(
                            "{} is not a profile and no display name was given",
                            root.display()
                        ),
                    })?;
                    StoppedManagedProfile::create_local(&root, &runtime_parent, name)
                        .map_err(runtime)?
                };
                (SessionProfile::Local, stopped)
            }
            ManagedTarget::Portable { selector, mounts, runtime_parent, inspector } => (
                SessionProfile::Portable,
                StoppedManagedProfile::open_portable(
                    &selector,
                    &mounts,
                    &runtime_parent,
                    inspector,
                )
                .map_err(runtime)?,
            ),
        };
        Self::start_managed(profile, stopped, None).await
    }

    /// Start a Quick profile under a private temporary directory. This is
    /// the older `embedded` shape; `EmbeddedConfig` paths are ignored, and
    /// Quick profiles are always ephemeral.
    pub async fn embedded(config: EmbeddedConfig) -> Result<Self, SessionError> {
        use styrened::operator_profile::StoppedManagedProfile;
        let _ = config;
        // Unix socket paths have a hard length limit, so the private
        // directory keeps a short name.
        let temp = tempfile::Builder::new()
            .prefix("ss-")
            .tempdir()
            .map_err(|source| SessionError::Resources { source })?;
        let profiles = temp.path().join("p");
        let runtime = temp.path().join("r");
        for dir in [&profiles, &runtime] {
            std::fs::create_dir_all(dir).map_err(|source| SessionError::Resources { source })?;
        }
        let stopped = StoppedManagedProfile::create_quick(&profiles, &runtime, "Quick session")
            .map_err(|error| SessionError::Runtime { message: error.to_string() })?;
        Self::start_managed(SessionProfile::Quick, stopped, Some(temp)).await
    }

    async fn start_managed(
        profile: SessionProfile,
        stopped: styrened::operator_profile::StoppedManagedProfile,
        temp: Option<tempfile::TempDir>,
    ) -> Result<Self, SessionError> {
        let running = stopped
            .start()
            .await
            .map_err(|failure| SessionError::Runtime { message: failure.to_string() })?;
        let socket = running.paths().socket.clone();
        match Client::connect_unix(&socket, next_connection_generation(), DEFAULT_DEADLINE).await {
            Ok((client, negotiation)) => {
                let mut session = Self::open(
                    profile,
                    socket,
                    client,
                    negotiation,
                    Owned::Managed { running: Some(Box::new(running)), _temp: temp },
                );
                session.load_profile_info().await;
                Ok(session)
            }
            Err(source) => {
                let _stopped = running.shutdown().await;
                Err(SessionError::Connect { profile, endpoint: socket, source })
            }
        }
    }

    /// Ask the daemon which profile it runs from. Daemons without managed
    /// profiles leave it unset.
    async fn load_profile_info(&mut self) {
        if let Ok(inventory) = self.client.profile_inventory().await
            && let Some(active) = inventory.active_profile_id.as_deref()
        {
            self.metadata.profile_info =
                inventory.profiles.into_iter().find(|profile| profile.id == active);
        }
    }

    /// Open a Fixture session answered by `script`. Returns the session and a
    /// handle for pushing events into it. Opens no daemon, socket file, or
    /// network interface.
    pub async fn fixture(script: FixtureScript) -> Result<(Self, FixtureEvents), SessionError> {
        let (client_end, server_end) = UnixStream::pair()
            .map_err(|error| SessionError::Fixture { message: error.to_string() })?;
        let (events_tx, events_rx) = mpsc::channel(64);
        let responder = tokio::spawn(run_fixture(server_end, script, events_rx));
        let client = Client::from_unix_stream(client_end, next_connection_generation());
        let negotiation = match client.negotiate(DEFAULT_DEADLINE).await {
            Ok(negotiation) => negotiation,
            Err(source) => {
                responder.abort();
                return Err(SessionError::Connect {
                    profile: SessionProfile::Fixture,
                    endpoint: PathBuf::from("fixture"),
                    source,
                });
            }
        };
        let session = Self::open(
            SessionProfile::Fixture,
            PathBuf::from("fixture"),
            client,
            negotiation,
            Owned::Fixture { responder: Some(responder) },
        );
        Ok((session, FixtureEvents { tx: events_tx }))
    }

    fn open(
        profile: SessionProfile,
        endpoint: PathBuf,
        client: Client,
        negotiation: Negotiation,
        owned: Owned,
    ) -> Self {
        let metadata = SessionMetadata {
            profile,
            generation: next_session_generation(),
            endpoint,
            daemon_generation: negotiation.daemon_generation,
            capabilities: negotiation.capabilities.clone(),
            status: negotiation.status.clone(),
            profile_info: None,
        };
        Self { metadata, negotiation, client, owned, closed: false }
    }

    #[must_use]
    pub fn profile(&self) -> SessionProfile {
        self.metadata.profile
    }

    #[must_use]
    pub fn generation(&self) -> SessionGeneration {
        self.metadata.generation
    }

    #[must_use]
    pub fn metadata(&self) -> &SessionMetadata {
        &self.metadata
    }

    /// The typed client for daemon operations.
    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// The daemon connection generation most recently observed by negotiation
    /// or compatibility polling.
    #[must_use]
    pub fn daemon_generation(&self) -> u64 {
        self.client.daemon_generation().unwrap_or(self.metadata.daemon_generation)
    }

    #[must_use]
    pub fn capabilities(&self) -> Option<&ActiveCapabilitiesInfo> {
        self.metadata.capabilities.as_ref()
    }

    /// The backend's description of the active profile, when managed.
    #[must_use]
    pub fn profile_info(&self) -> Option<&ProfileInfo> {
        self.metadata.profile_info.as_ref()
    }

    /// True until the session is closed or its connection drops.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !self.closed && self.client.is_connected()
    }

    /// A receiver for pushed daemon events; subscribe to topics first.
    #[must_use]
    pub fn events(&self) -> broadcast::Receiver<EventFrame> {
        self.client.events()
    }

    pub async fn subscribe(&self, topics: &[EventTopic]) -> Result<(), ClientError> {
        self.client.subscribe_all(topics).await
    }

    /// Poll daemon status every `interval`, reporting generation and
    /// capability changes relative to this session's negotiation.
    #[must_use]
    pub fn watch_compatibility(&self, interval: Duration) -> CompatibilityWatch {
        self.client.watch_compatibility(&self.negotiation, interval, DEFAULT_DEADLINE)
    }

    /// Shut down whatever the session owns and release its temporary
    /// resources. Idempotent: later calls do nothing.
    pub async fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        match std::mem::replace(&mut self.owned, Owned::None) {
            Owned::None => {}
            Owned::Managed { running, _temp } => {
                if let Some(running) = running {
                    let stopped = running.shutdown().await;
                    drop(stopped);
                }
                drop(_temp);
            }
            Owned::Fixture { responder } => {
                if let Some(responder) = responder {
                    responder.abort();
                }
            }
        }
    }
}

// ─── Fixture ─────────────────────────────────────────────────────────────────

/// A scripted answer for one request type.
#[derive(Clone, Debug)]
pub enum FixtureReply {
    /// A successful result payload, spelled as the daemon would spell it.
    Result(HashMap<String, Value>),
    /// A typed daemon error.
    Error(IpcError),
}

/// Why a record could not become a fixture payload.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FixtureError {
    #[error("fixture record cannot be projected: {message}")]
    Projection { message: String },
    #[error("{message_type:?} is not an event message type")]
    NotAnEvent { message_type: MessageType },
    #[error("fixture session is closed")]
    Closed,
}

/// Project a typed record the way the daemon does: through JSON so enums are
/// spelled as strings, then into a MessagePack value.
pub fn project_record<T: Serialize>(record: &T) -> Result<Value, FixtureError> {
    let json = serde_json::to_value(record)
        .map_err(|error| FixtureError::Projection { message: error.to_string() })?;
    rmpv::ext::to_value(json)
        .map_err(|error| FixtureError::Projection { message: error.to_string() })
}

fn payload_map(value: Value) -> Result<HashMap<String, Value>, FixtureError> {
    match value {
        Value::Map(fields) => fields
            .into_iter()
            .map(|(key, value)| {
                key.as_str().map(|key| (key.to_string(), value)).ok_or_else(|| {
                    FixtureError::Projection { message: "payload key is not a string".into() }
                })
            })
            .collect(),
        _ => Err(FixtureError::Projection { message: "record is not a map".into() }),
    }
}

/// The deterministic answers a Fixture session gives. Ping, subscriptions, and
/// the status query used by negotiation are always answered; every other
/// request type answers only when scripted, and otherwise returns a typed
/// not-implemented error.
#[derive(Clone, Debug)]
pub struct FixtureScript {
    status: DaemonStatusInfo,
    replies: HashMap<u8, FixtureReply>,
}

impl FixtureScript {
    /// Script around a status snapshot. A missing or zero connection
    /// generation becomes 1 so negotiation succeeds.
    #[must_use]
    pub fn new(mut status: DaemonStatusInfo) -> Self {
        if status.connection_generation.is_none_or(|generation| generation == 0) {
            status.connection_generation = Some(1);
        }
        Self { status, replies: HashMap::new() }
    }

    /// Answer `message_type` with a raw result payload.
    #[must_use]
    pub fn reply(mut self, message_type: MessageType, payload: HashMap<String, Value>) -> Self {
        self.replies.insert(message_type as u8, FixtureReply::Result(payload));
        self
    }

    /// Answer `message_type` with a typed record spread as the payload.
    pub fn reply_record<T: Serialize>(
        self,
        message_type: MessageType,
        record: &T,
    ) -> Result<Self, FixtureError> {
        let payload = payload_map(project_record(record)?)?;
        Ok(self.reply(message_type, payload))
    }

    /// Answer `message_type` with a typed record under one payload key.
    pub fn reply_record_under<T: Serialize>(
        self,
        message_type: MessageType,
        key: &str,
        record: &T,
    ) -> Result<Self, FixtureError> {
        let payload = HashMap::from([(key.to_string(), project_record(record)?)]);
        Ok(self.reply(message_type, payload))
    }

    /// Answer `message_type` with a typed daemon error.
    #[must_use]
    pub fn fail(mut self, message_type: MessageType, error: IpcError) -> Self {
        self.replies.insert(message_type as u8, FixtureReply::Error(error));
        self
    }

    fn answer(&self, message_type: MessageType) -> (MessageType, HashMap<String, Value>) {
        if message_type == MessageType::Ping {
            return (MessageType::Pong, HashMap::new());
        }
        if matches!(message_type as u8, 0x30..=0x3F) {
            return (MessageType::Result, HashMap::new());
        }
        match self.replies.get(&(message_type as u8)) {
            Some(FixtureReply::Result(payload)) => (MessageType::Result, payload.clone()),
            Some(FixtureReply::Error(error)) => (MessageType::Error, error_payload(error)),
            None if message_type == MessageType::QueryStatus => {
                let payload = project_record(&self.status).ok().and_then(|v| payload_map(v).ok());
                (MessageType::Result, payload.unwrap_or_default())
            }
            None => {
                let error = IpcError::not_implemented(format!("{message_type:?}"));
                (MessageType::Error, error_payload(&error))
            }
        }
    }
}

/// Spell an error the way the IPC server does, including the typed form the
/// client prefers.
fn error_payload(error: &IpcError) -> HashMap<String, Value> {
    let kind = match error {
        IpcError::NotImplemented { .. } => "not_implemented",
        IpcError::Unavailable { .. } => "unavailable",
        IpcError::Timeout { .. } => "timeout",
        IpcError::InvalidRequest { .. } => "invalid_request",
        IpcError::NotFound { .. } => "not_found",
        IpcError::Denied { .. } => "denied",
        _ => "internal",
    };
    let display = error.to_string();
    let mut payload = HashMap::from([
        ("error".to_string(), Value::from(display.as_str())),
        ("message".to_string(), Value::from(display)),
        ("kind".to_string(), Value::from(kind)),
        ("code".to_string(), Value::from(kind)),
    ]);
    if let Ok(value) = rmpv::ext::to_value(error) {
        payload.insert("typed_error".to_string(), value);
    }
    payload
}

/// Pushes events into a Fixture session as the daemon would.
#[derive(Clone, Debug)]
pub struct FixtureEvents {
    tx: mpsc::Sender<(MessageType, HashMap<String, Value>)>,
}

impl FixtureEvents {
    /// Push a raw event payload.
    pub async fn push(
        &self,
        message_type: MessageType,
        payload: HashMap<String, Value>,
    ) -> Result<(), FixtureError> {
        if !message_type.is_event() {
            return Err(FixtureError::NotAnEvent { message_type });
        }
        self.tx.send((message_type, payload)).await.map_err(|_| FixtureError::Closed)
    }

    /// Push a typed record spread as the event payload.
    pub async fn push_record<T: Serialize>(
        &self,
        message_type: MessageType,
        record: &T,
    ) -> Result<(), FixtureError> {
        self.push(message_type, payload_map(project_record(record)?)?).await
    }
}

type Outbound = (MessageType, [u8; REQUEST_ID_SIZE], HashMap<String, Value>);

async fn run_fixture(
    stream: UnixStream,
    script: FixtureScript,
    mut events: mpsc::Receiver<(MessageType, HashMap<String, Value>)>,
) {
    let (mut reader, mut writer) = stream.into_split();
    let (out_tx, mut out_rx) = mpsc::channel::<Outbound>(64);
    let writer_task = tokio::spawn(async move {
        while let Some((message_type, request_id, payload)) = out_rx.recv().await {
            if wire::write_frame_async(&mut writer, message_type, &request_id, &payload)
                .await
                .is_err()
            {
                break;
            }
        }
    });
    let event_out = out_tx.clone();
    let events_task = tokio::spawn(async move {
        while let Some((message_type, payload)) = events.recv().await {
            if event_out.send((message_type, [0; REQUEST_ID_SIZE], payload)).await.is_err() {
                break;
            }
        }
    });
    while let Ok(frame) = wire::read_frame_async(&mut reader).await {
        let (message_type, payload) = script.answer(frame.msg_type);
        if out_tx.send((message_type, frame.request_id, payload)).await.is_err() {
            break;
        }
    }
    events_task.abort();
    drop(out_tx);
    let _ = writer_task.await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_ipc::types::{ACTIVE_CAPABILITIES_VERSION, DeviceInfo};

    fn status() -> DaemonStatusInfo {
        let mut status = DaemonStatusInfo::default();
        status.daemon_version = "fixture".into();
        status.rns_initialized = true;
        status.connection_generation = Some(9);
        let mut capabilities = ActiveCapabilitiesInfo::default();
        capabilities.version = ACTIVE_CAPABILITIES_VERSION;
        capabilities.generation = Some(9);
        capabilities.runtime = vec!["runtime.lxmf.direct".into()];
        capabilities.authorized_operations = vec!["chat.send".into()];
        status.active_capabilities = Some(capabilities);
        status
    }

    #[tokio::test]
    async fn live_session_fails_recoverably_without_starting_anything() {
        let missing = std::env::temp_dir()
            .join(format!("styrene-session-missing-{}.sock", std::process::id()));
        let error = match Session::live(&missing).await {
            Ok(_) => panic!("a missing endpoint must not open"),
            Err(error) => error,
        };
        assert!(error.is_recoverable());
        assert!(matches!(
            error,
            SessionError::Connect { profile: SessionProfile::Connected, ref endpoint, .. } if *endpoint == missing
        ));
        assert!(!missing.exists(), "live sessions never create endpoints");
    }

    #[tokio::test]
    async fn fixture_session_serves_scripted_records_errors_and_events() {
        let mut device = DeviceInfo::default();
        device.destination_hash = "peer".into();
        device.name = "Peer".into();
        let script = FixtureScript::new(status())
            .reply_record_under(MessageType::QueryDevices, "devices", &vec![device.clone()])
            .expect("devices record")
            .fail(MessageType::QueryLinks, IpcError::not_implemented("links"));
        let (mut session, events) = Session::fixture(script).await.expect("fixture opens");

        assert_eq!(session.profile(), SessionProfile::Fixture);
        assert_eq!(session.metadata().daemon_generation, 9);
        assert_eq!(session.daemon_generation(), 9);
        assert_eq!(session.metadata().status.daemon_version, "fixture");
        assert_eq!(
            session.capabilities().map(|c| c.authorized_operations.clone()),
            Some(vec!["chat.send".to_string()])
        );
        assert!(session.is_open());

        assert_eq!(
            session.client().devices(false).await.expect("scripted devices"),
            vec![device.clone()]
        );
        assert!(matches!(
            session.client().links().await,
            Err(ClientError::Remote(IpcError::NotImplemented { ref method })) if method == "links"
        ));
        assert!(matches!(
            session.client().path_table().await,
            Err(ClientError::Remote(IpcError::NotImplemented { .. }))
        ));

        let mut pushed = session.events();
        session.subscribe(&[EventTopic::Devices]).await.expect("subscribe");
        events.push_record(MessageType::EventDevice, &device).await.expect("push event");
        let event = pushed.recv().await.expect("event arrives");
        assert_eq!(event.message_type, MessageType::EventDevice);
        assert_eq!(event.text("destination_hash"), "peer");
        assert!(matches!(
            events.push(MessageType::QueryStatus, HashMap::new()).await,
            Err(FixtureError::NotAnEvent { .. })
        ));

        session.close().await;
        session.close().await;
        assert!(!session.is_open() || session.client().ping().await.is_err());
    }

    #[tokio::test]
    async fn session_generations_increase_per_open() {
        let (mut first, _) = Session::fixture(FixtureScript::new(status())).await.expect("first");
        let (mut second, _) = Session::fixture(FixtureScript::new(status())).await.expect("second");
        assert!(second.generation() > first.generation());
        assert_ne!(first.client().generation(), second.client().generation());
        first.close().await;
        second.close().await;
    }

    #[tokio::test]
    async fn fixture_negotiation_rejects_a_foreign_capability_version() {
        let mut status = status();
        if let Some(capabilities) = status.active_capabilities.as_mut() {
            capabilities.version = ACTIVE_CAPABILITIES_VERSION + 1;
        }
        let error = match Session::fixture(FixtureScript::new(status)).await {
            Ok(_) => panic!("incompatible fixture must not open"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            SessionError::Connect { source: ClientError::Incompatible { .. }, .. }
        ));
    }
}
