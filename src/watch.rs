//! Runtime sensors. Diff host state → events. UI/plugins only subscribe.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use crate::monitor::{self, ListenPort, MonitorError};

const PORT_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Clone)]
pub struct PortDiff {
    pub opened: Vec<ListenPort>,
    pub closed: Vec<ListenPort>,
}

/// Listening-port sensor. One snapshot for the whole runtime (host-level).
pub struct PortWatch {
    rows: Vec<ListenPort>,
    error: Option<String>,
    last: Instant,
    interval: Duration,
}

impl PortWatch {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            error: None,
            last: Instant::now() - PORT_INTERVAL,
            interval: PORT_INTERVAL,
        }
    }

    pub fn rows(&self) -> &[ListenPort] {
        &self.rows
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn force(&mut self) {
        self.last = Instant::now() - self.interval;
    }

    /// Poll lsof if interval elapsed. `Some(diff)` when a poll ran.
    pub fn poll(&mut self) -> Option<PortDiff> {
        if self.last.elapsed() < self.interval {
            return None;
        }
        self.last = Instant::now();
        match monitor::list_listen_ports() {
            Ok(next) => {
                let diff = diff_listen_ports(&self.rows, &next);
                self.rows = next;
                self.error = None;
                Some(diff)
            }
            Err(MonitorError::Spawn(e) | MonitorError::Failed(e)) => {
                self.error = Some(e);
                Some(PortDiff::default())
            }
        }
    }
}

impl Default for PortWatch {
    fn default() -> Self {
        Self::new()
    }
}

pub fn diff_listen_ports(prev: &[ListenPort], next: &[ListenPort]) -> PortDiff {
    let prev_keys: HashSet<(&str, u32)> =
        prev.iter().map(|p| (p.addr.as_str(), p.pid)).collect();
    let next_keys: HashSet<(&str, u32)> =
        next.iter().map(|p| (p.addr.as_str(), p.pid)).collect();
    PortDiff {
        opened: next
            .iter()
            .filter(|p| !prev_keys.contains(&(p.addr.as_str(), p.pid)))
            .cloned()
            .collect(),
        closed: prev
            .iter()
            .filter(|p| !next_keys.contains(&(p.addr.as_str(), p.pid)))
            .cloned()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cmd: &str, pid: u32, addr: &str) -> ListenPort {
        ListenPort {
            command: cmd.into(),
            pid,
            addr: addr.into(),
        }
    }

    #[test]
    fn diff_detects_open_and_close() {
        let prev = vec![row("node", 1, "*:3000"), row("pg", 2, "127.0.0.1:5432")];
        let next = vec![row("node", 1, "*:3000"), row("vite", 3, "*:5173")];
        let diff = diff_listen_ports(&prev, &next);
        assert_eq!(diff.opened.len(), 1);
        assert_eq!(diff.opened[0].pid, 3);
        assert_eq!(diff.closed.len(), 1);
        assert_eq!(diff.closed[0].pid, 2);
    }

    #[test]
    fn diff_same_is_empty() {
        let rows = vec![row("node", 1, "*:3000")];
        let diff = diff_listen_ports(&rows, &rows);
        assert!(diff.opened.is_empty());
        assert!(diff.closed.is_empty());
    }
}
