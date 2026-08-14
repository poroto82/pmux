//! Locate pwctl next to the running binary and inject it into PTY PATH.

use std::path::PathBuf;

const APP: &str = "pmux";
const LEGACY_APP: &str = "pworkspaces";

/// `~/.config/pmux/pmux.toml`
pub fn config_file() -> PathBuf {
    config_dir().join("pmux.toml")
}

/// `~/.config/pmux/theme.conf` (kitty color syntax).
pub fn theme_file() -> PathBuf {
    config_dir().join("theme.conf")
}

/// Write a commented `pmux.toml` the first time config dir is created.
pub fn ensure_config_scaffold() {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    let toml = config_file();
    if toml.exists() {
        return;
    }
    let _ = std::fs::write(
        toml,
        "# pmux\n\
         #\n\
         # theme = caffeine   # bundled, muted (default)\n\
         # theme = kitty      # follow ~/.config/kitty/kitty.conf\n\
         # theme = theme.conf # ~/.config/pmux/theme.conf (kitty color syntax)\n\
         # theme = ~/path/to.conf\n\
         #\n\
         # Or set $PMUX_THEME. Copy themes/caffeine.conf → theme.conf to tweak.\n\
         theme = \"caffeine\"\n\
         #\n\
         # LAN TCP (token required). off = unix socket only.\n\
         # listen = \"0.0.0.0:7878\"\n\
         # listen = \"off\"\n",
    );
}

/// `~/.config/pmux`, or leftover `~/.config/pworkspaces` if that still has data.
pub fn config_dir() -> PathBuf {
    let Some(base) = xdg_config_home() else {
        return pick_dir(PathBuf::from(".pmux"), PathBuf::from(".pworkspaces"));
    };
    pick_dir(base.join(APP), base.join(LEGACY_APP))
}

fn xdg_config_home() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        std::env::var("HOME")
            .ok()
            .map(|h| std::path::Path::new(&h).join(".config"))
    }
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|h| std::path::Path::new(&h).join(".config"))
            })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA").ok().map(PathBuf::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        dirs::home_dir().map(|h| h.join(".config"))
    }
}

fn pick_dir(preferred: PathBuf, legacy: PathBuf) -> PathBuf {
    if preferred.exists() || !legacy.exists() {
        preferred
    } else {
        legacy
    }
}

/// Directory containing the current executable (`target/debug` when cargo-run).
pub fn bin_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// `pwctl` next to this exe, this exe if it *is* pwctl, or PATH.
pub fn find_pwctl() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if exe.file_name().is_some_and(|n| n == "pwctl") {
            return Some(exe);
        }
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("pwctl");
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    which("pwctl")
}

/// UI binary (`pmux`, or leftover `pworkspaces`).
pub fn find_ui_bin() -> Option<PathBuf> {
    if let Some(dir) = bin_dir() {
        for name in [APP, LEGACY_APP] {
            let sibling = dir.join(name);
            if sibling.is_file() {
                return Some(sibling);
            }
        }
    }
    which(APP).or_else(|| which(LEGACY_APP))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Ensure `pwctl` exists beside the UI binary.
/// Rebuild only if missing, or if `runtime/src/bin/pwctl.rs` is newer than the binary.
pub fn ensure_pwctl_built() {
    let Some(dir) = bin_dir() else { return };
    let pwctl = dir.join("pwctl");
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("runtime/src/bin/pwctl.rs");
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
                "could not build pwctl. run: cargo build --bin pwctl && cargo install --path runtime --bin pwctl"
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
