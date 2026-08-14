use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use pmux::ids::{ComponentId, PaneId, WorkspaceId};
use pmux::layout::{Direction, LayoutTree};

use crate::names;
use crate::pane::Pane;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    /// Project root for quick-open + new terminals. None → process cwd.
    #[serde(default)]
    pub cwd: Option<String>,
    layout: LayoutTree,
    panes: HashMap<PaneId, Pane>,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: WorkspaceId::new(),
            name: name.into(),
            cwd: None,
            layout: LayoutTree::new(),
            panes: HashMap::new(),
        }
    }

    pub fn layout(&self) -> &LayoutTree {
        &self.layout
    }

    pub fn layout_mut(&mut self) -> &mut LayoutTree {
        &mut self.layout
    }

    pub fn pane(&self, id: &PaneId) -> Option<&Pane> {
        self.panes.get(id)
    }

    pub fn pane_mut(&mut self, id: &PaneId) -> Option<&mut Pane> {
        self.panes.get_mut(id)
    }

    pub fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.panes.values()
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn focused_pane(&self) -> Option<&Pane> {
        self.layout.focused().and_then(|id| self.panes.get(id))
    }

    /// Add first pane to workspace (auto-named if no name given).
    pub fn add_pane(&mut self, component_id: ComponentId) -> PaneId {
        let name = self.next_pane_name();
        self.add_named_pane(component_id, name)
    }

    /// Add first pane with a name.
    pub fn add_named_pane(
        &mut self,
        component_id: ComponentId,
        name: impl Into<String>,
    ) -> PaneId {
        let pane = Pane::new(self.id.clone(), component_id).with_name(name);
        let pane_id = pane.id.clone();
        self.layout.add_pane(pane_id.clone());
        self.panes.insert(pane_id.clone(), pane);
        pane_id
    }

    /// Split focused pane horizontally, creating new pane.
    pub fn split_horizontal(&mut self, component_id: ComponentId) -> Option<PaneId> {
        self.split(None, Direction::Horizontal, component_id, None)
    }

    /// Split focused pane vertically, creating new pane.
    pub fn split_vertical(&mut self, component_id: ComponentId) -> Option<PaneId> {
        self.split(None, Direction::Vertical, component_id, None)
    }

    /// Split a specific pane (or focused) in given direction.
    /// If `name` is None, assigns a Docker-style random unique name.
    pub fn split(
        &mut self,
        target: Option<&PaneId>,
        direction: Direction,
        component_id: ComponentId,
        name: Option<String>,
    ) -> Option<PaneId> {
        let resolved = name.unwrap_or_else(|| self.next_pane_name());
        let pane = Pane::new(self.id.clone(), component_id).with_name(resolved);
        let pane_id = pane.id.clone();

        if self.layout.split(target, direction, pane_id.clone()) {
            self.panes.insert(pane_id.clone(), pane);
            Some(pane_id)
        } else {
            None
        }
    }

    fn is_placeholder_name(name: Option<&str>) -> bool {
        match name.map(|s| s.trim()) {
            None | Some("") => true,
            Some(n) => matches!(
                n,
                "Terminal" | "terminal" | "Pane" | "pane" | "Untitled"
            ),
        }
    }

    /// Assign random names to any pane still missing one (e.g. after restore of old state).
    pub fn ensure_pane_names(&mut self) {
        let missing: Vec<PaneId> = self
            .panes
            .values()
            .filter(|p| Self::is_placeholder_name(p.name.as_deref()))
            .map(|p| p.id.clone())
            .collect();

        for pane_id in missing {
            let name = self.next_pane_name();
            if let Some(pane) = self.panes.get_mut(&pane_id) {
                pane.name = Some(name);
            }
        }
    }

    fn next_pane_name(&self) -> String {
        names::generate_unique_pane_name(self.panes.values().map(|p| p.name.as_deref()))
    }

    /// Close a pane, removing it from layout and registry.
    pub fn close_pane(&mut self, pane_id: &PaneId) -> bool {
        if self.layout.close(pane_id) {
            self.panes.remove(pane_id);
            true
        } else {
            false
        }
    }

    /// Focus a pane.
    pub fn focus(&mut self, pane_id: &PaneId) -> bool {
        self.layout.set_focus(pane_id)
    }

    pub fn focus_next(&mut self) -> Option<PaneId> {
        self.layout.focus_next()
    }

    pub fn focus_prev(&mut self) -> Option<PaneId> {
        self.layout.focus_prev()
    }

    /// Swap two panes.
    pub fn swap(&mut self, a: &PaneId, b: &PaneId) -> bool {
        self.layout.swap(a, b)
    }

    /// Resize split containing pane.
    pub fn resize(&mut self, pane_id: &PaneId, ratio: f32) -> bool {
        self.layout.resize(pane_id, ratio)
    }

    /// Resize a split node by its depth-first index.
    pub fn resize_split(&mut self, split_index: usize, ratio: f32) -> bool {
        self.layout.resize_split(split_index, ratio)
    }

    /// Toggle fullscreen.
    pub fn toggle_fullscreen(&mut self, pane_id: &PaneId) -> bool {
        self.layout.toggle_fullscreen(pane_id)
    }

    /// Convert tiled pane to floating.
    pub fn float_pane(&mut self, pane_id: &PaneId, x: f32, y: f32, width: f32, height: f32) -> bool {
        if !self.panes.contains_key(pane_id) {
            return false;
        }
        self.layout.float_pane(pane_id, x, y, width, height)
    }

    /// Convert floating pane back to tiled.
    pub fn tile_pane(&mut self, pane_id: &PaneId, direction: Direction) -> bool {
        if !self.panes.contains_key(pane_id) {
            return false;
        }
        self.layout.tile_pane(pane_id, direction)
    }

    /// Check if pane is floating.
    pub fn is_floating(&self, pane_id: &PaneId) -> bool {
        self.layout.is_floating(pane_id)
    }

    /// Check if pane is tiled.
    pub fn is_tiled(&self, pane_id: &PaneId) -> bool {
        self.layout.is_tiled(pane_id)
    }

    /// Find pane by name within this workspace.
    pub fn find_pane_by_name(&self, name: &str) -> Option<&Pane> {
        self.panes
            .values()
            .find(|p| p.name.as_deref() == Some(name))
    }

    /// Resolve pane by human name or PaneId string.
    pub fn resolve_pane(&self, key: &str) -> Option<&Pane> {
        self.find_pane_by_name(key)
            .or_else(|| self.panes.values().find(|p| p.id.as_str() == key))
    }

    /// Detach all session ids (PTYs do not survive process restart yet).
    pub fn clear_session_ids(&mut self) {
        for pane in self.panes.values_mut() {
            pane.detach_session();
        }
    }
}

