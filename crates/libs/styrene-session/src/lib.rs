//! Frontend session profiles over the shared Styrene IPC client.
//!
//! A session owns one negotiated connection to a daemon contract and the
//! lifecycle of whatever it started to reach it. The three profiles never
//! fall back to one another:
//!
//! - **Live** connects to an existing daemon endpoint. A failure is a typed,
//!   recoverable connection error; no runtime is started.
//! - **Embedded** starts a `styrened` runtime in this process and reaches it
//!   over a private socket, so every operation takes the same typed client
//!   path as Live. Closing the session shuts the runtime down and releases the
//!   private socket directory.
//! - **Fixture** answers the client from a deterministic script over an
//!   in-process stream pair. It opens no daemon, socket file, or network
//!   interface, and exposes exactly the operations the script supports.
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
use styrene_ipc::types::{ActiveCapabilitiesInfo, DaemonStatusInfo};
use styrene_ipc_wire::{self as wire, MessageType, REQUEST_ID_SIZE};
use thiserror::Error;
use tokio::net::UnixStream;
use tokio::sync::{broadcast, mpsc};

pub use styrene_ipc_client::{
    Client, ClientError, CompatibilityEvent, CompatibilityWatch, ConnectionGeneration,
    DEFAULT_DEADLINE, EventFrame, EventTopic, Negotiation,
};

/// Which lifecycle a session owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SessionProfile {
    /// An existing daemon endpoint; nothing is started or owned.
    Live,
    /// A runtime started in this process and reached over a private socket.
    Embedded,
    /// A scripted responder over an in-process stream pair.
    Fixture,
}

impl SessionProfile {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Embedded => "embedded",
            Self::Fixture => "fixture",
        }
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
    /// The embedded runtime failed to start.
    #[error("embedded runtime failed to start: {message}")]
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
    Embedded { daemon: Option<Box<styrened::daemon::DaemonHandle>>, _temp: tempfile::TempDir },
    Fixture { responder: Option<tokio::task::JoinHandle<()>> },
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
    /// Open a Live session to an existing daemon endpoint with the default
    /// negotiation deadline. Never starts a runtime.
    pub async fn live(endpoint: &Path) -> Result<Self, SessionError> {
        Self::live_with_deadline(endpoint, DEFAULT_DEADLINE).await
    }

    /// Open a Live session, bounding negotiation by `deadline`.
    pub async fn live_with_deadline(
        endpoint: &Path,
        deadline: Duration,
    ) -> Result<Self, SessionError> {
        let (client, negotiation) =
            Client::connect_unix(endpoint, next_connection_generation(), deadline).await.map_err(
                |source| SessionError::Connect {
                    profile: SessionProfile::Live,
                    endpoint: endpoint.to_path_buf(),
                    source,
                },
            )?;
        Ok(Self::open(
            SessionProfile::Live,
            endpoint.to_path_buf(),
            client,
            negotiation,
            Owned::None,
        ))
    }

    /// Start a `styrened` runtime in this process and open a session to it
    /// over a private socket. A runtime that starts but cannot be negotiated
    /// is shut down before the error returns.
    pub async fn embedded(config: EmbeddedConfig) -> Result<Self, SessionError> {
        let temp = tempfile::Builder::new()
            .prefix("styrene-session-")
            .tempdir()
            .map_err(|source| SessionError::Resources { source })?;
        let socket = temp.path().join("daemon.sock");
        let daemon = styrened::daemon::start(styrened::daemon::DaemonConfig2 {
            db: config.db,
            config: config.config,
            identity: config.identity,
            socket: Some(socket.clone()),
            ephemeral: config.ephemeral,
        })
        .await
        .map_err(|error| SessionError::Runtime { message: error.to_string() })?;
        match Client::connect_unix(&socket, next_connection_generation(), DEFAULT_DEADLINE).await {
            Ok((client, negotiation)) => Ok(Self::open(
                SessionProfile::Embedded,
                socket,
                client,
                negotiation,
                Owned::Embedded { daemon: Some(Box::new(daemon)), _temp: temp },
            )),
            Err(source) => {
                daemon.shutdown().await;
                Err(SessionError::Connect {
                    profile: SessionProfile::Embedded,
                    endpoint: socket,
                    source,
                })
            }
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
            Owned::Embedded { daemon, _temp } => {
                if let Some(daemon) = daemon {
                    daemon.shutdown().await;
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
            SessionError::Connect { profile: SessionProfile::Live, ref endpoint, .. } if *endpoint == missing
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
