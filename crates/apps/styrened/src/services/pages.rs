//! PageService — NomadNet-compatible page serving over RNS links.
//!
//! Serves `.mu` (Micron markup) files from a configurable pages directory.
//! Registers the `("nomadnetwork", "node")` destination aspect so that
//! NomadNet, MeshChat, and Sideband can browse pages hosted by this node.
//!
//! ## Page Directory Structure
//!
//! ```text
//! ~/.config/styrene/pages/
//!   index.mu          ← default landing page
//!   status.mu         ← node status (could be dynamic)
//!   about.mu          ← static about page
//! ```
//!
//! ## Request Protocol
//!
//! Pages are served via RNS link requests. A client establishes a link to
//! the `("nomadnetwork", "node")` destination and sends a request with
//! path `/page/<filename>`. The server responds with the file content.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::{Duration, Instant};

use rns_core::hash::AddressHash;

/// Default pages directory.
pub fn default_pages_dir() -> PathBuf {
    crate::config::default_config_dir().join("pages")
}

/// A served page entry.
#[derive(Debug, Clone)]
pub struct PageEntry {
    /// Request path (e.g., "/page/index.mu").
    pub request_path: String,
    /// Filesystem path to the .mu file.
    pub file_path: PathBuf,
    /// Whether this is a dynamic (executable) page.
    pub dynamic: bool,
}

/// A static page or file exposed through the native RNS request protocol.
#[derive(Debug, Clone)]
pub struct NativePageEntry {
    pub request_path: String,
    content: NativePageContent,
    /// `Some` means the adjacent `.allowed` file exists. An empty set denies all callers.
    pub allowed_identities: Option<BTreeSet<AddressHash>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativePageInventoryEntry {
    pub request_path: String,
    pub dynamic: bool,
    pub restricted: bool,
}

impl NativePageEntry {
    pub fn inventory(&self) -> NativePageInventoryEntry {
        let dynamic = match &self.content {
            NativePageContent::Static(_) => false,
            #[cfg(unix)]
            NativePageContent::Executable(_) => true,
        };
        NativePageInventoryEntry {
            request_path: self.request_path.clone(),
            dynamic,
            restricted: self.allowed_identities.is_some(),
        }
    }
}

#[derive(Debug, Clone)]
enum NativePageContent {
    Static(Arc<[u8]>),
    #[cfg(unix)]
    Executable(Arc<ExecutablePage>),
}

#[cfg(unix)]
struct ExecutablePage {
    bytes: Arc<[u8]>,
}

#[cfg(unix)]
impl std::fmt::Debug for ExecutablePage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExecutablePage(..)")
    }
}

#[cfg(unix)]
#[derive(Debug, Clone, Copy)]
struct ExecutionLimits {
    timeout: Duration,
    max_output: usize,
}

#[cfg(unix)]
impl Default for ExecutionLimits {
    fn default() -> Self {
        Self { timeout: Duration::from_secs(2), max_output: 1024 * 1024 }
    }
}

/// Service managing NomadNet-compatible page hosting.
pub struct PageService {
    pages_dir: PathBuf,
    files_dir: PathBuf,
    pages: Mutex<HashMap<String, PageEntry>>,
    node_name: Mutex<String>,
    active_native_paths: Mutex<BTreeSet<String>>,
    #[cfg(unix)]
    execution_limits: ExecutionLimits,
}

impl PageService {
    pub fn new(pages_dir: PathBuf) -> Self {
        let files_dir = pages_dir.parent().unwrap_or(&pages_dir).join("files");
        Self::with_storage_dirs(pages_dir, files_dir)
    }

    pub fn with_storage_dirs(pages_dir: PathBuf, files_dir: PathBuf) -> Self {
        let svc = Self {
            pages_dir,
            files_dir,
            pages: Mutex::new(HashMap::new()),
            node_name: Mutex::new("Styrene Node".to_string()),
            active_native_paths: Mutex::new(BTreeSet::new()),
            #[cfg(unix)]
            execution_limits: ExecutionLimits::default(),
        };
        svc.scan_pages();
        svc
    }

    #[cfg(all(test, unix))]
    pub(crate) fn with_execution_limits(
        pages_dir: PathBuf,
        files_dir: PathBuf,
        timeout: Duration,
        max_output: usize,
    ) -> Self {
        let mut service = Self::with_storage_dirs(pages_dir, files_dir);
        service.execution_limits = ExecutionLimits { timeout, max_output };
        service
    }

    pub fn with_default_dir() -> Self {
        Self::new(default_pages_dir())
    }

