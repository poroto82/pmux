use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use eframe::egui;
use pworkspaces::component::{Component, ComponentInput, ComponentRegistry, RenderOutput};
use pworkspaces::ids::{PaneId, WorkspaceId};
use pworkspaces::ipc::{PaneSnap, UiSnapshot};
use pworkspaces::ipc_client::IpcClient;
use pworkspaces::layout::LayoutNode;
use pworkspaces::palette;
use pworkspaces::terminal::TermColor;
use pworkspaces::view::ViewNav;
use raw_window_handle::HasWindowHandle;

const APP_TITLE: &str = "poroto-workspace";

fn main() -> eframe::Result {
    if std::env::args().any(|a| a == "--daemon") {
        pworkspaces::daemon::run();
    }

    pworkspaces::paths::ensure_pwctl_built();
    let already = pworkspaces::ipc_client::IpcClient::ping();
    if let Err(e) = pworkspaces::daemon::ensure_running() {
        eprintln!("cannot start runtime daemon: {e}");
        std::process::exit(1);
    }
    if already {
        eprintln!(
            "UI attach → {} (runtime already up)",
            pworkspaces::ipc::socket_path().display()
        );
    } else {
        eprintln!(
            "UI attach → {} (started runtime)",
            pworkspaces::ipc::socket_path().display()
        );
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 840.0])
            .with_title(APP_TITLE)
            // macOS: paint behind traffic lights so titlebar matches window bg.
            .with_fullsize_content_view(true)
            .with_title_shown(false)
            .with_titlebar_shown(false)
            .with_titlebar_buttons_shown(true),
        ..Default::default()
    };

    eframe::run_native(
        APP_TITLE,
        options,
        Box::new(|cc| {
            let mut fonts = egui::FontDefinitions::default();

            let nerd_font_path =
                dirs::home_dir().unwrap().join("Library/Fonts/JetBrainsMonoNerdFont-Regular.ttf");
            if nerd_font_path.exists() {
                if let Ok(data) = std::fs::read(&nerd_font_path) {
                    fonts.font_data.insert(
                        "nerd".to_owned(),
                        std::sync::Arc::new(egui::FontData::from_owned(data)),
                    );
                    fonts
                        .families
                        .entry(egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, "nerd".to_owned());
                }
            } else {
                fonts.font_data.insert(
                    "mono".to_owned(),
                    std::sync::Arc::new(egui::FontData::from_static(
                        include_bytes!("/System/Library/Fonts/Menlo.ttc"),
                    )),
                );
                fonts
                    .families
                    .entry(egui::FontFamily::Monospace)
                    .or_default()
                    .insert(0, "mono".to_owned());
            }

            cc.egui_ctx.set_fonts(fonts);
            let mut visuals = egui::Visuals::dark();
            visuals.panel_fill = bg_app();
            visuals.window_fill = bg_pane();
            visuals.extreme_bg_color = bg_app();
            visuals.override_text_color = Some(text());
            visuals.widgets.noninteractive.bg_fill = bg_title();
            visuals.widgets.inactive.bg_fill = bg_title();
            visuals.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(42, 212, 163, 70);
            visuals.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
            visuals.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
            visuals.window_stroke = egui::Stroke::NONE;
            cc.egui_ctx.set_visuals(visuals);
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            Ok(Box::new(WorkspaceApp::new()))
        }),
    )
}

/// Snapshot of floating pane position for rendering (no lock needed).
#[derive(Clone)]
struct FloatingInfo {
    pane_id: PaneId,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

/// A split border that can be dragged to resize.
#[derive(Clone)]
struct SplitBorder {
    /// Screen rect of the draggable border zone.
    rect: egui::Rect,
    /// Parent rect that contains both children.
    parent_rect: egui::Rect,
    /// Direction of the split (Horizontal = vertical divider, Vertical = horizontal divider).
    direction: pworkspaces::layout::Direction,
    /// Depth-first index of this split in the layout tree.
    split_index: usize,
}

struct WorkspaceApp {
    client: IpcClient,
    snap: UiSnapshot,
    /// All visual components indexed by PaneId.
    components: ComponentRegistry,
    /// Active workspace for the UI.
    active_ws: WorkspaceId,
    status: String,
    /// Collected split borders from last frame for mouse interaction.
    split_borders: Vec<SplitBorder>,
    /// Currently dragging a split border (index into split_borders).
    dragging_split: Option<usize>,
    /// Pane rects from last frame for click-to-focus.
    pane_rects: HashMap<PaneId, egui::Rect>,
    /// Currently dragging a floating pane (pane_id, offset from pane origin).
    dragging_float: Option<(PaneId, egui::Vec2)>,
    /// Currently resizing a floating pane from edge.
    resizing_float: Option<PaneId>,
    /// Drag-to-swap: source pane being dragged by title bar.
    dragging_pane: Option<PaneId>,
    /// Terminal mouse selection in progress.
    selecting_term: Option<PaneId>,
    /// Drop target pane highlighted during drag-to-swap.
    swap_target: Option<PaneId>,
    /// Fractional trackpad leftover (pixels → terminal lines).
    scroll_accum: f32,
    /// Menu + status bar. Toggle: Ctrl/⌘+Shift+M.
    chrome_visible: bool,
    palette_kind: PaletteKind,
    palette_query: String,
    palette_index: usize,
    palette_focus_search: bool,
    file_root: Option<PathBuf>,
    file_root_edit: String,
    file_index: Vec<String>,
    renaming_ws: Option<WorkspaceId>,
    rename_buf: String,
    rename_focus: bool,
    want_close: bool,
    /// How this UI process is leaving. ✕ = kill runtime; ⌘⇧D = detach.
    leave: LeaveMode,
    /// Terminals waiting to replay PTY at the real pane size (avoids zsh `%` reflow).
    hydrate_pending: HashSet<PaneId>,
    /// View pane slots collected this frame (native WebView overlay).
    view_slots: HashMap<PaneId, ViewSlot>,
    overlays: HashMap<PaneId, WebOverlay>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LeaveMode {
    /// Still running / crash — Drop saves, leaves daemon.
    Running,
    /// ⌘⇧D / palette detach — close UI, keep daemon.
    Detach,
    /// Window ✕ / kill_runtime — stop daemon + PTYs.
    Kill,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaletteKind {
    Closed,
    Commands,
    Files,
}

impl PaletteKind {
    fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }
}

#[derive(Clone)]
struct ViewSlot {
    rect: egui::Rect,
    nav: ViewNav,
}

struct WebOverlay {
    webview: wry::WebView,
    loaded_key: String,
}

const PW_WEBVIEW_JS: &str = r#"
window.pworkspaces = {
  version: "0.1",
  post(msg) {
    try {
      const s = typeof msg === "string" ? msg : JSON.stringify(msg);
      window.ipc.postMessage(s);
    } catch (e) { console.error(e); }
  }
};
"#;

/// Kitty-ish chrome: mint active border. Pane fill follows Kitty theme bg.
const FOCUSED_BORDER: egui::Color32 = egui::Color32::from_rgb(42, 212, 163);
/// macOS traffic-light inset when content draws under the titlebar.
#[cfg(target_os = "macos")]
const TRAFFIC_LIGHTS_W: f32 = 76.0;
#[cfg(not(target_os = "macos"))]
const TRAFFIC_LIGHTS_W: f32 = 0.0;
const NORMAL_BORDER: egui::Color32 = egui::Color32::from_rgb(18, 18, 18);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(42, 212, 163);

fn tc(c: TermColor) -> egui::Color32 {
    egui::Color32::from_rgb(c.r, c.g, c.b)
}

fn bg_app() -> egui::Color32 {
    tc(palette::global().background)
}
fn bg_pane() -> egui::Color32 {
    tc(palette::global().background)
}
fn bg_title() -> egui::Color32 {
    tc(palette::global().background)
}
fn text() -> egui::Color32 {
    tc(palette::global().foreground)
}
fn text_dim() -> egui::Color32 {
    tc(palette::global().ansi(8))
}
fn cursor_fill() -> egui::Color32 {
    let c = palette::global().cursor;
    egui::Color32::from_rgba_unmultiplied(c.r, c.g, c.b, 160)
}
const TITLE_H: f32 = 28.0;
/// Extra gap between pane title strip and terminal cells.
const TITLE_GAP: f32 = 8.0;

const INITIAL_COLS: usize = 80;
const INITIAL_LINES: usize = 24;
const CELL_WIDTH: f32 = 8.0;
const CELL_HEIGHT: f32 = 16.0;
const FONT_SIZE: f32 = 13.0;
const PADDING_X: f32 = 12.0;
const PADDING_Y: f32 = 10.0;

fn term_color_to_egui(c: TermColor) -> egui::Color32 {
    tc(c)
}

fn key_to_pty_bytes(key: &egui::Key, modifiers: &egui::Modifiers) -> Option<Vec<u8>> {
    // Control chars use Ctrl only — not Cmd. On macOS Cmd+C/V are clipboard;
    // Ctrl+C remains SIGINT.
    if modifiers.mac_cmd && !modifiers.ctrl {
        return None;
    }

    // Ctrl+Shift+C/V are clipboard in many terminals; don't send to PTY.
    if modifiers.ctrl && modifiers.shift {
        match key {
            egui::Key::C | egui::Key::V => return None,
            _ => {}
        }
    }

    if modifiers.shift && *key == egui::Key::Tab && !modifiers.ctrl && !modifiers.mac_cmd {
        return Some(b"\x1b[Z".to_vec());
    }

    if modifiers.ctrl {
        let byte = match key {
            egui::Key::A => Some(0x01),
            egui::Key::B => Some(0x02),
            egui::Key::C => Some(0x03),
            egui::Key::D => Some(0x04),
            egui::Key::E => Some(0x05),
            egui::Key::G => Some(0x07),
            egui::Key::H => Some(0x08),
            egui::Key::I => Some(0x09),
            egui::Key::J => Some(0x0a),
            egui::Key::K => Some(0x0b),
            egui::Key::L => Some(0x0c),
            egui::Key::M => Some(0x0d),
            egui::Key::N => Some(0x0e),
            egui::Key::O => Some(0x0f),
            egui::Key::P => Some(0x10),
            egui::Key::Q => Some(0x11),
            egui::Key::R => Some(0x12),
            egui::Key::S => Some(0x13),
            egui::Key::T => Some(0x14),
            egui::Key::U => Some(0x15),
            egui::Key::V => Some(0x16),
            egui::Key::W => Some(0x17),
            egui::Key::X => Some(0x18),
            egui::Key::Y => Some(0x19),
            egui::Key::Z => Some(0x1a),
            _ => None,
        };
        return byte.map(|b| vec![b]);
    }

    match key {
        egui::Key::Enter => Some(b"\r".to_vec()),
        egui::Key::Tab => Some(b"\t".to_vec()),
        egui::Key::Backspace => Some(vec![0x7f]),
        egui::Key::Delete => Some(b"\x1b[3~".to_vec()),
        egui::Key::Escape => Some(vec![0x1b]),
        egui::Key::ArrowUp => Some(b"\x1b[A".to_vec()),
        egui::Key::ArrowDown => Some(b"\x1b[B".to_vec()),
        egui::Key::ArrowRight => Some(b"\x1b[C".to_vec()),
        egui::Key::ArrowLeft => Some(b"\x1b[D".to_vec()),
        egui::Key::Home => Some(b"\x1b[H".to_vec()),
        egui::Key::End => Some(b"\x1b[F".to_vec()),
        egui::Key::PageUp => Some(b"\x1b[5~".to_vec()),
        egui::Key::PageDown => Some(b"\x1b[6~".to_vec()),
        egui::Key::Insert => Some(b"\x1b[2~".to_vec()),
        egui::Key::Space => Some(b" ".to_vec()),
        _ => None,
    }
}

impl WorkspaceApp {
    fn new() -> Self {
        let client = IpcClient::connect().expect("connect to pworkspaces daemon");
        let snap = client.snapshot(None).expect("daemon snapshot");
        let mut components = ComponentRegistry::new();
        components.load_plugins();

        let mut hydrate_pending = HashSet::new();
        for pane in &snap.panes {
            install_pane_component(&mut components, pane);
            if pane.component_type == "terminal" {
                hydrate_pending.insert(PaneId::from_raw(pane.id.clone()));
            }
        }

        let ws_id = snap
            .active_id
            .as_deref()
            .map(WorkspaceId::from_raw)
            .or_else(|| {
                snap.workspaces
                    .first()
                    .map(|w| WorkspaceId::from_raw(w.id.clone()))
            })
            .unwrap_or_else(|| WorkspaceId::from_raw("ws_none"));

        Self {
            client,
            snap,
            components,
            active_ws: ws_id,
            status: "⌘⇧D / ✕ detach  ·  pwctl stop mata runtime  ·  ⌘P open".into(),
            split_borders: Vec::new(),
            dragging_split: None,
            pane_rects: HashMap::new(),
            dragging_float: None,
            resizing_float: None,
            dragging_pane: None,
            selecting_term: None,
            swap_target: None,
            scroll_accum: 0.0,
            chrome_visible: true,
            palette_kind: PaletteKind::Closed,
            palette_query: String::new(),
            palette_index: 0,
            palette_focus_search: false,
            file_root: None,
            file_root_edit: String::new(),
            file_index: Vec::new(),
            renaming_ws: None,
            rename_buf: String::new(),
            rename_focus: false,
            want_close: false,
            leave: LeaveMode::Running,
            hydrate_pending,
            view_slots: HashMap::new(),
            overlays: HashMap::new(),
        }
    }

