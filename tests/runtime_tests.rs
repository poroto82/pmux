use std::thread;
use std::time::Duration;

use pworkspaces::ids::ComponentId;
use pworkspaces::layout::Direction;
use pworkspaces::runtime::Runtime;

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pworkspaces_rt_test_{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &std::path::PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

fn comp() -> ComponentId {
    ComponentId::new()
}

// --- Workspace lifecycle ---

#[test]
fn create_workspace() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let id = rt.create_workspace("backend");
    assert_eq!(rt.workspaces.count(), 1);
    assert_eq!(rt.workspaces.get(&id).unwrap().name, "backend");

    cleanup(&dir);
}

#[test]
fn destroy_workspace_cleans_sessions() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let _pane = rt.add_pane(&ws_id, comp(), Some("api"), true);

    assert_eq!(rt.sessions.count(), 1);

    rt.destroy_workspace(&ws_id);

    assert_eq!(rt.workspaces.count(), 0);
    assert_eq!(rt.sessions.count(), 0);

    cleanup(&dir);
}

#[test]
fn switch_workspace() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let be = rt.create_workspace("backend");
    let fe = rt.create_workspace("frontend");

    assert!(rt.switch_workspace(&fe));
    assert_eq!(rt.workspaces.active().unwrap().name, "frontend");

    assert!(rt.switch_workspace(&be));
    assert_eq!(rt.workspaces.active().unwrap().name, "backend");

    cleanup(&dir);
}

#[test]
fn cycle_workspace_next_prev() {
    use pworkspaces::action::ActionContext;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let a = rt.create_workspace("a");
    let b = rt.create_workspace("b");
    rt.switch_workspace(&a);

    let ctx = ActionContext::new();
    assert!(rt.execute_action("next_workspace", &ctx).is_ok());
    assert_eq!(rt.workspaces.active_id(), Some(&b));
    assert!(rt.execute_action("next_workspace", &ctx).is_ok());
    assert_eq!(rt.workspaces.active_id(), Some(&a));
    assert!(rt.execute_action("prev_workspace", &ctx).is_ok());
    assert_eq!(rt.workspaces.active_id(), Some(&b));

    cleanup(&dir);
}

#[test]
fn new_workspace_gets_unique_name() {
    use pworkspaces::action::ActionContext;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    rt.create_workspace("alpha");

    let ctx = ActionContext::new();
    assert!(rt.execute_action("new_workspace", &ctx).is_ok());
    assert!(rt.execute_action("new_workspace", &ctx).is_ok());

    let names: Vec<String> = rt.workspaces.list().iter().map(|w| w.name.clone()).collect();
    assert_eq!(names.len(), 3);
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 3, "duplicate workspace names: {names:?}");

    cleanup(&dir);
}

#[test]
fn quick_open_root_uses_workspace_cwd() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let ws = rt.create_workspace("w");
    let cwd = std::env::temp_dir();
    rt.set_workspace_cwd(&ws, cwd.display().to_string()).unwrap();
    let root = rt.quick_open_root(&ws, None);
    assert_eq!(
        root.canonicalize().unwrap(),
        cwd.canonicalize().unwrap()
    );
    cleanup(&dir);
}

#[test]
fn rename_workspace_rejects_taken_name() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let a = rt.create_workspace("backend");
    rt.create_workspace("frontend");

    assert_eq!(rt.rename_workspace(&a, "api").unwrap(), "api");
    assert!(rt.rename_workspace(&a, "frontend").is_err());
    assert_eq!(rt.workspaces.get(&a).unwrap().name, "api");

    cleanup(&dir);
}

#[test]
fn close_last_pane_destroys_workspace() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let keep = rt.create_workspace("keep");
    rt.add_pane(&keep, comp(), None, false).unwrap();
    let drop = rt.create_workspace("drop");
    let pane = rt.add_pane(&drop, comp(), None, false).unwrap();
    rt.switch_workspace(&drop);

    assert!(rt.close_pane(&drop, &pane));
    assert!(rt.workspaces.get(&drop).is_none());
    assert!(rt.workspaces.get(&keep).is_some());
    assert_eq!(rt.workspaces.count(), 1);

    cleanup(&dir);
}

