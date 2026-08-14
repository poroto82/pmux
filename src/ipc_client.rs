//! IPC client: Unix socket or TCP (`$PMUX_HOST` + token).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::action::ActionInfo;
use crate::ipc::{
    self, ActionOutcome, PollInput, PollResize, PollUiData, Request, Response, UiSnapshot,
};
use crate::token;

const IO_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

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

impl IpcError {
    fn is_transport(&self) -> bool {
        match self {
            IpcError::Io(_) | IpcError::Protocol(_) => true,
            IpcError::Remote(_) => false,
        }
    }
}

struct Conn {
    writer: Box<dyn Write + Send>,
    reader: BufReader<Box<dyn Read + Send>>,
}

pub struct IpcClient {
    conn: Mutex<Conn>,
    generation: AtomicU64,
}

impl IpcClient {
    pub fn connect() -> Result<Self, IpcError> {
        Ok(Self {
            conn: Mutex::new(Self::open()?),
            generation: AtomicU64::new(0),
        })
    }

    fn open() -> Result<Conn, IpcError> {
        if let Some(addr) = ipc::tcp_connect_addr() {
            Self::connect_tcp(&addr)
        } else {
            Self::connect_unix()
        }
    }

    fn connect_unix() -> Result<Conn, IpcError> {
        let stream = unix_connect_timeout(&ipc::socket_path(), CONNECT_TIMEOUT)?;
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let reader = stream.try_clone()?;
        Ok(Conn {
            writer: Box::new(stream),
            reader: BufReader::new(Box::new(reader)),
        })
    }

    fn connect_tcp(addr: &str) -> Result<Conn, IpcError> {
        let sock = addr
            .to_socket_addrs()
            .map_err(IpcError::Io)?
            .next()
            .ok_or_else(|| IpcError::Protocol(format!("cannot resolve {addr}")))?;
        let stream = TcpStream::connect_timeout(&sock, CONNECT_TIMEOUT)?;
        let _ = stream.set_nodelay(true);
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        let mut writer: Box<dyn Write + Send> = Box::new(stream.try_clone()?);
        let mut reader = BufReader::new(Box::new(stream) as Box<dyn Read + Send>);
        let token = token::for_client().ok_or_else(|| {
            IpcError::Protocol("TCP needs $PMUX_TOKEN or ~/.config/pmux/token".into())
        })?;
        let json = serde_json::to_string(&Request::Auth { token })
            .map_err(|e| IpcError::Protocol(e.to_string()))?;
        ipc::write_message(&mut writer, &json)?;
        let mut line = String::new();
        reader.read_line(&mut line)?;
        match serde_json::from_str::<Response>(&line) {
            Ok(Response::Ok { .. }) => {}
            Ok(Response::Error { message }) => return Err(IpcError::Remote(message)),
            Err(e) => return Err(IpcError::Protocol(e.to_string())),
        }
        Ok(Conn { writer, reader })
    }

