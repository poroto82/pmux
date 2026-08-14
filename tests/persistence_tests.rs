use std::path::PathBuf;

use pworkspaces::ids::ComponentId;
use pworkspaces::layout::Direction;
use pworkspaces::persistence::PersistenceManager;
use pworkspaces::workspace::WorkspaceRegistry;

fn comp() -> ComponentId {
    ComponentId::new()
}

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pworkspaces_test_{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

fn build_registry() -> WorkspaceRegistry {
    let mut reg = WorkspaceRegistry::new();

    // Backend workspace with 3 panes
    let be_id = reg.create("backend");
    let ws = reg.get_mut(&be_id).unwrap();
    let api = ws.add_named_pane(comp(), "terminal-api");
    ws.split_horizontal(comp());
    ws.split(
        Some(&api),
        Direction::Vertical,
        comp(),
        Some("ports".into()),
    );

    // Frontend workspace with 2 panes
    let fe_id = reg.create("frontend");
    let ws = reg.get_mut(&fe_id).unwrap();
    ws.add_named_pane(comp(), "dev-server");
    ws.split_vertical(comp());

    // Active = backend
    reg.switch(&be_id);

    reg
}

#[test]
fn save_and_restore() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);
    let original = build_registry();

    pm.save(&original).unwrap();

    let restored = pm.restore().unwrap();

    assert_eq!(restored.count(), 2);
    assert!(restored.find_by_name("backend").is_some());
    assert!(restored.find_by_name("frontend").is_some());

    let be = restored.find_by_name("backend").unwrap();
    assert_eq!(be.pane_count(), 3);
    assert!(be.find_pane_by_name("terminal-api").is_some());
    assert!(be.find_pane_by_name("ports").is_some());

    let fe = restored.find_by_name("frontend").unwrap();
    assert_eq!(fe.pane_count(), 2);
    assert!(fe.find_pane_by_name("dev-server").is_some());

    // Active workspace preserved
    assert_eq!(
        restored.active().unwrap().name,
        original.active().unwrap().name
    );

    cleanup(&dir);
}

#[test]
fn restore_empty() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let restored = pm.restore().unwrap();
    assert_eq!(restored.count(), 0);

    cleanup(&dir);
}

#[test]
fn save_incremental_workspace() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let mut reg = WorkspaceRegistry::new();
    let id = reg.create("backend");
    let ws = reg.get_mut(&id).unwrap();
    ws.add_named_pane(comp(), "api");

    // Save full registry first
    pm.save(&reg).unwrap();

    // Modify and save just the workspace
    let ws = reg.get_mut(&id).unwrap();
    ws.split_horizontal(comp());
    pm.save_workspace(ws).unwrap();

    // Restore and verify change persisted
    let restored = pm.restore().unwrap();
    let be = restored.find_by_name("backend").unwrap();
    assert_eq!(be.pane_count(), 2);

    cleanup(&dir);
}

#[test]
fn delete_workspace_from_disk() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let mut reg = WorkspaceRegistry::new();
    let be_id = reg.create("backend");
    reg.create("frontend");

    pm.save(&reg).unwrap();

    let files_before = pm.list_on_disk().unwrap();
    assert_eq!(files_before.len(), 2);

    // Delete backend from registry and disk
    reg.destroy(&be_id);
    pm.delete_workspace(&be_id).unwrap();
    pm.save(&reg).unwrap();

    let files_after = pm.list_on_disk().unwrap();
    assert_eq!(files_after.len(), 1);

    let restored = pm.restore().unwrap();
    assert_eq!(restored.count(), 1);
    assert!(restored.find_by_name("frontend").is_some());

    cleanup(&dir);
}

#[test]
fn cleanup_orphans() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    // Save with 2 workspaces
    let mut reg = WorkspaceRegistry::new();
    let be_id = reg.create("backend");
    reg.create("frontend");
    pm.save(&reg).unwrap();
    assert_eq!(pm.list_on_disk().unwrap().len(), 2);

    // Destroy one and save — orphan should be cleaned
    reg.destroy(&be_id);
    pm.save(&reg).unwrap();
    assert_eq!(pm.list_on_disk().unwrap().len(), 1);

    cleanup(&dir);
}

#[test]
fn list_on_disk_empty() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let files = pm.list_on_disk().unwrap();
    assert!(files.is_empty());

    cleanup(&dir);
}

#[test]
fn ids_survive_roundtrip() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let mut reg = WorkspaceRegistry::new();
    let ws_id = reg.create("backend");
    let ws = reg.get_mut(&ws_id).unwrap();
    let pane_id = ws.add_named_pane(comp(), "api");

    pm.save(&reg).unwrap();
    let restored = pm.restore().unwrap();

    // Workspace ID preserved
    assert!(restored.get(&ws_id).is_some());

    // Pane ID preserved
    let ws = restored.get(&ws_id).unwrap();
    assert!(ws.pane(&pane_id).is_some());
    assert_eq!(ws.pane(&pane_id).unwrap().name.as_deref(), Some("api"));

    cleanup(&dir);
}

#[test]
fn layout_structure_survives_roundtrip() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let mut reg = WorkspaceRegistry::new();
    let ws_id = reg.create("backend");
    let ws = reg.get_mut(&ws_id).unwrap();
    ws.add_named_pane(comp(), "api");
    ws.split_horizontal(comp());
    ws.split_vertical(comp());

    // Capture layout order
    let original_ids: Vec<_> = ws.layout().pane_ids().iter().map(|p| p.to_string()).collect();
    let original_focus = ws.layout().focused().map(|p| p.to_string());

    pm.save(&reg).unwrap();
    let restored = pm.restore().unwrap();
    let ws = restored.get(&ws_id).unwrap();

    let restored_ids: Vec<_> = ws.layout().pane_ids().iter().map(|p| p.to_string()).collect();
    let restored_focus = ws.layout().focused().map(|p| p.to_string());

    assert_eq!(original_ids, restored_ids);
    assert_eq!(original_focus, restored_focus);

    cleanup(&dir);
}

#[test]
fn multiple_save_restore_cycles() {
    let dir = temp_dir();
    let pm = PersistenceManager::new(&dir);

    let mut reg = WorkspaceRegistry::new();
    let ws_id = reg.create("backend");

    // Cycle 1
    let ws = reg.get_mut(&ws_id).unwrap();
    ws.add_named_pane(comp(), "api");
    pm.save(&reg).unwrap();

    // Cycle 2 — add pane
    let mut reg = pm.restore().unwrap();
    let ws = reg.get_mut(&ws_id).unwrap();
    ws.split_horizontal(comp());
    pm.save(&reg).unwrap();

    // Cycle 3 — verify
    let reg = pm.restore().unwrap();
    let ws = reg.get(&ws_id).unwrap();
    assert_eq!(ws.pane_count(), 2);

    cleanup(&dir);
}

#[test]
fn default_path_not_empty() {
    let path = PersistenceManager::default_path();
    assert!(!path.as_os_str().is_empty());
}
