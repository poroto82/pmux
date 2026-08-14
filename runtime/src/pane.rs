use serde::{Deserialize, Serialize};

use pmux::ids::{ComponentId, PaneId, SessionId, WorkspaceId};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pane {
    pub id: PaneId,
    pub workspace_id: WorkspaceId,
    pub component_id: ComponentId,
    pub component_type: String,
    pub session_id: Option<SessionId>,
    pub name: Option<String>,
    /// File/URL/command associated with the pane (view preview, etc.).
    #[serde(default)]
    pub source: Option<String>,
}

impl Pane {
    pub fn new(workspace_id: WorkspaceId, component_id: ComponentId) -> Self {
        Self {
            id: PaneId::new(),
            workspace_id,
            component_id,
            component_type: "terminal".into(),
            session_id: None,
            name: None,
            source: None,
        }
    }

    pub fn with_component_type(mut self, ct: impl Into<String>) -> Self {
        self.component_type = ct.into();
        self
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    pub fn with_session(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    pub fn attach_session(&mut self, session_id: SessionId) {
        self.session_id = Some(session_id);
    }

    pub fn detach_session(&mut self) -> Option<SessionId> {
        self.session_id.take()
    }
}