    fn ws_key(&self) -> String {
        self.active_ws.to_string()
    }

    fn refresh_snap(&mut self) {
        match self.client.snapshot(Some(&self.ws_key())) {
            Ok(snap) => {
                if let Some(id) = snap.active_id.as_deref() {
                    self.active_ws = WorkspaceId::from_raw(id);
                }
                self.snap = snap;
            }
            Err(e) => {
                self.status = format!("daemon: {e}");
            }
        }
    }
}

fn install_pane_component(components: &mut ComponentRegistry, pane: &PaneSnap) {
    let id = PaneId::from_raw(pane.id.clone());
    let name = pane.name.clone().unwrap_or_else(|| "pane".into());
    match pane.component_type.as_str() {
        "ports" => components.create_ports(id, name),
        "processes" => components.create_processes(id, name),
        "job" => components.create_job(id, name),
        "view" => {
            components.create_view(id.clone(), name);
            if let Some(src) = &pane.source {
                if let Some(v) = components.get_view_mut(&id) {
                    let _ = v.load(src);
                }
            }
        }
        "script" => {
            if let Some(cfg) = components
                .available_plugins()
                .iter()
                .find(|p| p.name == name)
                .cloned()
            {
                components.create_script(id, cfg);
            } else {
                components.create_job(id, name);
            }
        }
        _ => {
            let cols = pane.pty_cols.map(|n| n as usize).unwrap_or(INITIAL_COLS);
            let lines = pane.pty_rows.map(|n| n as usize).unwrap_or(INITIAL_LINES);
            components.create_terminal(id, name, cols.max(1), lines.max(1));
        }
    }
}

fn hydrate_terminal(
    client: &IpcClient,
    components: &mut ComponentRegistry,
    ws: &str,
    pane_id: &str,
) {
    if let Ok(hist) = client.read_replay(ws, pane_id) {
        if !hist.is_empty() {
            let id = PaneId::from_raw(pane_id);
            if let Some(term) = components.get_terminal_mut(&id) {
                term.process(&hist);
            }
        }
    }
    // Unread was already included in replay — drop so poll_outputs does not double-print.
    let _ = client.read_output(ws, pane_id);
}

impl WorkspaceApp {
    fn poll_outputs(&mut self) {
        let ws = self.ws_key();
        let pane_ids: Vec<PaneId> = self
            .snap
            .panes
            .iter()
            .filter(|p| p.component_type == "terminal")
            .map(|p| PaneId::from_raw(p.id.clone()))
            .collect();
        for pane_id in &pane_ids {
            if self.hydrate_pending.contains(pane_id) {
                continue;
            }
            if let Ok(data) = self.client.read_output(&ws, pane_id.as_str()) {
                if !data.is_empty() {
                    if let Some(comp) = self.components.get_terminal_mut(pane_id) {
                        comp.process(&data);
                    }
                }
            }
        }
    }

    fn send_to_focused(&mut self, data: &[u8]) {
        let Some(focused) = self.snap.focused.as_deref().map(PaneId::from_raw) else {
            return;
        };
        let _ = self
            .client
            .send_input(&self.ws_key(), focused.as_str(), data);
        if let Some(comp) = self.components.get_mut(&focused) {
            comp.input(ComponentInput::KeyBytes(data.to_vec()));
        }
    }

    /// Clipboard: mouse selection if any, else visible screen.
    fn focused_terminal_text(&mut self) -> Option<String> {
        let focused = PaneId::from_raw(self.snap.focused.as_deref()?);
        let t = self.components.get_terminal_mut(&focused)?;
        t.selected_text().filter(|s| !s.trim().is_empty())
            .or_else(|| {
                let s = t.screen_text();
                if s.trim().is_empty() { None } else { Some(s) }
            })
    }

    fn palette_open(&self) -> bool {
        self.palette_kind.is_open()
    }

    fn open_palette(&mut self) {
        self.palette_kind = PaletteKind::Commands;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_focus_search = true;
    }

    fn open_file_palette(&mut self) {
        self.palette_kind = PaletteKind::Files;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_focus_search = true;
        let focused = self.snap.focused.clone();
        let root = self
            .client
            .quick_open_root(&self.ws_key(), focused.as_deref())
            .unwrap_or_else(|_| ".".into());
        self.set_file_root(PathBuf::from(root), false);
    }

    fn set_file_root(&mut self, root: PathBuf, persist: bool) {
        self.file_root_edit = pworkspaces::files::display_path(&root);
        self.file_index = pworkspaces::files::list_rel_paths(&root);
        self.file_root = Some(root.clone());
        if persist {
            match self
                .client
                .set_workspace_cwd(&self.ws_key(), &root.display().to_string())
            {
                Ok(s) => {
                    self.status = format!(
                        "root → {}",
                        pworkspaces::files::display_path(std::path::Path::new(&s))
                    );
                }
                Err(e) => self.status = format!("root: {e}"),
            }
        }
    }

    fn apply_file_root_edit(&mut self) {
        let path = pworkspaces::files::expand_path(&self.file_root_edit);
        if path.is_dir() {
            self.set_file_root(path, true);
            self.palette_index = 0;
        } else if let Some(cur) = &self.file_root {
            self.file_root_edit = pworkspaces::files::display_path(cur);
        }
    }

    fn close_palette(&mut self) {
        self.palette_kind = PaletteKind::Closed;
        self.palette_query.clear();
        self.palette_index = 0;
        self.palette_focus_search = false;
    }

    fn open_quick_file(&mut self, rel: &str) {
        let abs = match &self.file_root {
            Some(root) => pworkspaces::files::abs_path(root, rel).display().to_string(),
            None => rel.to_string(),
        };
        let result = self.client.open_view(&self.ws_key(), &abs);
        self.sync_components();
        self.sync_view_sources();
        match result {
            Ok(_) => self.status = format!("view → {rel}"),
            Err(e) => self.status = format!("view: {e}"),
        }
    }

