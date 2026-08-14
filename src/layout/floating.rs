use serde::{Deserialize, Serialize};

use crate::ids::PaneId;

/// A floating component (spec §6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FloatingPane {
    pub pane_id: PaneId,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub z_index: u32,
}

impl FloatingPane {
    pub fn new(pane_id: PaneId, x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            pane_id,
            x,
            y,
            width,
            height,
            z_index: 0,
        }
    }

    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
    }

    pub fn set_size(&mut self, width: f32, height: f32) {
        self.width = width;
        self.height = height;
    }
}

/// Manages floating panes alongside the tiled layout.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FloatingLayer {
    panes: Vec<FloatingPane>,
    next_z: u32,
}

impl FloatingLayer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a floating pane.
    pub fn add(&mut self, pane: FloatingPane) {
        self.next_z += 1;
        let mut pane = pane;
        pane.z_index = self.next_z;
        self.panes.push(pane);
    }

    /// Remove a floating pane, returning it.
    pub fn remove(&mut self, pane_id: &PaneId) -> Option<FloatingPane> {
        if let Some(idx) = self.panes.iter().position(|p| p.pane_id == *pane_id) {
            Some(self.panes.remove(idx))
        } else {
            None
        }
    }

    /// Get a floating pane.
    pub fn get(&self, pane_id: &PaneId) -> Option<&FloatingPane> {
        self.panes.iter().find(|p| p.pane_id == *pane_id)
    }

    /// Get mutable reference.
    pub fn get_mut(&mut self, pane_id: &PaneId) -> Option<&mut FloatingPane> {
        self.panes.iter_mut().find(|p| p.pane_id == *pane_id)
    }

    /// Bring a pane to front.
    pub fn bring_to_front(&mut self, pane_id: &PaneId) -> bool {
        let idx = self.panes.iter().position(|p| p.pane_id == *pane_id);
        if let Some(idx) = idx {
            self.next_z += 1;
            self.panes[idx].z_index = self.next_z;
            true
        } else {
            false
        }
    }

    /// Check if a pane is floating.
    pub fn contains(&self, pane_id: &PaneId) -> bool {
        self.panes.iter().any(|p| p.pane_id == *pane_id)
    }

    /// All floating pane IDs.
    pub fn pane_ids(&self) -> Vec<&PaneId> {
        self.panes.iter().map(|p| &p.pane_id).collect()
    }

    /// All floating panes sorted by z-index (back to front).
    pub fn sorted(&self) -> Vec<&FloatingPane> {
        let mut sorted: Vec<&FloatingPane> = self.panes.iter().collect();
        sorted.sort_by_key(|p| p.z_index);
        sorted
    }

    pub fn count(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// Move a floating pane.
    pub fn move_pane(&mut self, pane_id: &PaneId, x: f32, y: f32) -> bool {
        if let Some(pane) = self.get_mut(pane_id) {
            pane.set_position(x, y);
            true
        } else {
            false
        }
    }

    /// Resize a floating pane.
    pub fn resize_pane(&mut self, pane_id: &PaneId, width: f32, height: f32) -> bool {
        if let Some(pane) = self.get_mut(pane_id) {
            pane.set_size(width, height);
            true
        } else {
            false
        }
    }
}
