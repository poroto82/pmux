//! On-demand file index for quick-open (⌘P). Not a watcher — scan when palette opens.

use std::fs;
use std::path::{Path, PathBuf};

const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    ".next",
    ".nuxt",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    ".cache",
    ".turbo",
    "Pods",
    ".build",
    "coverage",
    ".idea",
    ".vscode",
];

const MAX_FILES: usize = 8000;
const MAX_DEPTH: usize = 12;

/// Relative paths (`/` separators) under `root`, skipping heavy dirs.
pub fn list_rel_paths(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            if skip_dir(name) {
                continue;
            }
            dirs.push(path);
            continue;
        }
        if let Some(rel) = rel_display(root, &path) {
            out.push(rel);
        }
    }
    for child in dirs {
        walk(root, &child, depth + 1, out);
    }
}

fn skip_dir(name: &str) -> bool {
    SKIP_DIRS.iter().any(|d| *d == name)
}

fn rel_display(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let joined = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Fuzzy-filter relative paths. Empty query → shallow files first.
pub fn search(files: &[String], query: &str, limit: usize) -> Vec<String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        let mut items: Vec<&String> = files.iter().collect();
        items.sort_by_key(|p| (p.matches('/').count(), p.len(), *p));
        return items.into_iter().take(limit).cloned().collect();
    }

    let mut scored: Vec<(i32, &String)> = files
        .iter()
        .filter_map(|p| {
            let score = fuzzy_score(p, &q);
            (score >= 0).then_some((score, p))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.matches('/').count().cmp(&b.1.matches('/').count()))
            .then_with(|| basename(a.1).len().cmp(&basename(b.1).len()))
            .then_with(|| a.1.cmp(b.1))
    });
    scored.into_iter().take(limit).map(|(_, p)| p.clone()).collect()
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((s, ext)) if !s.is_empty() && !ext.is_empty() => s,
        _ => name,
    }
}

fn fuzzy_score(path: &str, query: &str) -> i32 {
    let p = path.to_lowercase();
    let base = basename(&p);
    if base == query || stem(base) == query {
        400
    } else if base.starts_with(query) {
        300
    } else if p.starts_with(query) {
        220
    } else if base.contains(query) {
        180
    } else if p.contains(query) {
        100
    } else if subsequence(base, query) {
        60
    } else if subsequence(&p, query) {
        20
    } else {
        -1
    }
}

fn subsequence(hay: &str, needle: &str) -> bool {
    let mut it = hay.chars();
    for ch in needle.chars() {
        if it.find(|c| *c == ch).is_none() {
            return false;
        }
    }
    !needle.is_empty()
}

/// Expand `~` / `~/…` to an absolute path.
pub fn expand_path(raw: &str) -> PathBuf {
    let s = raw.trim();
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    PathBuf::from(s)
}

/// Pretty path: `$HOME/foo` → `~/foo`.
pub fn display_path(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rel) = path.strip_prefix(&home) {
            if rel.as_os_str().is_empty() {
                return "~".into();
            }
            return format!("~/{}", rel.display());
        }
    }
    path.display().to_string()
}

/// Resolve a relative quick-open hit against `root`.
pub fn abs_path(root: &Path, rel: &str) -> PathBuf {
    let mut out = root.to_path_buf();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            let _ = out.pop();
            continue;
        }
        out.push(part);
    }
    out
}