    fn run_action(&mut self, name: &str) {
        if name == "toggle_chrome" {
            self.chrome_visible = !self.chrome_visible;
            self.status = if self.chrome_visible {
                "Chrome ON  ·  ⌘⇧M to hide".into()
            } else {
                "Chrome OFF  ·  ⌘⇧M to show".into()
            };
            return;
        }
        if name == "quick_open" {
            self.open_file_palette();
            return;
        }
        if name == "rename_workspace" {
            self.chrome_visible = true;
            let label = self
                .snap
                .workspaces
                .iter()
                .find(|w| w.id == self.active_ws.as_str())
                .map(|w| w.name.clone())
                .unwrap_or_default();
            self.renaming_ws = Some(self.active_ws.clone());
            self.rename_buf = label;
            self.rename_focus = true;
            return;
        }
        if name == "detach" {
            let _ = self.client.save();
            self.leave = LeaveMode::Detach;
            self.want_close = true;
            self.status = "Detaching — runtime stays (pwctl / reopen UI)".into();
            return;
        }
        if name == "kill_runtime" {
            self.leave = LeaveMode::Kill;
            let _ = self.client.shutdown();
            self.want_close = true;
            return;
        }

        let focused = self.snap.focused.clone();
        match self
            .client
            .execute_action(name, Some(&self.ws_key()), focused.as_deref())
        {
            Ok(out) => {
                if let Some(id) = out.active_id {
                    self.active_ws = WorkspaceId::from_raw(id);
                }
                if let Some(msg) = out.message {
                    self.status = msg;
                }
            }
            Err(e) => self.status = format!("action {name}: {e}"),
        }

        self.sync_components();
    }

    fn sync_components(&mut self) {
        self.refresh_snap();
        let live: HashSet<PaneId> = self
            .snap
            .panes
            .iter()
            .map(|p| PaneId::from_raw(p.id.clone()))
            .collect();
        let existing: Vec<PaneId> = self
            .components
            .pane_ids()
            .into_iter()
            .cloned()
            .collect();
        for id in existing {
            if !live.contains(&id) {
                self.components.remove(&id);
                self.hydrate_pending.remove(&id);
            }
        }
        let panes = self.snap.panes.clone();
        for pane in &panes {
            let id = PaneId::from_raw(pane.id.clone());
            if self.components.get(&id).is_some() {
                continue;
            }
            install_pane_component(&mut self.components, pane);
            if pane.component_type == "terminal" {
                self.hydrate_pending.insert(id);
            }
        }
    }

    fn sync_view_sources(&mut self) {
        let sources: Vec<(PaneId, Option<String>)> = self
            .snap
            .panes
            .iter()
            .filter(|p| p.component_type == "view")
            .map(|p| (PaneId::from_raw(p.id.clone()), p.source.clone()))
            .collect();
        for (id, source) in sources {
            let Some(view) = self.components.get_view_mut(&id) else {
                continue;
            };
            match (source.as_deref(), view.loaded_path_str()) {
                (Some(p), Some(loaded)) if p == loaded => {}
                (Some(p), _) => {
                    let _ = view.load(p);
                }
                _ => {}
            }
        }
    }

    fn set_view_path(&mut self, pane_id: &PaneId, path: &str) {
        let _ = self.client.open_view(&self.ws_key(), path);
        if let Some(view) = self.components.get_view_mut(pane_id) {
            let _ = view.load(path);
        }
        self.status = format!("view → {path}");
        self.refresh_snap();
    }

    fn render_command_palette(&mut self, ctx: &egui::Context) {
        if !self.palette_open() {
            return;
        }

        let files_mode = self.palette_kind == PaletteKind::Files;
        let screen = ctx.screen_rect();
        let dim_layer = egui::LayerId::new(egui::Order::Background, egui::Id::new("palette_dim"));
        ctx.layer_painter(dim_layer).rect_filled(
            screen,
            0.0,
            egui::Color32::from_black_alpha(150),
        );

        let command_items = if files_mode {
            Vec::new()
        } else {
            self.client
                .palette_items(&self.palette_query)
                .unwrap_or_default()
        };
        let file_items = if files_mode {
            pworkspaces::files::search(&self.file_index, &self.palette_query, 80)
        } else {
            Vec::new()
        };
        let item_count = if files_mode {
            file_items.len()
        } else {
            command_items.len()
        };
        if item_count == 0 {
            self.palette_index = 0;
        } else if self.palette_index >= item_count {
            self.palette_index = item_count - 1;
        }

        let mut run_name: Option<String> = None;
        let mut open_rel: Option<String> = None;
        let mut close = false;
        let prev_query = self.palette_query.clone();
        let hint = if files_mode {
            "Open file…"
        } else {
            "Run action…"
        };
        let mut commit_root = false;
        let empty_label = if files_mode {
            if self.file_index.is_empty() {
                match &self.file_root {
                    Some(r) => format!("No files in {}", r.display()),
                    None => "No cwd".into(),
                }
            } else {
                "No matching files".into()
            }
        } else {
            "No matching actions".into()
        };

        egui::Window::new("command_palette")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 72.0])
            .fixed_size([560.0, if files_mode { 380.0 } else { 340.0 }])
            .frame(
                egui::Frame::popup(&ctx.style())
                    .fill(bg_pane())
                    .stroke(egui::Stroke::new(1.0, ACCENT))
                    .inner_margin(egui::Margin::same(10)),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(text());
                let mut root_focused = false;
                if files_mode {
                    let root_te = ui.add(
                        egui::TextEdit::singleline(&mut self.file_root_edit)
                            .hint_text("root (terminal cwd — edit to change)")
                            .font(egui::FontId::monospace(13.0))
                            .desired_width(f32::INFINITY)
                            .text_color(text_dim()),
                    );
                    root_focused = root_te.has_focus();
                    if root_te.lost_focus()
                        && !ui.input(|i| i.key_pressed(egui::Key::Escape))
                    {
                        commit_root = true;
                    }
                    ui.add_space(4.0);
                }
                let te = ui.add(
                    egui::TextEdit::singleline(&mut self.palette_query)
                        .hint_text(hint)
                        .font(egui::FontId::monospace(15.0))
                        .desired_width(f32::INFINITY),
                );
                if self.palette_focus_search {
                    te.request_focus();
                    self.palette_focus_search = false;
                }
                if self.palette_query != prev_query {
                    self.palette_index = 0;
                }

                ui.add_space(6.0);

                ui.input(|i| {
                    if i.key_pressed(egui::Key::Escape) {
                        close = true;
                    }
                    if i.key_pressed(egui::Key::ArrowDown) && item_count > 0 && !root_focused {
                        self.palette_index = (self.palette_index + 1) % item_count;
                    }
                    if i.key_pressed(egui::Key::ArrowUp) && item_count > 0 && !root_focused {
                        self.palette_index = (self.palette_index + item_count - 1) % item_count;
                    }
                    if i.key_pressed(egui::Key::Enter) {
                        if files_mode && root_focused {
                            commit_root = true;
                        } else if files_mode {
                            if let Some(rel) = file_items.get(self.palette_index) {
                                open_rel = Some(rel.clone());
                            }
                        } else if let Some(item) = command_items.get(self.palette_index) {
                            run_name = Some(item.name.clone());
                        }
                    }
                });

                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show(ui, |ui| {
                        if item_count == 0 {
                            ui.colored_label(text_dim(), empty_label);
                            return;
                        }
                        if files_mode {
                            for (i, rel) in file_items.iter().enumerate() {
                                let selected = i == self.palette_index;
                                let rich = if selected {
                                    egui::RichText::new(rel).color(text()).strong()
                                } else {
                                    egui::RichText::new(rel).color(text_dim())
                                };
                                let resp = ui.selectable_label(selected, rich);
                                if selected {
                                    resp.scroll_to_me(None);
                                }
                                if resp.hovered() {
                                    self.palette_index = i;
                                }
                                if resp.clicked() {
                                    open_rel = Some(rel.clone());
                                }
                            }
                        } else {
                            for (i, item) in command_items.iter().enumerate() {
                                let selected = i == self.palette_index;
                                let label = if let Some(sc) = &item.shortcut {
                                    format!("{}   {}", item.description, sc)
                                } else {
                                    item.description.clone()
                                };
                                let rich = if selected {
                                    egui::RichText::new(label).color(text()).strong()
                                } else {
                                    egui::RichText::new(label).color(text_dim())
                                };
                                let resp = ui.selectable_label(selected, rich);
                                if selected {
                                    resp.scroll_to_me(None);
                                }
                                if resp.hovered() {
                                    self.palette_index = i;
                                }
                                if resp.clicked() {
                                    run_name = Some(item.name.clone());
                                }
                            }
                        }
                    });
            });

        if close {
            self.close_palette();
            return;
        }
        if commit_root {
            self.apply_file_root_edit();
            return;
        }
        if let Some(rel) = open_rel {
            self.close_palette();
            self.open_quick_file(&rel);
            return;
        }
        if let Some(name) = run_name {
            self.close_palette();
            self.run_action(&name);
        }
    }
}

