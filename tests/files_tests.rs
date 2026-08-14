use std::fs;

use pworkspaces::files;

fn temp_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("pworkspaces_files_{}", ulid::Ulid::new()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &std::path::Path) {
    let _ = fs::remove_dir_all(dir);
}

fn touch(root: &std::path::Path, rel: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, b"").unwrap();
}

#[test]
fn list_skips_git_target_and_hidden_dirs() {
    let dir = temp_dir();
    touch(&dir, "README.md");
    touch(&dir, "spec.md");
    touch(&dir, "src/files.rs");
    touch(&dir, ".git/HEAD");
    touch(&dir, "target/debug/pworkspaces");
    touch(&dir, "node_modules/pkg/index.js");
    touch(&dir, ".github/workflows/ci.yml");
    touch(&dir, ".cache/x");

    let listed = files::list_rel_paths(&dir);
    assert!(listed.contains(&"README.md".into()));
    assert!(listed.contains(&"spec.md".into()));
    assert!(listed.contains(&"src/files.rs".into()));
    assert!(listed.contains(&".github/workflows/ci.yml".into()));
    assert!(!listed.iter().any(|p| p.starts_with(".git/")));
    assert!(!listed.iter().any(|p| p.starts_with("target/")));
    assert!(!listed.iter().any(|p| p.starts_with("node_modules/")));
    assert!(!listed.iter().any(|p| p.starts_with(".cache/")));

    cleanup(&dir);
}

#[test]
fn fuzzy_prefers_basename_over_path() {
    let files = vec![
        "src/palette.rs".into(),
        "spec.md".into(),
        "docs/spec-notes.md".into(),
    ];
    let hits = files::search(&files, "spec", 10);
    assert_eq!(hits[0], "spec.md");
    assert!(hits.contains(&"docs/spec-notes.md".into()));
}

#[test]
fn empty_query_shallow_first() {
    let files = vec![
        "src/deep/nested/x.rs".into(),
        "README.md".into(),
        "src/lib.rs".into(),
    ];
    let hits = files::search(&files, "", 10);
    assert_eq!(hits[0], "README.md");
}

#[test]
fn abs_path_joins_rel() {
    let root = std::path::Path::new("/tmp/ws");
    assert_eq!(
        files::abs_path(root, "src/files.rs"),
        std::path::PathBuf::from("/tmp/ws/src/files.rs")
    );
}

#[test]
fn expand_path_abs_and_trim() {
    assert_eq!(
        files::expand_path("  /tmp  "),
        std::path::PathBuf::from("/tmp")
    );
    assert_eq!(files::display_path(std::path::Path::new("/tmp")), "/tmp");
}
