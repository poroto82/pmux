use std::fs;
use std::path::PathBuf;

use crate::ids::WorkspaceId;
use crate::workspace::{Workspace, WorkspaceRegistry};

/// Manages saving/restoring workspaces to disk.
///
/// Directory structure:
/// ```text
/// base_dir/
/// ├── registry.json       # active workspace + workspace list
/// └── workspaces/
///     ├── ws_01ABC.json
///     └── ws_01DEF.json
/// ```
#[derive(Debug)]
pub struct PersistenceManager {
    base_dir: PathBuf,
}

/// What gets saved in registry.json (lightweight index).
#[derive(serde::Serialize, serde::Deserialize)]
struct RegistryIndex {
    active: Option<WorkspaceId>,
    workspaces: Vec<WorkspaceEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct WorkspaceEntry {
    id: WorkspaceId,
    name: String,
}

#[derive(Debug)]
pub enum PersistError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl From<std::io::Error> for PersistError {
    fn from(e: std::io::Error) -> Self {
        PersistError::Io(e)
    }
}

impl From<serde_json::Error> for PersistError {
    fn from(e: serde_json::Error) -> Self {
        PersistError::Json(e)
    }
}

impl std::fmt::Display for PersistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PersistError::Io(e) => write!(f, "io: {}", e),
            PersistError::Json(e) => write!(f, "json: {}", e),
        }
    }
}

impl PersistenceManager {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    /// Default config path: ~/.config/pmux/ (or leftover ~/.config/pworkspaces/).
    pub fn default_path() -> PathBuf {
        crate::paths::config_dir()
    }

    fn workspaces_dir(&self) -> PathBuf {
        self.base_dir.join("workspaces")
    }

    fn registry_path(&self) -> PathBuf {
        self.base_dir.join("registry.json")
    }

    fn workspace_path(&self, id: &WorkspaceId) -> PathBuf {
        self.workspaces_dir().join(format!("{}.json", id))
    }

    /// Ensure directories exist.
    fn ensure_dirs(&self) -> Result<(), PersistError> {
        fs::create_dir_all(self.workspaces_dir())?;
        Ok(())
    }

    /// Save entire registry to disk.
    pub fn save(&self, registry: &WorkspaceRegistry) -> Result<(), PersistError> {
        self.ensure_dirs()?;

        // Save each workspace as individual file
        let mut entries = Vec::new();
        for ws in registry.list() {
            let path = self.workspace_path(&ws.id);
            let json = serde_json::to_string_pretty(ws)?;
            fs::write(&path, json)?;
            entries.push(WorkspaceEntry {
                id: ws.id.clone(),
                name: ws.name.clone(),
            });
        }

        // Save index
        let index = RegistryIndex {
            active: registry.active_id().cloned(),
            workspaces: entries,
        };
        let json = serde_json::to_string_pretty(&index)?;
        fs::write(self.registry_path(), json)?;

        // Clean up orphaned workspace files
        self.cleanup_orphans(registry)?;

        Ok(())
    }

    /// Save a single workspace (incremental).
    pub fn save_workspace(&self, workspace: &Workspace) -> Result<(), PersistError> {
        self.ensure_dirs()?;
        let path = self.workspace_path(&workspace.id);
        let json = serde_json::to_string_pretty(workspace)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// Restore full registry from disk.
    pub fn restore(&self) -> Result<WorkspaceRegistry, PersistError> {
        let index_path = self.registry_path();
        if !index_path.exists() {
            return Ok(WorkspaceRegistry::new());
        }

        let index_json = fs::read_to_string(&index_path)?;
        let index: RegistryIndex = serde_json::from_str(&index_json)?;

        let mut registry = WorkspaceRegistry::new();

        for entry in &index.workspaces {
            let ws_path = self.workspace_path(&entry.id);
            if ws_path.exists() {
                let ws_json = fs::read_to_string(&ws_path)?;
                let ws: Workspace = serde_json::from_str(&ws_json)?;
                registry.insert(ws);
            }
        }

        if let Some(active_id) = &index.active {
            registry.switch(active_id);
        }

        Ok(registry)
    }

    /// Delete a workspace file from disk.
    pub fn delete_workspace(&self, id: &WorkspaceId) -> Result<(), PersistError> {
        let path = self.workspace_path(id);
        if path.exists() {
            fs::remove_file(path)?;
        }
        Ok(())
    }

    /// List workspace files on disk.
    pub fn list_on_disk(&self) -> Result<Vec<String>, PersistError> {
        let dir = self.workspaces_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut names = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if let Some(name) = entry.path().file_stem() {
                names.push(name.to_string_lossy().into_owned());
            }
        }
        names.sort();
        Ok(names)
    }

    /// Remove workspace files that no longer exist in registry.
    fn cleanup_orphans(&self, registry: &WorkspaceRegistry) -> Result<(), PersistError> {
        let dir = self.workspaces_dir();
        if !dir.exists() {
            return Ok(());
        }

        let valid_ids: std::collections::HashSet<String> = registry
            .list()
            .iter()
            .map(|ws| format!("{}.json", ws.id))
            .collect();

        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let filename = entry.file_name().to_string_lossy().into_owned();
            if filename.ends_with(".json") && !valid_ids.contains(&filename) {
                fs::remove_file(entry.path())?;
            }
        }

        Ok(())
    }
}