impl eframe::App for WorkspaceApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.refresh_snap();
        self.poll_outputs();
        self.sync_components();
        self.sync_view_sources();
        self.view_slots.clear();
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        ctx.send_viewport_cmd(egui::ViewportCommand::SetTheme(egui::SystemTheme::Dark));

        let mut pending_action: Option<&'static str> = None;
        let mut open_palette = false;
        let mut open_files = false;
        let mut pty_bytes: Vec<Vec<u8>> = Vec::new();
        let mut paste_texts: Vec<String> = Vec::new();
        let mut want_copy = false;

        ctx.input(|i| {
            if self.palette_open() {
                return;
            }
            for event in &i.events {
                match event {
                    egui::Event::Paste(text) => {
                        paste_texts.push(text.clone());
                    }
                    egui::Event::Copy => {
                        want_copy = true;
                    }
                    egui::Event::Cut => {
                        want_copy = true;
                    }
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => {
                        let ctrl = modifiers.ctrl || modifiers.mac_cmd;
                        let shift = modifiers.shift;

                        if ctrl && shift {
                            match key {
                                egui::Key::H => {
                                    pending_action = Some("split_horizontal");
                                    continue;
                                }
                                egui::Key::V => {
                                    pending_action = Some("split_vertical");
                                    continue;
                                }
                                egui::Key::W => {
                                    pending_action = Some("close_pane");
                                    continue;
                                }
                                egui::Key::F => {
                                    pending_action = Some("toggle_fullscreen");
                                    continue;
                                }
                                egui::Key::G => {
                                    pending_action = Some("toggle_float");
                                    continue;
                                }
                                egui::Key::P => {
                                    open_palette = true;
                                    continue;
                                }
                                egui::Key::M => {
                                    pending_action = Some("toggle_chrome");
                                    continue;
                                }
                                egui::Key::N => {
                                    pending_action = Some("new_terminal");
                                    continue;
                                }
                                egui::Key::D => {
                                    pending_action = Some("detach");
                                    continue;
                                }
                                #[cfg(not(target_os = "macos"))]
                                egui::Key::O => {
                                    open_files = true;
                                    continue;
                                }
                                _ => {}
                            }
                        }

                        #[cfg(target_os = "macos")]
                        if modifiers.mac_cmd && !shift && !modifiers.ctrl && *key == egui::Key::P
                        {
                            open_files = true;
                            continue;
                        }

                        if ctrl && *key == egui::Key::Tab {
                            pending_action = Some(if shift {
                                "prev_workspace"
                            } else {
                                "next_workspace"
                            });
                            continue;
                        }
                        if ctrl && *key == egui::Key::CloseBracket && !shift {
                            pending_action = Some("focus_next");
                            continue;
                        }
                        if ctrl && *key == egui::Key::OpenBracket && !shift {
                            pending_action = Some("focus_prev");
                            continue;
                        }

                        if let Some(bytes) = key_to_pty_bytes(key, modifiers) {
                            pty_bytes.push(bytes);
                        }
                    }
                    egui::Event::Text(text) => {
                        pty_bytes.push(text.as_bytes().to_vec());
                    }
                    _ => {}
                }
            }
        });

        for text in &paste_texts {
            self.send_to_focused(text.as_bytes());
            self.status = format!("Pasted {} bytes", text.len());
        }

        if want_copy {
            if let Some(text) = self.focused_terminal_text() {
                let trimmed = text.trim_end().to_string();
                if !trimmed.is_empty() {
                    ctx.copy_text(trimmed);
                    self.status = "Copied screen to clipboard".into();
                }
            }
        }

        for bytes in &pty_bytes {
            self.send_to_focused(bytes);
        }

        if open_palette {
            self.open_palette();
        }
        if open_files {
            self.open_file_palette();
        }
        if let Some(name) = pending_action {
            self.run_action(name);
        }

        // ✕ / Cmd+Q = detach (keep runtime). Kill only via pwctl stop / kill_runtime.
        if ctx.input(|i| i.viewport().close_requested()) && self.leave == LeaveMode::Running {
            self.leave = LeaveMode::Detach;
            let _ = self.client.save();
        }

        if self.want_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Mouse: collect actions inside ctx.input(), execute outside (no locks in closure)
        enum MouseAction {
            FocusPane(PaneId),
            StartDrag(usize),
            DragResize { split_index: usize, ratio: f32 },
            StopDrag,
            SetCursor(egui::CursorIcon),
            StartPaneDrag(PaneId),
            UpdateSwapTarget(Option<PaneId>),
            DoSwap(PaneId, PaneId),
            StopPaneDrag,
        }
        let mut mouse_actions: Vec<MouseAction> = Vec::new();

        ctx.input(|i| {
            let pointer = &i.pointer;
            if let Some(pos) = pointer.hover_pos() {
                // Click to focus pane
                if pointer.primary_pressed() && self.dragging_split.is_none() {
                    let on_border = self
                        .split_borders
                        .iter()
                        .any(|b| b.rect.expand(3.0).contains(pos));

                    if !on_border {
                        for (pane_id, pane_rect) in &self.pane_rects {
                            if pane_rect.contains(pos) {
                                mouse_actions.push(MouseAction::FocusPane(pane_id.clone()));
                                // Start pane drag if click on title bar
                                let title_rect = egui::Rect::from_min_size(
                                    pane_rect.left_top(),
                                    egui::vec2(pane_rect.width(), TITLE_H),
                                );
                                if title_rect.contains(pos) {
                                    mouse_actions.push(MouseAction::StartPaneDrag(pane_id.clone()));
                                }
                                break;
                            }
                        }
                    }
                }

                // Check if hovering over a split border
                let hovered = self
                    .split_borders
                    .iter()
                    .enumerate()
                    .find(|(_, b)| b.rect.expand(3.0).contains(pos));

                if let Some((idx, border)) = hovered {
                    let cursor = match border.direction {
                        pworkspaces::layout::Direction::Horizontal => egui::CursorIcon::ResizeHorizontal,
                        pworkspaces::layout::Direction::Vertical => egui::CursorIcon::ResizeVertical,
                    };
                    mouse_actions.push(MouseAction::SetCursor(cursor));

                    if pointer.primary_pressed() {
                        mouse_actions.push(MouseAction::StartDrag(idx));
                    }
                }

                // Handle active drag
                if let Some(drag_idx) = self.dragging_split {
                    if pointer.primary_down() {
                        if let Some(border) = self.split_borders.get(drag_idx) {
                            let parent = border.parent_rect;
                            let new_ratio = match border.direction {
                                pworkspaces::layout::Direction::Horizontal => {
                                    (pos.x - parent.left()) / parent.width()
                                }
                                pworkspaces::layout::Direction::Vertical => {
                                    (pos.y - parent.top()) / parent.height()
                                }
                            };
                            mouse_actions.push(MouseAction::DragResize {
                                split_index: border.split_index,
                                ratio: new_ratio.clamp(0.1, 0.9),
                            });
                            // Keep cursor during drag
                            let cursor = match border.direction {
                                pworkspaces::layout::Direction::Horizontal => egui::CursorIcon::ResizeHorizontal,
                                pworkspaces::layout::Direction::Vertical => egui::CursorIcon::ResizeVertical,
                            };
                            mouse_actions.push(MouseAction::SetCursor(cursor));
                        }
                    } else {
                        mouse_actions.push(MouseAction::StopDrag);
                    }
                }

                // Pane drag-to-swap
                if let Some(src) = &self.dragging_pane {
                    if pointer.primary_down() {
                        mouse_actions.push(MouseAction::SetCursor(egui::CursorIcon::Grabbing));
                        // Find pane under cursor (not the source)
                        let target = self.pane_rects.iter()
                            .find(|(pid, r)| *pid != src && r.contains(pos))
                            .map(|(pid, _)| pid.clone());
                        mouse_actions.push(MouseAction::UpdateSwapTarget(target));
                    } else {
                        // Released — swap if over target
                        if let Some(target) = self.pane_rects.iter()
                            .find(|(pid, r)| *pid != src && r.contains(pos))
                            .map(|(pid, _)| pid.clone())
                        {
                            mouse_actions.push(MouseAction::DoSwap(src.clone(), target));
                        }
                        mouse_actions.push(MouseAction::StopPaneDrag);
                    }
                }
            } else if !i.pointer.primary_down() {
                mouse_actions.push(MouseAction::StopDrag);
                if self.dragging_pane.is_some() {
                    mouse_actions.push(MouseAction::StopPaneDrag);
                }
            }
        });

        // Execute mouse actions (locks acquired here, outside ctx.input)
        for action in mouse_actions {
            match action {
                MouseAction::FocusPane(pane_id) => {
                    let _ = self.client.focus_pane(&self.ws_key(), pane_id.as_str());
                    self.refresh_snap();
                }
                MouseAction::StartDrag(idx) => {
                    self.dragging_split = Some(idx);
                }
                MouseAction::DragResize { split_index, ratio } => {
                    let _ = self
                        .client
                        .resize_split(&self.ws_key(), split_index, ratio);
                    self.refresh_snap();
                }
                MouseAction::StopDrag => {
                    self.dragging_split = None;
                }
                MouseAction::SetCursor(cursor) => {
                    ctx.set_cursor_icon(cursor);
                }
                MouseAction::StartPaneDrag(pane_id) => {
                    self.dragging_pane = Some(pane_id);
                }
                MouseAction::UpdateSwapTarget(target) => {
                    self.swap_target = target;
                }
                MouseAction::DoSwap(src, target) => {
                    if self
                        .client
                        .swap_panes(&self.ws_key(), src.as_str(), target.as_str())
                        .is_ok()
                    {
                        self.status = "Swapped panes".into();
                        self.refresh_snap();
                    }
                }
                MouseAction::StopPaneDrag => {
                    self.dragging_pane = None;
                    self.swap_target = None;
                }
            }
        }

        let snap = self.snap.clone();
        let ws_tabs: Vec<(WorkspaceId, String, bool)> = snap
            .workspaces
            .iter()
            .map(|w| {
                (
                    WorkspaceId::from_raw(w.id.clone()),
                    w.name.clone(),
                    w.active,
                )
            })
            .collect();
        let pane_count = snap
            .workspaces
            .iter()
            .find(|w| w.id == self.active_ws.as_str())
            .map(|w| w.pane_count)
            .unwrap_or(snap.panes.len());
        let root = snap.layout_root.clone();
        let focused = snap.focused.as_deref().map(PaneId::from_raw);
        let fullscreened = snap.fullscreen.as_deref().map(PaneId::from_raw);
        let pane_names: HashMap<PaneId, String> = snap
            .panes
            .iter()
            .map(|p| {
                (
                    PaneId::from_raw(p.id.clone()),
                    p.name.clone().unwrap_or_else(|| "pane".into()),
                )
            })
            .collect();
        let pane_types: HashMap<PaneId, String> = snap
            .panes
            .iter()
            .map(|p| (PaneId::from_raw(p.id.clone()), p.component_type.clone()))
            .collect();
        let floating_panes: Vec<FloatingInfo> = snap
            .floating
            .iter()
            .map(|fp| FloatingInfo {
                pane_id: PaneId::from_raw(fp.pane_id.clone()),
                x: fp.x,
                y: fp.y,
                width: fp.width,
                height: fp.height,
            })
            .collect();

        // Workspace tabs only — actions live in ⌘⇧P palette.
        // macOS: always draw this strip so traffic lights sit on bg_app.
        let mut tab_action: Option<TabAction> = None;
        let mut commit_rename: Option<WorkspaceId> = None;
        let show_top = self.chrome_visible || cfg!(target_os = "macos");
        if show_top {
            let mut top = egui::TopBottomPanel::top("top_bar").frame(
                egui::Frame::new()
                    .fill(bg_app())
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            );
            if !self.chrome_visible {
                top = top.exact_height(28.0);
            }
            top.show(ctx, |ui| {
                    ui.visuals_mut().panel_fill = bg_app();
                    ui.visuals_mut().widgets.inactive.bg_fill = bg_app();
                    ui.visuals_mut().widgets.hovered.bg_fill = egui::Color32::from_rgb(22, 22, 22);
                    ui.visuals_mut().widgets.active.bg_fill = egui::Color32::from_rgb(28, 28, 28);

                    let drag = ui.interact(
                        ui.max_rect(),
                        egui::Id::new("pw_title_drag"),
                        egui::Sense::drag(),
                    );
                    if drag.drag_started() {
                        ui.ctx()
                            .send_viewport_cmd(egui::ViewportCommand::StartDrag);
                    }

                    ui.horizontal(|ui| {
                        if TRAFFIC_LIGHTS_W > 0.0 {
                            ui.add_space(TRAFFIC_LIGHTS_W);
                        }
                        ui.label(
                            egui::RichText::new(APP_TITLE)
                                .strong()
                                .color(text())
                                .size(13.0),
                        );
                        ui.separator();
                        for (ws_id, name, is_active) in &ws_tabs {
                            if self.renaming_ws.as_ref() == Some(ws_id) {
                                let te = ui.add(
                                    egui::TextEdit::singleline(&mut self.rename_buf)
                                        .desired_width(140.0)
                                        .font(egui::FontId::proportional(13.0))
                                        .hint_text("workspace name"),
                                );
                                if self.rename_focus {
                                    te.request_focus();
                                    self.rename_focus = false;
                                }
                                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                    tab_action = Some(TabAction::CancelRename);
                                } else if te.lost_focus() {
                                    commit_rename = Some(ws_id.clone());
                                }
                                continue;
                            }

                            let label = if *is_active {
                                egui::RichText::new(name).strong().color(FOCUSED_BORDER)
                            } else {
                                egui::RichText::new(name).color(egui::Color32::from_rgb(170, 170, 180))
                            };

                            let resp = ui.add(
                                egui::Button::new(label)
                                    .frame(false)
                                    .selected(*is_active)
                                    .sense(egui::Sense::CLICK),
                            );
                            if resp.double_clicked() {
                                tab_action = Some(TabAction::StartRename {
                                    id: ws_id.clone(),
                                    name: name.clone(),
                                });
                            } else if resp.clicked() && !is_active {
                                tab_action = Some(TabAction::Switch(ws_id.clone()));
                            }
                            if resp.secondary_clicked() && ws_tabs.len() > 1 {
                                tab_action = Some(TabAction::Close(ws_id.clone()));
                            }
                        }

                        if ui
                            .add(egui::Button::new("+").small().sense(egui::Sense::CLICK))
                            .clicked()
                        {
                            tab_action = Some(TabAction::Create);
                        }

                        if self.chrome_visible {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(format!("Panes: {}", pane_count));
                                if let Some(ref f) = focused {
                                    let name = pane_names.get(f).cloned().unwrap_or_default();
                                    ui.label(format!("Focus: {}", name));
                                    ui.separator();
                                }
                            });
                        }
                    });
                });
        }

        if let Some(id) = commit_rename {
            if !matches!(tab_action, Some(TabAction::CancelRename)) {
                let buf = std::mem::take(&mut self.rename_buf);
                self.renaming_ws = None;
                match self.client.rename_workspace(id.as_str(), &buf) {
                    Ok(name) => self.status = format!("Workspace → {name}"),
                    Err(e) => self.status = format!("rename: {e}"),
                }
                self.refresh_snap();
            }
        }

        // Handle tab actions
        match tab_action {
            Some(TabAction::Switch(ws_id)) => {
                if self.client.switch_workspace(ws_id.as_str()).is_ok() {
                    self.active_ws = ws_id;
                    self.status = "Switched workspace".into();
                    self.refresh_snap();
                    self.sync_components();
                }
            }
            Some(TabAction::Close(ws_id)) => {
                let other = self
                    .snap
                    .workspaces
                    .iter()
                    .find(|w| w.id != ws_id.as_str())
                    .map(|w| WorkspaceId::from_raw(w.id.clone()));
                if let Some(other_id) = other {
                    let _ = self.client.destroy_workspace(ws_id.as_str());
                    let _ = self.client.switch_workspace(other_id.as_str());
                    self.active_ws = other_id;
                    self.status = "Closed workspace".into();
                    self.sync_components();
                }
            }
            Some(TabAction::Create) => self.run_action("new_workspace"),
            Some(TabAction::StartRename { id, name }) => {
                self.renaming_ws = Some(id);
                self.rename_buf = name;
                self.rename_focus = true;
            }
            Some(TabAction::CancelRename) => {
                self.renaming_ws = None;
                self.rename_buf.clear();
            }
            None => {}
        }

        // Bottom bar
        if self.chrome_visible {
            egui::TopBottomPanel::bottom("bottom_bar")
                .frame(
                    egui::Frame::new()
                        .fill(bg_app())
                        .inner_margin(egui::Margin::symmetric(8, 4)),
                )
                .show(ctx, |ui| {
                    ui.visuals_mut().panel_fill = bg_app();
                    ui.horizontal(|ui| {
                        ui.colored_label(text_dim(), &self.status);
                    });
                });
        }

        // Main area
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(bg_app()))
            .show(ctx, |ui| {
                let rect = ui.available_rect_before_wrap();

                if let Some(fs_pane) = &fullscreened {
                    self.render_pane(ui, rect, fs_pane, true, &pane_names, &pane_types);
                    return;
                }

                if let Some(root) = &root {
                    self.split_borders.clear();
                    self.pane_rects.clear();
                    let mut split_counter = 0;
                    self.render_node(ui, rect, root, focused.as_ref(), &pane_names, &pane_types, &mut split_counter);
                } else if floating_panes.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.heading("No panes. ⌘P open · ⌘⇧P palette · Ctrl+Shift+H/V split.");
                    });
                }

                // Render floating panes on top (sorted by z-index, back to front)
                for fp in &floating_panes {
                    let float_rect = egui::Rect::from_min_size(
                        egui::pos2(rect.left() + fp.x, rect.top() + fp.y),
                        egui::vec2(fp.width, fp.height),
                    );
                    // Clamp to viewport
                    let float_rect = float_rect.intersect(rect);
                    if float_rect.width() < 50.0 || float_rect.height() < 50.0 {
                        continue;
                    }

                    // Drop shadow
                    let shadow_rect = float_rect.translate(egui::vec2(4.0, 4.0));
                    ui.painter().rect_filled(
                        shadow_rect,
                        6.0,
                        egui::Color32::from_rgba_unmultiplied(0, 0, 0, 80),
                    );

                    let is_focused = focused.as_ref() == Some(&fp.pane_id);
                    self.render_pane(ui, float_rect, &fp.pane_id, is_focused, &pane_names, &pane_types);
                }
            });

            // Handle floating pane drag/resize (collect actions, execute outside)
            if !floating_panes.is_empty() {
                self.handle_floating_mouse(ctx, &floating_panes);
            }

        self.render_command_palette(ctx);
        self.sync_webviews(frame);
    }
}

