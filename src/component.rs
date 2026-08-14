use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::ids::PaneId;
use crate::terminal::{CursorState, RenderableCell, TerminalEmulator};
use crate::view::ViewComponent;
use crate::widgets::{JobComponent, PortsComponent, ProcessesComponent};

// ── Render output types (UI-agnostic) ───────────────────────────────

/// Color for styled text lines.
#[derive(Debug, Clone, Copy)]
pub struct TextColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl TextColor {
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
    pub fn white() -> Self {
        Self::rgb(211, 215, 207)
    }
    pub fn dim() -> Self {
        Self::rgb(100, 100, 110)
    }
    pub fn accent() -> Self {
        Self::rgb(99, 179, 237)
    }
}

/// A span of styled text.
#[derive(Debug, Clone)]
pub struct TextSpan {
    pub text: String,
    pub color: TextColor,
    pub bold: bool,
}

impl TextSpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            color: TextColor::white(),
            bold: false,
        }
    }

    pub fn colored(text: impl Into<String>, color: TextColor) -> Self {
        Self {
            text: text.into(),
            color,
            bold: false,
        }
    }
}

/// A line of styled text spans.
#[derive(Debug, Clone)]
pub struct StyledLine {
    pub spans: Vec<TextSpan>,
}

impl StyledLine {
    pub fn single(text: impl Into<String>, color: TextColor) -> Self {
        Self {
            spans: vec![TextSpan::colored(text, color)],
        }
    }

    pub fn multi(spans: Vec<TextSpan>) -> Self {
        Self { spans }
    }
}

/// What a component produces for the renderer.
#[derive(Debug)]
pub enum RenderOutput {
    /// Terminal grid (cells + cursor). Used by TerminalComponent.
    Grid {
        cells: Vec<RenderableCell>,
        cursor: CursorState,
        cols: usize,
        lines: usize,
        scroll_offset: usize,
    },
    /// Styled text lines. Used by scripts/plugins.
    Lines {
        header: Option<StyledLine>,
        subheader: Option<StyledLine>,
        lines: Vec<StyledLine>,
    },
    /// Nothing to render.
    Empty { message: String },
}

// ── Input types ─────────────────────────────────────────────────────

/// Input events sent to a component.
#[derive(Debug, Clone)]
pub enum ComponentInput {
    /// Raw key bytes (for terminal).
    KeyBytes(Vec<u8>),
    /// Mouse scroll (positive = up/history, negative = down).
    Scroll(i32),
    /// Pane resized to cols x lines.
    Resize { cols: usize, lines: usize },
    /// Focus gained.
    Focus,
    /// Focus lost.
    Blur,
}

// ── Component state ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ComponentState {
    Running,
    Stopped,
    Error(String),
}

// ── Component type tag ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ComponentType(pub String);

impl ComponentType {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ComponentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub const TERMINAL: &str = "terminal";
pub const SCRIPT: &str = "script";
pub const PORTS: &str = "ports";
pub const PROCESSES: &str = "processes";
pub const JOB: &str = "job";
pub const VIEW: &str = "view";

// ── The Component trait (spec §9) ───────────────────────────────────

pub trait Component: Send {
    /// Component type identifier.
    fn component_type(&self) -> &str;

    /// Human-readable name.
    fn display_name(&self) -> &str;

    /// Current state.
    fn state(&self) -> ComponentState;

    /// Produce render output (UI-agnostic).
    fn render(&self, cols: usize, lines: usize) -> RenderOutput;

    /// Handle input event.
    fn input(&mut self, input: ComponentInput);

    /// Actions this component exposes (for command palette / action registry).
    fn actions(&self) -> Vec<String> {
        vec![]
    }

    /// Periodic update (called each frame).
    fn tick(&mut self) {}

    /// Downcast support.
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

// ── TerminalComponent ───────────────────────────────────────────────

pub struct TerminalComponent {
    name: String,
    emulator: TerminalEmulator,
}

impl TerminalComponent {
    pub fn new(name: impl Into<String>, cols: usize, lines: usize) -> Self {
        Self {
            name: name.into(),
            emulator: TerminalEmulator::new(cols, lines),
        }
    }

    /// Feed raw PTY bytes into the emulator.
    pub fn process(&mut self, bytes: &[u8]) {
        self.emulator.process(bytes);
    }

    /// Visible screen text (for clipboard fallback).
    pub fn screen_text(&self) -> String {
        self.emulator.screen_text()
    }

    pub fn selection_begin(&mut self, col: usize, line: usize) {
        self.emulator.selection_begin(col, line);
    }

    pub fn selection_update(&mut self, col: usize, line: usize) {
        self.emulator.selection_update(col, line);
    }

    pub fn selection_clear(&mut self) {
        self.emulator.selection_clear();
    }

