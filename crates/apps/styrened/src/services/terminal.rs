//! TerminalService — manages local shell sessions with PTY support.
//!
//! Uses `portable-pty` for proper pseudo-terminal allocation, enabling
//! interactive programs (vim, htop, less) and resize (SIGWINCH) support.
//! Each session spawns a child process in a PTY and relays I/O via
//! a tokio mpsc channel for the event bridge.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;
use tokio::sync::mpsc;

use portable_pty::{native_pty_system, CommandBuilder, PtySize};

/// Events emitted by terminal sessions.
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    Output { session_id: String, data: Vec<u8> },
    Exited { session_id: String, exit_code: Option<i32> },
}

/// A running terminal session.
struct Session {
    input_tx: mpsc::Sender<Vec<u8>>,
    pty_pair: portable_pty::PtyPair,
}

/// Terminal session management service.
pub struct TerminalService {
    sessions: Mutex<HashMap<String, Session>>,
    event_tx: mpsc::Sender<TerminalEvent>,
    event_rx: Mutex<Option<mpsc::Receiver<TerminalEvent>>>,
}

impl TerminalService {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel(256);
        Self { sessions: Mutex::new(HashMap::new()), event_tx: tx, event_rx: Mutex::new(Some(rx)) }
    }

    /// Take the event receiver (call once, from the event bridge).
    pub fn take_event_rx(&self) -> Option<mpsc::Receiver<TerminalEvent>> {
        self.event_rx.lock().unwrap().take()
    }

    /// Open a new local terminal session with PTY. Returns the session ID.
    pub fn open(&self, shell: Option<&str>, rows: u16, cols: u16) -> Result<String, String> {
        let session_id = format!("{:016x}", session_rand());

        let pty_system = native_pty_system();
        let pty_size = PtySize {
            rows: if rows > 0 { rows } else { 24 },
            cols: if cols > 0 { cols } else { 80 },
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system.openpty(pty_size).map_err(|e| format!("failed to open PTY: {e}"))?;

        let shell_cmd = shell
            .map(|s| s.to_string())
            .unwrap_or_else(|| std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into()));

        let mut cmd = CommandBuilder::new(&shell_cmd);
        cmd.env("TERM", "xterm-256color");

        let _child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("failed to spawn {shell_cmd}: {e}"))?;

        let (input_tx, input_rx) = mpsc::channel::<Vec<u8>>(256);

        // Spawn PTY I/O relay tasks
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("failed to clone PTY reader: {e}"))?;

        let writer =
            pair.master.take_writer().map_err(|e| format!("failed to take PTY writer: {e}"))?;

        spawn_pty_io(session_id.clone(), reader, writer, input_rx, self.event_tx.clone());

        self.sessions
            .lock()
            .unwrap()
            .insert(session_id.clone(), Session { input_tx, pty_pair: pair });

        eprintln!("[terminal] opened PTY session {session_id} ({shell_cmd})");
        Ok(session_id)
    }

    /// Send input data to a session's PTY.
    pub async fn input(&self, session_id: &str, data: &[u8]) -> Result<(), String> {
        let tx = {
            let sessions = self.sessions.lock().unwrap();
            sessions
                .get(session_id)
                .map(|s| s.input_tx.clone())
                .ok_or_else(|| format!("session not found: {session_id}"))?
        };
        tx.send(data.to_vec()).await.map_err(|_| "session PTY closed".into())
    }

    /// Resize a terminal session's PTY.
    pub fn resize(&self, session_id: &str, rows: u16, cols: u16) -> Result<(), String> {
        let sessions = self.sessions.lock().unwrap();
        let session =
            sessions.get(session_id).ok_or_else(|| format!("session not found: {session_id}"))?;

        session
            .pty_pair
            .master
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| format!("resize failed: {e}"))?;

        eprintln!("[terminal] resized session {session_id} to {cols}x{rows}");
        Ok(())
    }

    /// Close a terminal session.
    pub fn close(&self, session_id: &str) -> Result<(), String> {
        let removed = self.sessions.lock().unwrap().remove(session_id).is_some();
        if removed {
            eprintln!("[terminal] closed session {session_id}");
            Ok(())
        } else {
            Err(format!("session not found: {session_id}"))
        }
    }

    /// Number of active sessions.
    #[allow(dead_code)]
    pub fn session_count(&self) -> usize {
        self.sessions.lock().unwrap().len()
    }
}

impl Default for TerminalService {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn background tasks for PTY I/O relay.
fn spawn_pty_io(
    session_id: String,
    mut reader: Box<dyn Read + Send>,
    mut writer: Box<dyn Write + Send>,
    mut input_rx: mpsc::Receiver<Vec<u8>>,
    event_tx: mpsc::Sender<TerminalEvent>,
) {
    // PTY reader → event channel (runs on blocking thread since PTY I/O is sync)
    let tx_read = event_tx.clone();
    let sid_read = session_id.clone();
    tokio::task::spawn_blocking(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = tx_read.blocking_send(TerminalEvent::Output {
                        session_id: sid_read.clone(),
                        data: buf[..n].to_vec(),
                    });
                }
                Err(_) => break,
            }
        }
        let _ =
            tx_read.blocking_send(TerminalEvent::Exited { session_id: sid_read, exit_code: None });
    });

    // Input channel → PTY writer (also blocking since PTY write is sync)
    tokio::task::spawn_blocking(move || {
        while let Some(data) = input_rx.blocking_recv() {
            if writer.write_all(&data).is_err() {
                break;
            }
            let _ = writer.flush();
        }
    });
}

fn session_rand() -> u64 {
    use std::time::SystemTime;
    let nanos =
        SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos()
            as u64;
    nanos ^ 0x517cc1b727220a95
}