    pub fn ping() -> bool {
        if let Some(addr) = ipc::tcp_connect_addr() {
            return Self::connect_tcp(&addr)
                .ok()
                .and_then(|mut c| Self::rpc_on(&mut c, Request::Ping).ok())
                .is_some_and(|r| matches!(r, Response::Ok { .. }));
        }
        let Ok(mut stream) = unix_connect_timeout(&ipc::socket_path(), Duration::from_millis(800))
        else {
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

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    fn reconnect(&self) -> Result<(), IpcError> {
        let fresh = Self::open()?;
        *self.conn.lock().unwrap() = fresh;
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn rpc_on(conn: &mut Conn, req: Request) -> Result<Response, IpcError> {
        let json = serde_json::to_string(&req).map_err(|e| IpcError::Protocol(e.to_string()))?;
        ipc::write_message(&mut conn.writer, &json)?;
        let mut line = String::new();
        conn.reader.read_line(&mut line)?;
        if line.is_empty() {
            return Err(IpcError::Protocol("empty response".into()));
        }
        serde_json::from_str(&line).map_err(|e| IpcError::Protocol(e.to_string()))
    }

    fn rpc(&self, req: Request) -> Result<Response, IpcError> {
        let first = {
            let mut conn = self.conn.lock().unwrap();
            Self::rpc_on(&mut conn, req.clone())
        };
        match first {
            Ok(r) => Ok(r),
            Err(e) if e.is_transport() => {
                self.reconnect()?;
                let mut conn = self.conn.lock().unwrap();
                Self::rpc_on(&mut conn, req)
            }
            Err(e) => Err(e),
        }
    }

    pub fn request(&self, req: Request) -> Result<Response, IpcError> {
        self.rpc(req)
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

    pub fn poll_ui(
        &self,
        workspace: Option<&str>,
        inputs: Vec<PollInput>,
        resizes: Vec<PollResize>,
    ) -> Result<PollUiData, IpcError> {
        self.ok_data(Request::PollUi {
            workspace: workspace.map(|s| s.to_string()),
            inputs,
            resizes,
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

enum PumpCmd {
    Input { pane: String, bytes: Vec<u8> },
    Resize { pane: String, cols: u16, rows: u16 },
    Hydrate { pane: String, cols: u16, rows: u16 },
}

struct PumpState {
    snap: UiSnapshot,
    outputs: Vec<(String, Vec<u8>)>,
    hydrates: Vec<(String, Vec<u8>)>,
    err: Option<String>,
    need_resync: bool,
}

/// Background IPC: one `poll_ui` RTT per tick (keys + resize + snapshot + PTY bytes).
/// Keeps the egui thread off the SSH round-trip.
pub struct IpcPump {
    state: Arc<Mutex<PumpState>>,
    tx: mpsc::Sender<PumpCmd>,
}

impl IpcPump {
    pub fn start(initial: UiSnapshot) -> Result<Self, IpcError> {
        let client = IpcClient::connect()?;
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(PumpState {
            snap: initial,
            outputs: Vec::new(),
            hydrates: Vec::new(),
            err: None,
            need_resync: false,
        }));
        let st = state.clone();
        thread::Builder::new()
            .name("pmux-ipc-pump".into())
            .spawn(move || pump_loop(client, rx, st))
            .map_err(|e| IpcError::Io(e))?;
        Ok(Self { state, tx })
    }

    pub fn send_input(&self, _workspace: &str, pane: &str, bytes: &[u8]) {
        let _ = self.tx.send(PumpCmd::Input {
            pane: pane.into(),
            bytes: bytes.to_vec(),
        });
    }

    pub fn resize_pty(&self, _workspace: &str, pane: &str, cols: u16, rows: u16) {
        let _ = self.tx.send(PumpCmd::Resize {
            pane: pane.into(),
            cols,
            rows,
        });
    }

    pub fn hydrate_pty(&self, pane: &str, cols: u16, rows: u16) {
        let _ = self.tx.send(PumpCmd::Hydrate {
            pane: pane.into(),
            cols,
            rows,
        });
    }

    pub fn take_snap(&self) -> UiSnapshot {
        self.state.lock().unwrap().snap.clone()
    }

    pub fn take_outputs(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.state.lock().unwrap().outputs)
    }

    pub fn take_hydrates(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.state.lock().unwrap().hydrates)
    }

    pub fn last_error(&self) -> Option<String> {
        self.state.lock().unwrap().err.clone()
    }

    pub fn take_resync(&self) -> bool {
        let mut st = self.state.lock().unwrap();
        std::mem::take(&mut st.need_resync)
    }
}

fn pump_loop(
    client: IpcClient,
    rx: mpsc::Receiver<PumpCmd>,
    state: Arc<Mutex<PumpState>>,
) {
    let mut seen_gen = client.generation();
    let mut backoff = Duration::from_millis(200);
    let mut pending_hydrates: Vec<(String, u16, u16)> = Vec::new();
    loop {
        let mut inputs: Vec<PollInput> = Vec::new();
        let mut resizes: Vec<PollResize> = Vec::new();
        let mut hydrates: Vec<(String, u16, u16)> = Vec::new();
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(cmd) => apply_cmd(cmd, &mut inputs, &mut resizes, &mut hydrates),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(cmd) = rx.try_recv() {
            apply_cmd(cmd, &mut inputs, &mut resizes, &mut hydrates);
        }
        coalesce_inputs(&mut inputs);
        pending_hydrates.extend(hydrates);

        let ws = state.lock().unwrap().snap.active_id.clone();
        if let Some((pane, cols, rows)) = pending_hydrates.pop() {
            match ws.as_deref() {
                Some(ws) => {
                    let _ = client.resize_pty(ws, &pane, cols, rows);
                    match client.read_replay(ws, &pane) {
                        Ok(hist) => {
                            let _ = client.read_output(ws, &pane);
                            state.lock().unwrap().hydrates.push((pane, hist));
                        }
                        Err(_) => pending_hydrates.push((pane, cols, rows)),
                    }
                }
                None => pending_hydrates.push((pane, cols, rows)),
            }
        }
        match client.poll_ui(ws.as_deref(), inputs, resizes) {
            Ok(data) => {
                let gen = client.generation();
                let mut st = state.lock().unwrap();
                st.snap = data.snapshot;
                for o in data.outputs {
                    if !o.bytes.is_empty() {
                        st.outputs.push((o.pane, o.bytes));
                    }
                }
                st.err = None;
                if gen > seen_gen {
                    seen_gen = gen;
                    st.need_resync = true;
                }
                backoff = Duration::from_millis(200);
            }
            Err(e) => {
                state.lock().unwrap().err = Some(e.to_string());
                thread::sleep(backoff);
                backoff = (backoff * 2).min(Duration::from_secs(2));
            }
        }
    }
}

fn apply_cmd(
    cmd: PumpCmd,
    inputs: &mut Vec<PollInput>,
    resizes: &mut Vec<PollResize>,
    hydrates: &mut Vec<(String, u16, u16)>,
) {
    match cmd {
        PumpCmd::Input { pane, bytes } => inputs.push(PollInput { pane, bytes }),
        PumpCmd::Resize { pane, cols, rows } => {
            if let Some(r) = resizes.iter_mut().find(|r| r.pane == pane) {
                r.cols = cols;
                r.rows = rows;
            } else {
                resizes.push(PollResize { pane, cols, rows });
            }
        }
        PumpCmd::Hydrate { pane, cols, rows } => {
            if let Some(h) = hydrates.iter_mut().find(|h| h.0 == pane) {
                h.1 = cols;
                h.2 = rows;
            } else {
                hydrates.push((pane, cols, rows));
            }
        }
    }
}

fn coalesce_inputs(inputs: &mut Vec<PollInput>) {
    if inputs.len() < 2 {
        return;
    }
    let mut merged: Vec<PollInput> = Vec::new();
    for inp in inputs.drain(..) {
        if let Some(last) = merged.last_mut() {
            if last.pane == inp.pane {
                last.bytes.extend_from_slice(&inp.bytes);
                continue;
            }
        }
        merged.push(inp);
    }
    *inputs = merged;
}

fn unix_connect_timeout(path: &std::path::Path, timeout: Duration) -> std::io::Result<UnixStream> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{FromRawFd, RawFd};

    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let close_fd = |fd: RawFd| unsafe {
        libc::close(fd);
    };

    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        close_fd(fd);
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        close_fd(fd);
        return Err(std::io::Error::last_os_error());
    }
    let fdflags = unsafe { libc::fcntl(fd, libc::F_GETFD, 0) };
    if fdflags >= 0 {
        let _ = unsafe { libc::fcntl(fd, libc::F_SETFD, fdflags | libc::FD_CLOEXEC) };
    }

    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= addr.sun_path.len() {
        close_fd(fd);
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unix path too long",
        ));
    }
    for (i, b) in bytes.iter().enumerate() {
        addr.sun_path[i] = *b as libc::c_char;
    }

    let rc = unsafe {
        libc::connect(
            fd,
            std::ptr::addr_of!(addr) as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        let inprogress = err.kind() == std::io::ErrorKind::WouldBlock
            || err.raw_os_error() == Some(libc::EINPROGRESS);
        if !inprogress {
            close_fd(fd);
            return Err(err);
        }
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let ms = timeout.as_millis().clamp(1, i32::MAX as u128) as libc::c_int;
        let n = unsafe { libc::poll(&mut pfd, 1, ms) };
        if n == 0 {
            close_fd(fd);
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "unix connect timeout",
            ));
        }
        if n < 0 {
            let e = std::io::Error::last_os_error();
            close_fd(fd);
            return Err(e);
        }
        let mut so_err: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let gs = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                std::ptr::addr_of_mut!(so_err) as *mut libc::c_void,
                &mut len,
            )
        };
        if gs < 0 {
            let e = std::io::Error::last_os_error();
            close_fd(fd);
            return Err(e);
        }
        if so_err != 0 {
            close_fd(fd);
            return Err(std::io::Error::from_raw_os_error(so_err));
        }
    }

    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags) } < 0 {
        close_fd(fd);
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { UnixStream::from_raw_fd(fd) })
}
