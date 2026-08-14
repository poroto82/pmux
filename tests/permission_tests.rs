use pworkspaces::ids::{PaneId, WorkspaceId};
use pworkspaces::permission::*;

// --- PermissionSet ---

#[test]
fn full_permissions() {
    let perms = PermissionSet::full();
    assert_eq!(perms.scope, Scope::Application);
    assert!(perms.has(Permission::ReadOutput));
    assert!(perms.has(Permission::SendCommand));
    assert!(perms.has(Permission::KillProcess));
    assert!(perms.has(Permission::CreateWorkspace));
    assert!(perms.has(Permission::DestroyWorkspace));
}

#[test]
fn agent_permissions() {
    let ws = WorkspaceId::new();
    let perms = PermissionSet::agent(ws.clone());

    assert_eq!(perms.scope, Scope::Workspace(ws));
    assert!(perms.has(Permission::ReadOutput));
    assert!(perms.has(Permission::SendCommand));
    assert!(perms.has(Permission::ReadProcesses));

    // Agent should NOT have these by default
    assert!(!perms.has(Permission::SendInput));
    assert!(!perms.has(Permission::KillProcess));
    assert!(!perms.has(Permission::CreateWorkspace));
    assert!(!perms.has(Permission::DestroyWorkspace));
    assert!(!perms.has(Permission::ClosePane));
}

#[test]
fn widget_permissions() {
    let ws = WorkspaceId::new();
    let perms = PermissionSet::widget(ws);

    assert!(perms.has(Permission::ReadOutput));
    assert!(perms.has(Permission::ReadProcesses));
    assert!(!perms.has(Permission::SendCommand));
    assert!(!perms.has(Permission::SendInput));
}

#[test]
fn pane_only_permissions() {
    let pane = PaneId::new();
    let perms = PermissionSet::pane_only(pane.clone());

    assert_eq!(perms.scope, Scope::Pane(pane));
    assert!(perms.has(Permission::ReadOutput));
    assert!(perms.has(Permission::SendCommand));
    assert!(perms.has(Permission::SendInput));
    assert!(!perms.has(Permission::KillProcess));
}

#[test]
fn grant_and_revoke() {
    let ws = WorkspaceId::new();
    let mut perms = PermissionSet::agent(ws);

    assert!(!perms.has(Permission::KillProcess));
    perms.grant(Permission::KillProcess);
    assert!(perms.has(Permission::KillProcess));

    perms.revoke(Permission::KillProcess);
    assert!(!perms.has(Permission::KillProcess));
}

// --- Authorization ---

#[test]
fn user_always_allowed() {
    let auth = Authorization::new();

    let ws = WorkspaceId::new();
    let pane = PaneId::new();

    let result = auth.check(&AccessRequest {
        actor: "user",
        permission: Permission::DestroyWorkspace,
        target_workspace: Some(&ws),
        target_pane: Some(&pane),
    });
    assert!(result.is_allowed());
}

#[test]
fn unknown_actor_denied() {
    let auth = Authorization::new();

    let result = auth.check(&AccessRequest {
        actor: "mystery",
        permission: Permission::ReadOutput,
        target_workspace: None,
        target_pane: None,
    });
    assert!(result.is_denied());
}

#[test]
fn agent_within_workspace_allowed() {
    let mut auth = Authorization::new();
    let ws = WorkspaceId::new();
    let pane = PaneId::new();

    auth.register("claude", PermissionSet::agent(ws.clone()));

    let result = auth.check(&AccessRequest {
        actor: "claude",
        permission: Permission::SendCommand,
        target_workspace: Some(&ws),
        target_pane: Some(&pane),
    });
    assert!(result.is_allowed());
}

#[test]
fn agent_cross_workspace_denied() {
    let mut auth = Authorization::new();
    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();

    auth.register("claude", PermissionSet::agent(backend));

    // Try to access frontend — should be denied (spec §35)
    let result = auth.check(&AccessRequest {
        actor: "claude",
        permission: Permission::SendCommand,
        target_workspace: Some(&frontend),
        target_pane: None,
    });
    assert!(result.is_denied());
}

#[test]
fn agent_lacks_permission_denied() {
    let mut auth = Authorization::new();
    let ws = WorkspaceId::new();

    auth.register("claude", PermissionSet::agent(ws.clone()));

    // Agent doesn't have KillProcess
    let result = auth.check(&AccessRequest {
        actor: "claude",
        permission: Permission::KillProcess,
        target_workspace: Some(&ws),
        target_pane: None,
    });
    assert!(result.is_denied());
}