#[test]
fn close_last_pane_of_only_workspace_spawns_replacement() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let ws = rt.create_workspace("only");
    let pane = rt.add_pane(&ws, comp(), None, false).unwrap();

    assert!(rt.close_pane(&ws, &pane));
    assert_eq!(rt.workspaces.count(), 1);
    let next = rt.workspaces.active().unwrap();
    assert_ne!(next.id, ws);
    assert_eq!(next.pane_count(), 1);

    cleanup(&dir);
}

#[test]
fn tick_watchers_emits_port_opened_on_first_poll() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    use pworkspaces::event::EventKind;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let n = Arc::new(AtomicU32::new(0));
    let c = n.clone();
    rt.events.on(EventKind::PortOpened, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });
    rt.tick_watchers();
    assert_eq!(n.load(Ordering::SeqCst) as usize, rt.listen_ports().len());
    cleanup(&dir);
}

// --- Pane lifecycle ---

#[test]
fn add_pane_with_session() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();

    let ws = rt.workspaces.get(&ws_id).unwrap();
    assert_eq!(ws.pane_count(), 1);
    assert!(ws.pane(&pane_id).unwrap().session_id.is_some());
    assert_eq!(rt.sessions.count(), 1);

    cleanup(&dir);
}

#[test]
fn add_pane_without_session() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("ports"), false).unwrap();

    let ws = rt.workspaces.get(&ws_id).unwrap();
    assert!(ws.pane(&pane_id).unwrap().session_id.is_none());
    assert_eq!(rt.sessions.count(), 0);

    cleanup(&dir);
}

#[test]
fn split_pane_with_session() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    rt.add_pane(&ws_id, comp(), Some("api"), true);

    let new_pane = rt
        .split(&ws_id, None, Direction::Horizontal, comp(), Some("tests"), true)
        .unwrap();

    let ws = rt.workspaces.get(&ws_id).unwrap();
    assert_eq!(ws.pane_count(), 2);
    assert!(ws.pane(&new_pane).unwrap().session_id.is_some());
    assert_eq!(rt.sessions.count(), 2);

    cleanup(&dir);
}

#[test]
fn close_pane_destroys_session() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    rt.add_pane(&ws_id, comp(), Some("api"), true);
    let p2 = rt
        .split(&ws_id, None, Direction::Horizontal, comp(), None, true)
        .unwrap();

    assert_eq!(rt.sessions.count(), 2);

    rt.close_pane(&ws_id, &p2);

    assert_eq!(rt.sessions.count(), 1);
    assert_eq!(rt.workspaces.get(&ws_id).unwrap().pane_count(), 1);

    cleanup(&dir);
}

#[test]
fn focus_pane() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), Some("api"), false).unwrap();
    let _p2 = rt
        .split(&ws_id, None, Direction::Horizontal, comp(), None, false)
        .unwrap();

    assert!(rt.focus_pane(&ws_id, &p1));
    assert_eq!(
        rt.workspaces.get(&ws_id).unwrap().layout().focused(),
        Some(&p1)
    );

    cleanup(&dir);
}

// --- Command API (spec §30-31) ---

#[test]
fn send_command_to_pane() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();

    // Wait for shell
    thread::sleep(Duration::from_millis(500));
    let _ = rt.read_output(&ws_id, &pane_id); // drain prompt

    rt.send_command(&ws_id, &pane_id, "echo RUNTIME_TEST")
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = rt.read_output(&ws_id, &pane_id).unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("RUNTIME_TEST"), "got: {}", text);

    cleanup(&dir);
}

#[test]
fn send_command_by_name() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    rt.add_pane(&ws_id, comp(), Some("api"), true);

    thread::sleep(Duration::from_millis(500));
    let _ = rt.read_output_by_name("backend", "api");

    rt.send_command_by_name("backend", "api", "echo NAME_TEST")
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = rt.read_output_by_name("backend", "api").unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("NAME_TEST"), "got: {}", text);

    cleanup(&dir);
}

