use serde::{Deserialize, Serialize};

use crate::ids::PaneId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Horizontal,
    Vertical,
}

impl Direction {
    pub fn opposite(&self) -> Self {
        match self {
            Direction::Horizontal => Direction::Vertical,
            Direction::Vertical => Direction::Horizontal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LayoutNode {
    Split {
        direction: Direction,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
    Leaf {
        pane_id: PaneId,
    },
}

impl LayoutNode {
    pub fn leaf(pane_id: PaneId) -> Self {
        LayoutNode::Leaf { pane_id }
    }

    pub fn split(direction: Direction, ratio: f32, first: LayoutNode, second: LayoutNode) -> Self {
        LayoutNode::Split {
            direction,
            ratio,
            first: Box::new(first),
            second: Box::new(second),
        }
    }

    /// Find and remove a pane, returning the sibling node if found.
    pub fn remove(&mut self, target: &PaneId) -> Option<LayoutNode> {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if pane_id == target {
                    // Can't remove self from self — caller handles this
                    None
                } else {
                    None
                }
            }
            LayoutNode::Split {
                first, second, ..
            } => {
                // Check if first child is the target leaf
                if let LayoutNode::Leaf { pane_id } = first.as_ref() {
                    if pane_id == target {
                        return Some(*second.clone());
                    }
                }
                // Check if second child is the target leaf
                if let LayoutNode::Leaf { pane_id } = second.as_ref() {
                    if pane_id == target {
                        return Some(*first.clone());
                    }
                }
                // Recurse into children
                if let Some(replacement) = first.remove(target) {
                    *first = Box::new(replacement);
                    return Some(self.clone());
                }
                if let Some(replacement) = second.remove(target) {
                    *second = Box::new(replacement);
                    return Some(self.clone());
                }
                None
            }
        }
    }

    /// Split a pane, inserting a new pane next to it.
    pub fn split_pane(
        &mut self,
        target: &PaneId,
        direction: Direction,
        new_pane: PaneId,
        ratio: f32,
    ) -> bool {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if pane_id == target {
                    let existing = LayoutNode::leaf(pane_id.clone());
                    let new = LayoutNode::leaf(new_pane);
                    *self = LayoutNode::split(direction, ratio, existing, new);
                    true
                } else {
                    false
                }
            }
            LayoutNode::Split {
                first, second, ..
            } => first.split_pane(target, direction, new_pane.clone(), ratio)
                || second.split_pane(target, direction, new_pane, ratio),
        }
    }

    /// Collect all pane IDs in order (left-to-right / top-to-bottom).
    pub fn pane_ids(&self) -> Vec<&PaneId> {
        match self {
            LayoutNode::Leaf { pane_id } => vec![pane_id],
            LayoutNode::Split {
                first, second, ..
            } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    /// Check if a pane exists in this tree.
    pub fn contains(&self, target: &PaneId) -> bool {
        match self {
            LayoutNode::Leaf { pane_id } => pane_id == target,
            LayoutNode::Split {
                first, second, ..
            } => first.contains(target) || second.contains(target),
        }
    }

    /// Swap two panes in the tree.
    pub fn swap(&mut self, a: &PaneId, b: &PaneId) -> bool {
        if !self.contains(a) || !self.contains(b) {
            return false;
        }
        self.swap_internal(a, b);
        true
    }

    fn swap_internal(&mut self, a: &PaneId, b: &PaneId) {
        match self {
            LayoutNode::Leaf { pane_id } => {
                if pane_id == a {
                    *pane_id = b.clone();
                } else if pane_id == b {
                    *pane_id = a.clone();
                }
            }
            LayoutNode::Split {
                first, second, ..
            } => {
                first.swap_internal(a, b);
                second.swap_internal(a, b);
            }
        }
    }

    /// Resize ratio at the split containing the target pane.
    pub fn resize(&mut self, target: &PaneId, new_ratio: f32) -> bool {
        let new_ratio = new_ratio.clamp(0.1, 0.9);
        match self {
            LayoutNode::Leaf { .. } => false,
            LayoutNode::Split {
                first,
                second,
                ratio,
                ..
            } => {
                if first.contains(target) || second.contains(target) {
                    // Only resize if target is a direct child
                    let is_direct = matches!(first.as_ref(), LayoutNode::Leaf { pane_id } if pane_id == target)
                        || matches!(second.as_ref(), LayoutNode::Leaf { pane_id } if pane_id == target);
                    if is_direct {
                        *ratio = new_ratio;
                        return true;
                    }
                }
                first.resize(target, new_ratio) || second.resize(target, new_ratio)
            }
        }
    }

    /// Resize ratio at a specific split node identified by index (depth-first order).
    /// Returns true if the split was found and resized.
    pub fn resize_split(&mut self, split_index: usize, new_ratio: f32) -> bool {
        let new_ratio = new_ratio.clamp(0.1, 0.9);
        let mut counter = 0;
        self.resize_split_inner(split_index, new_ratio, &mut counter)
    }

    fn resize_split_inner(&mut self, target: usize, new_ratio: f32, counter: &mut usize) -> bool {
        match self {
            LayoutNode::Leaf { .. } => false,
            LayoutNode::Split {
                first,
                second,
                ratio,
                ..
            } => {
                if *counter == target {
                    *ratio = new_ratio;
                    return true;
                }
                *counter += 1;
                if first.resize_split_inner(target, new_ratio, counter) {
                    return true;
                }
                second.resize_split_inner(target, new_ratio, counter)
            }
        }
    }

    /// Count total panes.
    pub fn pane_count(&self) -> usize {
        match self {
            LayoutNode::Leaf { .. } => 1,
            LayoutNode::Split {
                first, second, ..
            } => first.pane_count() + second.pane_count(),
        }
    }
}