    pub fn set_node_name(&self, name: &str) {
        *self.node_name.lock().unwrap() = name.to_string();
    }

    /// Scan the pages directory and register all .mu files.
    pub fn scan_pages(&self) {
        let mut pages = self.pages.lock().unwrap();
        pages.clear();

        if !self.pages_dir.exists() {
            return;
        }

        self.scan_dir(&self.pages_dir, &self.pages_dir, &mut pages);

        crate::daemon_diagnostic!(
            "[pages] scanned {} pages from {}",
            pages.len(),
            self.pages_dir.display()
        );
    }

    fn scan_dir(&self, dir: &Path, root: &Path, pages: &mut HashMap<String, PageEntry>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                self.scan_dir(&path, root, pages);
            } else if path.extension().is_some_and(|ext| ext == "mu") {
                let relative =
                    path.strip_prefix(root).unwrap_or(&path).to_string_lossy().to_string();
                let request_path = format!("/page/{relative}");

                #[cfg(unix)]
                let dynamic = {
                    use std::os::unix::fs::PermissionsExt;
                    path.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
                };
                #[cfg(not(unix))]
                let dynamic = false;

                pages.insert(
                    request_path.clone(),
                    PageEntry { request_path, file_path: path, dynamic },
                );
            }
        }
    }

    /// List all registered pages.
    pub fn list_pages(&self) -> Vec<PageEntry> {
        self.pages.lock().unwrap().values().cloned().collect()
    }

    /// Enumerate concrete paths for registration on `nomadnetwork.node`.
    pub fn native_entries(&self) -> Vec<NativePageEntry> {
        let mut entries = Vec::new();
        if let Ok(root) = secure_open_dir(&self.pages_dir) {
            self.scan_native_dir(
                &self.pages_dir,
                &self.pages_dir,
                &root,
                "/page",
                true,
                &mut entries,
            );
        }
        if let Ok(root) = secure_open_dir(&self.files_dir) {
            self.scan_native_dir(
                &self.files_dir,
                &self.files_dir,
                &root,
                "/file",
                false,
                &mut entries,
            );
        }

        if !entries.iter().any(|entry| entry.request_path == "/page/index.mu") {
            entries.push(NativePageEntry {
                request_path: "/page/index.mu".into(),
                content: NativePageContent::Static(self.serve_default_index().into()),
                allowed_identities: None,
            });
        }
        entries.sort_by(|left, right| left.request_path.cmp(&right.request_path));
        entries
    }

    pub fn native_inventory(&self) -> Vec<(NativePageInventoryEntry, bool)> {
        let active = self.active_native_paths.lock().unwrap_or_else(|value| value.into_inner());
        self.native_entries()
            .into_iter()
            .map(|entry| {
                let inventory = entry.inventory();
                let handler_active = active.contains(&inventory.request_path);
                (inventory, handler_active)
            })
            .collect()
    }

    pub fn clear_active_native_paths(&self) {
        self.active_native_paths.lock().unwrap_or_else(|value| value.into_inner()).clear();
    }

    pub fn mark_native_path_active(&self, request_path: String) {
        self.active_native_paths
            .lock()
            .unwrap_or_else(|value| value.into_inner())
            .insert(request_path);
    }

    fn scan_native_dir(
        &self,
        dir: &Path,
        root: &Path,
        root_handle: &std::fs::File,
        prefix: &str,
        pages_only: bool,
        entries: &mut Vec<NativePageEntry>,
    ) {
        let Ok(directory) = std::fs::read_dir(dir) else { return };
        for entry in directory.flatten() {
            if entry.file_type().is_ok_and(|file_type| file_type.is_symlink()) {
                continue;
            }
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else { continue };
            if name.starts_with('.') || name.ends_with(".allowed") {
                continue;
            }
            if path.is_dir() {
                self.scan_native_dir(&path, root, root_handle, prefix, pages_only, entries);
                continue;
            }
            if pages_only && path.extension().is_none_or(|extension| extension != "mu") {
                continue;
            }
            let Ok(relative) = path.strip_prefix(root) else { continue };
            let Ok(content) = secure_read(root_handle, relative) else { continue };
            let relative = relative.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
            let allowed_relative = Path::new(&relative).with_file_name(format!("{name}.allowed"));
            #[cfg(unix)]
            let native_content = if pages_only && content.executable {
                NativePageContent::Executable(Arc::new(snapshot_executable(content.bytes)))
            } else {
                NativePageContent::Static(content.bytes.into())
            };
            #[cfg(not(unix))]
            let native_content = NativePageContent::Static(content.bytes.into());
            entries.push(NativePageEntry {
                request_path: format!("{prefix}/{relative}"),
                allowed_identities: read_allowed_identities(root_handle, &allowed_relative),
                content: native_content,
            });
        }
    }

    /// Serve native content using the request data and authenticated link context.
    pub fn serve_native(
        &self,
        entry: &NativePageEntry,
        request_data: &[u8],
        remote_identity: Option<&rns_core::identity::Identity>,
        link_id: AddressHash,
    ) -> Option<Vec<u8>> {
        match &entry.content {
            NativePageContent::Static(content) => Some(content.to_vec()),
            #[cfg(unix)]
            NativePageContent::Executable(executable) => {
                match self.execute_native(executable, request_data, remote_identity, link_id) {
                    Ok(content) => Some(content),
                    Err(error) => {
                        crate::daemon_diagnostic!("[pages] dynamic page execution failed: {error}");
                        None
                    }
                }
            }
        }
    }

    #[cfg(unix)]
    fn execute_native(
        &self,
        executable: &ExecutablePage,
        request_data: &[u8],
        remote_identity: Option<&rns_core::identity::Identity>,
        link_id: AddressHash,
    ) -> io::Result<Vec<u8>> {
        use std::sync::mpsc;

        let fields = decode_request_environment(request_data)?;
        let working_directory = tempfile::Builder::new().prefix("styrene-page-work-").tempdir()?;
        let invocation = materialize_executable(executable)?;
        let mut command = invocation_command(executable, &invocation.path)?;
        command
            .env_clear()
            .env("link_id", hex::encode(link_id.as_slice()))
            .current_dir(working_directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        if let Some(path) = std::env::var_os("PATH") {
            command.env("PATH", path);
        }
        if let Some(identity) = remote_identity {
            command.env("remote_identity", hex::encode(identity.address_hash.as_slice()));
        }
        for (name, value) in &fields {
            command.env(name, value);
        }

        let mut child = command.spawn()?;
        let process_group = rustix::process::Pid::from_child(&child);
        let mut owned_processes = OwnedProcesses::new(child.id());
        let Some(mut stdout) = child.stdout.take() else {
            let mut exit_status = None;
            let _ = cleanup_process_tree(
                process_group,
                &mut child,
                &mut exit_status,
                &mut owned_processes,
            );
            return Err(io::Error::other("missing dynamic page output"));
        };
        let max_output = self.execution_limits.max_output;
        let (sender, receiver) = mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || {
            let mut output = Vec::new();
            let result = Read::by_ref(&mut stdout)
                .take(u64::try_from(max_output).unwrap_or(u64::MAX).saturating_add(1))
                .read_to_end(&mut output)
                .map(|_| output);
            let _ = sender.send(result);
        });

        let deadline = Instant::now() + self.execution_limits.timeout;
        let mut captured_output = None;
        let mut output_complete = false;
        let mut exit_status = None;
        let mut cleanup = None;
        let mut next_process_refresh = Instant::now() + PROCESS_REFRESH_INTERVAL;
        let result = loop {
            match receiver.try_recv() {
                Ok(Ok(output)) if output.len() > max_output => {
                    break Err(io::Error::other("dynamic page output limit exceeded"));
                }
                Ok(Err(error)) => break Err(error),
                Ok(Ok(output)) => {
                    captured_output = Some(output);
                    output_complete = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !output_complete {
                        break Err(io::Error::other("dynamic page output reader failed"));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
            if exit_status.is_none() {
                match child.try_wait() {
                    Ok(status) => exit_status = status,
                    Err(error) => break Err(error),
                }
            }
            if exit_status.as_ref().is_some_and(|status| !status.success()) {
                break Err(io::Error::other("dynamic page failed"));
            }
            if exit_status.is_some() && cleanup.is_none() {
                cleanup = Some(cleanup_process_tree(
                    process_group,
                    &mut child,
                    &mut exit_status,
                    &mut owned_processes,
                ));
                if let Some(Err(error)) = &cleanup {
                    break Err(io::Error::other(format!(
                        "dynamic page process cleanup failed: {error}"
                    )));
                }
            }
            if exit_status.is_some() && output_complete {
                break Ok(captured_output.take().unwrap_or_default());
            }
            if Instant::now() >= next_process_refresh {
                owned_processes.refresh();
                next_process_refresh = Instant::now() + PROCESS_REFRESH_INTERVAL;
            }
            if Instant::now() >= deadline {
                let pending = match (exit_status.is_none(), output_complete) {
                    (true, false) => "child exit and output",
                    (true, true) => "child exit",
                    (false, false) => "output",
                    (false, true) => "completion",
                };
                break Err(io::Error::other(format!(
                    "dynamic page timed out waiting for {pending}"
                )));
            }
            std::thread::sleep(PROCESS_POLL_INTERVAL);
        };

        let cleanup = cleanup.unwrap_or_else(|| {
            cleanup_process_tree(process_group, &mut child, &mut exit_status, &mut owned_processes)
        });
        let reader_deadline = Instant::now() + PIPE_DRAIN_TIMEOUT;
        while !reader.is_finished() && Instant::now() < reader_deadline {
            std::thread::sleep(PROCESS_POLL_INTERVAL);
        }
        if !reader.is_finished() {
            return Err(io::Error::other("dynamic page output did not close after cleanup"));
        }
        reader.join().map_err(|_| io::Error::other("dynamic page output reader panicked"))?;
        match (result, cleanup) {
            (Ok(content), Ok(())) => Ok(content),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup_error)) => Err(cleanup_error),
            (Err(error), Err(cleanup_error)) => {
                Err(io::Error::other(format!("{error}; process cleanup failed: {cleanup_error}")))
            }
        }
    }

    /// Serve a page request. Returns the page content bytes, or None if not found.
    pub fn serve_page(&self, request_path: &str) -> Option<Vec<u8>> {
        let pages = self.pages.lock().unwrap();
        let entry = pages.get(request_path)?;

        if entry.dynamic {
            None
        } else {
            self.serve_static(&entry.file_path)
        }
    }

    fn serve_static(&self, path: &Path) -> Option<Vec<u8>> {
        std::fs::read(path).ok()
    }

    /// Serve the default index page if no index.mu exists.
    pub fn serve_default_index(&self) -> Vec<u8> {
        let name = self.node_name.lock().unwrap().clone();
        let page_list: Vec<String> =
            self.pages.lock().unwrap().keys().map(|p| format!("`F444`[{p}]`{p}`f")).collect();

        let pages_section = if page_list.is_empty() {
            "No pages available.".to_string()
        } else {
            page_list.join("\n")
        };

        format!(
            ">Welcome to {name}\n\n\
             This node is running Styrene.\n\n\
             >Pages\n\n\
             {pages_section}\n"
        )
        .into_bytes()
    }

    /// Handle a page request by path, with fallback to default index.
    pub fn handle_request(&self, path: &str) -> Vec<u8> {
        // Normalize path
        let path = if path.is_empty() || path == "/" || path == "/page/" {
            "/page/index.mu"
        } else {
            path
        };

        if let Some(content) = self.serve_page(path) {
            content
        } else if path == "/page/index.mu" {
            self.serve_default_index()
        } else {
            format!("`F900`Page not found: {path}`f\n").into_bytes()
        }
    }

    /// Number of registered pages.
    pub fn page_count(&self) -> usize {
        self.pages.lock().unwrap().len()
    }

    /// Pages directory path.
    pub fn pages_dir(&self) -> &Path {
        &self.pages_dir
    }

    pub fn files_dir(&self) -> &Path {
        &self.files_dir
    }
}

