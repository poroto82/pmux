//! Host monitors: listening ports + processes. macOS/Linux via `lsof`/`ps`.

use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ListenPort {
    pub command: String,
    pub pid: u32,
    pub addr: String,
}

impl ListenPort {
    /// Parse port from `*:3000`, `127.0.0.1:5432`, `[::1]:3000`.
    pub fn port(&self) -> Option<u16> {
        parse_listen_port(&self.addr)
    }
}

pub fn parse_listen_port(addr: &str) -> Option<u16> {
    addr.rsplit_once(':')?.1.parse().ok()
}

#[derive(Debug, Clone)]
pub struct ProcRow {
    pub pid: u32,
    pub cpu: f32,
    pub mem: f32,
    pub rss_kb: u64,
    pub command: String,
}

#[derive(Debug)]
pub enum MonitorError {
    Spawn(String),
    Failed(String),
}

impl std::fmt::Display for MonitorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MonitorError::Spawn(e) => write!(f, "spawn: {}", e),
            MonitorError::Failed(e) => write!(f, "{}", e),
        }
    }
}

/// Listening TCP ports (`lsof -nP -iTCP -sTCP:LISTEN`).
pub fn list_listen_ports() -> Result<Vec<ListenPort>, MonitorError> {
    let out = Command::new("lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN"])
        .output()
        .map_err(|e| MonitorError::Spawn(e.to_string()))?;
    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(MonitorError::Failed(err.trim().to_string()));
    }
    Ok(parse_lsof_listen(&String::from_utf8_lossy(&out.stdout)))
}

/// Parse `lsof -nP -iTCP -sTCP:LISTEN` stdout.
pub fn parse_lsof_listen(text: &str) -> Vec<ListenPort> {
    let mut rows = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        let command = cols[0].to_string();
        let Ok(pid) = cols[1].parse::<u32>() else { continue };
        let addr = cols[8].trim_end_matches("(LISTEN)").trim().to_string();
        if addr.is_empty() {
            continue;
        }
        rows.push(ListenPort { command, pid, addr });
    }
    rows.sort_by(|a, b| a.addr.cmp(&b.addr).then(a.pid.cmp(&b.pid)));
    rows.dedup_by(|a, b| a.pid == b.pid && a.addr == b.addr);
    rows
}

/// Top processes by CPU (`ps`).
pub fn list_processes(limit: usize) -> Result<Vec<ProcRow>, MonitorError> {
    let out = Command::new("ps")
        .args(["-axo", "pid=,pcpu=,pmem=,rss=,comm="])
        .output()
        .map_err(|e| MonitorError::Spawn(e.to_string()))?;
    if !out.status.success() && out.stdout.is_empty() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(MonitorError::Failed(err.trim().to_string()));
    }
    let mut rows = parse_ps(&String::from_utf8_lossy(&out.stdout));
    rows.sort_by(|a, b| {
        b.cpu
            .partial_cmp(&a.cpu)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(limit.max(1));
    Ok(rows)
}

/// Parse `ps -axo pid=,pcpu=,pmem=,rss=,comm=` stdout.
pub fn parse_ps(text: &str) -> Vec<ProcRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(pid_s) = parts.next() else { continue };
        let Some(cpu_s) = parts.next() else { continue };
        let Some(mem_s) = parts.next() else { continue };
        let Some(rss_s) = parts.next() else { continue };
        let cmd: Vec<&str> = parts.collect();
        if cmd.is_empty() {
            continue;
        }
        let Ok(pid) = pid_s.parse::<u32>() else { continue };
        let Ok(cpu) = cpu_s.parse::<f32>() else { continue };
        let Ok(mem) = mem_s.parse::<f32>() else { continue };
        let Ok(rss_kb) = rss_s.parse::<u64>() else { continue };
        rows.push(ProcRow {
            pid,
            cpu,
            mem,
            rss_kb,
            command: cmd.join(" "),
        });
    }
    rows
}

/// Live cwd of a process (`lsof` on macOS, `/proc` on Linux).
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let out = Command::new("lsof")
            .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn", "-w"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some(path) = line.strip_prefix('n') {
                let p = PathBuf::from(path);
                if p.is_dir() {
                    return Some(p);
                }
            }
        }
        None
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// SIGTERM a pid. Returns stderr/ok message.
pub fn kill_pid(pid: u32) -> Result<(), MonitorError> {
    let out = Command::new("kill")
        .args(["-TERM", &pid.to_string()])
        .output()
        .map_err(|e| MonitorError::Spawn(e.to_string()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(MonitorError::Failed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_lsof_sample() {
        let sample = "\
COMMAND   PID USER   FD   TYPE DEVICE SIZE/OFF NODE NAME
node    12345 me    23u  IPv4  0x1      0t0  TCP *:3000 (LISTEN)
node    12345 me    24u  IPv6  0x2      0t0  TCP [::1]:3000 (LISTEN)
postgres  99 me    10u  IPv4  0x3      0t0  TCP 127.0.0.1:5432 (LISTEN)
";
        let rows = parse_lsof_listen(sample);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].addr, "*:3000");
        assert_eq!(rows[0].pid, 12345);
        assert_eq!(rows[0].port(), Some(3000));
        assert!(rows.iter().any(|r| r.addr.contains("5432")));
        assert_eq!(parse_listen_port("[::1]:3000"), Some(3000));
    }

    #[test]
    fn parse_ps_sample() {
        let sample = "\
  1  0.0  0.1   4096 /sbin/launchd
 42 12.5  2.0  81920 /usr/bin/node server.js
";
        let rows = parse_ps(sample);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].pid, 42);
        assert!((rows[1].cpu - 12.5).abs() < f32::EPSILON);
        assert_eq!(rows[1].command, "/usr/bin/node server.js");
    }

    #[test]
    fn process_cwd_self() {
        let pid = std::process::id();
        let cwd = process_cwd(pid).expect("cwd of test process");
        assert!(cwd.is_dir());
        let expected = std::env::current_dir().unwrap();
        assert_eq!(cwd.canonicalize().unwrap(), expected.canonicalize().unwrap());
    }
}