    pub fn selected_text(&self) -> Option<String> {
        self.emulator.selected_text()
    }

    /// Get current emulator dimensions.
    pub fn cols(&self) -> usize {
        self.emulator.cols()
    }

    pub fn lines(&self) -> usize {
        self.emulator.lines()
    }
}

impl Component for TerminalComponent {
    fn component_type(&self) -> &str {
        TERMINAL
    }

    fn display_name(&self) -> &str {
        &self.name
    }

    fn state(&self) -> ComponentState {
        ComponentState::Running
    }

    fn render(&self, cols: usize, lines: usize) -> RenderOutput {
        let _ = (cols, lines); // emulator already sized
        RenderOutput::Grid {
            cells: self.emulator.renderable_cells(),
            cursor: self.emulator.cursor(),
            cols: self.emulator.cols(),
            lines: self.emulator.lines(),
            scroll_offset: self.emulator.display_offset(),
        }
    }

    fn input(&mut self, input: ComponentInput) {
        match input {
            ComponentInput::Scroll(delta) => {
                self.emulator.scroll(delta);
            }
            ComponentInput::Resize { cols, lines } => {
                self.emulator.resize(cols, lines);
            }
            ComponentInput::KeyBytes(_) => {
                // Key input goes to PTY, not emulator directly.
                self.emulator.scroll_to_bottom();
                self.emulator.selection_clear();
            }
            _ => {}
        }
    }

    fn actions(&self) -> Vec<String> {
        vec!["send_command".into(), "clear".into()]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── ScriptComponent (Process Component / Plugin) ────────────────────

/// Configuration for a script/plugin component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_refresh")]
    pub refresh_secs: f64,
    #[serde(default)]
    pub description: String,
}

fn default_refresh() -> f64 {
    5.0
}

pub struct ScriptComponent {
    config: PluginConfig,
    output: Arc<Mutex<String>>,
    _handle: Option<std::thread::JoinHandle<()>>,
}

impl ScriptComponent {
    pub fn new(config: PluginConfig) -> Self {
        let output = Arc::new(Mutex::new(String::new()));
        let out_clone = output.clone();
        let cfg = config.clone();

        let handle = std::thread::spawn(move || loop {
            let result = std::process::Command::new(&cfg.command)
                .args(&cfg.args)
                .output();
            let text = match result {
                Ok(o) => {
                    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&o.stderr).to_string();
                    if stderr.is_empty() {
                        stdout
                    } else {
                        format!("{}\n--- stderr ---\n{}", stdout, stderr)
                    }
                }
                Err(e) => format!("Error running '{}': {}", cfg.command, e),
            };
            *out_clone.lock().unwrap() = text;
            std::thread::sleep(std::time::Duration::from_secs_f64(cfg.refresh_secs));
        });

        Self {
            config,
            output,
            _handle: Some(handle),
        }
    }

    pub fn config(&self) -> &PluginConfig {
        &self.config
    }
}

impl Component for ScriptComponent {
    fn component_type(&self) -> &str {
        SCRIPT
    }

    fn display_name(&self) -> &str {
        &self.config.name
    }

    fn state(&self) -> ComponentState {
        ComponentState::Running
    }

    fn render(&self, _cols: usize, _lines: usize) -> RenderOutput {
        let output = self.output.lock().unwrap().clone();
        let cmd_str = format!(
            "$ {} {}",
            self.config.command,
            self.config.args.join(" ")
        );

        let text_lines: Vec<StyledLine> = output
            .lines()
            .map(|l| StyledLine::single(l, TextColor::white()))
            .collect();

        RenderOutput::Lines {
            header: Some(StyledLine::multi(vec![
                TextSpan::colored("▶ ", TextColor::accent()),
                TextSpan::colored(
                    format!("{} — every {}s", self.config.name, self.config.refresh_secs),
                    TextColor::accent(),
                ),
            ])),
            subheader: Some(StyledLine::single(cmd_str, TextColor::dim())),
            lines: text_lines,
        }
    }

    fn input(&mut self, _input: ComponentInput) {
        // Scripts don't accept keyboard input
    }

    fn actions(&self) -> Vec<String> {
        vec!["refresh".into()]
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

// ── ComponentRegistry ───────────────────────────────────────────────

/// Manages live component instances indexed by PaneId.
/// Also loads plugin configs from disk.
pub struct ComponentRegistry {
    components: HashMap<PaneId, Box<dyn Component>>,
    plugin_configs: Vec<PluginConfig>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            components: HashMap::new(),
            plugin_configs: Vec::new(),
        }
    }

    /// Create a terminal component for a pane.
    pub fn create_terminal(
        &mut self,
        pane_id: PaneId,
        name: impl Into<String>,
        cols: usize,
        lines: usize,
    ) {
        let comp = TerminalComponent::new(name, cols, lines);
        self.components.insert(pane_id, Box::new(comp));
    }