#[cfg(unix)]
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(5);
#[cfg(unix)]
const PROCESS_REFRESH_INTERVAL: Duration = Duration::from_millis(50);
#[cfg(unix)]
const TERMINATION_GRACE: Duration = Duration::from_millis(150);
#[cfg(unix)]
const PIPE_DRAIN_TIMEOUT: Duration = Duration::from_millis(150);
#[cfg(unix)]
const PROCESS_SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(unix)]
const MAX_PROCESS_SNAPSHOT_SIZE: usize = 1024 * 1024;

#[cfg(unix)]
struct OwnedProcesses {
    root: u32,
    descendants: BTreeMap<u32, String>,
    group_present: bool,
    discovery_failed: bool,
}

#[cfg(unix)]
impl OwnedProcesses {
    fn new(root: u32) -> Self {
        Self { root, descendants: BTreeMap::new(), group_present: true, discovery_failed: false }
    }

    fn refresh(&mut self) {
        let Ok(processes) = process_snapshot() else {
            self.discovery_failed = true;
            return;
        };
        let mut owned = self.descendants.keys().copied().collect::<BTreeSet<_>>();
        owned.insert(self.root);
        self.group_present = processes.iter().any(|process| process.group == self.root);
        loop {
            let before = owned.len();
            for process in &processes {
                if process.group == self.root || owned.contains(&process.parent) {
                    owned.insert(process.pid);
                }
            }
            if owned.len() == before {
                break;
            }
        }
        for pid in owned {
            if pid == self.root {
                continue;
            }
            if let Some(process) = processes.iter().find(|process| process.pid == pid) {
                match self.descendants.get(&pid) {
                    Some(start) if start != &process.start => self.discovery_failed = true,
                    None => {
                        self.descendants.insert(pid, process.start.clone());
                    }
                    Some(_) => {}
                }
            }
        }
    }