#[test]
fn send_command_nonexistent_workspace() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let result = rt.send_command_by_name("nonexistent", "api", "echo test");
    assert!(result.is_err());

    cleanup(&dir);
}

#[test]
fn send_command_no_session() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("ports"), false).unwrap();

    let result = rt.send_command(&ws_id, &pane_id, "echo test");
    assert!(result.is_err());

    cleanup(&dir);
}

// --- Events ---

#[test]
fn events_fire_on_operations() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use pworkspaces::event::EventKind;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let count = Arc::new(AtomicU32::new(0));
    let c = count.clone();
    rt.events.on(EventKind::PaneCreated, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    let ws_id = rt.create_workspace("backend");
    rt.add_pane(&ws_id, comp(), None, false);
    rt.split(&ws_id, None, Direction::Horizontal, comp(), None, false);

    assert_eq!(count.load(Ordering::SeqCst), 2);

    cleanup(&dir);
}

#[test]
fn events_scoped_to_workspace() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use pworkspaces::event::EventKind;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let be = rt.create_workspace("backend");
    let fe = rt.create_workspace("frontend");

    let be_count = Arc::new(AtomicU32::new(0));
    let c = be_count.clone();
    rt.events.on_scoped(EventKind::PaneCreated, Some(be.clone()), move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    rt.add_pane(&be, comp(), None, false);
    rt.add_pane(&fe, comp(), None, false);
    rt.add_pane(&be, comp(), None, false);

    // Only backend panes counted
    assert_eq!(be_count.load(Ordering::SeqCst), 2);

    cleanup(&dir);
}

// --- Persistence through runtime ---

#[test]
fn save_and_restore() {
    let dir = temp_dir();

    {
        let mut rt = Runtime::new(&dir);
        let ws_id = rt.create_workspace("backend");
        rt.add_pane(&ws_id, comp(), Some("api"), false);
        rt.split(&ws_id, None, Direction::Horizontal, comp(), Some("tests"), false);
        rt.save().unwrap();
    }

    {
        let mut rt = Runtime::new(&dir);
        rt.restore().unwrap();

        assert_eq!(rt.workspaces.count(), 1);
        let ws = rt.workspaces.find_by_name("backend").unwrap();
        assert_eq!(ws.pane_count(), 2);
        assert!(ws.find_pane_by_name("api").is_some());
        assert!(ws.find_pane_by_name("tests").is_some());
    }

    cleanup(&dir);
}

#[test]
fn restore_clears_stale_sessions_then_respawn() {
    let dir = temp_dir();

    let (ws_id, pane_id, stale_sess) = {
        let mut rt = Runtime::new(&dir);
        let ws_id = rt.create_workspace("backend");
        let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();
        let stale_sess = rt
            .workspaces
            .get(&ws_id)
            .unwrap()
            .pane(&pane_id)
            .unwrap()
            .session_id
            .clone()
            .unwrap();
        rt.save().unwrap();
        (ws_id, pane_id, stale_sess)
    };

    // New process: sessions registry empty, JSON still has old session_id
    let mut rt = Runtime::new(&dir);
    rt.restore().unwrap();

    // Stale ids cleared on restore
    {
        let ws = rt.workspaces.get(&ws_id).unwrap();
        let pane = ws.pane(&pane_id).unwrap();
        assert!(pane.session_id.is_none(), "stale session_id must be cleared");
    }
    assert_eq!(rt.sessions.count(), 0);

    rt.respawn_sessions();

    assert!(rt.pane_has_live_session(&ws_id, &pane_id));
    let new_sess = rt
        .workspaces
        .get(&ws_id)
        .unwrap()
        .pane(&pane_id)
        .unwrap()
        .session_id
        .clone()
        .unwrap();
    assert_ne!(new_sess, stale_sess);

    // Pane usable after respawn
    thread::sleep(Duration::from_millis(400));
    rt.send_command(&ws_id, &pane_id, "echo AFTER_RESTORE")
        .unwrap();
    thread::sleep(Duration::from_millis(400));
    let raw = rt.read_output(&ws_id, &pane_id).unwrap();
    let out = String::from_utf8_lossy(&raw);
    assert!(out.contains("AFTER_RESTORE"), "got: {}", out);

    cleanup(&dir);
}

