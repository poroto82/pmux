use std::path::PathBuf;

use pmux::action::{ActionContext, ActionRegistry, ActionResult};
use pmux::component::ComponentRegistry;
use pmux::files;
use pmux::ids::{ComponentId, PaneId, SessionId, WorkspaceId};
use pmux::ipc;
use pmux::layout::Direction;
use pmux::monitor::{self, ListenPort};

use crate::event::{Event, EventBus};
use crate::names;
use crate::permission::{AccessRequest, AuthResult, Authorization, Permission};
use crate::persistence::PersistenceManager;
use crate::session::{SessionError, SessionRegistry, SessionSpawnEnv};
use crate::watch::PortWatch;
use crate::workspace::WorkspaceRegistry;

/// The Workspace Runtime — central coordinator.
///
/// Owns all registries and provides a unified API.
/// UI, CLI, and Agents are all clients of this.
pub struct Runtime {
    pub workspaces: WorkspaceRegistry,
    pub sessions: SessionRegistry,
    pub events: EventBus,
    pub actions: ActionRegistry,
    pub auth: Authorization,
    pub components: ComponentRegistry,
    persistence: PersistenceManager,
    port_watch: PortWatch,
}

impl Runtime {
    pub fn new(persistence_path: impl Into<std::path::PathBuf>) -> Self {
        let mut rt = Self {
            workspaces: WorkspaceRegistry::new(),
            sessions: SessionRegistry::new(),
            events: EventBus::new(),
            actions: ActionRegistry::new(),
            auth: Authorization::new(),
            components: {
                let mut reg = ComponentRegistry::new();
                reg.load_plugins();
                reg
            },
            persistence: PersistenceManager::new(persistence_path),
            port_watch: PortWatch::new(),
        };
        rt.register_default_actions();
        rt
    }

    pub fn with_defaults() -> Self {
        Self::new(PersistenceManager::default_path())
    }

    /// Poll host sensors (ports, …) and emit diffs on the event bus.
    pub fn tick_watchers(&mut self) {
        let Some(diff) = self.port_watch.poll() else {
            return;
        };
        for p in diff.opened {
            self.events.emit(&Event::PortOpened {
                port: p.port().unwrap_or(0),
                pid: p.pid,
                addr: p.addr,
                command: p.command,
            });
        }
        for p in diff.closed {
            self.events.emit(&Event::PortClosed {
                port: p.port().unwrap_or(0),
                pid: p.pid,
                addr: p.addr,
                command: p.command,
            });
        }
    }

    pub fn listen_ports(&self) -> &[ListenPort] {
        self.port_watch.rows()
    }

    pub fn listen_ports_error(&self) -> Option<&str> {
        self.port_watch.error()
    }

    /// SIGTERM + force next port poll (UI kill / plugin action).
    pub fn kill_listen_pid(&mut self, pid: u32) -> Result<(), String> {
        monitor::kill_pid(pid).map_err(|e| e.to_string())?;
        self.port_watch.force();
        self.tick_watchers();
        Ok(())
    }

    /// UI frame snapshot (daemon → UI).
    pub fn ui_snapshot(&self, workspace_key: Option<&str>) -> ipc::UiSnapshot {
        use ipc::{FloatSnap, PaneSnap, UiSnapshot, WorkspaceTabSnap};

        let active_id = self.workspaces.active_id().cloned();
        let mut tabs: Vec<WorkspaceTabSnap> = self
            .workspaces
            .list()
            .iter()
            .map(|ws| WorkspaceTabSnap {
                id: ws.id.to_string(),
                name: ws.name.clone(),
                cwd: ws.cwd.clone(),
                active: Some(&ws.id) == active_id.as_ref(),
                pane_count: ws.pane_count(),
            })
            .collect();
        tabs.sort_by(|a, b| a.id.cmp(&b.id));

        let ports = self.listen_ports().to_vec();
        let ports_err = self.listen_ports_error().map(|s| s.to_string());

        let ws = workspace_key
            .and_then(|k| self.workspaces.resolve_workspace(k))
            .or_else(|| self.workspaces.active());

        let Some(ws) = ws else {
            return UiSnapshot {
                active_id: active_id.map(|id| id.to_string()),
                workspaces: tabs,
                layout_root: None,
                focused: None,
                fullscreen: None,
                panes: Vec::new(),
                floating: Vec::new(),
                listen_ports: ports,
                listen_ports_error: ports_err,
            };
        };

        let panes: Vec<PaneSnap> = ws
            .panes()
            .map(|p| {
                let size = self.pane_pty_size(&ws.id, &p.id);
                PaneSnap {
                    id: p.id.to_string(),
                    name: p.name.clone(),
                    component_type: p.component_type.clone(),
                    source: p.source.clone(),
                    session_alive: self.pane_has_live_session(&ws.id, &p.id),
                    pty_cols: size.map(|s| s.0),
                    pty_rows: size.map(|s| s.1),
                }
            })
            .collect();

        let floating: Vec<FloatSnap> = ws
            .layout()
            .floating()
            .sorted()
            .iter()
            .map(|fp| FloatSnap {
                pane_id: fp.pane_id.to_string(),
                x: fp.x,
                y: fp.y,
                width: fp.width,
                height: fp.height,
            })
            .collect();

        UiSnapshot {
            active_id: active_id.map(|id| id.to_string()),
            workspaces: tabs,
            layout_root: ws.layout().root().cloned(),
            focused: ws.layout().focused().map(|id| id.to_string()),
            fullscreen: ws.layout().fullscreened().map(|id| id.to_string()),
            panes,
            floating,
            listen_ports: ports,
            listen_ports_error: ports_err,
        }
    }

