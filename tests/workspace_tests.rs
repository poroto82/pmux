use pworkspaces::ids::ComponentId;
use pworkspaces::workspace::{Workspace, WorkspaceRegistry};

fn comp() -> ComponentId {
    ComponentId::new()
}

// --- Workspace tests ---

#[test]
fn new_workspace_empty() {
    let ws = Workspace::new("backend");
    assert_eq!(ws.name, "backend");
    assert_eq!(ws.pane_count(), 0);
    assert!(ws.focused_pane().is_none());
}

#[test]
fn add_pane_to_workspace() {
    let mut ws = Workspace::new("backend");
    let pid = ws.add_pane(comp());

    assert_eq!(ws.pane_count(), 1);
    let pane = ws.pane(&pid).unwrap();
    assert_eq!(pane.workspace_id, ws.id);
    // Auto Docker-style name
    let name = pane.name.as_deref().unwrap();
    assert!(name.contains('_'), "expected adjective_noun, got {}", name);
}

#[test]
fn ensure_pane_names_replaces_placeholders() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_named_pane(comp(), "Terminal");
    let p2 = ws.add_named_pane(comp(), "api");
    ws.ensure_pane_names();
    let n1 = ws.pane(&p1).unwrap().name.clone().unwrap();
    let n2 = ws.pane(&p2).unwrap().name.clone().unwrap();
    assert_ne!(n1, "Terminal");
    assert!(n1.contains('_'), "got {}", n1);
    assert_eq!(n2, "api");
}

#[test]
fn split_auto_names_unique() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    let p2 = ws.split_horizontal(comp()).unwrap();
    let p3 = ws.split_vertical(comp()).unwrap();

    let n1 = ws.pane(&p1).unwrap().name.clone().unwrap();
    let n2 = ws.pane(&p2).unwrap().name.clone().unwrap();
    let n3 = ws.pane(&p3).unwrap().name.clone().unwrap();
    assert_ne!(n1, n2);
    assert_ne!(n2, n3);
    assert_ne!(n1, n3);
}

#[test]
fn add_named_pane() {
    let mut ws = Workspace::new("backend");
    let pid = ws.add_named_pane(comp(), "terminal-api");

    let pane = ws.pane(&pid).unwrap();
    assert_eq!(pane.name.as_deref(), Some("terminal-api"));
}

#[test]
fn split_creates_pane_in_registry() {
    let mut ws = Workspace::new("backend");
    ws.add_pane(comp());
    let p2 = ws.split_horizontal(comp()).unwrap();

    assert_eq!(ws.pane_count(), 2);
    assert!(ws.pane(&p2).is_some());
    assert_eq!(ws.pane(&p2).unwrap().workspace_id, ws.id);
}

#[test]
fn close_removes_from_registry() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    let p2 = ws.split_horizontal(comp()).unwrap();

    assert!(ws.close_pane(&p2));
    assert_eq!(ws.pane_count(), 1);
    assert!(ws.pane(&p2).is_none());
    assert!(ws.pane(&p1).is_some());
}

#[test]
fn find_pane_by_name() {
    let mut ws = Workspace::new("backend");
    ws.add_named_pane(comp(), "api");
    ws.split_horizontal(comp()); // unnamed

    let found = ws.find_pane_by_name("api");
    assert!(found.is_some());
    assert_eq!(found.unwrap().name.as_deref(), Some("api"));

    assert!(ws.find_pane_by_name("nonexistent").is_none());
}

#[test]
fn workspace_focus_navigation() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    let p2 = ws.split_horizontal(comp()).unwrap();

    // p2 focused after split
    assert_eq!(ws.focused_pane().unwrap().id, p2);

    ws.focus(&p1);
    assert_eq!(ws.focused_pane().unwrap().id, p1);

    ws.focus_next();
    assert_eq!(ws.focused_pane().unwrap().id, p2);
}

#[test]
fn workspace_swap() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    let p2 = ws.split_horizontal(comp()).unwrap();

    assert!(ws.swap(&p1, &p2));
    let ids: Vec<_> = ws.layout().pane_ids().into_iter().cloned().collect();
    assert_eq!(ids[0], p2);
    assert_eq!(ids[1], p1);
}

#[test]
fn workspace_resize() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    ws.split_horizontal(comp());

    assert!(ws.resize(&p1, 0.7));
}

#[test]
fn workspace_fullscreen() {
    let mut ws = Workspace::new("backend");
    let p1 = ws.add_pane(comp());
    ws.split_horizontal(comp());

    assert!(ws.toggle_fullscreen(&p1));
    assert_eq!(ws.layout().fullscreened(), Some(&p1));
}

#[test]
fn workspace_serialization() {
    let mut ws = Workspace::new("backend");
    let _p1 = ws.add_named_pane(comp(), "api");
    let _p2 = ws.split_horizontal(comp());

    let json = serde_json::to_string(&ws).unwrap();
    let restored: Workspace = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.name, "backend");
    assert_eq!(restored.pane_count(), 2);
    assert!(restored.find_pane_by_name("api").is_some());
}

#[test]
fn workspace_cwd_roundtrip() {
    let mut ws = Workspace::new("backend");
    ws.cwd = Some("/tmp".into());
    let json = serde_json::to_string(&ws).unwrap();
    let restored: Workspace = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.cwd.as_deref(), Some("/tmp"));

    let mut v: serde_json::Value = serde_json::to_value(&Workspace::new("legacy")).unwrap();
    v.as_object_mut().unwrap().remove("cwd");
    let restored: Workspace = serde_json::from_value(v).unwrap();
    assert!(restored.cwd.is_none());
}

