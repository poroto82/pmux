use pmux::ipc::{
    ActionOutcome, PaneSnap, Request, Response, UiSnapshot, WorkspaceTabSnap,
};

#[test]
fn snapshot_json_roundtrip() {
    let snap = UiSnapshot {
        active_id: Some("ws_1".into()),
        workspaces: vec![WorkspaceTabSnap {
            id: "ws_1".into(),
            name: "demo".into(),
            cwd: Some("/tmp".into()),
            active: true,
            pane_count: 1,
        }],
        layout_root: None,
        focused: Some("pane_1".into()),
        fullscreen: None,
        panes: vec![PaneSnap {
            id: "pane_1".into(),
            name: Some("term".into()),
            component_type: "terminal".into(),
            source: None,
            session_alive: true,
            pty_cols: None,
            pty_rows: None,
        }],
        floating: vec![],
        listen_ports: vec![],
        listen_ports_error: None,
    };

    let v = serde_json::to_value(&snap).unwrap();
    let back: UiSnapshot = serde_json::from_value(v).unwrap();
    assert_eq!(back.active_id.as_deref(), Some("ws_1"));
    assert_eq!(back.workspaces[0].name, "demo");
    assert_eq!(back.panes[0].id, "pane_1");
    assert!(back.panes[0].session_alive);
}

#[test]
fn snapshot_request_serde() {
    let json = serde_json::to_string(&Request::Snapshot {
        workspace: Some("demo".into()),
    })
    .unwrap();
    let req: Request = serde_json::from_str(&json).unwrap();
    match req {
        Request::Snapshot { workspace } => assert_eq!(workspace.as_deref(), Some("demo")),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn poll_ui_request_serde() {
    let json = serde_json::to_string(&Request::PollUi {
        workspace: Some("demo".into()),
        inputs: vec![pmux::ipc::PollInput {
            pane: "pane_1".into(),
            bytes: b"x".to_vec(),
        }],
        resizes: vec![],
    })
    .unwrap();
    let req: Request = serde_json::from_str(&json).unwrap();
    match req {
        Request::PollUi {
            workspace,
            inputs,
            ..
        } => {
            assert_eq!(workspace.as_deref(), Some("demo"));
            assert_eq!(inputs[0].bytes, b"x");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn shutdown_and_send_input_serde() {
    let shutdown = serde_json::to_string(&Request::Shutdown).unwrap();
    assert!(shutdown.contains("shutdown"));
    let back: Request = serde_json::from_str(&shutdown).unwrap();
    assert!(matches!(back, Request::Shutdown));

    let send = serde_json::to_string(&Request::SendInput {
        workspace: "ws".into(),
        pane: "pane".into(),
        bytes: b"ls\n".to_vec(),
    })
    .unwrap();
    let req: Request = serde_json::from_str(&send).unwrap();
    match req {
        Request::SendInput { bytes, .. } => assert_eq!(bytes, b"ls\n"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn auth_request_serde() {
    let json = serde_json::to_string(&Request::Auth {
        token: "abc".into(),
    })
    .unwrap();
    let req: Request = serde_json::from_str(&json).unwrap();
    match req {
        Request::Auth { token } => assert_eq!(token, "abc"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn action_outcome_ok_data() {
    let resp = Response::ok_data(ActionOutcome {
        ok: true,
        message: Some("Detached".into()),
        active_id: Some("ws_1".into()),
    });
    let json = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&json).unwrap();
    match back {
        Response::Ok { data: Some(v) } => {
            let out: ActionOutcome = serde_json::from_value(v).unwrap();
            assert!(out.ok);
            assert_eq!(out.message.as_deref(), Some("Detached"));
        }
        other => panic!("unexpected {other:?}"),
    }
}
