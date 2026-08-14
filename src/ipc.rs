//! IPC protocol for external clients (CLI, agents, plugins).
//!
//! JSON-based request/response over Unix domain socket or TCP.
//! Socket path: /tmp/pmux.sock (or $PMUX_SOCK).
//! Also accepts leftover $PWORKSPACES_SOCK / /tmp/pworkspaces.sock.
//! TCP: $PMUX_LISTEN / pmux.toml `listen` (default 0.0.0.0:7878). Client: $PMUX_HOST.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const DEFAULT_SOCK: &str = "/tmp/pmux.sock";
const LEGACY_SOCK: &str = "/tmp/pworkspaces.sock";
pub const DEFAULT_TCP: &str = "0.0.0.0:7878";
pub const DEFAULT_TCP_PORT: u16 = 7878;

/// Socket path. Env wins; else new default; else leftover daemon on the old path.
pub fn socket_path() -> PathBuf {
    if let Ok(p) = std::env::var("PMUX_SOCK").or_else(|_| std::env::var("PWORKSPACES_SOCK")) {
        return PathBuf::from(p);
    }
    let neu = PathBuf::from(DEFAULT_SOCK);
    let old = PathBuf::from(LEGACY_SOCK);
    if !neu.exists() && old.exists() {
        old
    } else {
        neu
    }
}

/// Bind address for LAN TCP. `None` = unix only.
pub fn tcp_listen_addr() -> Option<String> {
    if let Ok(v) = std::env::var("PMUX_LISTEN") {
        return parse_listen(&v);
    }
    if let Some(v) = toml_str("listen") {
        return parse_listen(&v);
    }
    Some(DEFAULT_TCP.into())
}

/// Client TCP target (`host:port`). Unix sock if unset.
pub fn tcp_connect_addr() -> Option<String> {
    let v = std::env::var("PMUX_HOST").ok()?;
    parse_listen(&v)
}

pub(crate) fn parse_listen(raw: &str) -> Option<String> {
    let v = raw.trim();
    if v.is_empty()
        || v.eq_ignore_ascii_case("off")
        || v.eq_ignore_ascii_case("none")
        || v.eq_ignore_ascii_case("false")
    {
        return None;
    }
    if v.contains(']') {
        return Some(v.to_string());
    }
    if let Some((_, port)) = v.rsplit_once(':') {
        if port.parse::<u16>().is_ok() {
            return Some(v.to_string());
        }
    }
    Some(format!("{v}:{DEFAULT_TCP_PORT}"))
}

fn toml_str(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(crate::paths::config_file()).ok()?;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let rest = line.strip_prefix(key)?;
        let rest = rest.trim().strip_prefix('=')?.trim();
        let val = rest.trim_matches('"').trim_matches('\'').trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// Request from client to runtime.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Request {
    /// List all workspaces.
    ListWorkspaces,

    /// Create a new workspace.
    CreateWorkspace { name: String },

    /// Destroy a workspace.
    DestroyWorkspace { workspace: String },

    /// Switch active workspace.
    SwitchWorkspace { workspace: String },

    /// List panes in a workspace.
    ListPanes { workspace: String },

    /// Add a pane to a workspace.
    AddPane {
        workspace: String,
        name: Option<String>,
        #[serde(default = "default_true")]
        spawn_session: bool,
    },

    /// Split a pane.
    SplitPane {
        workspace: String,
        #[serde(default = "default_horizontal")]
        direction: String,
        name: Option<String>,
        #[serde(default = "default_true")]
        spawn_session: bool,
    },

    /// Close a pane.
    ClosePane {
        workspace: String,
        pane: String,
    },

    /// Send a command to a pane's session.
    SendCommand {
        workspace: String,
        pane: String,
        command: String,
    },

    /// Read output from a pane's session.
    ReadOutput {
        workspace: String,
        pane: String,
    },

    /// Replay last PTY bytes without draining the live unread buffer.
    ReadReplay {
        workspace: String,
        pane: String,
    },

    /// Focus a pane.
    FocusPane {
        workspace: String,
        pane: String,
    },

    /// Open a file/URL in a view preview pane.
    OpenView {
        workspace: String,
        path: String,
    },

    /// Get runtime status.
    Status,

    /// Ping — health check.
    Ping,

    /// First message on TCP. Unix socket does not require this.
    Auth { token: String },

    /// Full UI snapshot for the active (or named) workspace.
    Snapshot {
        #[serde(default)]
        workspace: Option<String>,
    },

    /// One round-trip for a UI frame: optional input/resize, then snapshot + drained PTY bytes.
    PollUi {
        #[serde(default)]
        workspace: Option<String>,
        #[serde(default)]
        inputs: Vec<PollInput>,
        #[serde(default)]
        resizes: Vec<PollResize>,
    },

    /// Raw PTY bytes (keys, paste). `bytes` is a JSON array of u8.
    SendInput {
        workspace: String,
        pane: String,
        bytes: Vec<u8>,
    },

    /// Resize a pane PTY.
    ResizePty {
        workspace: String,
        pane: String,
        cols: u16,
        rows: u16,
    },

    /// Run a registered action.
    ExecuteAction {
        name: String,
        #[serde(default)]
        workspace: Option<String>,
        #[serde(default)]
        pane: Option<String>,
    },

    /// Command palette rows.
    PaletteItems {
        #[serde(default)]
        query: String,
    },

    RenameWorkspace {
        workspace: String,
        name: String,
    },

    SetWorkspaceCwd {
        workspace: String,
        cwd: String,
    },

    QuickOpenRoot {
        workspace: String,
        #[serde(default)]
        pane: Option<String>,
    },

    SwapPanes {
        workspace: String,
        a: String,
        b: String,
    },

    ResizeSplit {
        workspace: String,
        index: usize,
        ratio: f32,
    },

    SetFloatGeom {
        workspace: String,
        pane: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    },

    KillListenPid {
        pid: u32,
    },

    /// Persist layout now.
    Save,

    /// Stop daemon (tmux kill-server).
    Shutdown,
}

