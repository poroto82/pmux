use pmux::ids::PaneId;
use pmux::layout::{Direction, LayoutTree};

fn pane() -> PaneId {
    PaneId::new()
}

// --- FloatingLayer basics ---

#[test]
fn float_pane_from_tiled() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    assert_eq!(tree.pane_count(), 2);
    assert!(tree.is_tiled(&p1));

    // Float p1
    assert!(tree.float_pane(&p1, 100.0, 100.0, 400.0, 300.0));

    assert!(tree.is_floating(&p1));
    assert!(!tree.is_tiled(&p1));
    assert!(tree.is_tiled(&p2));
    assert_eq!(tree.pane_count(), 2); // still 2 total
}

#[test]
fn tile_pane_from_floating() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());

    // Float then tile
    tree.float_pane(&p2, 100.0, 100.0, 400.0, 300.0);
    assert!(tree.is_floating(&p2));

    assert!(tree.tile_pane(&p2, Direction::Vertical));
    assert!(tree.is_tiled(&p2));
    assert!(!tree.is_floating(&p2));
}

#[test]
fn float_single_pane() {
    let mut tree = LayoutTree::new();
    let p = pane();

    tree.add_pane(p.clone());
    assert!(tree.float_pane(&p, 0.0, 0.0, 400.0, 300.0));

    assert!(tree.is_floating(&p));
    assert!(tree.root().is_none()); // No tiled root
    assert_eq!(tree.pane_count(), 1);
    assert!(!tree.is_empty());
}

#[test]
fn tile_into_empty_tree() {
    let mut tree = LayoutTree::new();
    let p = pane();

    tree.add_pane(p.clone());
    tree.float_pane(&p, 0.0, 0.0, 400.0, 300.0);

    assert!(tree.tile_pane(&p, Direction::Horizontal));
    assert!(tree.is_tiled(&p));
    assert!(tree.root().is_some());
}

#[test]
fn close_floating_pane() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.float_pane(&p2, 0.0, 0.0, 400.0, 300.0);

    assert!(tree.close(&p2));
    assert_eq!(tree.pane_count(), 1);
    assert!(!tree.is_floating(&p2));
}

#[test]
fn focus_floating_pane() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.float_pane(&p2, 0.0, 0.0, 400.0, 300.0);

    assert!(tree.set_focus(&p2));
    assert_eq!(tree.focused(), Some(&p2));
}

#[test]
fn floating_pane_ids_in_pane_ids() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    tree.float_pane(&p3, 0.0, 0.0, 400.0, 300.0);

    let all_ids = tree.pane_ids();
    assert_eq!(all_ids.len(), 3);
    assert!(all_ids.contains(&&p1));
    assert!(all_ids.contains(&&p2));
    assert!(all_ids.contains(&&p3));

    let tiled_ids = tree.tiled_pane_ids();
    assert_eq!(tiled_ids.len(), 2);
    assert!(!tiled_ids.contains(&&p3));
}

#[test]
fn fullscreen_floating_pane() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.float_pane(&p2, 0.0, 0.0, 400.0, 300.0);

    assert!(tree.toggle_fullscreen(&p2));
    assert_eq!(tree.fullscreened(), Some(&p2));
}

#[test]
fn float_nonexistent_fails() {
    let mut tree = LayoutTree::new();
    let p = pane();
    let fake = pane();

    tree.add_pane(p.clone());
    assert!(!tree.float_pane(&fake, 0.0, 0.0, 400.0, 300.0));
}

#[test]
fn tile_nonexistent_fails() {
    let mut tree = LayoutTree::new();
    let fake = pane();

    assert!(!tree.tile_pane(&fake, Direction::Horizontal));
}

#[test]
fn floating_z_order() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    tree.float_pane(&p2, 0.0, 0.0, 400.0, 300.0);
    tree.float_pane(&p3, 50.0, 50.0, 400.0, 300.0);

    let sorted = tree.floating().sorted();
    assert_eq!(sorted.len(), 2);
    // p3 added later → higher z
    assert!(sorted[1].z_index > sorted[0].z_index);

    // Bring p2 to front
    tree.set_focus(&p2); // brings to front
    let sorted = tree.floating().sorted();
    assert_eq!(sorted.last().unwrap().pane_id, p2);
}

#[test]
fn serialization_with_floating() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.float_pane(&p2, 100.0, 200.0, 400.0, 300.0);

    let json = serde_json::to_string(&tree).unwrap();
    let restored: LayoutTree = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.pane_count(), 2);
    assert!(restored.is_tiled(&p1));
    assert!(restored.is_floating(&p2));

    let fp = restored.floating().get(&p2).unwrap();
    assert!((fp.x - 100.0).abs() < f32::EPSILON);
    assert!((fp.y - 200.0).abs() < f32::EPSILON);
}

#[test]
fn focus_navigation_includes_floating() {
    let mut tree = LayoutTree::new();
    let p1 = pane();
    let p2 = pane();
    let p3 = pane();

    tree.add_pane(p1.clone());
    tree.split_horizontal(p2.clone());
    tree.split_vertical(p3.clone());

    tree.float_pane(&p3, 0.0, 0.0, 400.0, 300.0);

    // Focus should cycle through all panes including floating
    tree.set_focus(&p1);
    let ids = tree.pane_ids();
    assert_eq!(ids.len(), 3);
}