    fn signal_descendants(&self, signal: rustix::process::Signal) -> io::Result<()> {
        let processes = process_snapshot()?;
        for (raw_pid, start) in &self.descendants {
            if !processes.iter().any(|process| process.pid == *raw_pid && process.start == *start) {
                continue;
            }
            let Some(pid) =
                rustix::process::Pid::from_raw(i32::try_from(*raw_pid).unwrap_or(i32::MAX))
            else {
                continue;
            };
            match rustix::process::kill_process(pid, signal) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn remaining(&self) -> io::Result<Vec<u32>> {
        let processes = process_snapshot()?;
        Ok(self
            .descendants
            .iter()
            .filter_map(|(pid, start)| {
                processes
                    .iter()
                    .any(|process| process.pid == *pid && process.start == *start)
                    .then_some(*pid)
            })
            .collect())
    }
}

#[cfg(unix)]
struct ProcessRecord {
    pid: u32,
    parent: u32,
    group: u32,
    start: String,
}

#[cfg(unix)]
struct InvocationExecutable {
    _root: tempfile::TempDir,
    path: PathBuf,
}

#[cfg(unix)]
fn snapshot_executable(bytes: Vec<u8>) -> ExecutablePage {
    ExecutablePage { bytes: bytes.into() }
}

#[cfg(unix)]
fn materialize_executable(executable: &ExecutablePage) -> io::Result<InvocationExecutable> {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::Builder::new().prefix("styrene-page-run-").tempdir()?;
    let path = root.path().join("page");
    let mut destination = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    destination.write_all(&executable.bytes)?;
    destination.sync_all()?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o500))?;
    Ok(InvocationExecutable { _root: root, path })
}

