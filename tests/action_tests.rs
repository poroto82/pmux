use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use pworkspaces::action::{ActionArg, ActionContext, ActionRegistry, ActionResult};
use pworkspaces::ids::{PaneId, WorkspaceId};

fn setup_registry() -> ActionRegistry {
    let mut reg = ActionRegistry::new();

    reg.register("split_horizontal", "Split pane horizontally", |_ctx| {
        ActionResult::Ok
    });
    reg.register("split_vertical", "Split pane vertically", |_ctx| {
        ActionResult::Ok
    });
    reg.register("close_pane", "Close the focused pane", |_ctx| {
        ActionResult::Ok
    });
    reg.register("focus_next", "Focus next pane", |_ctx| ActionResult::Ok);
    reg.register("focus_prev", "Focus previous pane", |_ctx| ActionResult::Ok);
    reg.register("new_terminal", "Create new terminal", |_ctx| {
        ActionResult::Ok
    });

    reg
}

#[test]
fn register_and_list() {
    let reg = setup_registry();
    assert_eq!(reg.count(), 6);

    let names = reg.list();
    assert!(names.contains(&"split_horizontal"));
    assert!(names.contains(&"close_pane"));
    assert!(names.contains(&"new_terminal"));
}

#[test]
fn execute_action() {
    let reg = setup_registry();
    let ctx = ActionContext::new();

    let result = reg.execute("split_horizontal", &ctx);
    assert!(result.is_ok());
}

#[test]
fn execute_unknown_action() {
    let reg = setup_registry();
    let ctx = ActionContext::new();

    let result = reg.execute("nonexistent", &ctx);
    assert!(result.is_err());
}

#[test]
fn unregister() {
    let mut reg = setup_registry();
    assert!(reg.has("close_pane"));

    assert!(reg.unregister("close_pane"));
    assert!(!reg.has("close_pane"));
    assert_eq!(reg.count(), 5);

    // Double unregister
    assert!(!reg.unregister("close_pane"));
}

#[test]
fn action_with_context() {
    let mut reg = ActionRegistry::new();

    let called_with_ws = Arc::new(std::sync::Mutex::new(None));
    let cw = called_with_ws.clone();

    reg.register("test_action", "Test", move |ctx| {
        *cw.lock().unwrap() = ctx.workspace_id.clone();
        ActionResult::Ok
    });

    let ws = WorkspaceId::new();
    let ctx = ActionContext::new().with_workspace(ws.clone());
    reg.execute("test_action", &ctx);

    assert_eq!(*called_with_ws.lock().unwrap(), Some(ws));
}

#[test]
fn action_with_pane_context() {
    let mut reg = ActionRegistry::new();

    let called_with_pane = Arc::new(std::sync::Mutex::new(None));
    let cp = called_with_pane.clone();

    reg.register("focus_pane", "Focus specific pane", move |ctx| {
        *cp.lock().unwrap() = ctx.pane_id.clone();
        ActionResult::Ok
    });

    let pane = PaneId::new();
    let ctx = ActionContext::new().with_pane(pane.clone());
    reg.execute("focus_pane", &ctx);

    assert_eq!(*called_with_pane.lock().unwrap(), Some(pane));
}

#[test]
fn action_with_args() {
    let mut reg = ActionRegistry::new();

    let received_ratio = Arc::new(std::sync::Mutex::new(0.0f64));
    let rr = received_ratio.clone();

    reg.register("resize", "Resize pane", move |ctx| {
        if let Some(ratio) = ctx.args.get("ratio").and_then(|a| a.as_f64()) {
            *rr.lock().unwrap() = ratio;
            ActionResult::Ok
        } else {
            ActionResult::Error("missing ratio arg".into())
        }
    });

    let ctx = ActionContext::new().with_arg("ratio", ActionArg::Float(0.7));
    let result = reg.execute("resize", &ctx);
    assert!(result.is_ok());
    assert!((*received_ratio.lock().unwrap() - 0.7).abs() < f64::EPSILON);
}

#[test]
fn action_returns_error() {
    let mut reg = ActionRegistry::new();

    reg.register("failing", "Always fails", |_ctx| {
        ActionResult::Error("something broke".into())
    });

    let result = reg.execute("failing", &ActionContext::new());
    assert!(result.is_err());
    if let ActionResult::Error(msg) = result {
        assert_eq!(msg, "something broke");
    }
}

#[test]
fn search_actions() {
    let reg = setup_registry();

    let results = reg.search("split");
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|(name, _)| *name == "split_horizontal"));
    assert!(results.iter().any(|(name, _)| *name == "split_vertical"));
}

#[test]
fn search_by_description() {
    let reg = setup_registry();

    let results = reg.search("terminal");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, "new_terminal");
}

#[test]
fn search_case_insensitive() {
    let reg = setup_registry();

    let results = reg.search("SPLIT");
    assert_eq!(results.len(), 2);
}

#[test]
fn search_no_match() {
    let reg = setup_registry();

    let results = reg.search("zzzzz");
    assert!(results.is_empty());
}

#[test]
fn palette_items_empty_query_lists_all() {
    let reg = setup_registry();
    let items = reg.palette_items("");
    assert_eq!(items.len(), reg.count());
}

#[test]
fn palette_items_scores_name_prefix_first() {
    let mut reg = ActionRegistry::new();
    reg.register("split_horizontal", "Split pane horizontally", |_| {
        ActionResult::Ok
    });
    reg.register("close_pane", "Close the focused pane", |_| ActionResult::Ok);
    let items = reg.palette_items("split");
    assert_eq!(items[0].name, "split_horizontal");
    assert!(items.iter().all(|i| i.name.contains("split") || i.description.to_lowercase().contains("split")));
}

#[test]
fn handler_execution_count() {
    let mut reg = ActionRegistry::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    reg.register("inc", "Increment counter", move |_| {
        c.fetch_add(1, Ordering::SeqCst);
        ActionResult::Ok
    });

    let ctx = ActionContext::new();
    for _ in 0..5 {
        reg.execute("inc", &ctx);
    }

    assert_eq!(count.load(Ordering::SeqCst), 5);
}

#[test]
fn list_sorted() {
    let reg = setup_registry();
    let names = reg.list();

    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn action_arg_types() {
    let s = ActionArg::String("hello".into());
    assert_eq!(s.as_str(), Some("hello"));
    assert!(s.as_f64().is_none());
    assert!(s.as_bool().is_none());

    let f = ActionArg::Float(3.14);
    assert!(f.as_str().is_none());
    assert!((f.as_f64().unwrap() - 3.14).abs() < f64::EPSILON);

    let b = ActionArg::Bool(true);
    assert_eq!(b.as_bool(), Some(true));
    assert!(b.as_str().is_none());
}
