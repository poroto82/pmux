//! Headless runtime: owns PTYs + IPC. UI attaches/detaches.

use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use pmux::attach;
use pmux::ids::ComponentId;
use pmux::ipc;
use pmux::ipc_client::IpcClient;
use pmux::paths;
use pmux::token;

use crate::ipc_server;
use crate::runtime::Runtime;

/// Foreground daemon loop (`pwctl --daemon`).
pub fn run() -> ! {
    if IpcClient::ping() {
        eprintln!("pmux daemon already running");
        std::process::exit(0);
    }

    paths::ensure_config_scaffold();
    if let Err(e) = token::ensure(false) {
        eprintln!("token: {e}");
        std::process::exit(1);
    }
    let _ = fs::remove_file(ipc::socket_path());
    if let Ok(pid) = fs::File::create(attach::pid_path()) {
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
    if let Err(e) = ipc_server::start_with_shutdown(runtime.clone(), Some(shutdown.clone())) {
        eprintln!("ipc bind failed: {e}");
        let _ = fs::remove_file(attach::pid_path());
        std::process::exit(1);
    }
    eprintln!(
        "pmux daemon pid={} sock={}{}",
        std::process::id(),
        ipc::socket_path().display(),
        ipc::tcp_listen_addr()
            .map(|a| format!(" tcp={a}"))
            .unwrap_or_default()
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
    let _ = fs::remove_file(ipc::socket_path());
    let _ = fs::remove_file(attach::pid_path());
    eprintln!("pmux daemon stopped");
    std::process::exit(0);
}