#[cfg(unix)]
fn invocation_command(executable: &ExecutablePage, path: &Path) -> io::Result<Command> {
    let Some(shebang) = executable.bytes.strip_prefix(b"#!") else {
        return Ok(Command::new(path));
    };
    let line = shebang.split(|byte| *byte == b'\n').next().unwrap_or(shebang);
    let line = std::str::from_utf8(line)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid dynamic page shebang"))?
        .trim_end_matches('\r')
        .trim();
    let mut fields = line.split_whitespace();
    let interpreter = fields.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "dynamic page shebang has no interpreter")
    })?;
    if !Path::new(interpreter).is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dynamic page shebang interpreter is not absolute",
        ));
    }
    let mut command = Command::new(interpreter);
    let argument = fields.collect::<Vec<_>>().join(" ");
    if !argument.is_empty() {
        command.arg(argument);
    }
    command.arg(path);
    Ok(command)
}

#[cfg(unix)]
fn decode_request_environment(data: &[u8]) -> io::Result<Vec<(String, String)>> {
    if data.len() > 64 * 1024 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "dynamic page request too large"));
    }
    let mut cursor = io::Cursor::new(data);
    let value = rmpv::decode::read_value(&mut cursor)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid dynamic page request"))?;
    if usize::try_from(cursor.position()).ok() != Some(data.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "trailing dynamic page request data",
        ));
    }
    let rmpv::Value::Map(entries) = value else {
        if value.is_nil() {
            return Ok(Vec::new());
        }
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "dynamic page request is not a map",
        ));
    };
    let mut environment = Vec::with_capacity(entries.len().min(64));
    if entries.len() > 64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "too many dynamic page fields"));
    }
    let mut names = BTreeSet::new();
    for (name, value) in entries {
        let Some(name) = name.as_str() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "dynamic page field name is not text",
            ));
        };
        if !(name.starts_with("field_") || name.starts_with("var_")) {
            continue;
        }
        if name.len() > 128 || name.as_bytes().contains(&b'=') || name.as_bytes().contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid dynamic page field name",
            ));
        }
        if !names.insert(name.to_owned()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "duplicate dynamic page field name",
            ));
        }
        let Some(value) = value.as_str() else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "dynamic page field value is not text",
            ));
        };
        if value.len() > 16 * 1024 || value.as_bytes().contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid dynamic page field value",
            ));
        }
        environment.push((name.to_owned(), value.to_owned()));
    }
    Ok(environment)
}