#[test]
fn resolve_address_by_id_or_name() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), false).unwrap();

    let (ws, pane) = rt
        .workspaces
        .resolve(ws_id.as_str(), pane_id.as_str())
        .unwrap();
    assert_eq!(ws.id, ws_id);
    assert_eq!(pane.id, pane_id);

    let (ws2, pane2) = rt.workspaces.resolve("backend", "api").unwrap();
    assert_eq!(ws2.id, ws_id);
    assert_eq!(pane2.id, pane_id);

    let (ws3, pane3) = rt
        .workspaces
        .resolve(ws_id.as_str(), "api")
        .unwrap();
    assert_eq!(ws3.id, ws_id);
    assert_eq!(pane3.id, pane_id);

    cleanup(&dir);
}

#[test]
fn pane_shell_gets_pw_env() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("claude"), true).unwrap();

    thread::sleep(Duration::from_millis(600));
    let _ = rt.read_output(&ws_id, &pane_id);

    rt.send_command(
        &ws_id,
        &pane_id,
        "echo WS=$PW_WORKSPACE_NAME PN=$PW_PANE_NAME PI=$PW_PANE_ID",
    )
    .unwrap();

    let mut found = false;
    let mut last = String::new();
    for _ in 0..20 {
        thread::sleep(Duration::from_millis(150));
        let raw = rt.read_output(&ws_id, &pane_id).unwrap();
        last.push_str(&String::from_utf8_lossy(&raw));
        if last.contains("WS=backend") && last.contains("PN=claude") {
            found = true;
            break;
        }
    }
    assert!(
        found,
        "expected PW_* env in shell, got: {}",
        last
    );
    assert!(
        last.contains(&format!("PI={}", pane_id)),
        "got: {}",
        last
    );

    cleanup(&dir);
}

// --- Full workflow (spec §43) ---

#[test]
fn agent_workflow_simulation() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    // Create workspace with named panes
    let ws_id = rt.create_workspace("backend");
    let api_pane = rt.add_pane(&ws_id, comp(), Some("terminal-api"), true).unwrap();
    let test_pane = rt
        .split(&ws_id, Some(&api_pane), Direction::Horizontal, comp(), Some("terminal-tests"), true)
        .unwrap();

    thread::sleep(Duration::from_millis(500));

    // Simulate agent reading output from api pane
    let _ = rt.read_output(&ws_id, &api_pane);

    // Simulate agent sending command to tests pane (spec §31)
    rt.send_command(&ws_id, &test_pane, "echo AGENT_RAN_TESTS")
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = rt.read_output(&ws_id, &test_pane).unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("AGENT_RAN_TESTS"), "got: {}", text);

    // Same via name-based addressing (spec §28)
    rt.send_command_by_name("backend", "terminal-api", "echo API_CMD")
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = rt.read_output_by_name("backend", "terminal-api").unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("API_CMD"), "got: {}", text);

    cleanup(&dir);
}

// --- Default actions ---

#[test]
fn default_actions_registered() {
    let dir = temp_dir();
    let rt = Runtime::new(&dir);

    assert!(rt.actions.has("split_horizontal"));
    assert!(rt.actions.has("split_vertical"));
    assert!(rt.actions.has("close_pane"));
    assert!(rt.actions.has("focus_next"));
    assert!(rt.actions.has("focus_prev"));
    assert!(rt.actions.has("new_terminal"));
    assert!(rt.actions.has("toggle_fullscreen"));
    assert!(rt.actions.has("float_pane"));
    assert!(rt.actions.has("tile_pane"));
    assert!(rt.actions.has("toggle_float"));
    assert!(rt.actions.has("new_ports"));
    assert!(rt.actions.has("new_workspace"));
    assert!(rt.actions.has("next_workspace"));
    assert!(rt.actions.has("prev_workspace"));
    assert!(rt.actions.has("save_layout"));
    assert!(rt.actions.has("new_view"));
    assert!(rt.actions.has("quick_open"));
    assert!(rt.actions.has("rename_workspace"));
    assert!(rt.actions.has("detach"));
    assert!(rt.actions.has("refresh_terminals"));
    assert!(rt.actions.has("kill_runtime"));

    cleanup(&dir);
}