    /// Close terminal panes whose PTY died (runs in daemon, including while detached).
    pub fn reap_dead_panes(&mut self) {
        let ws_ids: Vec<WorkspaceId> = self
            .workspaces
            .list()
            .iter()
            .map(|w| w.id.clone())
            .collect();
        for ws_id in ws_ids {
            let dead: Vec<PaneId> = {
                let Some(ws) = self.workspaces.get(&ws_id) else {
                    continue;
                };
                ws.panes()
                    .filter_map(|p| match &p.session_id {
                        Some(sid) if !self.sessions.is_alive(sid) => Some(p.id.clone()),
                        _ => None,
                    })
                    .collect()
            };
            for pane_id in dead {
                self.close_pane(&ws_id, &pane_id);
            }
        }
    }

    pub fn resize_pty(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        cols: u16,
        rows: u16,
    ) -> Result<(), RuntimeError> {
        let sess_id = self.resolve_session_ref(workspace_id, pane_id)?;
        self.sessions
            .get_mut(&sess_id)
            .ok_or_else(|| RuntimeError::NoSession(pane_id.to_string()))?
            .resize(cols, rows)
            .map_err(RuntimeError::Session)
    }

    pub fn swap_panes(&mut self, ws: &WorkspaceId, a: &PaneId, b: &PaneId) -> bool {
        self.workspaces
            .get_mut(ws)
            .map(|w| w.swap(a, b))
            .unwrap_or(false)
    }

    pub fn resize_split_at(&mut self, ws: &WorkspaceId, index: usize, ratio: f32) -> bool {
        self.workspaces
            .get_mut(ws)
            .map(|w| w.resize_split(index, ratio))
            .unwrap_or(false)
    }