#[cfg(unix)]
fn process_snapshot() -> io::Result<Vec<ProcessRecord>> {
    let mut child = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,pgid=,lstart="])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let mut stdout = child.stdout.take().ok_or_else(|| io::Error::other("missing ps output"))?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        Read::by_ref(&mut stdout)
            .take(u64::try_from(MAX_PROCESS_SNAPSHOT_SIZE).unwrap_or(u64::MAX).saturating_add(1))
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + PROCESS_SNAPSHOT_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait()?;
            let _ = reader.join();
            return Err(io::Error::new(io::ErrorKind::TimedOut, "process snapshot timed out"));
        }
        std::thread::sleep(PROCESS_POLL_INTERVAL);
    };
    let bytes =
        reader.join().map_err(|_| io::Error::other("process snapshot reader panicked"))??;
    if !status.success() || bytes.len() > MAX_PROCESS_SNAPSHOT_SIZE {
        return Err(io::Error::other("process snapshot failed"));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid process snapshot"))?;
    Ok(text
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let pid = fields.next()?.parse().ok()?;
            let parent = fields.next()?.parse().ok()?;
            let group = fields.next()?.parse().ok()?;
            let start = fields.collect::<Vec<_>>().join(" ");
            if start.is_empty() {
                return None;
            }
            Some(ProcessRecord { pid, parent, group, start })
        })
        .collect())
}

#[cfg(unix)]
fn signal_process_group(
    process_group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> io::Result<()> {
    match rustix::process::kill_process_group(process_group, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn signal_process(pid: rustix::process::Pid, signal: rustix::process::Signal) -> io::Result<()> {
    match rustix::process::kill_process(pid, signal) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(unix)]
fn cleanup_process_tree(
    process_group: rustix::process::Pid,
    child: &mut std::process::Child,
    exit_status: &mut Option<ExitStatus>,
    owned: &mut OwnedProcesses,
) -> io::Result<()> {
    owned.refresh();
    if exit_status.is_none() {
        *exit_status = child.try_wait()?;
    }
    if exit_status.is_some() && !owned.group_present && owned.remaining()?.is_empty() {
        if owned.discovery_failed {
            return Err(io::Error::other("dynamic page descendant discovery failed"));
        }
        return exit_status
            .is_some()
            .then_some(())
            .ok_or_else(|| io::Error::other("dynamic page root was not reaped"));
    }

    if exit_status.is_none() {
        signal_process(process_group, rustix::process::Signal::TERM)?;
    }
    if owned.group_present {
        signal_process_group(process_group, rustix::process::Signal::TERM)?;
    }
    owned.signal_descendants(rustix::process::Signal::TERM)?;
    if wait_for_process_tree(child, exit_status, owned, TERMINATION_GRACE)? {
        return Ok(());
    }

    if exit_status.is_none() {
        signal_process(process_group, rustix::process::Signal::KILL)?;
    }
    if owned.group_present {
        signal_process_group(process_group, rustix::process::Signal::KILL)?;
    }
    owned.signal_descendants(rustix::process::Signal::KILL)?;
    if wait_for_process_tree(child, exit_status, owned, TERMINATION_GRACE)? {
        return Ok(());
    }
    Err(io::Error::other("dynamic page process cleanup could not be confirmed"))
}

#[cfg(unix)]
fn wait_for_process_tree(
    child: &mut std::process::Child,
    exit_status: &mut Option<ExitStatus>,
    owned: &mut OwnedProcesses,
    timeout: Duration,
) -> io::Result<bool> {
    let deadline = Instant::now() + timeout;
    loop {
        if exit_status.is_none() {
            *exit_status = child.try_wait()?;
        }
        owned.refresh();
        let group_gone = !owned.group_present;
        let descendants_gone = owned.remaining()?.is_empty();
        if group_gone && descendants_gone && exit_status.is_some() {
            if owned.discovery_failed {
                return Err(io::Error::other("dynamic page descendant discovery failed"));
            }
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(PROCESS_POLL_INTERVAL);
    }
}

const MAX_NATIVE_FILE_SIZE: u64 = 32 * 1024 * 1024;

struct SecureContent {
    bytes: Vec<u8>,
    executable: bool,
}

fn secure_read(root: &std::fs::File, relative: &Path) -> io::Result<SecureContent> {
    let mut file = secure_open_file(root, relative)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_NATIVE_FILE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "native file is not bounded regular content",
        ));
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(0));
    Read::by_ref(&mut file).take(MAX_NATIVE_FILE_SIZE + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_NATIVE_FILE_SIZE {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "native file exceeds size limit"));
    }
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    Ok(SecureContent { bytes, executable })
}

