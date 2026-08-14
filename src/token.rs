//! LAN auth token. Unix socket stays unauthenticated (filesystem perms).

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crate::paths;

pub fn token_path() -> PathBuf {
    paths::config_dir().join("token")
}

/// Load existing token, or create one. `rotate` always writes a new value.
pub fn ensure(rotate: bool) -> io::Result<String> {
    paths::ensure_config_scaffold();
    let path = token_path();
    if !rotate {
        if let Ok(existing) = load() {
            return Ok(existing);
        }
    }
    let token = generate()?;
    write_token(&path, &token)?;
    Ok(token)
}

pub fn load() -> io::Result<String> {
    let s = fs::read_to_string(token_path())?;
    let t = s.trim().to_string();
    if t.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "empty token"));
    }
    Ok(t)
}

pub fn eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        d |= x ^ y;
    }
    d == 0
}

/// Client: `$PMUX_TOKEN`, else `~/.config/pmux/token`.
pub fn for_client() -> Option<String> {
    if let Ok(t) = std::env::var("PMUX_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Some(t);
        }
    }
    load().ok()
}

fn generate() -> io::Result<String> {
    let mut buf = [0u8; 24];
    fs::File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(to_hex(&buf))
}

fn write_token(path: &std::path::Path, token: &str) -> io::Result<()> {
    fs::write(path, format!("{token}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn to_hex(bytes: &[u8]) -> String {
    const H: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(H[(b >> 4) as usize] as char);
        out.push(H[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_same() {
        assert!(eq("abc", "abc"));
        assert!(!eq("abc", "abd"));
        assert!(!eq("abc", "ab"));
    }

    #[test]
    fn hex_len() {
        assert_eq!(to_hex(&[0xde, 0xad]), "dead");
    }
}
