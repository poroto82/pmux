//! First-class monitor / job components (spec §10 Ports + Processes).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::component::{
    Component, ComponentInput, ComponentState, RenderOutput, StyledLine, TextColor, JOB, PORTS,
    PROCESSES,
};
use crate::monitor::{self, ListenPort, ProcRow};

const REFRESH: Duration = Duration::from_secs(2);

// ── Ports ───────────────────────────────────────────────────────────
// View over Runtime::listen_ports(). Sensor lives in `watch::PortWatch`.

pub struct PortsComponent {
    name: String,
    rows: Vec<ListenPort>,
    error: Option<String>,
    flash: Option<String>,
}

impl PortsComponent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            rows: Vec::new(),
            error: None,
            flash: None,
        }
    }

    pub fn rows(&self) -> &[ListenPort] {
        &self.rows
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn flash(&self) -> Option<&str> {
        self.flash.as_deref()
    }

    /// Pull snapshot from runtime watcher (no lsof here).
    pub fn sync_from(&mut self, rows: &[ListenPort], error: Option<&str>) {
        self.rows = rows.to_vec();
        self.error = error.map(|e| e.to_string());
    }

    pub fn pid_at(&self, index: usize) -> Option<(u32, String)> {
        self.rows
            .get(index)
            .map(|r| (r.pid, r.addr.clone()))
    }

    pub fn set_flash(&mut self, msg: impl Into<String>) {
        self.flash = Some(msg.into());
    }
}