fn read_allowed_identities(root: &std::fs::File, relative: &Path) -> Option<BTreeSet<AddressHash>> {
    let contents = match secure_read(root, relative) {
        Ok(content) => content.bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return None,
        Err(_) => return Some(BTreeSet::new()),
    };
    Some(
        String::from_utf8_lossy(&contents)
            .lines()
            .filter_map(|line| {
                let bytes: [u8; 16] = hex::decode(line.trim()).ok()?.try_into().ok()?;
                Some(AddressHash::new(bytes))
            })
            .collect(),
    )
}

#[cfg(unix)]
fn secure_open_dir(path: &Path) -> io::Result<std::fs::File> {
    use rustix::fs::{open, Mode, OFlags};

    Ok(std::fs::File::from(open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?))
}

#[cfg(unix)]
fn secure_open_file(root: &std::fs::File, relative: &Path) -> io::Result<std::fs::File> {
    use rustix::fs::{openat, Mode, OFlags};
    use std::path::Component;

    let components = relative.components().collect::<Vec<_>>();
    if components.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty native path"));
    }
    let mut directory = root.try_clone()?;
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(name) = component else {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe native path"));
        };
        let last = index + 1 == components.len();
        let flags = if last {
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC
        } else {
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
        };
        directory = std::fs::File::from(openat(&directory, *name, flags, Mode::empty())?);
    }
    Ok(directory)
}

#[cfg(not(unix))]
fn secure_open_dir(_path: &Path) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native file hosting requires descriptor-relative no-follow opens",
    ))
}

#[cfg(not(unix))]
fn secure_open_file(_root: &std::fs::File, _relative: &Path) -> io::Result<std::fs::File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "native file hosting requires descriptor-relative no-follow opens",
    ))
}

impl Default for PageService {
    fn default() -> Self {
        Self::with_default_dir()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rns_core::identity::PrivateIdentity;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn empty_dir_serves_default_index() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 0);

        let content = svc.handle_request("/page/index.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("Welcome to"));
        assert!(text.contains("Styrene"));
    }