#[test]
fn execute_split_horizontal_creates_pane() {
    use pworkspaces::action::ActionContext;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let ws = rt.create_workspace("w");
    rt.add_pane(&ws, comp(), None, false).unwrap();
    assert_eq!(rt.workspaces.get(&ws).unwrap().pane_count(), 1);

    let ctx = ActionContext::new().with_workspace(ws.clone());
    let result = rt.execute_action("split_horizontal", &ctx);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(rt.workspaces.get(&ws).unwrap().pane_count(), 2);

    cleanup(&dir);
}

#[test]
fn open_view_reuses_pane_and_sets_source() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    let ws = rt.create_workspace("w");
    rt.add_pane(&ws, comp(), None, false).unwrap();

    let md = dir.join("note.md");
    std::fs::write(&md, "# hi").unwrap();
    let path = md.display().to_string();

    let pane_a = rt.open_view(&ws, &path).unwrap();
    let pane_b = rt.open_view(&ws, &path).unwrap();
    assert_eq!(pane_a, pane_b);

    let pane = rt.workspaces.get(&ws).unwrap().pane(&pane_a).unwrap();
    assert_eq!(pane.component_type, "view");
    assert_eq!(pane.source.as_deref(), Some(path.as_str()));

    cleanup(&dir);
}

// --- Authorization through runtime (spec §32) ---

#[test]
fn agent_authorized_within_workspace() {
    use pworkspaces::permission::PermissionSet;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();

    rt.auth.register("claude", PermissionSet::agent(ws_id.clone()));

    thread::sleep(Duration::from_millis(500));
    let _ = rt.read_output_as("claude", &ws_id, &pane_id);

    rt.send_command_as("claude", &ws_id, &pane_id, "echo AUTH_TEST")
        .unwrap();
    thread::sleep(Duration::from_millis(500));

    let output = rt.read_output_as("claude", &ws_id, &pane_id).unwrap();
    let text = String::from_utf8_lossy(&output);
    assert!(text.contains("AUTH_TEST"), "got: {}", text);

    cleanup(&dir);
}

#[test]
fn agent_denied_cross_workspace() {
    use pworkspaces::permission::PermissionSet;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let backend = rt.create_workspace("backend");
    let frontend = rt.create_workspace("frontend");
    let pane = rt.add_pane(&frontend, comp(), Some("dev"), true).unwrap();

    rt.auth.register("claude", PermissionSet::agent(backend));

    let result = rt.send_command_as("claude", &frontend, &pane, "echo HACK");
    assert!(result.is_err());
    match result {
        Err(pworkspaces::runtime::RuntimeError::PermissionDenied(_)) => {}
        other => panic!("expected PermissionDenied, got {:?}", other),
    }

    cleanup(&dir);
}

#[test]
fn agent_denied_missing_permission() {
    use pworkspaces::permission::PermissionSet;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();

    rt.auth.register("widget", PermissionSet::widget(ws_id.clone()));

    let _ = rt.read_output_as("widget", &ws_id, &pane_id);

    let result = rt.send_command_as("widget", &ws_id, &pane_id, "echo NOPE");
    assert!(result.is_err());

    cleanup(&dir);
}

#[test]
fn user_always_authorized() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt.add_pane(&ws_id, comp(), Some("api"), true).unwrap();

    thread::sleep(Duration::from_millis(500));

    rt.send_command_as("user", &ws_id, &pane_id, "echo USER_OK")
        .unwrap();

    cleanup(&dir);
}

// --- Float/Tile through runtime ---