impl WorkspaceApp {
    fn render_node(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        node: &LayoutNode,
        focused: Option<&PaneId>,
        pane_names: &HashMap<PaneId, String>,
        pane_types: &HashMap<PaneId, String>,
        split_counter: &mut usize,
    ) {
        match node {
            LayoutNode::Leaf { pane_id } => {
                let is_focused = focused == Some(pane_id);
                self.render_pane(ui, rect, pane_id, is_focused, pane_names, pane_types);
            }
            LayoutNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let gap = 5.0; // Wider gap for easier drag
                let (first_rect, second_rect) = split_rect(rect, *direction, *ratio, gap);

                // Compute the border rect between the two children
                let border_rect = match direction {
                    pworkspaces::layout::Direction::Horizontal => {
                        egui::Rect::from_min_max(
                            egui::pos2(first_rect.right(), rect.top()),
                            egui::pos2(second_rect.left(), rect.bottom()),
                        )
                    }
                    pworkspaces::layout::Direction::Vertical => {
                        egui::Rect::from_min_max(
                            egui::pos2(rect.left(), first_rect.bottom()),
                            egui::pos2(rect.right(), second_rect.top()),
                        )
                    }
                };

                let current_split_index = *split_counter;
                self.split_borders.push(SplitBorder {
                    rect: border_rect,
                    parent_rect: rect,
                    direction: *direction,
                    split_index: current_split_index,
                });

                // Draw the border with a subtle color
                let painter = ui.painter();
                painter.rect_filled(border_rect, 0.0, egui::Color32::from_rgb(50, 50, 50));

                *split_counter += 1;
                self.render_node(ui, first_rect, first, focused, pane_names, pane_types, split_counter);
                self.render_node(ui, second_rect, second, focused, pane_names, pane_types, split_counter);
            }
        }
    }

    fn render_pane(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        pane_id: &PaneId,
        is_focused: bool,
        pane_names: &HashMap<PaneId, String>,
        pane_types: &HashMap<PaneId, String>,
    ) {
        self.pane_rects.insert(pane_id.clone(), rect);
        let component_type = pane_types
            .get(pane_id)
            .map(|s| s.as_str())
            .unwrap_or("terminal");
        let border_color = if is_focused { FOCUSED_BORDER } else { NORMAL_BORDER };
        let border_width = if is_focused { 2.0 } else { 1.0 };
        let title_height = TITLE_H;
        let title_rect = egui::Rect::from_min_size(
            rect.left_top(),
            egui::vec2(rect.width(), title_height),
        );

        let name = pane_names
            .get(pane_id)
            .cloned()
            .unwrap_or_else(|| "Pane".into());

        let content_rect = egui::Rect::from_min_max(
            egui::pos2(rect.left() + 4.0, rect.top() + title_height + TITLE_GAP),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
        );

        let painter = ui.painter();

        // Common frame
        painter.rect_filled(rect, 4.0, bg_pane());
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(border_width, border_color),
            egui::StrokeKind::Outside,
        );

        // Swap target highlight
        let is_swap_target = self.swap_target.as_ref() == Some(pane_id);
        if is_swap_target {
            painter.rect_filled(rect, 4.0, egui::Color32::from_rgba_premultiplied(99, 179, 237, 40));
            painter.rect_stroke(
                rect,
                4.0,
                egui::Stroke::new(2.0, egui::Color32::from_rgb(99, 179, 237)),
                egui::StrokeKind::Outside,
            );
        }

        // Drag source dim
        let is_drag_source = self.dragging_pane.as_ref() == Some(pane_id);
        if is_drag_source {
            painter.rect_filled(rect, 4.0, egui::Color32::from_rgba_premultiplied(0, 0, 0, 80));
        }

        // Title sits on same fill as terminal (kitty tab, not a gray bar).
        painter.rect_filled(
            title_rect,
            egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 },
            bg_title(),
        );
        painter.line_segment(
            [
                egui::pos2(title_rect.left() + 6.0, title_rect.bottom() - 1.0),
                egui::pos2(title_rect.right() - 6.0, title_rect.bottom() - 1.0),
            ],
            egui::Stroke::new(
                1.0,
                if is_focused {
                    ACCENT
                } else {
                    egui::Color32::from_rgb(22, 26, 34)
                },
            ),
        );

        let type_badge = match component_type {
            "terminal" => "",
            "ports" => "  ports",
            "processes" => "  proc",
            "job" => "  job",
            "view" => "  view",
            "script" => "  script",
            other => {
                let _ = other;
                "  pane"
            }
        };

        let fit_cols = ((content_rect.width() - PADDING_X) / CELL_WIDTH).max(1.0) as usize;
        let fit_lines = ((content_rect.height() - PADDING_Y) / CELL_HEIGHT).max(1.0) as usize;

        let need_hydrate =
            component_type == "terminal" && self.hydrate_pending.contains(pane_id);
        if need_hydrate {
            if let Some(tc) = self.components.get_terminal_mut(pane_id) {
                if tc.cols() != fit_cols || tc.lines() != fit_lines {
                    tc.input(ComponentInput::Resize {
                        cols: fit_cols,
                        lines: fit_lines,
                    });
                }
            }
            let ws = self.ws_key();
            let _ = self.client.resize_pty(
                &ws,
                pane_id.as_str(),
                fit_cols as u16,
                fit_lines as u16,
            );
            hydrate_terminal(&self.client, &mut self.components, &ws, pane_id.as_str());
            self.hydrate_pending.remove(pane_id);
        } else if let Some(tc) = self.components.get_terminal_mut(pane_id) {
            if tc.cols() != fit_cols || tc.lines() != fit_lines {
                tc.input(ComponentInput::Resize { cols: fit_cols, lines: fit_lines });
                let _ = self.client.resize_pty(
                    &self.ws_key(),
                    pane_id.as_str(),
                    fit_cols as u16,
                    fit_lines as u16,
                );
            }
        }

        // Wheel/trackpad on *this* pane, before render, so display_offset is visible.
        if component_type == "terminal"
            && ui.rect_contains_pointer(content_rect)
            && self.dragging_split.is_none()
            && self.dragging_pane.is_none()
        {
            let scroll_px = ui.input(|i| {
                let mut from_events = 0.0;
                let mut saw_wheel = false;
                for event in &i.events {
                    if let egui::Event::MouseWheel {
                        unit,
                        delta,
                        modifiers,
                        ..
                    } = event
                    {
                        if modifiers.ctrl || modifiers.mac_cmd {
                            continue;
                        }
                        saw_wheel = true;
                        from_events += match unit {
                            egui::MouseWheelUnit::Point => delta.y,
                            egui::MouseWheelUnit::Line => delta.y * CELL_HEIGHT,
                            egui::MouseWheelUnit::Page => delta.y * CELL_HEIGHT * 20.0,
                        };
                    }
                }
                if saw_wheel {
                    return from_events;
                }
                if i.smooth_scroll_delta.y.abs() > f32::EPSILON {
                    i.smooth_scroll_delta.y
                } else {
                    i.raw_scroll_delta.y
                }
            });
            if scroll_px.abs() > f32::EPSILON {
                self.scroll_accum += scroll_px;
                let mut lines = (self.scroll_accum / CELL_HEIGHT).trunc() as i32;
                self.scroll_accum -= lines as f32 * CELL_HEIGHT;
                // Tiny flicks still move one line.
                if lines == 0 && scroll_px.abs() >= 4.0 {
                    lines = scroll_px.signum() as i32;
                    self.scroll_accum = 0.0;
                }
                if lines != 0 {
                    if let Some(comp) = self.components.get_mut(pane_id) {
                        comp.input(ComponentInput::Scroll(lines));
                    }
                }
            }
        }

        if component_type == "terminal"
            && self.dragging_split.is_none()
            && self.dragging_pane.is_none()
        {
            let x_offset = content_rect.left() + PADDING_X / 2.0;
            let y_offset = content_rect.top() + PADDING_Y / 2.0;
            let (pressed, down, released, pos) = ui.input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.primary_released(),
                    i.pointer.interact_pos().or(i.pointer.hover_pos()),
                )
            });
            if let Some(pos) = pos {
                if content_rect.contains(pos) {
                    let col = ((pos.x - x_offset) / CELL_WIDTH)
                        .floor()
                        .clamp(0.0, (fit_cols.saturating_sub(1)) as f32)
                        as usize;
                    let line = ((pos.y - y_offset) / CELL_HEIGHT)
                        .floor()
                        .clamp(0.0, (fit_lines.saturating_sub(1)) as f32)
                        as usize;
                    if pressed {
                        if let Some(t) = self.components.get_terminal_mut(pane_id) {
                            t.selection_begin(col, line);
                        }
                        self.selecting_term = Some(pane_id.clone());
                    } else if down && self.selecting_term.as_ref() == Some(pane_id) {
                        if let Some(t) = self.components.get_terminal_mut(pane_id) {
                            t.selection_update(col, line);
                        }
                    }
                }
            }
            if released && self.selecting_term.as_ref() == Some(pane_id) {
                if let Some(t) = self.components.get_terminal_mut(pane_id) {
                    if let Some(text) = t.selected_text() {
                        if !text.trim().is_empty() {
                            ui.ctx().copy_text(text);
                            self.status = "Copied selection".into();
                        }
                    }
                }
                self.selecting_term = None;
            }
        }

        // Get render output from component
        let render_output = self.components.get(pane_id)
            .map(|comp| comp.render(fit_cols, fit_lines));

        let scroll_info = match &render_output {
            Some(RenderOutput::Grid { scroll_offset, .. }) if *scroll_offset > 0 => {
                format!(" [↑{}]", scroll_offset)
            }
            _ => String::new(),
        };

        painter.text(
            title_rect.left_center() + egui::vec2(12.0, 0.0),
            egui::Align2::LEFT_CENTER,
            format!("{}{}{}", name, type_badge, scroll_info),
            egui::FontId::monospace(12.0),
            if is_focused { text() } else { text_dim() },
        );

        let title_resp = ui.interact(
            title_rect,
            egui::Id::new(("pane_title", pane_id.as_str())),
            egui::Sense::hover(),
        );
        title_resp.on_hover_text(format!("{}   id: {}", name, pane_id));

        if is_focused {
            painter.circle_filled(
                title_rect.right_top() + egui::vec2(-14.0, 12.0),
                4.0,
                FOCUSED_BORDER,
            );
        }

        if let Some(comp) = self.components.get_mut(pane_id) {
            comp.tick();
        }

        match component_type {
            "ports" => {
                self.render_ports_ui(ui, content_rect, pane_id);
                return;
            }
            "processes" => {
                self.render_processes_ui(ui, content_rect, pane_id);
                return;
            }
            "job" => {
                self.render_job_ui(ui, content_rect, pane_id);
                return;
            }
            "view" => {
                self.render_view_ui(ui, content_rect, pane_id);
                return;
            }
            _ => {}
        }

        if is_focused && !self.palette_open() {
            let resp = ui.interact(
                content_rect,
                egui::Id::new(("pty_focus", pane_id.as_str())),
                egui::Sense::focusable_noninteractive(),
            );
            resp.request_focus();
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    egui::EventFilter {
                        tab: true,
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        escape: true,
                    },
                );
            });
        }

        // Render component content
        match render_output {
            Some(RenderOutput::Grid { cells, cursor, cols, lines, .. }) => {
                Self::render_grid(ui, content_rect, &cells, &cursor, cols, lines, is_focused);
            }
            Some(RenderOutput::Lines { header, subheader, lines }) => {
                Self::render_lines(ui, content_rect, header.as_ref(), subheader.as_ref(), &lines);
            }
            Some(RenderOutput::Empty { message }) => {
                painter.text(
                    content_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    &message,
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgb(120, 120, 130),
                );
            }
            None => {
                painter.text(
                    content_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "No component",
                    egui::FontId::proportional(14.0),
                    egui::Color32::from_rgb(120, 120, 130),
                );
            }
        }
    }

    fn render_ports_ui(&mut self, ui: &mut egui::Ui, rect: egui::Rect, pane_id: &PaneId) {
        let rows = self.snap.listen_ports.clone();
        let err = self.snap.listen_ports_error.clone();
        let mut kill: Option<(u32, String)> = None;
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.set_clip_rect(rect);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
            let Some(comp) = self.components.get_ports_mut(pane_id) else {
                return;
            };
            comp.sync_from(&rows, err.as_deref());
            if let Some(f) = comp.flash() {
                ui.colored_label(ACCENT, f);
            }
            if let Some(e) = comp.error() {
                ui.colored_label(egui::Color32::from_rgb(239, 100, 100), e);
            }
            ui.label(
                egui::RichText::new("click row → SIGTERM")
                    .small()
                    .color(text_dim()),
            );
            let mut kill_idx = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("  {:<14} {:>7}  {}", "COMMAND", "PID", "ADDR"))
                            .small()
                            .color(text_dim())
                            .monospace(),
                    );
                    for (i, row) in comp.rows().iter().enumerate() {
                        let label = format!("  {:<14} {:>7}  {}", row.command, row.pid, row.addr);
                        if ui
                            .selectable_label(false, egui::RichText::new(label).monospace())
                            .clicked()
                        {
                            kill_idx = Some(i);
                        }
                    }
                });
            if let Some(i) = kill_idx {
                kill = comp.pid_at(i);
            }
        });
        if let Some((pid, addr)) = kill {
            let result = self.client.kill_listen_pid(pid);
            if let Some(comp) = self.components.get_ports_mut(pane_id) {
                match result {
                    Ok(()) => comp.set_flash(format!("SIGTERM {pid} ({addr})")),
                    Err(e) => comp.set_flash(format!("kill {pid}: {e}")),
                }
            }
        }
    }

    fn render_processes_ui(&mut self, ui: &mut egui::Ui, rect: egui::Rect, pane_id: &PaneId) {
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.set_clip_rect(rect);
            ui.spacing_mut().item_spacing = egui::vec2(6.0, 3.0);
            let Some(comp) = self.components.get_processes_mut(pane_id) else {
                return;
            };
            if let Some(f) = comp.flash() {
                ui.colored_label(ACCENT, f);
            }
            if let Some(e) = comp.error() {
                ui.colored_label(egui::Color32::from_rgb(239, 100, 100), e);
            }
            ui.label(
                egui::RichText::new("click row → SIGTERM")
                    .small()
                    .color(text_dim()),
            );
            let mut kill = None;
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!(
                            "  {:>7} {:>6} {:>6}  {}",
                            "PID", "%CPU", "%MEM", "CMD"
                        ))
                        .small()
                        .color(text_dim())
                        .monospace(),
                    );
                    for (i, row) in comp.rows().iter().enumerate() {
                        let cmd = if row.command.len() > 52 {
                            format!("{}…", &row.command[..51])
                        } else {
                            row.command.clone()
                        };
                        let label = format!(
                            "  {:>7} {:>6.1} {:>6.1}  {}",
                            row.pid, row.cpu, row.mem, cmd
                        );
                        if ui
                            .selectable_label(false, egui::RichText::new(label).monospace())
                            .clicked()
                        {
                            kill = Some(i);
                        }
                    }
                });
            if let Some(i) = kill {
                comp.kill_at(i);
            }
        });
    }

    fn render_job_ui(&mut self, ui: &mut egui::Ui, rect: egui::Rect, pane_id: &PaneId) {
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.set_clip_rect(rect);
            let Some(job) = self.components.get_job_mut(pane_id) else {
                return;
            };
            let running = job.is_running();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!running, egui::Button::new("Start"))
                    .clicked()
                {
                    job.start();
                }
                if ui
                    .add_enabled(running, egui::Button::new("Stop"))
                    .clicked()
                {
                    job.stop();
                }
                ui.label(
                    egui::RichText::new(job.status_text())
                        .color(if running { ACCENT } else { text_dim() })
                        .small(),
                );
            });
            let mut cmd = job.command().to_string();
            let te = ui.add_enabled(
                !running,
                egui::TextEdit::singleline(&mut cmd)
                    .hint_text("cargo test · npm start · python -m http.server")
                    .font(egui::TextStyle::Monospace),
            );
            if te.changed() {
                job.set_command(cmd);
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(job.output_text()).monospace())
                            .wrap(),
                    );
                });
        });
    }

    fn render_view_ui(&mut self, ui: &mut egui::Ui, rect: egui::Rect, pane_id: &PaneId) {
        let mut load_path: Option<String> = None;
        ui.allocate_ui_at_rect(rect, |ui| {
            ui.set_clip_rect(rect);
            if ui.rect_contains_pointer(rect) {
                let dropped: Vec<String> = ui.input(|i| {
                    i.raw
                        .dropped_files
                        .iter()
                        .filter_map(|f| f.path.as_ref().map(|p| p.display().to_string()))
                        .collect()
                });
                if let Some(p) = dropped.into_iter().next() {
                    load_path = Some(p);
                }
            }

            let nav = {
                let Some(view) = self.components.get_view_mut(pane_id) else {
                    return;
                };

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("path").small().color(text_dim()));
                    let te = ui.add(
                        egui::TextEdit::singleline(view.path_edit_mut())
                            .hint_text("README.md  ·  index.html  ·  http://localhost:5173")
                            .font(egui::FontId::monospace(13.0))
                            .desired_width((ui.available_width() - 72.0).max(80.0)),
                    );
                    let enter = te.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if ui.button("Load").clicked() || enter {
                        let p = view.path_edit().trim().to_string();
                        if !p.is_empty() {
                            load_path = Some(p);
                        }
                    }
                });
                ui.add_space(6.0);
                view.nav()
            };
            let remaining = ui.available_rect_before_wrap();
            ui.allocate_rect(remaining, egui::Sense::hover());
            match &nav {
                ViewNav::Message(msg) => {
                    ui.painter().rect_filled(remaining, 0.0, bg_pane());
                    ui.painter().text(
                        remaining.left_top() + egui::vec2(12.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        msg,
                        egui::FontId::proportional(13.0),
                        text_dim(),
                    );
                }
                ViewNav::Html { .. } | ViewNav::Url { .. } => {
                    ui.painter().rect_filled(remaining, 0.0, bg_pane());
                    self.view_slots.insert(
                        pane_id.clone(),
                        ViewSlot {
                            rect: remaining,
                            nav: nav.clone(),
                        },
                    );
                }
                ViewNav::None => {}
            }
        });
        if let Some(p) = load_path {
            self.set_view_path(pane_id, &p);
        }
    }

    fn sync_webviews(&mut self, frame: &mut eframe::Frame) {
        let hide = self.palette_open();
        let live: HashSet<PaneId> = self.view_slots.keys().cloned().collect();

        self.overlays.retain(|id, ov| {
            let keep = self.components.get(id).is_some();
            if !keep || !live.contains(id) || hide {
                let _ = ov.webview.set_visible(false);
            }
            keep
        });

        if hide {
            return;
        }

        let slots: Vec<(PaneId, ViewSlot)> = self
            .view_slots
            .iter()
            .map(|(id, slot)| (id.clone(), slot.clone()))
            .collect();

        for (id, slot) in slots {
            let bounds = wry_rect(slot.rect);
            if !self.overlays.contains_key(&id) {
                match create_overlay(frame, bounds) {
                    Ok(ov) => {
                        self.overlays.insert(id.clone(), ov);
                    }
                    Err(e) => {
                        self.status = format!("webview: {e}");
                        continue;
                    }
                }
            }
            let Some(ov) = self.overlays.get_mut(&id) else {
                continue;
            };
            let _ = ov.webview.set_bounds(bounds);
            let _ = ov.webview.set_visible(true);
            let key = match &slot.nav {
                ViewNav::Html { key, .. } | ViewNav::Url { key, .. } => key.as_str(),
                _ => continue,
            };
            if ov.loaded_key == key {
                continue;
            }
            let ok = match &slot.nav {
                ViewNav::Html { html, .. } => ov.webview.load_html(html),
                ViewNav::Url { url, .. } => ov.webview.load_url(url),
                _ => Ok(()),
            };
            if let Err(e) = ok {
                self.status = format!("webview load: {e}");
            } else {
                ov.loaded_key = key.to_string();
            }
        }
    }

    /// Render a terminal grid (cells + cursor) — static, no &self needed.
    fn render_grid(
        ui: &mut egui::Ui,
        content_rect: egui::Rect,
        cells: &[pworkspaces::terminal::RenderableCell],
        cursor: &pworkspaces::terminal::CursorState,
        cols: usize,
        lines: usize,
        is_focused: bool,
    ) {
        let painter = ui.painter();
        let x_offset = content_rect.left() + PADDING_X / 2.0;
        let y_offset = content_rect.top() + PADDING_Y / 2.0;

        for cell in cells {
            if cell.line >= lines || cell.col >= cols {
                continue;
            }
            if cell.attrs.wide_spacer {
                continue;
            }

            let x = x_offset + cell.col as f32 * CELL_WIDTH;
            let y = y_offset + cell.line as f32 * CELL_HEIGHT;

            let bg = term_color_to_egui(cell.bg);
            if bg != bg_pane() || cell.selected {
                let fill = if cell.selected {
                    egui::Color32::from_rgb(42, 212, 163)
                } else {
                    bg
                };
                let cell_rect = egui::Rect::from_min_size(
                    egui::pos2(x, y),
                    egui::vec2(
                        if cell.attrs.wide { CELL_WIDTH * 2.0 } else { CELL_WIDTH },
                        CELL_HEIGHT,
                    ),
                );
                painter.rect_filled(cell_rect, 0.0, fill);
            }

            if cell.c != ' ' && cell.c != '\0' {
                let fg = if cell.selected {
                    egui::Color32::from_rgb(12, 12, 12)
                } else {
                    term_color_to_egui(cell.fg)
                };
                let font = egui::FontId::monospace(FONT_SIZE);
                let mut buf = [0u8; 4];
                let s = cell.c.encode_utf8(&mut buf);
                painter.text(egui::pos2(x, y), egui::Align2::LEFT_TOP, s, font, fg);
            }

            if cell.attrs.underline && cell.c != ' ' {
                let fg = term_color_to_egui(cell.fg);
                painter.line_segment(
                    [
                        egui::pos2(x, y + CELL_HEIGHT - 1.0),
                        egui::pos2(x + CELL_WIDTH, y + CELL_HEIGHT - 1.0),
                    ],
                    egui::Stroke::new(1.0, fg),
                );
            }

            if cell.attrs.strikethrough && cell.c != ' ' {
                let fg = term_color_to_egui(cell.fg);
                painter.line_segment(
                    [
                        egui::pos2(x, y + CELL_HEIGHT / 2.0),
                        egui::pos2(x + CELL_WIDTH, y + CELL_HEIGHT / 2.0),
                    ],
                    egui::Stroke::new(1.0, fg),
                );
            }
        }

        if is_focused && cursor.visible && cursor.line < lines && cursor.col < cols {
            let cx = x_offset + cursor.col as f32 * CELL_WIDTH;
            let cy = y_offset + cursor.line as f32 * CELL_HEIGHT;
            let cursor_rect = egui::Rect::from_min_size(
                egui::pos2(cx, cy),
                egui::vec2(CELL_WIDTH, CELL_HEIGHT),
            );
            painter.rect_filled(
                cursor_rect,
                0.0,
                cursor_fill(),
            );
        }
    }

    /// Render styled text lines — static, no &self needed.
    fn render_lines(
        ui: &mut egui::Ui,
        content_rect: egui::Rect,
        header: Option<&pworkspaces::component::StyledLine>,
        subheader: Option<&pworkspaces::component::StyledLine>,
        lines: &[pworkspaces::component::StyledLine],
    ) {
        let painter = ui.painter();
        let font = egui::FontId::monospace(FONT_SIZE);
        let x = content_rect.left() + PADDING_X;
        let mut y = content_rect.top() + PADDING_Y;

        if let Some(hdr) = header {
            let mut hx = x;
            for span in &hdr.spans {
                let color = egui::Color32::from_rgb(span.color.r, span.color.g, span.color.b);
                let galley = painter.layout_no_wrap(
                    span.text.clone(),
                    egui::FontId::proportional(13.0),
                    color,
                );
                painter.galley(egui::pos2(hx, y), galley.clone(), color);
                hx += galley.size().x;
            }
            y += 20.0;

            painter.line_segment(
                [
                    egui::pos2(content_rect.left() + 4.0, y),
                    egui::pos2(content_rect.right() - 4.0, y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_rgb(50, 50, 50)),
            );
            y += 8.0;
        }

        if let Some(sub) = subheader {
            for span in &sub.spans {
                let color = egui::Color32::from_rgb(span.color.r, span.color.g, span.color.b);
                painter.text(
                    egui::pos2(x, y),
                    egui::Align2::LEFT_TOP,
                    &span.text,
                    font.clone(),
                    color,
                );
            }
            y += CELL_HEIGHT + 6.0;
        }

        for line in lines {
            if y + CELL_HEIGHT > content_rect.bottom() {
                break;
            }
            let mut lx = x;
            for span in &line.spans {
                let color = egui::Color32::from_rgb(span.color.r, span.color.g, span.color.b);
                let galley = painter.layout_no_wrap(span.text.clone(), font.clone(), color);
                painter.galley(egui::pos2(lx, y), galley.clone(), color);
                lx += galley.size().x;
            }
            y += CELL_HEIGHT;
        }
    }

    fn handle_floating_mouse(&mut self, ctx: &egui::Context, floating_panes: &[FloatingInfo]) {
        // Collect actions, execute outside ctx.input()
        enum FloatAction {
            StartDrag(PaneId, egui::Vec2),
            Drag(PaneId, f32, f32),
            StartResize(PaneId),
            Resize(PaneId, f32, f32),
            StopDrag,
            Focus(PaneId),
            SetCursor(egui::CursorIcon),
        }
        let mut actions: Vec<FloatAction> = Vec::new();

        ctx.input(|i| {
            let pointer = &i.pointer;
            let Some(pos) = pointer.hover_pos() else { return };

            // Check floating panes in reverse z-order (front first)
            for fp in floating_panes.iter().rev() {
                let pane_rect = match self.pane_rects.get(&fp.pane_id) {
                    Some(r) => *r,
                    None => continue,
                };

                // Title bar
                let title_rect = egui::Rect::from_min_size(
                    pane_rect.left_top(),
                    egui::vec2(pane_rect.width(), TITLE_H),
                );

                // Resize zone = bottom-right 12px corner
                let resize_zone = egui::Rect::from_min_max(
                    egui::pos2(pane_rect.right() - 12.0, pane_rect.bottom() - 12.0),
                    pane_rect.right_bottom(),
                );

                // Hover cursor for resize zone
                if resize_zone.contains(pos) {
                    actions.push(FloatAction::SetCursor(egui::CursorIcon::ResizeNwSe));
                }

                if pointer.primary_pressed() {
                    if resize_zone.contains(pos) {
                        actions.push(FloatAction::StartResize(fp.pane_id.clone()));
                        actions.push(FloatAction::Focus(fp.pane_id.clone()));
                        break;
                    } else if title_rect.contains(pos) {
                        let offset = pos - pane_rect.left_top();
                        actions.push(FloatAction::StartDrag(fp.pane_id.clone(), offset));
                        actions.push(FloatAction::Focus(fp.pane_id.clone()));
                        break;
                    } else if pane_rect.contains(pos) {
                        actions.push(FloatAction::Focus(fp.pane_id.clone()));
                        break;
                    }
                }
            }

            // Handle active floating drag
            if let Some((ref pane_id, offset)) = self.dragging_float {
                if pointer.primary_down() {
                    let new_x = pos.x - offset.x;
                    let new_y = pos.y - offset.y;
                    actions.push(FloatAction::Drag(pane_id.clone(), new_x, new_y));
                    actions.push(FloatAction::SetCursor(egui::CursorIcon::Grabbing));
                } else {
                    actions.push(FloatAction::StopDrag);
                }
            }

            // Handle active floating resize
            if let Some(ref pane_id) = self.resizing_float {
                if pointer.primary_down() {
                    if floating_panes.iter().any(|f| f.pane_id == *pane_id) {
                        if let Some(pane_rect) = self.pane_rects.get(pane_id) {
                            let new_w = (pos.x - pane_rect.left()).max(150.0);
                            let new_h = (pos.y - pane_rect.top()).max(100.0);
                            actions.push(FloatAction::Resize(pane_id.clone(), new_w, new_h));
                            actions.push(FloatAction::SetCursor(egui::CursorIcon::ResizeNwSe));
                        }
                    }
                } else {
                    actions.push(FloatAction::StopDrag);
                }
            }
        });

        // Execute float actions
        for action in actions {
            match action {
                FloatAction::StartDrag(pane_id, offset) => {
                    self.dragging_float = Some((pane_id, offset));
                    self.resizing_float = None;
                }
                FloatAction::Drag(pane_id, x, y) => {
                    let ws = self.ws_key();
                    let geom = self.snap.floating.iter_mut().find(|f| f.pane_id == pane_id.as_str()).map(|fp| {
                        fp.x = x;
                        fp.y = y;
                        (fp.x, fp.y, fp.width, fp.height)
                    });
                    if let Some((x, y, w, h)) = geom {
                        let _ = self.client.set_float_geom(&ws, pane_id.as_str(), x, y, w, h);
                    }
                }
                FloatAction::StartResize(pane_id) => {
                    self.resizing_float = Some(pane_id);
                    self.dragging_float = None;
                }
                FloatAction::Resize(pane_id, w, h) => {
                    let ws = self.ws_key();
                    let geom = self.snap.floating.iter_mut().find(|f| f.pane_id == pane_id.as_str()).map(|fp| {
                        fp.width = w;
                        fp.height = h;
                        (fp.x, fp.y, fp.width, fp.height)
                    });
                    if let Some((x, y, w, h)) = geom {
                        let _ = self.client.set_float_geom(&ws, pane_id.as_str(), x, y, w, h);
                    }
                }
                FloatAction::StopDrag => {
                    self.dragging_float = None;
                    self.resizing_float = None;
                }
                FloatAction::Focus(pane_id) => {
                    let _ = self.client.focus_pane(&self.ws_key(), pane_id.as_str());
                    self.refresh_snap();
                }
                FloatAction::SetCursor(cursor) => {
                    ctx.set_cursor_icon(cursor);
                }
            }
        }
    }
}

