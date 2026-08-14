use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::{PaneId, WorkspaceId};

/// Context passed to action handlers.
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub workspace_id: Option<WorkspaceId>,
    pub pane_id: Option<PaneId>,
    pub args: HashMap<String, ActionArg>,
}

/// Action argument value.
#[derive(Debug, Clone)]
pub enum ActionArg {
    String(String),
    Float(f64),
    Bool(bool),
}

impl ActionArg {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            ActionArg::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            ActionArg::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            ActionArg::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl ActionContext {
    pub fn new() -> Self {
        Self {
            workspace_id: None,
            pane_id: None,
            args: HashMap::new(),
        }
    }

    pub fn with_workspace(mut self, id: WorkspaceId) -> Self {
        self.workspace_id = Some(id);
        self
    }

    pub fn with_pane(mut self, id: PaneId) -> Self {
        self.pane_id = Some(id);
        self
    }

    pub fn with_arg(mut self, key: impl Into<String>, val: ActionArg) -> Self {
        self.args.insert(key.into(), val);
        self
    }
}

/// Result of executing an action.
#[derive(Debug)]
pub enum ActionResult {
    Ok,
    Message(String),
    Error(String),
}

impl ActionResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, ActionResult::Ok | ActionResult::Message(_))
    }

    pub fn is_err(&self) -> bool {
        matches!(self, ActionResult::Error(_))
    }

    pub fn message(&self) -> Option<&str> {
        match self {
            ActionResult::Message(s) => Some(s),
            _ => None,
        }
    }
}

/// Source that can invoke actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionSource {
    Keyboard,
    CommandPalette,
    ContextMenu,
    Agent,
    Automation,
    Plugin,
}

/// Metadata about a registered action.
pub struct ActionDef {
    pub name: String,
    pub description: String,
    pub shortcut: Option<String>,
    pub handler: Box<dyn Fn(&ActionContext) -> ActionResult + Send>,
}

impl fmt::Debug for ActionDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionDef")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("shortcut", &self.shortcut)
            .finish()
    }
}

/// Palette row (spec §16): registry data only, no hardcoded UI list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionInfo {
    pub name: String,
    pub description: String,
    pub shortcut: Option<String>,
}

/// Central registry for all actions in the system.
/// Command palette and keybindings consume this directly (spec §16).
pub struct ActionRegistry {
    actions: HashMap<String, ActionDef>,
}

impl ActionRegistry {
    pub fn new() -> Self {
        Self {
            actions: HashMap::new(),
        }
    }

    /// Register an action.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        handler: impl Fn(&ActionContext) -> ActionResult + Send + 'static,
    ) {
        self.register_with_shortcut(name, description, None, handler);
    }

    /// Register with optional keybinding hint (shown in command palette).
    pub fn register_with_shortcut(
        &mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        shortcut: Option<&str>,
        handler: impl Fn(&ActionContext) -> ActionResult + Send + 'static,
    ) {
        let name = name.into();
        let def = ActionDef {
            name: name.clone(),
            description: description.into(),
            shortcut: shortcut.map(|s| s.to_string()),
            handler: Box::new(handler),
        };
        self.actions.insert(name, def);
    }

    /// Unregister an action.
    pub fn unregister(&mut self, name: &str) -> bool {
        self.actions.remove(name).is_some()
    }

    /// Execute an action by name.
    pub fn execute(&self, name: &str, ctx: &ActionContext) -> ActionResult {
        match self.actions.get(name) {
            Some(def) => (def.handler)(ctx),
            None => ActionResult::Error(format!("unknown action: {}", name)),
        }
    }

    /// List all registered action names (for command palette).
    pub fn list(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.actions.keys().map(|s| s.as_str()).collect();
        names.sort();
        names
    }

    /// Search actions by substring (for command palette fuzzy filter).
    pub fn search(&self, query: &str) -> Vec<(&str, &str)> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<(&str, &str)> = self
            .actions
            .values()
            .filter(|def| {
                def.name.to_lowercase().contains(&query_lower)
                    || def.description.to_lowercase().contains(&query_lower)
            })
            .map(|def| (def.name.as_str(), def.description.as_str()))
            .collect();
        results.sort_by_key(|(name, _)| *name);
        results
    }

    /// Command palette items: filter + score, include shortcut hints.
    pub fn palette_items(&self, query: &str) -> Vec<ActionInfo> {
        let q = query.trim().to_lowercase();
        let mut items: Vec<(i32, ActionInfo)> = self
            .actions
            .values()
            .filter_map(|def| {
                let info = ActionInfo {
                    name: def.name.clone(),
                    description: def.description.clone(),
                    shortcut: def.shortcut.clone(),
                };
                if q.is_empty() {
                    return Some((0, info));
                }
                let score = fuzzy_score(&def.name, &def.description, &q);
                (score >= 0).then_some((score, info))
            })
            .collect();
        items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        items.into_iter().map(|(_, info)| info).collect()
    }

    /// Check if an action exists.
    pub fn has(&self, name: &str) -> bool {
        self.actions.contains_key(name)
    }

    /// Get action count.
    pub fn count(&self) -> usize {
        self.actions.len()
    }
}

fn fuzzy_score(name: &str, description: &str, query: &str) -> i32 {
    let n = name.to_lowercase();
    let d = description.to_lowercase();
    if n == query {
        300
    } else if n.starts_with(query) {
        200
    } else if n.contains(query) {
        100
    } else if d.contains(query) {
        50
    } else {
        -1
    }
}

impl fmt::Debug for ActionRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActionRegistry")
            .field("count", &self.actions.len())
            .field("actions", &self.list())
            .finish()
    }
}
