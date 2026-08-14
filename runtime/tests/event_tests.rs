use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use pmux_runtime::event::{Event, EventBus, EventKind};
use pmux::ids::{PaneId, SessionId, WorkspaceId};

#[test]
fn subscribe_and_emit() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    bus.on(EventKind::PaneCreated, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    let ws = WorkspaceId::new();
    bus.emit(&Event::PaneCreated {
        workspace_id: ws.clone(),
        pane_id: PaneId::new(),
    });

    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn no_fire_on_wrong_kind() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    bus.on(EventKind::PaneClosed, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    bus.emit(&Event::PaneCreated {
        workspace_id: WorkspaceId::new(),
        pane_id: PaneId::new(),
    });

    assert_eq!(count.load(Ordering::SeqCst), 0);
}

#[test]
fn multiple_subscribers_same_kind() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    for _ in 0..3 {
        let c = count.clone();
        bus.on(EventKind::PaneCreated, move |_| {
            c.fetch_add(1, Ordering::SeqCst);
        });
    }

    bus.emit(&Event::PaneCreated {
        workspace_id: WorkspaceId::new(),
        pane_id: PaneId::new(),
    });

    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[test]
fn unsubscribe() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    let id = bus.on(EventKind::PaneCreated, move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    bus.emit(&Event::PaneCreated {
        workspace_id: WorkspaceId::new(),
        pane_id: PaneId::new(),
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);

    bus.unsubscribe(id);

    bus.emit(&Event::PaneCreated {
        workspace_id: WorkspaceId::new(),
        pane_id: PaneId::new(),
    });
    assert_eq!(count.load(Ordering::SeqCst), 1); // no change
}

#[test]
fn workspace_scoped_subscriber() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();

    let c = count.clone();
    bus.on_scoped(EventKind::PaneCreated, Some(backend.clone()), move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    // Event from backend — should fire
    bus.emit(&Event::PaneCreated {
        workspace_id: backend.clone(),
        pane_id: PaneId::new(),
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);

    // Event from frontend — should NOT fire
    bus.emit(&Event::PaneCreated {
        workspace_id: frontend,
        pane_id: PaneId::new(),
    });
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[test]
fn global_subscriber() {
    let mut bus = EventBus::new();
    let kinds = Arc::new(std::sync::Mutex::new(Vec::new()));

    let k = kinds.clone();
    bus.on_all(move |event| {
        k.lock().unwrap().push(event.kind());
    });

    let ws = WorkspaceId::new();
    bus.emit(&Event::PaneCreated {
        workspace_id: ws.clone(),
        pane_id: PaneId::new(),
    });
    bus.emit(&Event::PaneClosed {
        workspace_id: ws.clone(),
        pane_id: PaneId::new(),
    });
    bus.emit(&Event::WorkspaceCreated {
        workspace_id: ws,
        name: "test".into(),
    });

    let received = kinds.lock().unwrap();
    assert_eq!(received.len(), 3);
    assert_eq!(received[0], EventKind::PaneCreated);
    assert_eq!(received[1], EventKind::PaneClosed);
    assert_eq!(received[2], EventKind::WorkspaceCreated);
}

#[test]
fn global_scoped_subscriber() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();

    let c = count.clone();
    bus.on_all_scoped(Some(backend.clone()), move |_| {
        c.fetch_add(1, Ordering::SeqCst);
    });

    bus.emit(&Event::PaneCreated {
        workspace_id: backend.clone(),
        pane_id: PaneId::new(),
    });
    bus.emit(&Event::PaneFocused {
        workspace_id: backend,
        pane_id: PaneId::new(),
    });
    bus.emit(&Event::PaneCreated {
        workspace_id: frontend,
        pane_id: PaneId::new(),
    });

    // Only 2 from backend, not the frontend one
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn event_workspace_id() {
    let ws = WorkspaceId::new();

    let e = Event::PaneCreated {
        workspace_id: ws.clone(),
        pane_id: PaneId::new(),
    };
    assert_eq!(e.workspace_id(), Some(&ws));

    let e = Event::SessionCreated {
        session_id: SessionId::new(),
    };
    assert!(e.workspace_id().is_none());

    let e = Event::PortOpened {
        port: 3000,
        pid: 1,
        addr: "*:3000".into(),
        command: "node".into(),
    };
    assert!(e.workspace_id().is_none());
}

#[test]
fn handler_receives_event_data() {
    let mut bus = EventBus::new();
    let received_name = Arc::new(std::sync::Mutex::new(String::new()));

    let rn = received_name.clone();
    bus.on(EventKind::WorkspaceCreated, move |event| {
        if let Event::WorkspaceCreated { name, .. } = event {
            *rn.lock().unwrap() = name.clone();
        }
    });

    bus.emit(&Event::WorkspaceCreated {
        workspace_id: WorkspaceId::new(),
        name: "backend".into(),
    });

    assert_eq!(*received_name.lock().unwrap(), "backend");
}

#[test]
fn process_and_port_events() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    bus.on(EventKind::ProcessExited, move |event| {
        if let Event::ProcessExited { exit_code, .. } = event {
            c.store(*exit_code as u32, Ordering::SeqCst);
        }
    });

    bus.emit(&Event::ProcessExited {
        pane_id: PaneId::new(),
        pid: 1234,
        exit_code: 42,
    });

    assert_eq!(count.load(Ordering::SeqCst), 42);
}

#[test]
fn session_events() {
    let mut bus = EventBus::new();
    let count = Arc::new(AtomicU32::new(0));

    let c = count.clone();
    bus.on(EventKind::SessionExited, move |event| {
        if let Event::SessionExited { exit_code, .. } = event {
            c.store(*exit_code as u32, Ordering::SeqCst);
        }
    });

    bus.emit(&Event::SessionExited {
        session_id: SessionId::new(),
        exit_code: 1,
    });

    assert_eq!(count.load(Ordering::SeqCst), 1);
}
