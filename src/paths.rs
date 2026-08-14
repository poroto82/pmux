//! Locate pwctl next to the running binary and inject it into PTY PATH.

use std::path::PathBuf;

/// `~/.config/pworkspaces` (same base as persistence).
pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join(".config").join("pworkspaces"))
        .unwrap_or_else(|| PathBuf::from(".pworkspaces"))
}

/// Directory containing the current executable (`target/debug` when cargo-run).
pub fn bin_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// Ensure `pwctl` exists beside the UI binary.
/// Rebuild only if missing, or if `src/bin/pwctl.rs` is newer than the binary.
pub fn ensure_pwctl_built() {
    let Some(dir) = bin_dir() else { return };
    let pwctl = dir.join("pwctl");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/bin/pwctl.rs");
    let src_mtime = src.metadata().ok().and_then(|m| m.modified().ok());
    let pwctl_mtime = pwctl.metadata().ok().and_then(|m| m.modified().ok());
    let stale = match (pwctl_mtime, src_mtime) {
        (Some(p), Some(s)) => p < s,
        (None, _) => true,
        _ => false,
    };
    if pwctl.exists() && !stale {
        eprintln!("pwctl: {}", pwctl.display());
        return;
    }
    eprintln!("pwctl missing or stale — building…");
    let status = std::process::Command::new("cargo")
        .args(["build", "--bin", "pwctl"])
        .status();
    match status {
        Ok(s) if s.success() && pwctl.exists() => {
            eprintln!("pwctl: {}", pwctl.display());
        }
        Ok(_) | Err(_) => {
            eprintln!(
                "could not build pwctl. run: cargo build --bin pwctl && cargo install --path . --bin pwctl"
            );
        }
    }
}

/// PATH with pwctl dir + ~/.cargo/bin prepended.
pub fn path_with_pwctl() -> String {
    let mut parts = Vec::new();
    if let Some(dir) = bin_dir() {
        parts.push(dir.display().to_string());
    }
    if let Ok(home) = std::env::var("HOME") {
        parts.push(format!("{}/.cargo/bin", home));
    }
    if let Ok(existing) = std::env::var("PATH") {
        parts.push(existing);
    }
    parts.join(":")
}
