use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

use pmux::ids::SessionId;

/// Session state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SessionState {
    Running,
    Exited(i32),
}

/// Metadata that can be serialized for persistence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub cwd: String,
    pub shell: String,
    pub cols: u16,
    pub rows: u16,
    pub state: SessionState,
}

/// Extra environment injected into the PTY (e.g. PW_* for agents).
#[derive(Debug, Clone, Default)]
pub struct SessionSpawnEnv {
    pub vars: Vec<(String, String)>,
}

impl SessionSpawnEnv {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.vars.push((key.into(), value.into()));
        self
    }

    pub fn extend(mut self, iter: impl IntoIterator<Item = (String, String)>) -> Self {
        self.vars.extend(iter);
        self
    }
}

/// A live session with a PTY.
pub struct Session {
    pub meta: SessionMeta,
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    output: Arc<Mutex<Vec<u8>>>,
    replay: Arc<Mutex<Vec<u8>>>,
    reader_alive: Arc<AtomicBool>,
    child_alive: Arc<AtomicBool>,
    pid: Option<u32>,
    _reader_handle: thread::JoinHandle<()>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("meta", &self.meta)
            .finish()
    }
}

impl Session {
    /// Spawn a new session with the default shell.
    pub fn spawn(cwd: Option<&str>, cols: u16, rows: u16) -> Result<Self, SessionError> {
        let shell = default_shell();
        Self::spawn_with_shell(&shell, cwd, cols, rows, SessionSpawnEnv::new())
    }

    /// Spawn with default shell and extra env.
    pub fn spawn_with_env(
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        env: SessionSpawnEnv,
    ) -> Result<Self, SessionError> {
        let shell = default_shell();
        Self::spawn_with_shell(&shell, cwd, cols, rows, env)
    }

    /// Spawn a session with a specific shell/command.
    pub fn spawn_with_shell(
        shell: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        env: SessionSpawnEnv,
    ) -> Result<Self, SessionError> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SessionError::PtyOpen(e.to_string()))?;

        let mut cmd = CommandBuilder::new(shell);
        cmd.arg("--login");
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        for (k, v) in &env.vars {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.cwd(dir);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| SessionError::SpawnFailed(e.to_string()))?;

        let pid = child.process_id();

        // Drop slave — we only need master
        drop(pair.slave);

        // Wait for child in background to detect exit
        let child_alive = Arc::new(AtomicBool::new(true));
        let child_alive_clone = child_alive.clone();
        thread::spawn(move || {
            let _ = child.wait();
            child_alive_clone.store(false, Ordering::SeqCst);
        });

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| SessionError::PtyOpen(e.to_string()))?;

        // Reader thread — accumulates output
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| SessionError::PtyOpen(e.to_string()))?;

        let output = Arc::new(Mutex::new(Vec::new()));
        let output_clone = output.clone();
        let replay = Arc::new(Mutex::new(Vec::new()));
        let replay_clone = replay.clone();
        let reader_alive = Arc::new(AtomicBool::new(true));
        let reader_alive_clone = reader_alive.clone();

        let reader_handle = thread::spawn(move || {
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        let chunk = &buf[..n];
                        {
                            let mut out = output_clone.lock().unwrap();
                            out.extend_from_slice(chunk);
                            if out.len() > 1_048_576 {
                                let start = out.len() - 524_288;
                                *out = out[start..].to_vec();
                            }
                        }
                        {
                            let mut hist = replay_clone.lock().unwrap();
                            hist.extend_from_slice(chunk);
                            const REPLAY_CAP: usize = 256 * 1024;
                            if hist.len() > REPLAY_CAP {
                                let start = hist.len() - REPLAY_CAP / 2;
                                *hist = hist[start..].to_vec();
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
            reader_alive_clone.store(false, Ordering::SeqCst);
        });

        let id = SessionId::new();
        let resolved_cwd = cwd.unwrap_or(".").to_string();

        Ok(Self {
            meta: SessionMeta {
                id,
                cwd: resolved_cwd,
                shell: shell.to_string(),
                cols,
                rows,
                state: SessionState::Running,
            },
            master: pair.master,
            writer,
            output,
            replay,
            reader_alive,
            child_alive,
            pid,
            _reader_handle: reader_handle,
        })
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub fn id(&self) -> &SessionId {
        &self.meta.id
    }

    /// Check if the session's shell process is still alive.
    /// Dead if either child exited OR reader got EOF.
    pub fn is_alive(&self) -> bool {
        self.child_alive.load(Ordering::SeqCst) && self.reader_alive.load(Ordering::SeqCst)
    }

    /// Send raw bytes to PTY (low-level: Ctrl+C, arrows, etc).
    pub fn send_input(&mut self, data: &[u8]) -> Result<(), SessionError> {
        self.writer
            .write_all(data)
            .map_err(|e| SessionError::WriteFailed(e.to_string()))?;
        self.writer
            .flush()
            .map_err(|e| SessionError::WriteFailed(e.to_string()))?;
        Ok(())
    }

    /// Send a command string (high-level: appends newline).
    pub fn send_command(&mut self, cmd: &str) -> Result<(), SessionError> {
        let with_newline = format!("{}\n", cmd);
        self.send_input(with_newline.as_bytes())
    }

    /// Read accumulated output (drains buffer).
    pub fn read_output(&self) -> Vec<u8> {
        let mut out = self.output.lock().unwrap();
        let data = out.clone();
        out.clear();
        data
    }

    /// Peek at output without draining.
    pub fn peek_output(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    /// Last ~256KB of PTY bytes (survives UI detach; not drained by `read_output`).
    /// Starts at a newline so restore does not begin mid-line (zsh `%` / garbage).
    pub fn replay_output(&self) -> Vec<u8> {
        strip_zsh_eol_marks(&trim_replay(&self.replay.lock().unwrap()))
    }

    /// Output buffer size.
    pub fn output_len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    /// Resize PTY. No-op if size already matches (avoids zsh `%` on reattach WINCH).
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), SessionError> {
        if self.meta.cols == cols && self.meta.rows == rows {
            return Ok(());
        }
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| SessionError::ResizeFailed(e.to_string()))?;
        self.meta.cols = cols;
        self.meta.rows = rows;
        Ok(())
    }
}