#[test]
fn pane_scoped_actor() {
    let mut auth = Authorization::new();
    let pane_a = PaneId::new();
    let pane_b = PaneId::new();

    auth.register("limited_agent", PermissionSet::pane_only(pane_a.clone()));

    // Own pane — allowed
    let result = auth.check(&AccessRequest {
        actor: "limited_agent",
        permission: Permission::SendCommand,
        target_workspace: None,
        target_pane: Some(&pane_a),
    });
    assert!(result.is_allowed());

    // Other pane — denied
    let result = auth.check(&AccessRequest {
        actor: "limited_agent",
        permission: Permission::SendCommand,
        target_workspace: None,
        target_pane: Some(&pane_b),
    });
    assert!(result.is_denied());
}

#[test]
fn widget_read_only() {
    let mut auth = Authorization::new();
    let ws = WorkspaceId::new();
    let pane = PaneId::new();

    auth.register("ports_widget", PermissionSet::widget(ws.clone()));

    // Read — allowed
    let result = auth.check(&AccessRequest {
        actor: "ports_widget",
        permission: Permission::ReadOutput,
        target_workspace: Some(&ws),
        target_pane: Some(&pane),
    });
    assert!(result.is_allowed());

    // Write — denied
    let result = auth.check(&AccessRequest {
        actor: "ports_widget",
        permission: Permission::SendCommand,
        target_workspace: Some(&ws),
        target_pane: Some(&pane),
    });
    assert!(result.is_denied());
}

#[test]
fn none_scope_denies_all() {
    let mut auth = Authorization::new();
    auth.register("blocked", PermissionSet {
        scope: Scope::None,
        allowed: std::collections::HashSet::from([Permission::ReadOutput]),
    });

    let result = auth.check(&AccessRequest {
        actor: "blocked",
        permission: Permission::ReadOutput,
        target_workspace: None,
        target_pane: None,
    });
    assert!(result.is_denied());
}

#[test]
fn unregister_actor() {
    let mut auth = Authorization::new();
    let ws = WorkspaceId::new();

    auth.register("claude", PermissionSet::agent(ws));
    assert_eq!(auth.actor_count(), 2); // user + claude

    assert!(auth.unregister("claude"));
    assert_eq!(auth.actor_count(), 1);

    // Can't unregister user
    assert!(!auth.unregister("user"));
    assert_eq!(auth.actor_count(), 1);
}

#[test]
fn denied_message_contains_context() {
    let mut auth = Authorization::new();
    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();

    auth.register("claude", PermissionSet::agent(backend.clone()));

    let result = auth.check(&AccessRequest {
        actor: "claude",
        permission: Permission::SendCommand,
        target_workspace: Some(&frontend),
        target_pane: None,
    });

    match result {
        AuthResult::Denied(msg) => {
            assert!(msg.contains("claude"), "msg: {}", msg);
            assert!(msg.contains("scope"), "msg: {}", msg);
        }
        _ => panic!("expected denied"),
    }
}

#[test]
fn convenience_is_allowed() {
    let mut auth = Authorization::new();
    let ws = WorkspaceId::new();

    auth.register("claude", PermissionSet::agent(ws.clone()));

    assert!(auth.is_allowed(&AccessRequest {
        actor: "claude",
        permission: Permission::ReadOutput,
        target_workspace: Some(&ws),
        target_pane: None,
    }));

    assert!(!auth.is_allowed(&AccessRequest {
        actor: "claude",
        permission: Permission::DestroyWorkspace,
        target_workspace: Some(&ws),
        target_pane: None,
    }));
}

// --- Scenario: spec §35 cross-workspace communication ---

#[test]
fn cross_workspace_blocked_by_default() {
    let mut auth = Authorization::new();
    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();
    let pane_tests = PaneId::new();

    // Claude in backend workspace
    auth.register("claude", PermissionSet::agent(backend));

    // Try: backend/claude → frontend/tests (spec §35)
    let result = auth.check(&AccessRequest {
        actor: "claude",
        permission: Permission::SendCommand,
        target_workspace: Some(&frontend),
        target_pane: Some(&pane_tests),
    });

    assert!(result.is_denied());
}

#[test]
fn application_scope_allows_cross_workspace() {
    let mut auth = Authorization::new();
    let backend = WorkspaceId::new();
    let frontend = WorkspaceId::new();

    // Super agent with application scope
    auth.register("orchestrator", PermissionSet::full());

    let result = auth.check(&AccessRequest {
        actor: "orchestrator",
        permission: Permission::SendCommand,
        target_workspace: Some(&frontend),
        target_pane: None,
    });
    assert!(result.is_allowed());

    let result = auth.check(&AccessRequest {
        actor: "orchestrator",
        permission: Permission::SendCommand,
        target_workspace: Some(&backend),
        target_pane: None,
    });
    assert!(result.is_allowed());
}
