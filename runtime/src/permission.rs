use std::collections::{HashMap, HashSet};

use pmux::ids::{PaneId, WorkspaceId};

/// Scope of access (spec §34).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Scope {
    /// No access.
    None,
    /// Access only to own pane.
    Pane(PaneId),
    /// Access to all panes in a workspace.
    Workspace(WorkspaceId),
    /// Access to everything.
    Application,
}

/// Individual permission (spec §33).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Permission {
    // Pane operations
    ReadOutput,
    SendCommand,
    SendInput,

    // Pane management
    CreatePane,
    ClosePane,
    FocusPane,
    SplitPane,
    ResizePane,

    // Float/tile operations
    FloatPane,
    TilePane,

    // Process operations
    ReadProcesses,
    KillProcess,
    RestartProcess,

    // Workspace operations
    CreateWorkspace,
    DestroyWorkspace,
    SwitchWorkspace,
}

/// Permission set for an actor (agent, plugin, widget).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PermissionSet {
    pub scope: Scope,
    pub allowed: HashSet<Permission>,
}

impl PermissionSet {
    /// Full access — everything, application-wide. For the user.
    pub fn full() -> Self {
        use Permission::*;
        Self {
            scope: Scope::Application,
            allowed: HashSet::from([
                ReadOutput, SendCommand, SendInput,
                CreatePane, ClosePane, FocusPane, SplitPane, ResizePane, FloatPane, TilePane,
                ReadProcesses, KillProcess, RestartProcess,
                CreateWorkspace, DestroyWorkspace, SwitchWorkspace,
            ]),
        }
    }

    /// Agent default — read + send within workspace scope (spec §33).
    pub fn agent(workspace_id: WorkspaceId) -> Self {
        use Permission::*;
        Self {
            scope: Scope::Workspace(workspace_id),
            allowed: HashSet::from([
                ReadOutput, SendCommand, ReadProcesses,
            ]),
        }
    }

    /// Widget — read only within workspace.
    pub fn widget(workspace_id: WorkspaceId) -> Self {
        use Permission::*;
        Self {
            scope: Scope::Workspace(workspace_id),
            allowed: HashSet::from([ReadOutput, ReadProcesses]),
        }
    }

    /// Pane-scoped — only access own pane.
    pub fn pane_only(pane_id: PaneId) -> Self {
        use Permission::*;
        Self {
            scope: Scope::Pane(pane_id),
            allowed: HashSet::from([ReadOutput, SendCommand, SendInput]),
        }
    }

    pub fn has(&self, permission: Permission) -> bool {
        self.allowed.contains(&permission)
    }

    pub fn grant(&mut self, permission: Permission) {
        self.allowed.insert(permission);
    }

    pub fn revoke(&mut self, permission: Permission) {
        self.allowed.remove(&permission);
    }
}

/// Request to perform an action — checked against permissions.
#[derive(Debug)]
pub struct AccessRequest<'a> {
    pub actor: &'a str,
    pub permission: Permission,
    pub target_workspace: Option<&'a WorkspaceId>,
    pub target_pane: Option<&'a PaneId>,
}

/// Result of authorization check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthResult {
    Allowed,
    Denied(String),
}

impl AuthResult {
    pub fn is_allowed(&self) -> bool {
        matches!(self, AuthResult::Allowed)
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, AuthResult::Denied(_))
    }
}

/// Authorization manager — holds permission sets for actors.
#[derive(Debug)]
pub struct Authorization {
    actors: HashMap<String, PermissionSet>,
}

impl Authorization {
    pub fn new() -> Self {
        let mut auth = Self {
            actors: HashMap::new(),
        };
        // User always has full access
        auth.actors.insert("user".into(), PermissionSet::full());
        auth
    }

    /// Register an actor with permissions.
    pub fn register(&mut self, actor: impl Into<String>, permissions: PermissionSet) {
        self.actors.insert(actor.into(), permissions);
    }

    /// Remove an actor.
    pub fn unregister(&mut self, actor: &str) -> bool {
        if actor == "user" {
            return false; // Can't remove user
        }
        self.actors.remove(actor).is_some()
    }

    /// Get permissions for an actor.
    pub fn get(&self, actor: &str) -> Option<&PermissionSet> {
        self.actors.get(actor)
    }

    /// Check if an access request is authorized.
    pub fn check(&self, request: &AccessRequest) -> AuthResult {
        let perms = match self.actors.get(request.actor) {
            Some(p) => p,
            None => return AuthResult::Denied(format!("unknown actor: {}", request.actor)),
        };

        // Check permission exists
        if !perms.has(request.permission) {
            return AuthResult::Denied(format!(
                "actor '{}' lacks {:?} permission",
                request.actor, request.permission
            ));
        }

        // Check scope
        match &perms.scope {
            Scope::Application => AuthResult::Allowed,

            Scope::Workspace(allowed_ws) => {
                if let Some(target_ws) = request.target_workspace {
                    if target_ws == allowed_ws {
                        AuthResult::Allowed
                    } else {
                        AuthResult::Denied(format!(
                            "actor '{}' scope is workspace {} but target is {}",
                            request.actor, allowed_ws, target_ws
                        ))
                    }
                } else {
                    // No workspace target — allow (workspace-level ops like create)
                    AuthResult::Allowed
                }
            }

            Scope::Pane(allowed_pane) => {
                if let Some(target_pane) = request.target_pane {
                    if target_pane == allowed_pane {
                        AuthResult::Allowed
                    } else {
                        AuthResult::Denied(format!(
                            "actor '{}' scope is pane {} but target is {}",
                            request.actor, allowed_pane, target_pane
                        ))
                    }
                } else {
                    AuthResult::Denied(format!(
                        "actor '{}' has pane scope but no target pane specified",
                        request.actor
                    ))
                }
            }

            Scope::None => AuthResult::Denied(format!(
                "actor '{}' has no access scope",
                request.actor
            )),
        }
    }

    /// Convenience: check and return bool.
    pub fn is_allowed(&self, request: &AccessRequest) -> bool {
        self.check(request).is_allowed()
    }

    pub fn actor_count(&self) -> usize {
        self.actors.len()
    }
}
