//! Start or find the local runtime (`pwctl --daemon`).
//!
//! UI and CLI call this. They never become the daemon.

use std::fs;
use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::ipc;
use crate::ipc_client::IpcClient;
use crate::paths;

pub fn pid_path() -> PathBuf {
    paths::config_dir().join("daemon.pid")
}

pub fn log_path() -> PathBuf {
    paths::config_dir().join("daemon.log")
}

fn daemon_bin() -> io::Result<PathBuf> {
    paths::find_pwctl().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "pwctl not found — cargo build --bin pwctl",
        )
    })
}

/// Spawn `pwctl --daemon` if ping fails.
/// Never unlinks the socket while a live daemon answers ping.
/// With `$PMUX_HOST`, never spawns — the runtime is remote.
pub fn ensure_running() -> io::Result<()> {
    if IpcClient::ping() {
        return Ok(());
    }
    if let Some(addr) = ipc::tcp_connect_addr() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("cannot reach {addr} ($PMUX_HOST) — is pwctl start running there?"),
        ));
    }

    eprintln!(
        "no runtime on {} — starting daemon…",
        ipc::socket_path().display()
    );
    let _ = fs::remove_file(ipc::socket_path());
    let _ = fs::remove_file(pid_path());

    let exe = daemon_bin()?;
    let log = {
        paths::ensure_config_scaffold();
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
        format!("daemon did not start (see {})", log_path().display()),
    ))
}
