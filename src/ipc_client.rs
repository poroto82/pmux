//! Unix-socket client for the workspace daemon.

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::action::ActionInfo;
use crate::ipc::{
    self, ActionOutcome, Request, Response, UiSnapshot,
};

#[derive(Debug)]
pub enum IpcError {
    Io(std::io::Error),
    Protocol(String),
    Remote(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Io(e) => write!(f, "ipc: {e}"),
            IpcError::Protocol(e) => write!(f, "ipc: {e}"),
            IpcError::Remote(e) => write!(f, "{e}"),
        }
    }
}

impl From<std::io::Error> for IpcError {
    fn from(e: std::io::Error) -> Self {
        IpcError::Io(e)
    }
}

pub struct IpcClient {
    writer: Mutex<UnixStream>,
    reader: Mutex<BufReader<UnixStream>>,
}

impl IpcClient {
    pub fn connect() -> Result<Self, IpcError> {
        let stream = UnixStream::connect(ipc::socket_path())?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(8)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(8)));
        let reader = BufReader::new(stream.try_clone()?);
        Ok(Self {
            writer: Mutex::new(stream),
            reader: Mutex::new(reader),
        })
    }

    pub fn ping() -> bool {
        let Ok(mut stream) = UnixStream::connect(ipc::socket_path()) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(400)));
        let json = serde_json::to_string(&Request::Ping).unwrap();
        if ipc::write_message(&mut stream, &json).is_err() {
            return false;
        }
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() || line.is_empty() {
            return false;
        }
        matches!(serde_json::from_str::<Response>(&line), Ok(Response::Ok { .. }))
    }

    pub fn wait_ready(timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if Self::ping() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
        false
    }

    fn rpc(&self, req: Request) -> Result<Response, IpcError> {
        let json = serde_json::to_string(&req).map_err(|e| IpcError::Protocol(e.to_string()))?;
        let mut w = self.writer.lock().unwrap();
        let mut r = self.reader.lock().unwrap();
        ipc::write_message(&mut *w, &json)?;
        let mut line = String::new();
        r.read_line(&mut line)?;
        if line.is_empty() {
            return Err(IpcError::Protocol("empty response".into()));
        }
        serde_json::from_str(&line).map_err(|e| IpcError::Protocol(e.to_string()))
    }

    fn ok_data<T: serde::de::DeserializeOwned>(&self, req: Request) -> Result<T, IpcError> {
        match self.rpc(req)? {
            Response::Ok { data: Some(v) } => {
                serde_json::from_value(v).map_err(|e| IpcError::Protocol(e.to_string()))
            }
            Response::Ok { data: None } => Err(IpcError::Protocol("missing data".into())),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    fn ok(&self, req: Request) -> Result<(), IpcError> {
        match self.rpc(req)? {
            Response::Ok { .. } => Ok(()),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    pub fn snapshot(&self, workspace: Option<&str>) -> Result<UiSnapshot, IpcError> {
        self.ok_data(Request::Snapshot {
            workspace: workspace.map(|s| s.to_string()),
        })
    }

    pub fn read_output(&self, workspace: &str, pane: &str) -> Result<Vec<u8>, IpcError> {
        self.read_bytes(Request::ReadOutput {
            workspace: workspace.into(),
            pane: pane.into(),
        })
    }

    pub fn read_replay(&self, workspace: &str, pane: &str) -> Result<Vec<u8>, IpcError> {
        self.read_bytes(Request::ReadReplay {
            workspace: workspace.into(),
            pane: pane.into(),
        })
    }

    fn read_bytes(&self, req: Request) -> Result<Vec<u8>, IpcError> {
        match self.rpc(req)? {
            Response::Ok { data: Some(v) } => {
                if let Some(arr) = v.get("bytes").and_then(|b| b.as_array()) {
                    return Ok(arr
                        .iter()
                        .filter_map(|x| x.as_u64().map(|n| n as u8))
                        .collect());
                }
                if let Some(s) = v.get("output").and_then(|o| o.as_str()) {
                    return Ok(s.as_bytes().to_vec());
                }
                Ok(Vec::new())
            }
            Response::Ok { data: None } => Ok(Vec::new()),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    pub fn send_input(&self, workspace: &str, pane: &str, bytes: &[u8]) -> Result<(), IpcError> {
        self.ok(Request::SendInput {
            workspace: workspace.into(),
            pane: pane.into(),
            bytes: bytes.to_vec(),
        })
    }

    pub fn resize_pty(
        &self,
        workspace: &str,
        pane: &str,
        cols: u16,
        rows: u16,
    ) -> Result<(), IpcError> {
        self.ok(Request::ResizePty {
            workspace: workspace.into(),
            pane: pane.into(),
            cols,
            rows,
        })
    }

    pub fn execute_action(
        &self,
        name: &str,
        workspace: Option<&str>,
        pane: Option<&str>,
    ) -> Result<ActionOutcome, IpcError> {
        self.ok_data(Request::ExecuteAction {
            name: name.into(),
            workspace: workspace.map(|s| s.to_string()),
            pane: pane.map(|s| s.to_string()),
        })
    }

    pub fn palette_items(&self, query: &str) -> Result<Vec<ActionInfo>, IpcError> {
        self.ok_data(Request::PaletteItems {
            query: query.into(),
        })
    }

    pub fn open_view(&self, workspace: &str, path: &str) -> Result<(), IpcError> {
        self.ok(Request::OpenView {
            workspace: workspace.into(),
            path: path.into(),
        })
    }

    pub fn rename_workspace(&self, workspace: &str, name: &str) -> Result<String, IpcError> {
        match self.rpc(Request::RenameWorkspace {
            workspace: workspace.into(),
            name: name.into(),
        })? {
            Response::Ok { data: Some(v) } => Ok(v
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or(name)
                .to_string()),
            Response::Ok { .. } => Ok(name.into()),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    pub fn set_workspace_cwd(&self, workspace: &str, cwd: &str) -> Result<String, IpcError> {
        match self.rpc(Request::SetWorkspaceCwd {
            workspace: workspace.into(),
            cwd: cwd.into(),
        })? {
            Response::Ok { data: Some(v) } => Ok(v
                .get("cwd")
                .and_then(|x| x.as_str())
                .unwrap_or(cwd)
                .to_string()),
            Response::Ok { .. } => Ok(cwd.into()),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    pub fn quick_open_root(
        &self,
        workspace: &str,
        pane: Option<&str>,
    ) -> Result<String, IpcError> {
        match self.rpc(Request::QuickOpenRoot {
            workspace: workspace.into(),
            pane: pane.map(|s| s.to_string()),
        })? {
            Response::Ok { data: Some(v) } => Ok(v
                .get("root")
                .and_then(|x| x.as_str())
                .or_else(|| v.as_str())
                .unwrap_or(".")
                .to_string()),
            Response::Ok { .. } => Ok(".".into()),
            Response::Error { message } => Err(IpcError::Remote(message)),
        }
    }

    pub fn switch_workspace(&self, workspace: &str) -> Result<(), IpcError> {
        self.ok(Request::SwitchWorkspace {
            workspace: workspace.into(),
        })
    }

    pub fn destroy_workspace(&self, workspace: &str) -> Result<(), IpcError> {
        self.ok(Request::DestroyWorkspace {
            workspace: workspace.into(),
        })
    }

    pub fn close_pane(&self, workspace: &str, pane: &str) -> Result<(), IpcError> {
        self.ok(Request::ClosePane {
            workspace: workspace.into(),
            pane: pane.into(),
        })
    }

    pub fn focus_pane(&self, workspace: &str, pane: &str) -> Result<(), IpcError> {
        self.ok(Request::FocusPane {
            workspace: workspace.into(),
            pane: pane.into(),
        })
    }

    pub fn swap_panes(&self, workspace: &str, a: &str, b: &str) -> Result<(), IpcError> {
        self.ok(Request::SwapPanes {
            workspace: workspace.into(),
            a: a.into(),
            b: b.into(),
        })
    }

    pub fn resize_split(&self, workspace: &str, index: usize, ratio: f32) -> Result<(), IpcError> {
        self.ok(Request::ResizeSplit {
            workspace: workspace.into(),
            index,
            ratio,
        })
    }

    pub fn set_float_geom(
        &self,
        workspace: &str,
        pane: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<(), IpcError> {
        self.ok(Request::SetFloatGeom {
            workspace: workspace.into(),
            pane: pane.into(),
            x,
            y,
            width,
            height,
        })
    }

    pub fn kill_listen_pid(&self, pid: u32) -> Result<(), IpcError> {
        self.ok(Request::KillListenPid { pid })
    }

    pub fn save(&self) -> Result<(), IpcError> {
        self.ok(Request::Save)
    }

    pub fn shutdown(&self) -> Result<(), IpcError> {
        match self.rpc(Request::Shutdown) {
            Ok(Response::Ok { .. }) | Err(_) => Ok(()),
            Ok(Response::Error { message }) => Err(IpcError::Remote(message)),
        }
    }
}
