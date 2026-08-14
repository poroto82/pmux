use pmux::ids::PaneId;
use pmux::layout::{Direction, LayoutNode, LayoutTree};

fn pane() -> PaneId {
    PaneId::new()
}

#[test]
fn empty_tree() {
    let tree = LayoutTree::new();
    assert!(tree.is_empty());
    assert_eq!(tree.pane_count(), 0);
    assert!(tree.focused().is_none());
}

#[test]
fn add_single_pane() {
    let mut tree = LayoutTree::new();
    let p = pane();
    tree.add_pane(p.clone());

    assert!(!tree.is_empty());
    assert_eq!(tree.pane_count(), 1);
    assert_eq!(tree.focused(), Some(&p));
}

#[test]
fn split_horizontal() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    assert!(tree.split_horizontal(p2.clone()));

    assert_eq!(tree.pane_count(), 2);
    // Focus moves to new pane after split
    assert_eq!(tree.focused(), Some(&p2));
}

#[test]
fn split_vertical() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    assert!(tree.split_vertical(p2.clone()));

    assert_eq!(tree.pane_count(), 2);
    assert_eq!(tree.focused(), Some(&p2));
}

#[test]
fn split_creates_correct_tree_structure() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    let root = tree.root().unwrap();
    match root {
        LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            assert_eq!(*direction, Direction::Horizontal);
            assert!((ratio - 0.5).abs() < f32::EPSILON);
            assert!(matches!(first.as_ref(), LayoutNode::Leaf { pane_id } if *pane_id == p1));
            assert!(matches!(second.as_ref(), LayoutNode::Leaf { pane_id } if *pane_id == p2));
        }
        _ => panic!("expected Split node"),
    }
}

#[test]
fn nested_split() {
    // p1 | (p2 / p3)
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    // p2 is focused, split it vertically
    tree.split_vertical(p3.clone());

    assert_eq!(tree.pane_count(), 3);
    let ids = tree.pane_ids();
    assert!(ids.contains(&&p1));
    assert!(ids.contains(&&p2));
    assert!(ids.contains(&&p3));
}

#[test]
fn close_last_pane() {
    let mut tree = LayoutTree::new();
    let p = pane();
    tree.add_pane(p.clone());

    assert!(tree.close(&p));
    assert!(tree.is_empty());
    assert!(tree.focused().is_none());
}

#[test]
fn close_one_of_two() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(tree.close(&p2));
    assert_eq!(tree.pane_count(), 1);
    // Focus should move to remaining pane
    assert_eq!(tree.focused(), Some(&p1));
}

#[test]
fn close_from_nested() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    // Close p2 from nested split
    tree.set_focus(&p2);
    assert!(tree.close(&p2));
    assert_eq!(tree.pane_count(), 2);
    assert!(!tree.pane_ids().contains(&&p2));
}

#[test]
fn close_nonexistent_pane() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    tree.add_pane(p1.clone());

    assert!(!tree.close(&p2));
    assert_eq!(tree.pane_count(), 1);
}

#[test]
fn focus_navigation() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    // p3 is focused (last split)
    assert_eq!(tree.focused(), Some(&p3));

    // Navigate
    let next = tree.focus_next();
    assert!(next.is_some());
    // Should wrap around
    assert_eq!(next.unwrap(), p1);
}

#[test]
fn focus_prev() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    // p2 focused
    let prev = tree.focus_prev();
    assert_eq!(prev, Some(p1.clone()));
    assert_eq!(tree.focused(), Some(&p1));
}

#[test]
fn set_focus() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(tree.set_focus(&p1));
    assert_eq!(tree.focused(), Some(&p1));

    // Nonexistent pane
    let fake = pane();
    assert!(!tree.set_focus(&fake));
    assert_eq!(tree.focused(), Some(&p1));
}

#[test]
fn swap_panes() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(tree.swap(&p1, &p2));

    let ids = tree.pane_ids();
    // After swap, order should be reversed
    assert_eq!(ids[0], &p2);
    assert_eq!(ids[1], &p1);
}

#[test]
fn swap_nonexistent_fails() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let fake = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(!tree.swap(&p1, &fake));
}

#[test]
fn resize() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(tree.resize(&p1, 0.7));

    match tree.root().unwrap() {
        LayoutNode::Split { ratio, .. } => {
            assert!((ratio - 0.7).abs() < f32::EPSILON);
        }
        _ => panic!("expected split"),
    }
}

#[test]
fn resize_clamped() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    // Should clamp to 0.9
    assert!(tree.resize(&p1, 1.5));
    match tree.root().unwrap() {
        LayoutNode::Split { ratio, .. } => {
            assert!((ratio - 0.9).abs() < f32::EPSILON);
        }
        _ => panic!("expected split"),
    }
}

#[test]
fn fullscreen_toggle() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert!(tree.toggle_fullscreen(&p1));
    assert_eq!(tree.fullscreened(), Some(&p1));

    // Toggle off
    assert!(tree.toggle_fullscreen(&p1));
    assert!(tree.fullscreened().is_none());
}

#[test]
fn fullscreen_nonexistent_fails() {
    let mut tree = LayoutTree::new();
    let p = pane();
    let fake = pane();
    tree.add_pane(p.clone());

    assert!(!tree.toggle_fullscreen(&fake));
}

#[test]
fn close_fullscreened_pane_clears_fullscreen() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    tree.toggle_fullscreen(&p1);
    tree.close(&p1);

    assert!(tree.fullscreened().is_none());
}

#[test]
fn serialization_roundtrip() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    let json = serde_json::to_string(&tree).unwrap();
    let restored: LayoutTree = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.pane_count(), 3);
    assert_eq!(restored.focused(), tree.focused());
    assert_eq!(restored.pane_ids().len(), 3);
}

#[test]
fn four_panes_grid() {
    // ┌─────┬─────┐
    // │ p1  │ p2  │
    // ├─────┼─────┤
    // │ p3  │ p4  │
    // └─────┴─────┘
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();
    let p4 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    // Focus p1, split vertical
    tree.set_focus(&p1);
    tree.split_vertical(p3.clone());

    // Focus p2, split vertical
    tree.set_focus(&p2);
    tree.split_vertical(p4.clone());

    assert_eq!(tree.pane_count(), 4);
}

#[test]
fn split_on_empty_tree_fails() {
    let mut tree = LayoutTree::new();
    let p = pane();
    assert!(!tree.split_horizontal(p));
}
