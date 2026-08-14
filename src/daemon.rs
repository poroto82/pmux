//! Headless runtime: owns PTYs + IPC. UI attaches/detaches.

use std::fs;
use std::io::{self, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ids::ComponentId;
use crate::ipc_client::IpcClient;
use crate::paths;
use crate::runtime::Runtime;

pub fn pid_path() -> PathBuf {
    paths::config_dir().join("daemon.pid")
}

pub fn log_path() -> PathBuf {
    paths::config_dir().join("daemon.log")
}

fn daemon_bin() -> io::Result<PathBuf> {
    paths::find_pwctl()
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "cannot find pwctl or current exe"))
}

/// Spawn `pwctl --daemon` (fallback: current exe) if ping fails.
/// Never unlinks the socket while a live daemon answers ping.
pub fn ensure_running() -> io::Result<()> {
    if IpcClient::ping() {
        return Ok(());
    }

    eprintln!("no runtime on {} — starting daemon…", crate::ipc::socket_path().display());
    let _ = fs::remove_file(crate::ipc::socket_path());
    let _ = fs::remove_file(pid_path());

    let exe = daemon_bin()?;
    let log = {
        let _ = fs::create_dir_all(paths::config_dir());
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path())?
    };
    let err = log.try_clone()?;

    Command::new(&exe)
        .arg("--daemon")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(err))
        .process_group(0)
        .spawn()?;

    if IpcClient::wait_ready(Duration::from_secs(8)) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        format!(
            "daemon did not start (see {})",
            log_path().display()
        ),
    ))
}

/// Foreground daemon loop (`pwctl --daemon` / `pmux --daemon`).
pub fn run() -> ! {
    if IpcClient::ping() {
        eprintln!("pmux daemon already running");
        std::process::exit(0);
    }

    let _ = fs::create_dir_all(paths::config_dir());
    let _ = fs::remove_file(crate::ipc::socket_path());
    if let Ok(pid) = fs::File::create(pid_path()) {
        let _ = writeln!(&pid, "{}", std::process::id());
    }

    let runtime = Arc::new(Mutex::new(Runtime::with_defaults()));
    {
        let mut rt = runtime.lock().unwrap();
        let restored = rt.restore().is_ok() && rt.workspaces.count() > 0;
        if restored {
            rt.respawn_sessions();
        } else {
            let name = rt.next_workspace_name();
            let ws_id = rt.create_workspace(&name);
            let _ = rt.add_pane(&ws_id, ComponentId::new(), None, true);
        }
        let _ = rt.save();
    }

    let shutdown = Arc::new(AtomicBool::new(false));
    if let Err(e) =
        crate::ipc_server::start_with_shutdown(runtime.clone(), Some(shutdown.clone()))
    {
        eprintln!("ipc bind failed: {e}");
        let _ = fs::remove_file(pid_path());
        std::process::exit(1);
    }
    eprintln!(
        "pmux daemon pid={} sock={}",
        std::process::id(),
        crate::ipc::socket_path().display()
    );

    let mut last_save = Instant::now();
    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        {
            let mut rt = runtime.lock().unwrap();
            rt.tick_watchers();
            rt.reap_dead_panes();
            if last_save.elapsed() > Duration::from_secs(30) {
                let _ = rt.save();
                last_save = Instant::now();
            }
        }
        std::thread::sleep(Duration::from_millis(80));
    }

    {
        let rt = runtime.lock().unwrap();
        let _ = rt.save();
    }
    let _ = fs::remove_file(crate::ipc::socket_path());
    let _ = fs::remove_file(pid_path());
    eprintln!("pmux daemon stopped");
    std::process::exit(0);
}