enum TabAction {
    Switch(WorkspaceId),
    Close(WorkspaceId),
    Create,
    StartRename { id: WorkspaceId, name: String },
    CancelRename,
}

impl Drop for WorkspaceApp {
    fn drop(&mut self) {
        match self.leave {
            LeaveMode::Detach => {
                if let Err(e) = self.client.save() {
                    eprintln!("detach save: {e}");
                } else {
                    eprintln!(
                        "detached — runtime alive at {}",
                        pworkspaces::ipc::socket_path().display()
                    );
                }
            }
            LeaveMode::Kill => {
                eprintln!("runtime stopped");
            }
            LeaveMode::Running => {
                let _ = self.client.save();
                eprintln!(
                    "detached — runtime alive at {}",
                    pworkspaces::ipc::socket_path().display()
                );
            }
        }
    }
}

fn split_rect(
    rect: egui::Rect,
    direction: pworkspaces::layout::Direction,
    ratio: f32,
    gap: f32,
) -> (egui::Rect, egui::Rect) {
    use pworkspaces::layout::Direction;

    match direction {
        Direction::Horizontal => {
            let split_x = rect.left() + rect.width() * ratio - gap / 2.0;
            let first =
                egui::Rect::from_min_max(rect.left_top(), egui::pos2(split_x, rect.bottom()));
            let second = egui::Rect::from_min_max(
                egui::pos2(split_x + gap, rect.top()),
                rect.right_bottom(),
            );
            (first, second)
        }
        Direction::Vertical => {
            let split_y = rect.top() + rect.height() * ratio - gap / 2.0;
            let first =
                egui::Rect::from_min_max(rect.left_top(), egui::pos2(rect.right(), split_y));
            let second = egui::Rect::from_min_max(
                egui::pos2(rect.left(), split_y + gap),
                rect.right_bottom(),
            );
            (first, second)
        }
    }
}

fn wry_rect(rect: egui::Rect) -> wry::Rect {
    wry::Rect {
        position: wry::dpi::LogicalPosition::new(rect.min.x as f64, rect.min.y as f64).into(),
        size: wry::dpi::LogicalSize::new(
            rect.width().max(1.0) as f64,
            rect.height().max(1.0) as f64,
        )
        .into(),
    }
}

fn create_overlay(
    window: &impl HasWindowHandle,
    bounds: wry::Rect,
) -> Result<WebOverlay, String> {
    let webview = wry::WebViewBuilder::new()
        .with_bounds(bounds)
        .with_visible(true)
        .with_devtools(cfg!(debug_assertions))
        .with_accept_first_mouse(true)
        .with_initialization_script(PW_WEBVIEW_JS)
        .with_ipc_handler(|req| {
            eprintln!("[pw webview ipc] {}", req.body());
        })
        .with_html("<!DOCTYPE html><html><body style=\"background:#121212\"></body></html>")
        .build_as_child(window)
        .map_err(|e| e.to_string())?;
    Ok(WebOverlay {
        webview,
        loaded_key: String::new(),
    })
}