impl Component for PortsComponent {
    fn component_type(&self) -> &str {
        PORTS
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ComponentState {
        ComponentState::Running
    }
    fn render(&self, _cols: usize, _lines: usize) -> RenderOutput {
        let mut lines: Vec<StyledLine> = Vec::new();
        if let Some(err) = &self.error {
            lines.push(StyledLine::single(err, TextColor::rgb(239, 100, 100)));
        }
        for r in &self.rows {
            lines.push(StyledLine::single(
                format!("{:<8} {:>6}  {}", r.command, r.pid, r.addr),
                TextColor::white(),
            ));
        }
        RenderOutput::Lines {
            header: Some(StyledLine::single(
                format!("ports — {} listening", self.rows.len()),
                TextColor::accent(),
            )),
            subheader: Some(StyledLine::single(
                "click row → SIGTERM    (runtime port watch)",
                TextColor::dim(),
            )),
            lines,
        }
    }
    fn input(&mut self, _input: ComponentInput) {}
    fn tick(&mut self) {}
    fn actions(&self) -> Vec<String> {
        vec!["refresh".into(), "kill".into()]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── Processes ───────────────────────────────────────────────────────

pub struct ProcessesComponent {
    name: String,
    rows: Vec<ProcRow>,
    error: Option<String>,
    flash: Option<String>,
    last: Instant,
}

impl ProcessesComponent {
    pub fn new(name: impl Into<String>) -> Self {
        let mut c = Self {
            name: name.into(),
            rows: Vec::new(),
            error: None,
            flash: None,
            last: Instant::now() - REFRESH,
        };
        c.refresh();
        c
    }

    pub fn rows(&self) -> &[ProcRow] {
        &self.rows
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn flash(&self) -> Option<&str> {
        self.flash.as_deref()
    }

    pub fn refresh(&mut self) {
        match monitor::list_processes(40) {
            Ok(rows) => {
                self.rows = rows;
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        self.last = Instant::now();
    }

    pub fn kill_at(&mut self, index: usize) {
        let Some(row) = self.rows.get(index) else { return };
        let pid = row.pid;
        let cmd = row.command.clone();
        match monitor::kill_pid(pid) {
            Ok(()) => self.flash = Some(format!("SIGTERM {} ({})", pid, short_cmd(&cmd))),
            Err(e) => self.flash = Some(format!("kill {}: {}", pid, e)),
        }
        self.refresh();
    }
}

impl Component for ProcessesComponent {
    fn component_type(&self) -> &str {
        PROCESSES
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ComponentState {
        ComponentState::Running
    }
    fn render(&self, _cols: usize, _lines: usize) -> RenderOutput {
        let mut lines = Vec::new();
        if let Some(err) = &self.error {
            lines.push(StyledLine::single(err, TextColor::rgb(239, 100, 100)));
        }
        for r in &self.rows {
            lines.push(StyledLine::single(
                format!(
                    "{:>6} {:>5.1} {:>5.1}  {}",
                    r.pid,
                    r.cpu,
                    r.mem,
                    short_cmd(&r.command)
                ),
                TextColor::white(),
            ));
        }
        RenderOutput::Lines {
            header: Some(StyledLine::single("processes — by CPU", TextColor::accent())),
            subheader: Some(StyledLine::single(
                "PID    %CPU  %MEM   click row → SIGTERM",
                TextColor::dim(),
            )),
            lines,
        }
    }
    fn input(&mut self, _input: ComponentInput) {}
    fn tick(&mut self) {
        if self.last.elapsed() >= REFRESH {
            self.refresh();
        }
    }
    fn actions(&self) -> Vec<String> {
        vec!["refresh".into(), "kill".into()]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── Job (start / stop) ──────────────────────────────────────────────

pub struct JobComponent {
    name: String,
    command: String,
    running: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
    output: Arc<Mutex<String>>,
    status: Arc<Mutex<String>>,
}

impl JobComponent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: String::new(),
            running: Arc::new(AtomicBool::new(false)),
            child: Arc::new(Mutex::new(None)),
            output: Arc::new(Mutex::new(String::new())),
            status: Arc::new(Mutex::new("idle".into())),
        }
    }

    pub fn command(&self) -> &str {
        &self.command
    }

    pub fn set_command(&mut self, cmd: String) {
        if !self.is_running() {
            self.command = cmd;
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn status_text(&self) -> String {
        self.status.lock().unwrap().clone()
    }

    pub fn output_text(&self) -> String {
        self.output.lock().unwrap().clone()
    }

    pub fn start(&mut self) {
        if self.is_running() {
            return;
        }
        let cmd = self.command.trim().to_string();
        if cmd.is_empty() {
            *self.status.lock().unwrap() = "empty command".into();
            return;
        }

        *self.output.lock().unwrap() = String::new();
        *self.status.lock().unwrap() = "starting".into();

        let mut child = match Command::new("sh")
            .arg("-lc")
            .arg(&cmd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                *self.status.lock().unwrap() = format!("spawn failed: {}", e);
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        *self.child.lock().unwrap() = Some(child);
        self.running.store(true, Ordering::SeqCst);
        *self.status.lock().unwrap() = "running".into();

        let out_buf = self.output.clone();
        let running = self.running.clone();
        let status = self.status.clone();
        let child_slot = self.child.clone();

        thread::spawn(move || {
            if let Some(mut so) = stdout {
                let buf = out_buf.clone();
                thread::spawn(move || pipe_into(&mut so, &buf));
            }
            if let Some(mut se) = stderr {
                let buf = out_buf.clone();
                thread::spawn(move || pipe_into(&mut se, &buf));
            }

            let code = child_slot
                .lock()
                .unwrap()
                .as_mut()
                .and_then(|c| c.wait().ok())
                .and_then(|s| s.code());
            running.store(false, Ordering::SeqCst);
            *status.lock().unwrap() = match code {
                Some(0) => "exited 0".into(),
                Some(c) => format!("exited {}", c),
                None => "stopped".into(),
            };
            *child_slot.lock().unwrap() = None;
        });
    }

    pub fn stop(&mut self) {
        if let Some(child) = self.child.lock().unwrap().as_mut() {
            let _ = child.kill();
        }
        self.running.store(false, Ordering::SeqCst);
        *self.status.lock().unwrap() = "stopped".into();
    }
}

fn pipe_into<R: Read>(reader: &mut R, buf: &Arc<Mutex<String>>) {
    let mut tmp = [0u8; 4096];
    loop {
        match reader.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&tmp[..n]);
                let mut out = buf.lock().unwrap();
                out.push_str(&chunk);
                if out.len() > 64 * 1024 {
                    let start = out.len() - 32 * 1024;
                    *out = out[start..].to_string();
                }
            }
            Err(_) => break,
        }
    }
}

impl Component for JobComponent {
    fn component_type(&self) -> &str {
        JOB
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ComponentState {
        if self.is_running() {
            ComponentState::Running
        } else {
            ComponentState::Stopped
        }
    }
    fn render(&self, _cols: usize, _lines: usize) -> RenderOutput {
        let lines: Vec<StyledLine> = self
            .output_text()
            .lines()
            .map(|l| StyledLine::single(l, TextColor::white()))
            .collect();
        RenderOutput::Lines {
            header: Some(StyledLine::single(
                format!("job [{}]  {}", self.status_text(), self.command),
                TextColor::accent(),
            )),
            subheader: Some(StyledLine::single(
                "start / stop from pane UI",
                TextColor::dim(),
            )),
            lines,
        }
    }
    fn input(&mut self, _input: ComponentInput) {}
    fn actions(&self) -> Vec<String> {
        vec!["start".into(), "stop".into()]
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn short_cmd(cmd: &str) -> String {
    const MAX: usize = 48;
    if cmd.len() <= MAX {
        cmd.to_string()
    } else {
        format!("{}…", &cmd[..MAX - 1])
    }
}