    /// Create a script component for a pane.
    pub fn create_script(&mut self, pane_id: PaneId, config: PluginConfig) {
        let comp = ScriptComponent::new(config);
        self.components.insert(pane_id, Box::new(comp));
    }

    pub fn create_ports(&mut self, pane_id: PaneId, name: impl Into<String>) {
        self.components
            .insert(pane_id, Box::new(PortsComponent::new(name)));
    }

    pub fn create_processes(&mut self, pane_id: PaneId, name: impl Into<String>) {
        self.components
            .insert(pane_id, Box::new(ProcessesComponent::new(name)));
    }

    pub fn create_job(&mut self, pane_id: PaneId, name: impl Into<String>) {
        self.components
            .insert(pane_id, Box::new(JobComponent::new(name)));
    }

    pub fn create_view(&mut self, pane_id: PaneId, name: impl Into<String>) {
        self.components
            .insert(pane_id, Box::new(ViewComponent::new(name)));
    }

    pub fn get_ports_mut(&mut self, pane_id: &PaneId) -> Option<&mut PortsComponent> {
        self.components
            .get_mut(pane_id)
            .and_then(|c| c.as_any_mut().downcast_mut::<PortsComponent>())
    }

    pub fn get_processes_mut(&mut self, pane_id: &PaneId) -> Option<&mut ProcessesComponent> {
        self.components
            .get_mut(pane_id)
            .and_then(|c| c.as_any_mut().downcast_mut::<ProcessesComponent>())
    }

    pub fn get_job_mut(&mut self, pane_id: &PaneId) -> Option<&mut JobComponent> {
        self.components
            .get_mut(pane_id)
            .and_then(|c| c.as_any_mut().downcast_mut::<JobComponent>())
    }

    pub fn get_view_mut(&mut self, pane_id: &PaneId) -> Option<&mut ViewComponent> {
        self.components
            .get_mut(pane_id)
            .and_then(|c| c.as_any_mut().downcast_mut::<ViewComponent>())
    }

    /// Get component ref.
    pub fn get(&self, pane_id: &PaneId) -> Option<&Box<dyn Component>> {
        self.components.get(pane_id)
    }

    /// Get component mut ref.
    pub fn get_mut(&mut self, pane_id: &PaneId) -> Option<&mut Box<dyn Component>> {
        self.components.get_mut(pane_id)
    }

    /// Downcast to TerminalComponent (for PTY data feed).
    pub fn get_terminal_mut(&mut self, pane_id: &PaneId) -> Option<&mut TerminalComponent> {
        self.components.get_mut(pane_id).and_then(|c| {
            c.as_any_mut().downcast_mut::<TerminalComponent>()
        })
    }

    /// Remove a component.
    pub fn remove(&mut self, pane_id: &PaneId) -> bool {
        self.components.remove(pane_id).is_some()
    }

    /// List all pane IDs with components.
    pub fn pane_ids(&self) -> Vec<&PaneId> {
        self.components.keys().collect()
    }

    /// Number of live components.
    pub fn count(&self) -> usize {
        self.components.len()
    }

    // ── Plugin loading from disk ────────────────────────────────────

    /// Load plugin configs from ~/.config/pworkspaces/plugins/*.toml
    pub fn load_plugins(&mut self) {
        let dir = plugin_dir();
        if !dir.exists() {
            // Create dir + example plugin
            let _ = std::fs::create_dir_all(&dir);
            let example = PluginConfig {
                name: "Clock".into(),
                command: "date".into(),
                args: vec!["+%Y-%m-%d %H:%M:%S".into()],
                refresh_secs: 1.0,
                description: "Shows current date and time".into(),
            };
            let toml = toml::to_string_pretty(&example).unwrap_or_default();
            let _ = std::fs::write(dir.join("clock.toml"), toml);
        }

        self.plugin_configs.clear();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "toml") {
                    match std::fs::read_to_string(&path) {
                        Ok(contents) => match toml::from_str::<PluginConfig>(&contents) {
                            Ok(config) => {
                                self.plugin_configs.push(config);
                            }
                            Err(e) => {
                                eprintln!("Failed to parse plugin {:?}: {}", path, e);
                            }
                        },
                        Err(e) => {
                            eprintln!("Failed to read plugin {:?}: {}", path, e);
                        }
                    }
                }
            }
        }
    }

    /// Available plugin configs (loaded from disk).
    pub fn available_plugins(&self) -> &[PluginConfig] {
        &self.plugin_configs
    }
}

impl std::fmt::Debug for ComponentRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentRegistry")
            .field("components", &self.components.len())
            .field("plugins", &self.plugin_configs.len())
            .finish()
    }
}

fn plugin_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("pworkspaces")
        .join("plugins")
}