// --- WorkspaceRegistry tests ---

#[test]
fn registry_create() {
    let mut reg = WorkspaceRegistry::new();
    let id = reg.create("backend");

    assert_eq!(reg.count(), 1);
    assert_eq!(reg.get(&id).unwrap().name, "backend");
    // First workspace becomes active
    assert_eq!(reg.active_id(), Some(&id));
}

#[test]
fn registry_multiple_workspaces() {
    let mut reg = WorkspaceRegistry::new();
    let be = reg.create("backend");
    let fe = reg.create("frontend");

    assert_eq!(reg.count(), 2);
    // First created stays active
    assert_eq!(reg.active_id(), Some(&be));

    // Switch
    assert!(reg.switch(&fe));
    assert_eq!(reg.active_id(), Some(&fe));
}

#[test]
fn registry_destroy() {
    let mut reg = WorkspaceRegistry::new();
    let be = reg.create("backend");
    let fe = reg.create("frontend");

    // Destroy active
    reg.switch(&be);
    assert!(reg.destroy(&be));
    assert_eq!(reg.count(), 1);
    // Active switches to remaining
    assert_eq!(reg.active_id(), Some(&fe));
}

#[test]
fn registry_destroy_last() {
    let mut reg = WorkspaceRegistry::new();
    let id = reg.create("backend");

    assert!(reg.destroy(&id));
    assert_eq!(reg.count(), 0);
    assert!(reg.active_id().is_none());
}

#[test]
fn registry_find_by_name() {
    let mut reg = WorkspaceRegistry::new();
    reg.create("backend");
    reg.create("frontend");

    assert!(reg.find_by_name("backend").is_some());
    assert!(reg.find_by_name("agents").is_none());
}

#[test]
fn registry_rename_unique() {
    let mut reg = WorkspaceRegistry::new();
    let a = reg.create("backend");
    let _b = reg.create("frontend");

    assert_eq!(reg.rename(&a, "api").unwrap(), "api");
    assert_eq!(reg.get(&a).unwrap().name, "api");
    assert!(reg.rename(&a, "frontend").is_err());
    assert_eq!(reg.rename(&a, "api").unwrap(), "api");
}

#[test]
fn registry_resolve_address() {
    let mut reg = WorkspaceRegistry::new();
    let ws_id = reg.create("backend");

    // Add named panes
    let ws = reg.get_mut(&ws_id).unwrap();
    ws.add_named_pane(comp(), "terminal-api");
    ws.add_named_pane(comp(), "terminal-tests");

    // Resolve
    let (ws, pane) = reg.resolve("backend", "terminal-api").unwrap();
    assert_eq!(ws.name, "backend");
    assert_eq!(pane.name.as_deref(), Some("terminal-api"));

    // Resolve by ids
    let pane_id = pane.id.clone();
    let (ws2, pane2) = reg.resolve(ws_id.as_str(), pane_id.as_str()).unwrap();
    assert_eq!(ws2.id, ws_id);
    assert_eq!(pane2.id, pane_id);

    // Cross-workspace miss
    assert!(reg.resolve("frontend", "terminal-api").is_none());
    assert!(reg.resolve("backend", "nonexistent").is_none());
}

#[test]
fn registry_active_mut_operations() {
    let mut reg = WorkspaceRegistry::new();
    reg.create("backend");

    let ws = reg.active_mut().unwrap();
    let p1 = ws.add_named_pane(comp(), "api");
    ws.split_horizontal(comp());

    assert_eq!(reg.active().unwrap().pane_count(), 2);
    assert!(reg.active().unwrap().pane(&p1).is_some());
}

#[test]
fn registry_serialization() {
    let mut reg = WorkspaceRegistry::new();
    let ws_id = reg.create("backend");
    reg.create("frontend");

    let ws = reg.get_mut(&ws_id).unwrap();
    ws.add_named_pane(comp(), "api");
    ws.split_horizontal(comp());

    let json = serde_json::to_string(&reg).unwrap();
    let restored: WorkspaceRegistry = serde_json::from_str(&json).unwrap();

    assert_eq!(restored.count(), 2);
    assert!(restored.find_by_name("backend").is_some());
    assert!(restored.find_by_name("frontend").is_some());
    assert_eq!(restored.find_by_name("backend").unwrap().pane_count(), 2);
}

#[test]
fn full_workflow() {
    let mut reg = WorkspaceRegistry::new();

    // Create workspaces
    let be_id = reg.create("backend");
    let _fe_id = reg.create("frontend");

    // Setup backend workspace
    let ws = reg.get_mut(&be_id).unwrap();
    let api = ws.add_named_pane(comp(), "terminal-api");
    let tests = ws.split_horizontal(comp()).unwrap();
    ws.pane_mut(&tests).unwrap().name = Some("terminal-tests".into());

    // Split tests vertically for ports
    ws.focus(&tests);
    let ports = ws.split(
        Some(&tests),
        pworkspaces::layout::Direction::Vertical,
        comp(),
        Some("ports".into()),
    )
    .unwrap();

    assert_eq!(ws.pane_count(), 3);

    // Verify addressing
    let (_, pane) = reg.resolve("backend", "terminal-api").unwrap();
    assert_eq!(pane.id, api);

    let (_, pane) = reg.resolve("backend", "ports").unwrap();
    assert_eq!(pane.id, ports);

    // Switch workspace
    assert!(reg.switch(&_fe_id));
    assert_eq!(reg.active().unwrap().name, "frontend");

    // Switch back
    assert!(reg.switch(&be_id));
    assert_eq!(reg.active().unwrap().pane_count(), 3);
}