/// Registry of all workspaces.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceRegistry {
    workspaces: HashMap<WorkspaceId, Workspace>,
    active: Option<WorkspaceId>,
}

impl WorkspaceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self, name: impl Into<String>) -> WorkspaceId {
        let ws = Workspace::new(name);
        let id = ws.id.clone();
        if self.active.is_none() {
            self.active = Some(id.clone());
        }
        self.workspaces.insert(id.clone(), ws);
        id
    }

    pub fn get(&self, id: &WorkspaceId) -> Option<&Workspace> {
        self.workspaces.get(id)
    }

    pub fn get_mut(&mut self, id: &WorkspaceId) -> Option<&mut Workspace> {
        self.workspaces.get_mut(id)
    }

    pub fn active(&self) -> Option<&Workspace> {
        self.active.as_ref().and_then(|id| self.workspaces.get(id))
    }

    pub fn active_mut(&mut self) -> Option<&mut Workspace> {
        self.active
            .as_ref()
            .cloned()
            .and_then(move |id| self.workspaces.get_mut(&id))
    }

    pub fn active_id(&self) -> Option<&WorkspaceId> {
        self.active.as_ref()
    }

    pub fn switch(&mut self, id: &WorkspaceId) -> bool {
        if self.workspaces.contains_key(id) {
            self.active = Some(id.clone());
            true
        } else {
            false
        }
    }

    pub fn destroy(&mut self, id: &WorkspaceId) -> bool {
        if self.workspaces.remove(id).is_some() {
            if self.active.as_ref() == Some(id) {
                self.active = self.workspaces.keys().next().cloned();
            }
            true
        } else {
            false
        }
    }

    pub fn list(&self) -> Vec<&Workspace> {
        self.workspaces.values().collect()
    }

    pub fn count(&self) -> usize {
        self.workspaces.len()
    }

    pub fn find_by_name(&self, name: &str) -> Option<&Workspace> {
        self.workspaces.values().find(|ws| ws.name == name)
    }

    /// Rename a workspace. Name must be unique among workspaces.
    pub fn rename(&mut self, id: &WorkspaceId, name: impl Into<String>) -> Result<String, String> {
        let name = name.into();
        if name.is_empty() {
            return Err("empty name".into());
        }
        if let Some(other) = self.find_by_name(&name) {
            if other.id != *id {
                return Err(format!("name taken: {name}"));
            }
            return Ok(name);
        }
        let ws = self
            .workspaces
            .get_mut(id)
            .ok_or_else(|| "workspace not found".to_string())?;
        ws.name = name.clone();
        Ok(name)
    }

    /// Resolve workspace by human name or WorkspaceId string.
    pub fn resolve_workspace(&self, key: &str) -> Option<&Workspace> {
        self.find_by_name(key)
            .or_else(|| self.workspaces.values().find(|ws| ws.id.as_str() == key))
    }

    /// Resolve a pane address: workspace/pane (name or id for either side).
    pub fn resolve(&self, workspace_key: &str, pane_key: &str) -> Option<(&Workspace, &Pane)> {
        let ws = self.resolve_workspace(workspace_key)?;
        let pane = ws.resolve_pane(pane_key)?;
        Some((ws, pane))
    }

    /// Clear all persisted session ids across workspaces.
    pub fn clear_all_session_ids(&mut self) {
        for ws in self.workspaces.values_mut() {
            ws.clear_session_ids();
        }
    }

    /// Fill in missing pane names (pre-random-name saves).
    pub fn ensure_all_pane_names(&mut self) {
        for ws in self.workspaces.values_mut() {
            ws.ensure_pane_names();
        }
    }

    /// Insert a pre-built workspace (used by persistence restore).
    pub fn insert(&mut self, workspace: Workspace) {
        let id = workspace.id.clone();
        if self.active.is_none() {
            self.active = Some(id.clone());
        }
        self.workspaces.insert(id, workspace);
    }
}