    pub fn set_float_geom(
        &mut self,
        ws: &WorkspaceId,
        pane: &PaneId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> bool {
        let Some(w) = self.workspaces.get_mut(ws) else {
            return false;
        };
        let Some(fp) = w.layout_mut().floating_mut().get_mut(pane) else {
            return false;
        };
        fp.set_position(x, y);
        fp.set_size(width, height);
        true
    }

    // --- Workspace API ---

    pub fn create_workspace(&mut self, name: impl Into<String>) -> WorkspaceId {
        let name = name.into();
        let id = self.workspaces.create(&name);
        self.events.emit(&Event::WorkspaceCreated {
            workspace_id: id.clone(),
            name,
        });
        id
    }

    /// Next unused adjective_noun workspace label.
    pub fn next_workspace_name(&self) -> String {
        names::generate_unique_workspace_name(
            self.workspaces.list().iter().map(|ws| ws.name.as_str()),
        )
    }

    pub fn rename_workspace(
        &mut self,
        id: &WorkspaceId,
        name: &str,
    ) -> Result<String, RuntimeError> {
        let clean = names::sanitize_workspace_name(name)
            .ok_or_else(|| RuntimeError::InvalidName("empty name".into()))?;
        self.workspaces
            .rename(id, clean)
            .map_err(RuntimeError::InvalidName)
    }

    pub fn set_workspace_cwd(
        &mut self,
        id: &WorkspaceId,
        cwd: impl Into<String>,
    ) -> Result<String, RuntimeError> {
        let path = files::expand_path(&cwd.into());
        if !path.is_dir() {
            return Err(RuntimeError::InvalidName(format!(
                "not a directory: {}",
                path.display()
            )));
        }
        let s = path.display().to_string();
        let ws = self
            .workspaces
            .get_mut(id)
            .ok_or_else(|| RuntimeError::WorkspaceNotFound(id.to_string()))?;
        ws.cwd = Some(s.clone());
        Ok(s)
    }

    /// Quick-open root: live terminal cwd → workspace.cwd → process cwd.
    pub fn quick_open_root(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: Option<&PaneId>,
    ) -> PathBuf {
        if let Some(pid) = pane_id.and_then(|p| self.session_pid(workspace_id, p)) {
            if let Some(cwd) = monitor::process_cwd(pid) {
                return cwd;
            }
        }
        if let Some(cwd) = self
            .workspaces
            .get(workspace_id)
            .and_then(|ws| ws.cwd.as_deref())
        {
            let p = files::expand_path(cwd);
            if p.is_dir() {
                return p;
            }
        }
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    }

    fn session_pid(&self, workspace_id: &WorkspaceId, pane_id: &PaneId) -> Option<u32> {
        let sid = self
            .workspaces
            .get(workspace_id)?
            .pane(pane_id)?
            .session_id
            .as_ref()?;
        self.sessions.get(sid)?.pid()
    }

    pub fn destroy_workspace(&mut self, id: &WorkspaceId) -> bool {
        // Destroy all sessions in this workspace
        if let Some(ws) = self.workspaces.get(id) {
            let pane_ids: Vec<PaneId> = ws.panes().map(|p| p.id.clone()).collect();
            for pane_id in &pane_ids {
                if let Some(pane) = ws.pane(pane_id) {
                    if let Some(sess_id) = &pane.session_id {
                        self.sessions.destroy(sess_id);
                    }
                }
            }
        }

        if self.workspaces.destroy(id) {
            self.events.emit(&Event::WorkspaceDestroyed {
                workspace_id: id.clone(),
            });
            true
        } else {
            false
        }
    }

    pub fn switch_workspace(&mut self, id: &WorkspaceId) -> bool {
        let from = self.workspaces.active_id().cloned();
        if self.workspaces.switch(id) {
            self.events.emit(&Event::WorkspaceSwitched {
                from,
                to: id.clone(),
            });
            true
        } else {
            false
        }
    }

    // --- Pane API ---

    /// Add first pane to a workspace, optionally spawning a session.
    pub fn add_pane(
        &mut self,
        workspace_id: &WorkspaceId,
        component_id: ComponentId,
        name: Option<&str>,
        spawn_session: bool,
    ) -> Option<PaneId> {
        let ws = self.workspaces.get_mut(workspace_id)?;
        let pane_id = if let Some(n) = name {
            ws.add_named_pane(component_id, n)
        } else {
            ws.add_pane(component_id)
        };

        if spawn_session {
            let _ = self.attach_fresh_session(workspace_id, &pane_id);
        }

        self.events.emit(&Event::PaneCreated {
            workspace_id: workspace_id.clone(),
            pane_id: pane_id.clone(),
        });

        Some(pane_id)
    }

    /// Add a pane with a specific component type (e.g. "logs", "processes").
    /// Only spawns a session if component_type is "terminal".
    pub fn add_pane_typed(
        &mut self,
        workspace_id: &WorkspaceId,
        component_id: ComponentId,
        name: Option<&str>,
        component_type: &str,
    ) -> Option<PaneId> {
        let spawn_session = component_type == "terminal";

        // If tree is empty, add as root. Otherwise, split from focused.
        let is_empty = self.workspaces.get(workspace_id)?.layout().is_empty();
        let pane_id = if is_empty {
            self.add_pane(workspace_id, component_id, name, spawn_session)?
        } else {
            self.split(
                workspace_id,
                None,
                Direction::Horizontal,
                component_id,
                name,
                spawn_session,
            )?
        };

        // Set component type on the pane
        let ws = self.workspaces.get_mut(workspace_id)?;
        if let Some(pane) = ws.pane_mut(&pane_id) {
            pane.component_type = component_type.to_string();
        }

        Some(pane_id)
    }

    /// Open a local file (or URL) in a view pane. Reuses the first view pane
    /// in the workspace, or creates one.
    pub fn open_view(
        &mut self,
        workspace_id: &WorkspaceId,
        path: &str,
    ) -> Result<PaneId, RuntimeError> {
        if self.workspaces.get(workspace_id).is_none() {
            return Err(RuntimeError::WorkspaceNotFound(workspace_id.to_string()));
        }

        let existing = self
            .workspaces
            .get(workspace_id)
            .and_then(|ws| {
                ws.panes()
                    .find(|p| p.component_type == "view")
                    .map(|p| p.id.clone())
            });

        let pane_id = match existing {
            Some(id) => id,
            None => self
                .add_pane_typed(workspace_id, ComponentId::new(), Some("preview"), "view")
                .ok_or_else(|| RuntimeError::PaneNotFound("view".into()))?,
        };

        let label = std::path::Path::new(path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("preview")
            .to_string();

        if let Some(pane) = self
            .workspaces
            .get_mut(workspace_id)
            .and_then(|ws| ws.pane_mut(&pane_id))
        {
            pane.source = Some(path.to_string());
            pane.name = Some(label.clone());
        }
        self.focus_pane(workspace_id, &pane_id);
        Ok(pane_id)
    }

    /// Split a pane (or focused pane) in the active workspace.
    pub fn split(
        &mut self,
        workspace_id: &WorkspaceId,
        target: Option<&PaneId>,
        direction: Direction,
        component_id: ComponentId,
        name: Option<&str>,
        spawn_session: bool,
    ) -> Option<PaneId> {
        let ws = self.workspaces.get_mut(workspace_id)?;
        let source_pane = target
            .cloned()
            .or_else(|| ws.layout().focused().cloned())?;

        let new_pane_id = ws.split(
            target,
            direction,
            component_id,
            name.map(|s| s.to_string()),
        )?;

        if spawn_session {
            let _ = self.attach_fresh_session(workspace_id, &new_pane_id);
        }

        self.events.emit(&Event::PaneCreated {
            workspace_id: workspace_id.clone(),
            pane_id: new_pane_id.clone(),
        });
        self.events.emit(&Event::PaneSplit {
            workspace_id: workspace_id.clone(),
            source_pane,
            new_pane: new_pane_id.clone(),
        });

        Some(new_pane_id)
    }

    /// Close a pane, destroying its session.
    pub fn close_pane(&mut self, workspace_id: &WorkspaceId, pane_id: &PaneId) -> bool {
        // Get session before closing
        let sess_id = self
            .workspaces
            .get(workspace_id)
            .and_then(|ws| ws.pane(pane_id))
            .and_then(|p| p.session_id.clone());

        let ws = match self.workspaces.get_mut(workspace_id) {
            Some(ws) => ws,
            None => return false,
        };

        if ws.close_pane(pane_id) {
            if let Some(sid) = sess_id {
                self.sessions.destroy(&sid);
            }
            self.events.emit(&Event::PaneClosed {
                workspace_id: workspace_id.clone(),
                pane_id: pane_id.clone(),
            });
            let empty = self
                .workspaces
                .get(workspace_id)
                .map(|w| w.pane_count() == 0)
                .unwrap_or(false);
            if empty {
                self.reap_empty_workspace(workspace_id);
            }
            true
        } else {
            false
        }
    }

    /// Last pane closed → destroy workspace. App always keeps ≥1 workspace.
    fn reap_empty_workspace(&mut self, workspace_id: &WorkspaceId) {
        self.destroy_workspace(workspace_id);
        if self.workspaces.count() == 0 {
            let name = self.next_workspace_name();
            let id = self.create_workspace(&name);
            let _ = self.add_pane(&id, ComponentId::new(), None, true);
            self.switch_workspace(&id);
        }
    }

    /// Focus a pane.
    pub fn focus_pane(&mut self, workspace_id: &WorkspaceId, pane_id: &PaneId) -> bool {
        let ws = match self.workspaces.get_mut(workspace_id) {
            Some(ws) => ws,
            None => return false,
        };
        if ws.focus(pane_id) {
            self.events.emit(&Event::PaneFocused {
                workspace_id: workspace_id.clone(),
                pane_id: pane_id.clone(),
            });
            true
        } else {
            false
        }
    }

    /// Float a tiled pane.
    pub fn float_pane(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> bool {
        let ws = match self.workspaces.get_mut(workspace_id) {
            Some(ws) => ws,
            None => return false,
        };
        if ws.float_pane(pane_id, x, y, width, height) {
            self.events.emit(&Event::PaneFloated {
                workspace_id: workspace_id.clone(),
                pane_id: pane_id.clone(),
            });
            true
        } else {
            false
        }
    }

    /// Tile a floating pane.
    pub fn tile_pane(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        direction: Direction,
    ) -> bool {
        let ws = match self.workspaces.get_mut(workspace_id) {
            Some(ws) => ws,
            None => return false,
        };
        if ws.tile_pane(pane_id, direction) {
            self.events.emit(&Event::PaneTiled {
                workspace_id: workspace_id.clone(),
                pane_id: pane_id.clone(),
            });
            true
        } else {
            false
        }
    }

    /// Float with auth check.
    pub fn float_pane_as(
        &mut self,
        actor: &str,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Result<bool, RuntimeError> {
        self.check_auth(actor, Permission::FloatPane, Some(workspace_id), Some(pane_id))?;
        Ok(self.float_pane(workspace_id, pane_id, x, y, width, height))
    }

    /// Tile with auth check.
    pub fn tile_pane_as(
        &mut self,
        actor: &str,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        direction: Direction,
    ) -> Result<bool, RuntimeError> {
        self.check_auth(actor, Permission::TilePane, Some(workspace_id), Some(pane_id))?;
        Ok(self.tile_pane(workspace_id, pane_id, direction))
    }


    // --- Session/Command API (spec §30-31) ---

    /// Send a command to a pane's session. This is the high-level API
    /// that agents and automation use (spec §37: send_command).
    pub fn send_command(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        command: &str,
    ) -> Result<(), RuntimeError> {
        let sess_id = self.resolve_session(workspace_id, pane_id)?;
        self.sessions
            .send_command(&sess_id, command)
            .map_err(RuntimeError::Session)
    }

    /// Send raw input to a pane's session (spec §37: send_input).
    pub fn send_input(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        data: &[u8],
    ) -> Result<(), RuntimeError> {
        let sess_id = self.resolve_session(workspace_id, pane_id)?;
        self.sessions
            .send_input(&sess_id, data)
            .map_err(RuntimeError::Session)
    }

    /// Read output from a pane's session (spec §36: read_output).
    pub fn read_output(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<Vec<u8>, RuntimeError> {
        let sess_id = self.resolve_session_ref(workspace_id, pane_id)?;
        self.sessions
            .read_output(&sess_id)
            .map_err(RuntimeError::Session)
    }

    /// Replay buffer for UI reattach (does not drain live unread).
    pub fn replay_output(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<Vec<u8>, RuntimeError> {
        let sess_id = self.resolve_session_ref(workspace_id, pane_id)?;
        self.sessions
            .replay_output(&sess_id)
            .map_err(RuntimeError::Session)
    }

    /// Resolve workspace_name/pane_name → send command (convenience).
    pub fn send_command_by_name(
        &mut self,
        workspace_name: &str,
        pane_name: &str,
        command: &str,
    ) -> Result<(), RuntimeError> {
        let (ws_id, pane_id) = self.resolve_address(workspace_name, pane_name)?;
        self.send_command(&ws_id, &pane_id, command)
    }

    /// Read output by name (convenience).
    pub fn read_output_by_name(
        &self,
        workspace_name: &str,
        pane_name: &str,
    ) -> Result<Vec<u8>, RuntimeError> {
        let (ws_id, pane_id) = self.resolve_address_ref(workspace_name, pane_name)?;
        self.read_output(&ws_id, &pane_id)
    }

    // --- Authorized API (spec §32: Intent → Authorization → Runtime) ---

    /// Send command with authorization check.
    pub fn send_command_as(
        &mut self,
        actor: &str,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        command: &str,
    ) -> Result<(), RuntimeError> {
        self.check_auth(actor, Permission::SendCommand, Some(workspace_id), Some(pane_id))?;
        self.send_command(workspace_id, pane_id, command)
    }

    /// Read output with authorization check.
    pub fn read_output_as(
        &self,
        actor: &str,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.check_auth(actor, Permission::ReadOutput, Some(workspace_id), Some(pane_id))?;
        self.read_output(workspace_id, pane_id)
    }

    /// Send raw input with authorization check.
    pub fn send_input_as(
        &mut self,
        actor: &str,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
        data: &[u8],
    ) -> Result<(), RuntimeError> {
        self.check_auth(actor, Permission::SendInput, Some(workspace_id), Some(pane_id))?;
        self.send_input(workspace_id, pane_id, data)
    }

    fn check_auth(
        &self,
        actor: &str,
        permission: Permission,
        workspace_id: Option<&WorkspaceId>,
        pane_id: Option<&PaneId>,
    ) -> Result<(), RuntimeError> {
        let result = self.auth.check(&AccessRequest {
            actor,
            permission,
            target_workspace: workspace_id,
            target_pane: pane_id,
        });
        match result {
            AuthResult::Allowed => Ok(()),
            AuthResult::Denied(reason) => Err(RuntimeError::PermissionDenied(reason)),
        }
    }

    // --- Action API ---

    /// Execute a registered action. Built-ins mutate runtime; unknown names
    /// fall through to custom handlers (spec §15–16).
    pub fn execute_action(&mut self, name: &str, ctx: &ActionContext) -> ActionResult {
        if !self.actions.has(name) {
            return ActionResult::Error(format!("unknown action: {}", name));
        }

        match name {
            "split_horizontal" => self.act_split(ctx, Direction::Horizontal),
            "split_vertical" => self.act_split(ctx, Direction::Vertical),
            "close_pane" => self.act_close_pane(ctx),
            "focus_next" => self.act_focus_step(ctx, true),
            "focus_prev" => self.act_focus_step(ctx, false),
            "new_terminal" => self.act_new_typed(ctx, "terminal"),
            "new_ports" => self.act_new_typed(ctx, "ports"),
            "new_processes" => self.act_new_typed(ctx, "processes"),
            "new_job" => self.act_new_typed(ctx, "job"),
            "new_view" => self.act_new_typed(ctx, "view"),
            "toggle_fullscreen" => self.act_toggle_fullscreen(ctx),
            "float_pane" => self.act_float(ctx),
            "tile_pane" => self.act_tile(ctx),
            "toggle_float" => self.act_toggle_float(ctx),
            "new_workspace" => self.act_new_workspace(),
            "next_workspace" => self.act_cycle_workspace(true),
            "prev_workspace" => self.act_cycle_workspace(false),
            "save_layout" => match self.save() {
                Ok(()) => ActionResult::Message("Layout saved".into()),
                Err(e) => ActionResult::Error(e.to_string()),
            },
            "toggle_chrome" => ActionResult::Ok,
            "quick_open" => ActionResult::Ok,
            "rename_workspace" => ActionResult::Ok,
            "detach" => ActionResult::Ok,
            "refresh_terminals" => ActionResult::Ok,
            "connect_runtime" => ActionResult::Ok,
            "kill_runtime" => ActionResult::Ok,
            _ if name.starts_with("new_script:") => {
                let script = name.trim_start_matches("new_script:");
                self.act_new_script(ctx, script)
            }
            _ => self.actions.execute(name, ctx),
        }
    }

    fn ctx_workspace(&self, ctx: &ActionContext) -> Option<WorkspaceId> {
        ctx.workspace_id
            .clone()
            .or_else(|| self.workspaces.active_id().cloned())
    }

    fn ctx_pane(&self, ctx: &ActionContext) -> Option<(WorkspaceId, PaneId)> {
        let ws_id = self.ctx_workspace(ctx)?;
        let pane_id = ctx.pane_id.clone().or_else(|| {
            self.workspaces
                .get(&ws_id)
                .and_then(|ws| ws.layout().focused().cloned())
        })?;
        Some((ws_id, pane_id))
    }

    fn pane_label(&self, ws_id: &WorkspaceId, pane_id: &PaneId) -> String {
        self.workspaces
            .get(ws_id)
            .and_then(|ws| ws.pane(pane_id))
            .and_then(|p| p.name.clone())
            .unwrap_or_else(|| pane_id.to_string())
    }

    fn act_split(&mut self, ctx: &ActionContext, direction: Direction) -> ActionResult {
        let Some(ws_id) = self.ctx_workspace(ctx) else {
            return ActionResult::Error("no workspace".into());
        };
        let target = ctx.pane_id.as_ref();
        match self.split(&ws_id, target, direction, ComponentId::new(), None, true) {
            Some(pane_id) => {
                let name = self.pane_label(&ws_id, &pane_id);
                let dir = if matches!(direction, Direction::Horizontal) {
                    "H"
                } else {
                    "V"
                };
                ActionResult::Message(format!("Split {dir} → {name}"))
            }
            None => ActionResult::Error("split failed".into()),
        }
    }

    fn act_close_pane(&mut self, ctx: &ActionContext) -> ActionResult {
        let Some((ws_id, pane_id)) = self.ctx_pane(ctx) else {
            return ActionResult::Error("no focused pane".into());
        };
        let ws_name = self
            .workspaces
            .get(&ws_id)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        let last = self
            .workspaces
            .get(&ws_id)
            .map(|w| w.pane_count() <= 1)
            .unwrap_or(false);
        if self.close_pane(&ws_id, &pane_id) {
            if last {
                ActionResult::Message(format!("Closed workspace {ws_name}"))
            } else {
                ActionResult::Message("Closed pane".into())
            }
        } else {
            ActionResult::Error("close failed".into())
        }
    }

    fn act_focus_step(&mut self, ctx: &ActionContext, next: bool) -> ActionResult {
        let Some(ws_id) = self.ctx_workspace(ctx) else {
            return ActionResult::Error("no workspace".into());
        };
        let ws = match self.workspaces.get_mut(&ws_id) {
            Some(ws) => ws,
            None => return ActionResult::Error("workspace not found".into()),
        };
        let focused = if next { ws.focus_next() } else { ws.focus_prev() };
        match focused {
            Some(id) => {
                let label = ws
                    .pane(&id)
                    .and_then(|p| p.name.clone())
                    .unwrap_or_else(|| id.to_string());
                ActionResult::Message(format!("Focus → {label}"))
            }
            None => ActionResult::Error("no pane to focus".into()),
        }
    }

    fn act_new_typed(&mut self, ctx: &ActionContext, component_type: &str) -> ActionResult {
        let Some(ws_id) = self.ctx_workspace(ctx) else {
            return ActionResult::Error("no workspace".into());
        };
        match self.add_pane_typed(&ws_id, ComponentId::new(), None, component_type) {
            Some(pane_id) => {
                let name = self.pane_label(&ws_id, &pane_id);
                ActionResult::Message(format!("{component_type} → {name}"))
            }
            None => ActionResult::Error(format!("could not add {component_type}")),
        }
    }

    fn act_new_script(&mut self, ctx: &ActionContext, script: &str) -> ActionResult {
        let Some(ws_id) = self.ctx_workspace(ctx) else {
            return ActionResult::Error("no workspace".into());
        };
        match self.add_pane_typed(&ws_id, ComponentId::new(), Some(script), "script") {
            Some(pane_id) => {
                let name = self.pane_label(&ws_id, &pane_id);
                ActionResult::Message(format!("script → {name}"))
            }
            None => ActionResult::Error("could not add script".into()),
        }
    }

    fn act_toggle_fullscreen(&mut self, ctx: &ActionContext) -> ActionResult {
        let Some((ws_id, pane_id)) = self.ctx_pane(ctx) else {
            return ActionResult::Error("no focused pane".into());
        };
        let ws = match self.workspaces.get_mut(&ws_id) {
            Some(ws) => ws,
            None => return ActionResult::Error("workspace not found".into()),
        };
        if !ws.toggle_fullscreen(&pane_id) {
            return ActionResult::Error("fullscreen failed".into());
        }
        let on = ws.layout().fullscreened().is_some();
        ActionResult::Message(if on {
            "Fullscreen ON".into()
        } else {
            "Fullscreen OFF".into()
        })
    }

    fn act_float(&mut self, ctx: &ActionContext) -> ActionResult {
        let Some((ws_id, pane_id)) = self.ctx_pane(ctx) else {
            return ActionResult::Error("no focused pane".into());
        };
        if self.float_pane(&ws_id, &pane_id, 100.0, 100.0, 500.0, 350.0) {
            ActionResult::Message("Floated pane".into())
        } else {
            ActionResult::Error("float failed".into())
        }
    }

    fn act_tile(&mut self, ctx: &ActionContext) -> ActionResult {
        let Some((ws_id, pane_id)) = self.ctx_pane(ctx) else {
            return ActionResult::Error("no focused pane".into());
        };
        if self.tile_pane(&ws_id, &pane_id, Direction::Horizontal) {
            ActionResult::Message("Tiled pane".into())
        } else {
            ActionResult::Error("tile failed".into())
        }
    }

    fn act_toggle_float(&mut self, ctx: &ActionContext) -> ActionResult {
        let Some((ws_id, pane_id)) = self.ctx_pane(ctx) else {
            return ActionResult::Error("no focused pane".into());
        };
        let floating = self
            .workspaces
            .get(&ws_id)
            .map(|ws| ws.is_floating(&pane_id))
            .unwrap_or(false);
        if floating {
            self.act_tile(ctx)
        } else {
            self.act_float(ctx)
        }
    }

    fn act_new_workspace(&mut self) -> ActionResult {
        let name = self.next_workspace_name();
        let ws_id = self.create_workspace(&name);
        let _ = self.add_pane(&ws_id, ComponentId::new(), None, true);
        self.switch_workspace(&ws_id);
        ActionResult::Message(format!("Created {name}"))
    }

    fn act_cycle_workspace(&mut self, next: bool) -> ActionResult {
        let mut ids: Vec<WorkspaceId> = self
            .workspaces
            .list()
            .iter()
            .map(|ws| ws.id.clone())
            .collect();
        if ids.len() < 2 {
            return ActionResult::Message("Only one workspace".into());
        }
        ids.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        let current = self.workspaces.active_id().cloned();
        let idx = current
            .and_then(|id| ids.iter().position(|x| *x == id))
            .unwrap_or(0);
        let new_idx = if next {
            (idx + 1) % ids.len()
        } else {
            (idx + ids.len() - 1) % ids.len()
        };
        let target = ids[new_idx].clone();
        let name = self
            .workspaces
            .get(&target)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        self.switch_workspace(&target);
        ActionResult::Message(format!("Workspace {name}"))
    }

    // --- Persistence API ---

    pub fn save(&self) -> Result<(), crate::persistence::PersistError> {
        self.persistence.save(&self.workspaces)
    }

    pub fn restore(&mut self) -> Result<(), crate::persistence::PersistError> {
        self.workspaces = self.persistence.restore()?;
        // Daemon restart only: previous PTYs are gone. UI detach does not restore —
        // daemon keeps sessions alive. Drop stale ids so respawn_sessions can attach.
        self.workspaces.clear_all_session_ids();
        // Old saves may lack human names — assign Docker-style ones.
        self.workspaces.ensure_all_pane_names();
        Ok(())
    }

    /// Spawn sessions for terminal panes that need one (after restore or repair).
    ///
    /// Respawns when:
    /// - pane has no session_id
    /// - session_id points at a missing/dead session in the registry
    pub fn respawn_sessions(&mut self) {
        let ws_ids: Vec<WorkspaceId> = self.workspaces.list().iter().map(|ws| ws.id.clone()).collect();
        for ws_id in &ws_ids {
            let panes: Vec<(PaneId, Option<SessionId>, String)> = self
                .workspaces
                .get(ws_id)
                .map(|ws| {
                    ws.panes()
                        .map(|p| {
                            (
                                p.id.clone(),
                                p.session_id.clone(),
                                p.component_type.clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();

            for (pane_id, sess_id, component_type) in panes {
                if component_type != "terminal" {
                    continue;
                }
                let needs = match &sess_id {
                    None => true,
                    Some(sid) => !self.sessions.is_alive(sid),
                };
                if needs {
                    let _ = self.attach_fresh_session(ws_id, &pane_id);
                }
            }
        }
    }

    /// Build PW_* env and attach a new session to a pane.
    fn attach_fresh_session(
        &mut self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<SessionId, RuntimeError> {
        // Drop previous dead/missing session id if any
        let old = self
            .workspaces
            .get_mut(workspace_id)
            .and_then(|ws| ws.pane_mut(pane_id))
            .and_then(|pane| pane.detach_session());
        if let Some(old) = old {
            self.sessions.destroy(&old);
        }

        let env = self.pw_env_for(workspace_id, pane_id)?;
        let cwd = self
            .workspaces
            .get(workspace_id)
            .and_then(|ws| ws.cwd.clone());
        let sess_id = self
            .sessions
            .create_with_env(cwd.as_deref(), 80, 24, env)
            .map_err(RuntimeError::Session)?;

        let ws = self
            .workspaces
            .get_mut(workspace_id)
            .ok_or(RuntimeError::WorkspaceNotFound(workspace_id.to_string()))?;
        let pane = ws
            .pane_mut(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id.to_string()))?;
        pane.attach_session(sess_id.clone());
        Ok(sess_id)
    }

    /// Agent-facing environment for a pane's shell.
    fn pw_env_for(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<SessionSpawnEnv, RuntimeError> {
        let ws = self
            .workspaces
            .get(workspace_id)
            .ok_or(RuntimeError::WorkspaceNotFound(workspace_id.to_string()))?;
        let pane = ws
            .pane(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id.to_string()))?;

        let sock = ipc::socket_path().display().to_string();
        let mut env = SessionSpawnEnv::new()
            .insert("PW_WORKSPACE_ID", ws.id.as_str())
            .insert("PW_WORKSPACE_NAME", &ws.name)
            .insert("PW_PANE_ID", pane.id.as_str())
            .insert("PMUX_SOCK", &sock)
            .insert("PATH", pmux::paths::path_with_pwctl());

        if let Some(name) = &pane.name {
            env = env.insert("PW_PANE_NAME", name);
        } else {
            env = env.insert("PW_PANE_NAME", "");
        }

        Ok(env)
    }

    /// True if pane has a live session in the registry.
    pub fn pane_has_live_session(&self, workspace_id: &WorkspaceId, pane_id: &PaneId) -> bool {
        self.workspaces
            .get(workspace_id)
            .and_then(|ws| ws.pane(pane_id))
            .and_then(|p| p.session_id.as_ref())
            .map(|sid| self.sessions.is_alive(sid))
            .unwrap_or(false)
    }

    fn pane_pty_size(&self, workspace_id: &WorkspaceId, pane_id: &PaneId) -> Option<(u16, u16)> {
        let sid = self.resolve_session_ref(workspace_id, pane_id).ok()?;
        let s = self.sessions.get(&sid)?;
        Some((s.meta.cols, s.meta.rows))
    }

    // --- Resolution helpers ---

    fn resolve_session(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<SessionId, RuntimeError> {
        self.resolve_session_ref(workspace_id, pane_id)
    }

    fn resolve_session_ref(
        &self,
        workspace_id: &WorkspaceId,
        pane_id: &PaneId,
    ) -> Result<SessionId, RuntimeError> {
        let ws = self
            .workspaces
            .get(workspace_id)
            .ok_or(RuntimeError::WorkspaceNotFound(workspace_id.to_string()))?;
        let pane = ws
            .pane(pane_id)
            .ok_or(RuntimeError::PaneNotFound(pane_id.to_string()))?;
        pane.session_id
            .clone()
            .ok_or(RuntimeError::NoSession(pane_id.to_string()))
    }

    fn resolve_address(
        &self,
        workspace_name: &str,
        pane_name: &str,
    ) -> Result<(WorkspaceId, PaneId), RuntimeError> {
        self.resolve_address_ref(workspace_name, pane_name)
    }

    fn resolve_address_ref(
        &self,
        workspace_name: &str,
        pane_name: &str,
    ) -> Result<(WorkspaceId, PaneId), RuntimeError> {
        let (ws, pane) = self
            .workspaces
            .resolve(workspace_name, pane_name)
            .ok_or(RuntimeError::AddressNotFound(format!(
                "{}/{}",
                workspace_name, pane_name
            )))?;
        Ok((ws.id.clone(), pane.id.clone()))
    }

    // --- Default actions ---

    fn register_default_actions(&mut self) {
        let mut add = |name: &str, desc: &str, shortcut: Option<&str>| {
            self.actions
                .register_with_shortcut(name, desc, shortcut, |_| ActionResult::Ok);
        };

        add(
            "split_horizontal",
            "Split pane horizontally",
            Some("Ctrl+Shift+H"),
        );
        add(
            "split_vertical",
            "Split pane vertically",
            Some("Ctrl+Shift+V"),
        );
        add("close_pane", "Close the focused pane", Some("Ctrl+Shift+W"));
        add("focus_next", "Focus next pane", Some("Ctrl+]"));
        add("focus_prev", "Focus previous pane", Some("Ctrl+["));
        add("new_terminal", "New terminal pane", Some("Ctrl+Shift+N"));
        add("new_ports", "New ports pane (runtime watch)", None);
        add("new_processes", "New processes monitor", None);
        add("new_job", "New job (start/stop)", None);
        add("new_view", "New preview pane (WebView)", None);
        add(
            "toggle_fullscreen",
            "Toggle fullscreen",
            Some("Ctrl+Shift+F"),
        );
        add("float_pane", "Float the focused pane", None);
        add("tile_pane", "Tile a floating pane", None);
        add(
            "toggle_float",
            "Toggle floating / tiled",
            Some("Ctrl+Shift+G"),
        );
        add("new_workspace", "New workspace", None);
        add(
            "next_workspace",
            "Next workspace",
            Some("Ctrl+Tab"),
        );
        add(
            "prev_workspace",
            "Previous workspace",
            Some("Ctrl+Shift+Tab"),
        );
        add("save_layout", "Save layout", None);
        add(
            "toggle_chrome",
            "Toggle menu chrome",
            Some("Ctrl+Shift+M"),
        );
        add(
            "quick_open",
            "Quick open file in preview",
            if cfg!(target_os = "macos") {
                Some("⌘P")
            } else {
                Some("Ctrl+Shift+O")
            },
        );
        add(
            "rename_workspace",
            "Rename active workspace",
            Some("double-click tab"),
        );
        add(
            "detach",
            "Detach UI (keep runtime) — ✕ does the same; pwctl stop kills",
            Some("Ctrl+Shift+D"),
        );
        add(
            "refresh_terminals",
            "Re-hydrate terminals from runtime replay (second UI / desync)",
            Some("Ctrl+Shift+R"),
        );
        add(
            "connect_runtime",
            "Connect UI to a sock path or host:port (no terminal env needed)",
            None,
        );
        add(
            "kill_runtime",
            "Stop runtime and kill all sessions",
            None,
        );

        let plugins: Vec<_> = self.components.available_plugins().to_vec();
        for plugin in plugins {
            let key = format!("new_script:{}", plugin.name);
            let desc = if plugin.description.is_empty() {
                format!("Add script: {}", plugin.name)
            } else {
                format!("Add script: {} — {}", plugin.name, plugin.description)
            };
            self.actions
                .register_with_shortcut(&key, desc, None, |_| ActionResult::Ok);
        }
    }
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("workspaces", &self.workspaces.count())
            .field("sessions", &self.sessions.count())
            .field("actions", &self.actions.count())
            .field("components", &self.components.count())
            .finish()
    }
}

#[derive(Debug)]
pub enum RuntimeError {
    WorkspaceNotFound(String),
    PaneNotFound(String),
    NoSession(String),
    AddressNotFound(String),
    PermissionDenied(String),
    InvalidName(String),
    Session(SessionError),
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuntimeError::WorkspaceNotFound(id) => write!(f, "workspace not found: {}", id),
            RuntimeError::PaneNotFound(id) => write!(f, "pane not found: {}", id),
            RuntimeError::NoSession(id) => write!(f, "pane has no session: {}", id),
            RuntimeError::AddressNotFound(addr) => write!(f, "address not found: {}", addr),
            RuntimeError::PermissionDenied(reason) => write!(f, "permission denied: {}", reason),
            RuntimeError::InvalidName(reason) => write!(f, "invalid name: {}", reason),
            RuntimeError::Session(e) => write!(f, "session error: {}", e),
        }
    }
}
