use serde::{Deserialize, Serialize};

use super::floating::{FloatingLayer, FloatingPane};
use super::node::{Direction, LayoutNode};
use crate::ids::PaneId;

/// Manages the layout tree, floating layer, and focus state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutTree {
    root: Option<LayoutNode>,
    floating: FloatingLayer,
    focus: Option<PaneId>,
    fullscreen: Option<PaneId>,
}

impl LayoutTree {
    pub fn new() -> Self {
        Self {
            root: None,
            floating: FloatingLayer::new(),
            focus: None,
            fullscreen: None,
        }
    }

    pub fn root(&self) -> Option<&LayoutNode> {
        self.root.as_ref()
    }

    pub fn focused(&self) -> Option<&PaneId> {
        self.focus.as_ref()
    }

    pub fn fullscreened(&self) -> Option<&PaneId> {
        self.fullscreen.as_ref()
    }

    pub fn floating(&self) -> &FloatingLayer {
        &self.floating
    }

    pub fn floating_mut(&mut self) -> &mut FloatingLayer {
        &mut self.floating
    }

    pub fn is_empty(&self) -> bool {
        self.root.is_none() && self.floating.is_empty()
    }

    pub fn pane_count(&self) -> usize {
        let tiled = self.root.as_ref().map_or(0, |r| r.pane_count());
        tiled + self.floating.count()
    }

    /// Tiled pane IDs only.
    pub fn tiled_pane_ids(&self) -> Vec<&PaneId> {
        self.root.as_ref().map_or(vec![], |r| r.pane_ids())
    }

    /// All pane IDs (tiled + floating).
    pub fn pane_ids(&self) -> Vec<&PaneId> {
        let mut ids = self.tiled_pane_ids();
        ids.extend(self.floating.pane_ids());
        ids
    }

    /// Add first pane to empty tree.
    pub fn add_pane(&mut self, pane_id: PaneId) {
        if self.root.is_none() {
            self.root = Some(LayoutNode::leaf(pane_id.clone()));
            self.focus = Some(pane_id);
        }
    }

    /// Split the focused pane (or given pane) in a direction.
    pub fn split(
        &mut self,
        target: Option<&PaneId>,
        direction: Direction,
        new_pane: PaneId,
    ) -> bool {
        let target = target.or(self.focus.as_ref());
        let Some(target) = target else { return false };
        let Some(root) = &mut self.root else {
            return false;
        };

        let target = target.clone();
        if root.split_pane(&target, direction, new_pane.clone(), 0.5) {
            self.focus = Some(new_pane);
            true
        } else {
            false
        }
    }

    pub fn split_horizontal(&mut self, new_pane: PaneId) -> bool {
        self.split(None, Direction::Horizontal, new_pane)
    }

    pub fn split_vertical(&mut self, new_pane: PaneId) -> bool {
        self.split(None, Direction::Vertical, new_pane)
    }

    /// Close a pane (tiled or floating). If it's the last one, tree becomes empty.
    pub fn close(&mut self, target: &PaneId) -> bool {
        // Try floating first
        if self.floating.remove(target).is_some() {
            if self.focus.as_ref() == Some(target) {
                // Move focus to another pane
                self.focus = self.pane_ids().first().map(|id| (*id).clone());
            }
            if self.fullscreen.as_ref() == Some(target) {
                self.fullscreen = None;
            }
            return true;
        }

        // Tiled
        let Some(root) = &mut self.root else {
            return false;
        };

        // Single pane case
        if let LayoutNode::Leaf { pane_id } = root {
            if pane_id == target {
                self.root = None;
                if self.focus.as_ref() == Some(target) {
                    self.focus = self.floating.pane_ids().first().map(|id| (*id).clone());
                }
                self.fullscreen = None;
                return true;
            }
            return false;
        }

        if let Some(replacement) = root.remove(target) {
            if self.focus.as_ref() == Some(target) {
                self.focus = replacement.pane_ids().first().map(|id| (*id).clone());
            }
            if self.fullscreen.as_ref() == Some(target) {
                self.fullscreen = None;
            }
            self.root = Some(replacement);
            true
        } else {
            false
        }
    }

    /// Set focus to a pane (tiled or floating).
    pub fn set_focus(&mut self, pane_id: &PaneId) -> bool {
        let in_tiled = self
            .root
            .as_ref()
            .map_or(false, |r| r.contains(pane_id));
        let in_floating = self.floating.contains(pane_id);

        if in_tiled || in_floating {
            if in_floating {
                self.floating.bring_to_front(pane_id);
            }
            self.focus = Some(pane_id.clone());
            true
        } else {
            false
        }
    }

