use std::collections::HashMap;

use crate::ids::{PaneId, SessionId, WorkspaceId};

/// All events in the system.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum EventKind {
    WorkspaceCreated,
    WorkspaceDestroyed,
    WorkspaceSwitched,

    PaneCreated,
    PaneClosed,
    PaneFocused,
    PaneSplit,
    PaneFloated,
    PaneTiled,

    SessionCreated,
    SessionAttached,
    SessionDetached,
    SessionExited,

    ProcessStarted,
    ProcessExited,

    PortOpened,
    PortClosed,
}

/// Event payload.
#[derive(Debug, Clone)]
pub enum Event {
    WorkspaceCreated {
        workspace_id: WorkspaceId,
        name: String,
    },
    WorkspaceDestroyed {
        workspace_id: WorkspaceId,
    },
    WorkspaceSwitched {
        from: Option<WorkspaceId>,
        to: WorkspaceId,
    },

    PaneCreated {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    PaneClosed {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    PaneFocused {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    PaneSplit {
        workspace_id: WorkspaceId,
        source_pane: PaneId,
        new_pane: PaneId,
    },
    PaneFloated {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },
    PaneTiled {
        workspace_id: WorkspaceId,
        pane_id: PaneId,
    },

    SessionCreated {
        session_id: SessionId,
    },
    SessionAttached {
        session_id: SessionId,
        pane_id: PaneId,
    },
    SessionDetached {
        session_id: SessionId,
        pane_id: PaneId,
    },
    SessionExited {
        session_id: SessionId,
        exit_code: i32,
    },

    ProcessStarted {
        pane_id: PaneId,
        pid: u32,
    },
    ProcessExited {
        pane_id: PaneId,
        pid: u32,
        exit_code: i32,
    },

    PortOpened {
        port: u16,
        pid: u32,
        addr: String,
        command: String,
    },
    PortClosed {
        port: u16,
        pid: u32,
        addr: String,
        command: String,
    },
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Event::WorkspaceCreated { .. } => EventKind::WorkspaceCreated,
            Event::WorkspaceDestroyed { .. } => EventKind::WorkspaceDestroyed,
            Event::WorkspaceSwitched { .. } => EventKind::WorkspaceSwitched,
            Event::PaneCreated { .. } => EventKind::PaneCreated,
            Event::PaneClosed { .. } => EventKind::PaneClosed,
            Event::PaneFocused { .. } => EventKind::PaneFocused,
            Event::PaneSplit { .. } => EventKind::PaneSplit,
            Event::PaneFloated { .. } => EventKind::PaneFloated,
            Event::PaneTiled { .. } => EventKind::PaneTiled,
            Event::SessionCreated { .. } => EventKind::SessionCreated,
            Event::SessionAttached { .. } => EventKind::SessionAttached,
            Event::SessionDetached { .. } => EventKind::SessionDetached,
            Event::SessionExited { .. } => EventKind::SessionExited,
            Event::ProcessStarted { .. } => EventKind::ProcessStarted,
            Event::ProcessExited { .. } => EventKind::ProcessExited,
            Event::PortOpened { .. } => EventKind::PortOpened,
            Event::PortClosed { .. } => EventKind::PortClosed,
        }
    }

    /// Workspace this event belongs to, if any.
    pub fn workspace_id(&self) -> Option<&WorkspaceId> {
        match self {
            Event::WorkspaceCreated { workspace_id, .. }
            | Event::WorkspaceDestroyed { workspace_id }
            | Event::PaneCreated { workspace_id, .. }
            | Event::PaneClosed { workspace_id, .. }
            | Event::PaneFocused { workspace_id, .. }
            | Event::PaneSplit { workspace_id, .. }
            | Event::PaneFloated { workspace_id, .. }
            | Event::PaneTiled { workspace_id, .. } => Some(workspace_id),
            Event::WorkspaceSwitched { to, .. } => Some(to),
            _ => None,
        }
    }
}

type SubscriberId = u64;
type HandlerFn = Box<dyn Fn(&Event) + Send>;

struct Subscription {
    handler: HandlerFn,
    /// If set, only receive events from this workspace.
    scope: Option<WorkspaceId>,
}

/// Central event bus. Sync dispatch — handlers run inline.
pub struct EventBus {
    next_id: SubscriberId,
    subscribers: HashMap<EventKind, Vec<(SubscriberId, Subscription)>>,
    /// Global subscribers receive all events regardless of kind.
    global: Vec<(SubscriberId, Subscription)>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            subscribers: HashMap::new(),
            global: Vec::new(),
        }
    }

    /// Subscribe to a specific event kind.
    pub fn on(&mut self, kind: EventKind, handler: impl Fn(&Event) + Send + 'static) -> SubscriberId {
        self.on_scoped(kind, None, handler)
    }

    /// Subscribe scoped to a workspace — only events from that workspace fire.
    pub fn on_scoped(
        &mut self,
        kind: EventKind,
        scope: Option<WorkspaceId>,
        handler: impl Fn(&Event) + Send + 'static,
    ) -> SubscriberId {
        let id = self.next_id;
        self.next_id += 1;

        let sub = Subscription {
            handler: Box::new(handler),
            scope,
        };
        self.subscribers
            .entry(kind)
            .or_default()
            .push((id, sub));
        id
    }

    /// Subscribe to all events.
    pub fn on_all(&mut self, handler: impl Fn(&Event) + Send + 'static) -> SubscriberId {
        self.on_all_scoped(None, handler)
    }

    /// Subscribe to all events, scoped to workspace.
    pub fn on_all_scoped(
        &mut self,
        scope: Option<WorkspaceId>,
        handler: impl Fn(&Event) + Send + 'static,
    ) -> SubscriberId {
        let id = self.next_id;
        self.next_id += 1;
        let sub = Subscription {
            handler: Box::new(handler),
            scope,
        };
        self.global.push((id, sub));
        id
    }

    /// Unsubscribe by ID.
    pub fn unsubscribe(&mut self, id: SubscriberId) {
        for subs in self.subscribers.values_mut() {
            subs.retain(|(sid, _)| *sid != id);
        }
        self.global.retain(|(sid, _)| *sid != id);
    }

    /// Emit an event — all matching handlers run synchronously.
    pub fn emit(&self, event: &Event) {
        let event_ws = event.workspace_id();

        // Kind-specific subscribers
        if let Some(subs) = self.subscribers.get(&event.kind()) {
            for (_, sub) in subs {
                if scope_matches(&sub.scope, event_ws) {
                    (sub.handler)(event);
                }
            }
        }

        // Global subscribers
        for (_, sub) in &self.global {
            if scope_matches(&sub.scope, event_ws) {
                (sub.handler)(event);
            }
        }
    }
}

fn scope_matches(scope: &Option<WorkspaceId>, event_ws: Option<&WorkspaceId>) -> bool {
    match scope {
        None => true,
        Some(ws) => event_ws == Some(ws),
    }
}

impl std::fmt::Debug for EventBus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventBus")
            .field("subscriber_count", &self.subscribers.values().map(|v| v.len()).sum::<usize>())
            .field("global_count", &self.global.len())
            .finish()
    }
}