    #[test]
    fn serves_static_page() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("test.mu"), b">Test Page\nHello!").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 1);

        let content = svc.handle_request("/page/test.mu");
        assert_eq!(content, b">Test Page\nHello!");
    }

    #[test]
    fn not_found_returns_error_page() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());

        let content = svc.handle_request("/page/nonexistent.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("not found"));
    }

    #[test]
    fn nested_pages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/deep.mu"), b">Deep Page").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        assert_eq!(svc.page_count(), 1);

        let content = svc.handle_request("/page/sub/deep.mu");
        assert_eq!(content, b">Deep Page");
    }

    #[test]
    fn list_pages() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.mu"), b"A").unwrap();
        std::fs::write(dir.path().join("b.mu"), b"B").unwrap();

        let svc = PageService::new(dir.path().to_path_buf());
        let pages = svc.list_pages();
        assert_eq!(pages.len(), 2);
    }

    #[test]
    fn custom_node_name_in_default_index() {
        let dir = tempfile::tempdir().unwrap();
        let svc = PageService::new(dir.path().to_path_buf());
        svc.set_node_name("My Hub");

        let content = svc.handle_request("/page/index.mu");
        let text = String::from_utf8_lossy(&content);
        assert!(text.contains("My Hub"));
    }

    #[test]
    fn native_inventory_includes_pages_files_and_fail_closed_allowed_policy() {
        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let pages = root_path.join("pages");
        let files = root_path.join("files");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(pages.join("index.mu"), b">Index").unwrap();
        std::fs::write(files.join("manual.bin"), b"manual").unwrap();
        std::fs::write(files.join("manual.bin.allowed"), b"invalid\n").unwrap();

        let service = PageService::with_storage_dirs(pages, files);
        let entries = service.native_entries();

        assert_eq!(
            entries.iter().map(|entry| entry.request_path.as_str()).collect::<Vec<_>>(),
            ["/file/manual.bin", "/page/index.mu",]
        );
        assert!(entries[0].allowed_identities.as_ref().is_some_and(BTreeSet::is_empty));
        let inventory = service.native_inventory();
        assert_eq!(
            inventory
                .iter()
                .map(|(entry, active)| {
                    (entry.request_path.as_str(), entry.dynamic, entry.restricted, *active)
                })
                .collect::<Vec<_>>(),
            [("/file/manual.bin", false, true, false), ("/page/index.mu", false, false, false),]
        );
        service.mark_native_path_active("/page/index.mu".into());
        assert!(service
            .native_inventory()
            .iter()
            .any(|(entry, active)| entry.request_path == "/page/index.mu" && *active));
    }

    #[cfg(unix)]
    #[test]
    fn native_reads_reject_traversal_and_symlink_components() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let pages = root_path.join("pages");
        let outside = root_path.join("outside");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.mu"), b"secret").unwrap();
        symlink(&outside, pages.join("escape")).unwrap();
        symlink(outside.join("secret.mu"), pages.join("linked.mu")).unwrap();

        let directory = secure_open_dir(&pages).unwrap();
        assert!(secure_read(&directory, Path::new("../outside/secret.mu")).is_err());
        assert!(secure_read(&directory, Path::new("escape/secret.mu")).is_err());
        assert!(secure_read(&directory, Path::new("linked.mu")).is_err());

        let service = PageService::with_storage_dirs(pages, root_path.join("files"));
        let paths = service
            .native_entries()
            .into_iter()
            .map(|entry| entry.request_path)
            .collect::<Vec<_>>();
        assert_eq!(paths, ["/page/index.mu"]);
    }

    #[cfg(unix)]
    #[test]
    fn registered_content_and_allowed_policy_cannot_be_replaced_with_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let root_path = root.path().canonicalize().unwrap();
        let pages = root_path.join("pages");
        let files = root_path.join("files");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::create_dir_all(&files).unwrap();
        let allowed = PrivateIdentity::new_from_name("original-native-reader");
        let replacement = PrivateIdentity::new_from_name("replacement-native-reader");
        std::fs::write(pages.join("private.mu"), b"original").unwrap();
        std::fs::write(
            pages.join("private.mu.allowed"),
            hex::encode(allowed.address_hash().as_slice()),
        )
        .unwrap();
        let service = PageService::with_storage_dirs(pages.clone(), files);
        let entry = service
            .native_entries()
            .into_iter()
            .find(|entry| entry.request_path == "/page/private.mu")
            .unwrap();

        let outside_content = root_path.join("outside.mu");
        let outside_allowed = root_path.join("outside.allowed");
        std::fs::write(&outside_content, b"replacement").unwrap();
        std::fs::write(&outside_allowed, hex::encode(replacement.address_hash().as_slice()))
            .unwrap();
        std::fs::remove_file(pages.join("private.mu")).unwrap();
        std::fs::remove_file(pages.join("private.mu.allowed")).unwrap();
        symlink(outside_content, pages.join("private.mu")).unwrap();
        symlink(outside_allowed, pages.join("private.mu.allowed")).unwrap();

        assert_eq!(
            service.serve_native(&entry, &[0xc0], None, AddressHash::new([0; 16])).unwrap(),
            b"original"
        );
        let policy = entry.allowed_identities.unwrap();
        assert!(policy.contains(allowed.address_hash()));
        assert!(!policy.contains(replacement.address_hash()));
    }

    #[cfg(unix)]
    #[test]
    fn executable_snapshot_is_in_memory_and_invocation_rejects_accidental_writes() {
        let original = b"#!/bin/sh\nprintf immutable\n";
        let executable = snapshot_executable(original.to_vec());
        assert_eq!(executable.bytes.as_ref(), original);
        assert!(!format!("{executable:?}").contains("immutable"));
        let invocation = materialize_executable(&executable).unwrap();
        assert_eq!(
            std::fs::metadata(&invocation.path).unwrap().permissions().mode() & 0o777,
            0o500
        );
        if !rustix::process::geteuid().is_root() {
            assert!(std::fs::OpenOptions::new().write(true).open(&invocation.path).is_err());
        }
    }

    #[cfg(not(unix))]
    #[test]
    fn non_unix_native_host_fails_closed_to_default_static_page() {
        let root = tempfile::tempdir().unwrap();
        let pages = root.path().join("pages");
        std::fs::create_dir_all(&pages).unwrap();
        std::fs::write(pages.join("dynamic.mu"), b"untrusted executable").unwrap();
        let service = PageService::with_storage_dirs(pages, root.path().join("files"));
        let entries = service.native_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].request_path, "/page/index.mu");
    }
}