fn default_true() -> bool {
    true
}

fn default_horizontal() -> String {
    "horizontal".into()
}

/// Response from runtime to client.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ok {
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<serde_json::Value>,
    },
    Error {
        message: String,
    },
}

impl Response {
    pub fn ok() -> Self {
        Response::Ok { data: None }
    }

    pub fn ok_data(data: impl Serialize) -> Self {
        Response::Ok {
            data: Some(serde_json::to_value(data).unwrap()),
        }
    }

    pub fn error(msg: impl Into<String>) -> Self {
        Response::Error {
            message: msg.into(),
        }
    }
}

/// Workspace info for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceInfo {
    pub id: String,
    pub name: String,
    pub pane_count: usize,
    pub active: bool,
}

/// Pane info for listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneInfo {
    pub id: String,
    pub name: Option<String>,
    pub has_session: bool,
    pub focused: bool,
}

/// Runtime status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusInfo {
    pub workspaces: usize,
    pub sessions: usize,
    pub actions: usize,
    pub components: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTabSnap {
    pub id: String,
    pub name: String,
    pub cwd: Option<String>,
    pub active: bool,
    pub pane_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneSnap {
    pub id: String,
    pub name: Option<String>,
    pub component_type: String,
    pub source: Option<String>,
    pub session_alive: bool,
    #[serde(default)]
    pub pty_cols: Option<u16>,
    #[serde(default)]
    pub pty_rows: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatSnap {
    pub pane_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollInput {
    pub pane: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollResize {
    pub pane: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneBytes {
    pub pane: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PollUiData {
    pub snapshot: UiSnapshot,
    #[serde(default)]
    pub outputs: Vec<PaneBytes>,
}

/// Layout + panes + ports for one UI frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub active_id: Option<String>,
    pub workspaces: Vec<WorkspaceTabSnap>,
    pub layout_root: Option<crate::layout::LayoutNode>,
    pub focused: Option<String>,
    pub fullscreen: Option<String>,
    pub panes: Vec<PaneSnap>,
    pub floating: Vec<FloatSnap>,
    pub listen_ports: Vec<crate::monitor::ListenPort>,
    pub listen_ports_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionOutcome {
    pub ok: bool,
    pub message: Option<String>,
    pub active_id: Option<String>,
}

/// Wire format: each message is a JSON line (newline-delimited).
/// Read a JSON line from a reader.
pub fn read_message<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<String> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line)
}

/// Write a JSON line to a writer.
pub fn write_message<W: std::io::Write>(writer: &mut W, msg: &str) -> std::io::Result<()> {
    writer.write_all(msg.as_bytes())?;
    writer.write_all(b"\n")?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_listen_off() {
        assert!(parse_listen("off").is_none());
        assert!(parse_listen("none").is_none());
        assert_eq!(parse_listen("10.0.0.5").as_deref(), Some("10.0.0.5:7878"));
        assert_eq!(parse_listen("10.0.0.5:9000").as_deref(), Some("10.0.0.5:9000"));
    }
}