#[test]
fn runtime_float_pane() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), None, false).unwrap();
    let p2 = rt
        .split(&ws_id, None, Direction::Horizontal, comp(), None, false)
        .unwrap();

    assert!(rt.float_pane(&ws_id, &p1, 100.0, 100.0, 400.0, 300.0));

    let ws = rt.workspaces.get(&ws_id).unwrap();
    assert!(ws.is_floating(&p1));
    assert!(ws.is_tiled(&p2));

    cleanup(&dir);
}

#[test]
fn runtime_tile_pane() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), None, false).unwrap();
    let _p2 = rt
        .split(&ws_id, None, Direction::Horizontal, comp(), None, false)
        .unwrap();

    rt.float_pane(&ws_id, &p1, 0.0, 0.0, 400.0, 300.0);
    assert!(rt.tile_pane(&ws_id, &p1, Direction::Vertical));

    let ws = rt.workspaces.get(&ws_id).unwrap();
    assert!(ws.is_tiled(&p1));

    cleanup(&dir);
}

#[test]
fn runtime_float_emits_event() {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;
    use pworkspaces::event::EventKind;

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let float_count = Arc::new(AtomicU32::new(0));
    let c = float_count.clone();
    rt.events.on(EventKind::PaneFloated, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    let tile_count = Arc::new(AtomicU32::new(0));
    let c2 = tile_count.clone();
    rt.events.on(EventKind::PaneTiled, move |_| {
        c2.fetch_add(1, Ordering::SeqCst);
    });

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), None, false).unwrap();
    rt.split(&ws_id, None, Direction::Horizontal, comp(), None, false);

    rt.float_pane(&ws_id, &p1, 0.0, 0.0, 400.0, 300.0);
    assert_eq!(float_count.load(Ordering::SeqCst), 1);

    rt.tile_pane(&ws_id, &p1, Direction::Horizontal);
    assert_eq!(tile_count.load(Ordering::SeqCst), 1);

    cleanup(&dir);
}

#[test]
fn runtime_float_with_auth() {
    use pworkspaces::permission::{Permission, PermissionSet};

    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), None, false).unwrap();
    rt.split(&ws_id, None, Direction::Horizontal, comp(), None, false);

    // Agent without FloatPane permission
    rt.auth.register("claude", PermissionSet::agent(ws_id.clone()));
    let result = rt.float_pane_as("claude", &ws_id, &p1, 0.0, 0.0, 400.0, 300.0);
    assert!(result.is_err());

    // Grant FloatPane
    let perms = rt.auth.get("claude").unwrap().clone();
    let mut new_perms = perms;
    new_perms.grant(Permission::FloatPane);
    rt.auth.register("claude", new_perms);

    let result = rt.float_pane_as("claude", &ws_id, &p1, 0.0, 0.0, 400.0, 300.0);
    assert!(result.unwrap());

    cleanup(&dir);
}

#[test]
fn runtime_float_nonexistent_workspace() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let p1 = rt.add_pane(&ws_id, comp(), None, false).unwrap();

    let fake_ws = pworkspaces::ids::WorkspaceId::new();
    assert!(!rt.float_pane(&fake_ws, &p1, 0.0, 0.0, 400.0, 300.0));

    cleanup(&dir);
}

// --- Component registry in runtime ---

#[test]
fn runtime_has_default_components() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);
    // load_plugins runs in Runtime::new — registry usable
    let _ = rt.components.available_plugins();
    assert_eq!(rt.components.count(), 0); // no live panes yet

    cleanup(&dir);
}

#[test]
fn runtime_add_pane_with_component_type() {
    let dir = temp_dir();
    let mut rt = Runtime::new(&dir);

    let ws_id = rt.create_workspace("backend");
    let pane_id = rt
        .add_pane_typed(&ws_id, comp(), Some("api"), "terminal")
        .unwrap();

    let ws = rt.workspaces.get(&ws_id).unwrap();
    let pane = ws.pane(&pane_id).unwrap();
    assert_eq!(pane.component_type, "terminal");
    assert_eq!(pane.name.as_deref(), Some("api"));
    assert!(rt.pane_has_live_session(&ws_id, &pane_id));

    cleanup(&dir);
}