    /// Navigate focus in a direction.
    /// Returns the newly focused pane ID, or None if navigation wasn't possible.
    pub fn focus_direction(&mut self, direction: Direction, forward: bool) -> Option<PaneId> {
        let panes = self.pane_ids().into_iter().cloned().collect::<Vec<_>>();
        if panes.len() < 2 {
            return None;
        }
        let Some(current) = &self.focus else {
            return None;
        };
        let Some(idx) = panes.iter().position(|p| p == current) else {
            return None;
        };

        // Simple linear navigation for now — direction awareness requires
        // spatial coordinates which we'll add when rendering.
        let _ = direction;
        let new_idx = if forward {
            (idx + 1) % panes.len()
        } else {
            (idx + panes.len() - 1) % panes.len()
        };

        let new_focus = panes[new_idx].clone();
        self.focus = Some(new_focus.clone());
        Some(new_focus)
    }

    pub fn focus_next(&mut self) -> Option<PaneId> {
        self.focus_direction(Direction::Horizontal, true)
    }

    pub fn focus_prev(&mut self) -> Option<PaneId> {
        self.focus_direction(Direction::Horizontal, false)
    }

    /// Swap two panes.
    pub fn swap(&mut self, a: &PaneId, b: &PaneId) -> bool {
        self.root.as_mut().map_or(false, |r| r.swap(a, b))
    }

    /// Resize the split containing a pane.
    pub fn resize(&mut self, target: &PaneId, ratio: f32) -> bool {
        self.root.as_mut().map_or(false, |r| r.resize(target, ratio))
    }

    /// Resize a split node by its depth-first index.
    pub fn resize_split(&mut self, split_index: usize, ratio: f32) -> bool {
        self.root
            .as_mut()
            .map_or(false, |r| r.resize_split(split_index, ratio))
    }

    /// Toggle fullscreen for a pane (tiled or floating).
    pub fn toggle_fullscreen(&mut self, pane_id: &PaneId) -> bool {
        let in_tiled = self
            .root
            .as_ref()
            .map_or(false, |r| r.contains(pane_id));
        let in_floating = self.floating.contains(pane_id);

        if !in_tiled && !in_floating {
            return false;
        }

        if self.fullscreen.as_ref() == Some(pane_id) {
            self.fullscreen = None;
        } else {
            self.fullscreen = Some(pane_id.clone());
        }
        true
    }

    /// Convert a tiled pane to floating (spec §6: tiled → floating).
    pub fn float_pane(&mut self, pane_id: &PaneId, x: f32, y: f32, width: f32, height: f32) -> bool {
        let in_tiled = self
            .root
            .as_ref()
            .map_or(false, |r| r.contains(pane_id));

        if !in_tiled {
            return false;
        }

        // Remove from tiled tree
        let was_focused = self.focus.as_ref() == Some(pane_id);

        if let Some(root) = &mut self.root {
            // Single pane — remove root
            if let LayoutNode::Leaf { pane_id: leaf_id } = root {
                if leaf_id == pane_id {
                    let pid = pane_id.clone();
                    self.root = None;
                    self.floating.add(FloatingPane::new(pid.clone(), x, y, width, height));
                    if was_focused {
                        self.focus = Some(pid);
                    }
                    return true;
                }
            }

            // Multi-pane — remove and get replacement
            if let Some(replacement) = root.remove(pane_id) {
                let pid = pane_id.clone();
                self.root = Some(replacement);
                self.floating.add(FloatingPane::new(pid.clone(), x, y, width, height));
                if was_focused {
                    self.focus = Some(pid);
                }
                return true;
            }
        }

        false
    }

    /// Convert a floating pane back to tiled (spec §6: floating → tiled).
    /// Adds it as a split of the focused tiled pane, or as root if tree is empty.
    pub fn tile_pane(&mut self, pane_id: &PaneId, direction: Direction) -> bool {
        let floating = match self.floating.remove(pane_id) {
            Some(f) => f,
            None => return false,
        };

        let pid = floating.pane_id;

        if self.root.is_none() {
            // No tiled panes — make it the root
            self.root = Some(LayoutNode::leaf(pid.clone()));
            self.focus = Some(pid);
            return true;
        }

        // Split a tiled pane — prefer focused if it's tiled, else first tiled
        let focus_is_tiled = self.focus.as_ref().map_or(false, |f| {
            self.root.as_ref().map_or(false, |r| r.contains(f))
        });
        let target = if focus_is_tiled {
            self.focus.clone()
        } else {
            self.tiled_pane_ids().first().map(|id| (*id).clone())
        };

        if let Some(target) = target {
            if let Some(root) = &mut self.root {
                if root.split_pane(&target, direction, pid.clone(), 0.5) {
                    self.focus = Some(pid);
                    return true;
                }
            }
        }

        // Couldn't tile — put it back
        self.floating.add(FloatingPane::new(pid, 0.0, 0.0, 400.0, 300.0));
        false
    }

    /// Check if a pane is floating.
    pub fn is_floating(&self, pane_id: &PaneId) -> bool {
        self.floating.contains(pane_id)
    }

    /// Check if a pane is tiled.
    pub fn is_tiled(&self, pane_id: &PaneId) -> bool {
        self.root.as_ref().map_or(false, |r| r.contains(pane_id))
    }
}