/// Registry of all live sessions.
pub struct SessionRegistry {
    sessions: HashMap<SessionId, Session>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    /// Spawn a new session and register it.
    pub fn create(
        &mut self,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<SessionId, SessionError> {
        self.create_with_env(cwd, cols, rows, SessionSpawnEnv::new())
    }

    /// Spawn with extra environment variables.
    pub fn create_with_env(
        &mut self,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        env: SessionSpawnEnv,
    ) -> Result<SessionId, SessionError> {
        let session = Session::spawn_with_env(cwd, cols, rows, env)?;
        let id = session.meta.id.clone();
        self.sessions.insert(id.clone(), session);
        Ok(id)
    }

    /// Spawn with specific shell.
    pub fn create_with_shell(
        &mut self,
        shell: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
    ) -> Result<SessionId, SessionError> {
        self.create_with_shell_env(shell, cwd, cols, rows, SessionSpawnEnv::new())
    }

    pub fn create_with_shell_env(
        &mut self,
        shell: &str,
        cwd: Option<&str>,
        cols: u16,
        rows: u16,
        env: SessionSpawnEnv,
    ) -> Result<SessionId, SessionError> {
        let session = Session::spawn_with_shell(shell, cwd, cols, rows, env)?;
        let id = session.meta.id.clone();
        self.sessions.insert(id.clone(), session);
        Ok(id)
    }

    pub fn get(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &SessionId) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    /// True if session exists and is still alive.
    pub fn is_alive(&self, id: &SessionId) -> bool {
        self.sessions.get(id).map(|s| s.is_alive()).unwrap_or(false)
    }

    /// Send command to a session.
    pub fn send_command(&mut self, id: &SessionId, cmd: &str) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or(SessionError::NotFound(id.to_string()))?;
        session.send_command(cmd)
    }

    /// Send raw input to a session.
    pub fn send_input(&mut self, id: &SessionId, data: &[u8]) -> Result<(), SessionError> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or(SessionError::NotFound(id.to_string()))?;
        session.send_input(data)
    }

    /// Read output from a session.
    pub fn read_output(&self, id: &SessionId) -> Result<Vec<u8>, SessionError> {
        let session = self
            .sessions
            .get(id)
            .ok_or(SessionError::NotFound(id.to_string()))?;
        Ok(session.read_output())
    }

    pub fn replay_output(&self, id: &SessionId) -> Result<Vec<u8>, SessionError> {
        let session = self
            .sessions
            .get(id)
            .ok_or(SessionError::NotFound(id.to_string()))?;
        Ok(session.replay_output())
    }

    /// Remove and drop a session (kills PTY).
    pub fn destroy(&mut self, id: &SessionId) -> bool {
        self.sessions.remove(id).is_some()
    }

    pub fn count(&self) -> usize {
        self.sessions.len()
    }

    pub fn list_ids(&self) -> Vec<&SessionId> {
        self.sessions.keys().collect()
    }

    /// Get metadata for all sessions (serializable).
    pub fn list_meta(&self) -> Vec<&SessionMeta> {
        self.sessions.values().map(|s| &s.meta).collect()
    }
}

impl std::fmt::Debug for SessionRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionRegistry")
            .field("count", &self.sessions.len())
            .finish()
    }
}

#[derive(Debug)]
pub enum SessionError {
    PtyOpen(String),
    SpawnFailed(String),
    WriteFailed(String),
    ResizeFailed(String),
    NotFound(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionError::PtyOpen(e) => write!(f, "pty open: {}", e),
            SessionError::SpawnFailed(e) => write!(f, "spawn: {}", e),
            SessionError::WriteFailed(e) => write!(f, "write: {}", e),
            SessionError::ResizeFailed(e) => write!(f, "resize: {}", e),
            SessionError::NotFound(id) => write!(f, "session not found: {}", id),
        }
    }
}

/// Drop a leading partial line so restore starts on a real newline.
pub fn trim_replay(bytes: &[u8]) -> Vec<u8> {
    let Some(nl) = bytes.iter().position(|&b| b == b'\n') else {
        return bytes.to_vec();
    };
    let rest = &bytes[nl + 1..];
    if !rest.contains(&b'\n') {
        bytes.to_vec()
    } else {
        rest.to_vec()
    }
}

/// Drop zsh PROMPT_EOL_MARK lines (`%` + spaces, often inverse).
pub fn strip_zsh_eol_marks(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|&b| b == b'\n') {
        if is_zsh_eol_mark_line(line) {
            continue;
        }
        out.extend_from_slice(line);
    }
    out
}

fn strip_ansi(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
            i += 2;
            while i < bytes.len() && !bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

fn is_zsh_eol_mark_line(line: &[u8]) -> bool {
    let vis = strip_ansi(line);
    let s = vis
        .iter()
        .copied()
        .filter(|&b| b != b'\n' && b != b'\r')
        .collect::<Vec<_>>();
    let t = String::from_utf8_lossy(&s);
    let t = t.trim();
    t == "%" || t == "#"
}

fn default_shell() -> String {
    #[cfg(unix)]
    {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
    }
    #[cfg(windows)]
    {
        std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string())
    }
}
