//! Terminal + chrome palette.
//!
//! Default: bundled **caffeine** (muted coffee). Override:
//!   `$PMUX_THEME` → `~/.config/pmux/pmux.toml` `theme` → `theme.conf` → caffeine
//! Spec: `caffeine` | `kitty` | path to a kitty-syntax `.conf`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use alacritty_terminal::vte::ansi::NamedColor;

/// RGB color for terminal cells / UI chrome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl TermColor {
    pub fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn to_rgba(&self) -> [u8; 4] {
        [self.r, self.g, self.b, 255]
    }
}

/// Full terminal palette (ANSI 16 + defaults + chrome).
#[derive(Debug, Clone)]
pub struct TermPalette {
    pub colors: [TermColor; 16],
    pub foreground: TermColor,
    pub background: TermColor,
    pub cursor: TermColor,
    pub selection: TermColor,
    pub active_border: TermColor,
    pub inactive_border: TermColor,
}

impl TermPalette {
    /// Muted coffee — default when nothing is configured.
    pub fn caffeine() -> Self {
        Self {
            colors: [
                TermColor::new(0x2a, 0x29, 0x26),
                TermColor::new(0xc4, 0x5c, 0x4a),
                TermColor::new(0x8a, 0x9a, 0x62),
                TermColor::new(0xc9, 0xa8, 0x6c),
                TermColor::new(0x6a, 0x84, 0x94),
                TermColor::new(0xa0, 0x7a, 0x8c),
                TermColor::new(0x7a, 0x94, 0x8c),
                TermColor::new(0xd0, 0xcc, 0xc0),
                TermColor::new(0x6e, 0x6a, 0x62),
                TermColor::new(0xd4, 0x78, 0x68),
                TermColor::new(0xa0, 0xb0, 0x78),
                TermColor::new(0xd4, 0xbc, 0x84),
                TermColor::new(0x82, 0xa0, 0xb0),
                TermColor::new(0xb8, 0x94, 0xa4),
                TermColor::new(0x94, 0xac, 0xa4),
                TermColor::new(0xec, 0xe8, 0xdc),
            ],
            foreground: TermColor::new(0xc8, 0xc4, 0xb8),
            background: TermColor::new(0x1c, 0x1b, 0x19),
            cursor: TermColor::new(0xc8, 0xc4, 0xb8),
            selection: TermColor::new(0x3a, 0x38, 0x34),
            active_border: TermColor::new(0xa8, 0x98, 0x68),
            inactive_border: TermColor::new(0x3a, 0x38, 0x34),
        }
    }

    pub fn fallback() -> Self {
        Self::caffeine()
    }

    pub fn ansi(&self, idx: u8) -> TermColor {
        self.colors[(idx as usize).min(15)]
    }

    pub fn named(&self, c: NamedColor) -> TermColor {
        match c {
            NamedColor::Black => self.colors[0],
            NamedColor::Red => self.colors[1],
            NamedColor::Green => self.colors[2],
            NamedColor::Yellow => self.colors[3],
            NamedColor::Blue => self.colors[4],
            NamedColor::Magenta => self.colors[5],
            NamedColor::Cyan => self.colors[6],
            NamedColor::White => self.colors[7],
            NamedColor::BrightBlack => self.colors[8],
            NamedColor::BrightRed => self.colors[9],
            NamedColor::BrightGreen => self.colors[10],
            NamedColor::BrightYellow => self.colors[11],
            NamedColor::BrightBlue => self.colors[12],
            NamedColor::BrightMagenta => self.colors[13],
            NamedColor::BrightCyan => self.colors[14],
            NamedColor::BrightWhite => self.colors[15],
            NamedColor::Foreground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            _ => self.foreground,
        }
    }
}

static PALETTE: OnceLock<TermPalette> = OnceLock::new();

/// Process-wide palette.
pub fn global() -> &'static TermPalette {
    PALETTE.get_or_init(load_configured)
}

fn load_configured() -> TermPalette {
    let spec = std::env::var("PMUX_THEME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(theme_from_toml)
        .unwrap_or_else(|| {
            if crate::paths::theme_file().is_file() {
                "theme.conf".into()
            } else {
                "caffeine".into()
            }
        });
    load_theme_spec(&spec)
}

fn theme_from_toml() -> Option<String> {
    let text = std::fs::read_to_string(crate::paths::config_file()).ok()?;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let rest = line.strip_prefix("theme")?;
        let rest = rest.trim().strip_prefix('=')?.trim();
        let val = rest
            .trim_matches('"')
            .trim_matches('\'')
            .trim();
        if !val.is_empty() {
            return Some(val.to_string());
        }
    }
    None
}

/// `caffeine` | `kitty` | path (relative to `~/.config/pmux` unless absolute / `~/`).
pub fn load_theme_spec(spec: &str) -> TermPalette {
    let spec = spec.trim();
    if spec.eq_ignore_ascii_case("caffeine") {
        return TermPalette::caffeine();
    }
    if spec.eq_ignore_ascii_case("kitty") {
        return kitty_config_path()
            .and_then(|p| load_kitty_file(&p).ok())
            .unwrap_or_else(TermPalette::caffeine);
    }
    let path = resolve_theme_path(spec);
    load_kitty_file(&path).unwrap_or_else(|_| TermPalette::caffeine())
}

fn resolve_theme_path(spec: &str) -> PathBuf {
    if let Some(rest) = spec.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }
    let p = PathBuf::from(spec);
    if p.is_absolute() {
        return p;
    }
    crate::paths::config_dir().join(spec)
}

fn kitty_config_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    let path = base.join("kitty").join("kitty.conf");
    path.is_file().then_some(path)
}

/// Load a kitty.conf (and recursive `include`s).
pub fn load_kitty_file(path: &Path) -> Result<TermPalette, std::io::Error> {
    let mut pal = TermPalette::caffeine();
    let mut seen = HashSet::new();
    load_kitty_into(&mut pal, path, &mut seen, 0)?;
    Ok(pal)
}

fn load_kitty_into(
    pal: &mut TermPalette,
    path: &Path,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) -> Result<(), std::io::Error> {
    if depth > 8 {
        return Ok(());
    }
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon) {
        return Ok(());
    }
    let text = std::fs::read_to_string(path)?;
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    apply_kitty_text(pal, &text, base, seen, depth);
    Ok(())
}

fn apply_kitty_text(
    pal: &mut TermPalette,
    text: &str,
    base: &Path,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) {
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        let Some(val) = parts.next() else {
            continue;
        };

        if key.eq_ignore_ascii_case("include") {
            let inc = resolve_include(base, val);
            let _ = load_kitty_into(pal, &inc, seen, depth + 1);
            continue;
        }

        let Some(color) = parse_hex(val) else {
            continue;
        };

        if let Some(idx) = parse_color_index(key) {
            pal.colors[idx] = color;
            continue;
        }

        match key {
            "foreground" => pal.foreground = color,
            "background" => pal.background = color,
            "cursor" => pal.cursor = color,
            "selection_background" => pal.selection = color,
            "active_border_color" | "active_tab_background" => pal.active_border = color,
            "inactive_border_color" | "inactive_tab_background" => pal.inactive_border = color,
            _ => {}
        }
    }
}

/// Drop `# comment` but keep `#RRGGBB` color tokens.
fn strip_comment(line: &str) -> &str {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') && !looks_like_hex_token(trimmed) {
        return "";
    }
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i > 0 && bytes[i - 1].is_ascii_whitespace() {
            let rest = &line[i..];
            if !looks_like_hex_token(rest) {
                return &line[..i];
            }
        }
        i += 1;
    }
    line
}

fn looks_like_hex_token(s: &str) -> bool {
    let t = s.trim();
    if !t.starts_with('#') {
        return false;
    }
    let hex = t[1..]
        .split(|c: char| c.is_ascii_whitespace())
        .next()
        .unwrap_or("");
    (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|c| c.is_ascii_hexdigit())
}

fn parse_color_index(key: &str) -> Option<usize> {
    let rest = key.strip_prefix("color")?;
    let idx: usize = rest.parse().ok()?;
    (idx <= 15).then_some(idx)
}

fn parse_hex(s: &str) -> Option<TermColor> {
    let s = s.strip_prefix('#').unwrap_or(s);
    match s.len() {
        3 => {
            let n = u16::from_str_radix(s, 16).ok()?;
            let r = (((n >> 8) & 0xF) * 0x11) as u8;
            let g = (((n >> 4) & 0xF) * 0x11) as u8;
            let b = ((n & 0xF) * 0x11) as u8;
            Some(TermColor::new(r, g, b))
        }
        6 => {
            let n = u32::from_str_radix(s, 16).ok()?;
            Some(TermColor::new(
                ((n >> 16) & 0xFF) as u8,
                ((n >> 8) & 0xFF) as u8,
                (n & 0xFF) as u8,
            ))
        }
        _ => None,
    }
}

fn resolve_include(base: &Path, spec: &str) -> PathBuf {
    if let Some(rest) = spec.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }
    if spec.starts_with('/') {
        return PathBuf::from(spec);
    }
    base.join(spec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parse_hex_rrggbb() {
        assert_eq!(parse_hex("#f92672"), Some(TermColor::new(0xf9, 0x26, 0x72)));
        assert_eq!(parse_hex("272822"), Some(TermColor::new(0x27, 0x28, 0x22)));
        assert_eq!(parse_hex("#fff"), Some(TermColor::new(255, 255, 255)));
    }

    #[test]
    fn load_kitty_include_and_colors() {
        let dir = std::env::temp_dir().join(format!("pw-kitty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let theme = dir.join("theme.conf");
        let conf = dir.join("kitty.conf");
        std::fs::write(
            &theme,
            "background #272822\nforeground #f8f8f2\ncolor1 #f92672\ncolor2 #a6e22e\n",
        )
        .unwrap();
        let mut f = std::fs::File::create(&conf).unwrap();
        writeln!(f, "include theme.conf").unwrap();
        writeln!(f, "color0 #111111 # comment after hex").unwrap();
        writeln!(f, "cursor #f8f8f2").unwrap();

        let pal = load_kitty_file(&conf).unwrap();
        assert_eq!(pal.background, TermColor::new(0x27, 0x28, 0x22));
        assert_eq!(pal.foreground, TermColor::new(0xf8, 0xf8, 0xf2));
        assert_eq!(pal.colors[0], TermColor::new(0x11, 0x11, 0x11));
        assert_eq!(pal.colors[1], TermColor::new(0xf9, 0x26, 0x72));
        assert_eq!(pal.colors[2], TermColor::new(0xa6, 0xe2, 0x2e));
        assert_eq!(pal.cursor, TermColor::new(0xf8, 0xf8, 0xf2));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn caffeine_is_muted_not_vga() {
        let pal = TermPalette::caffeine();
        assert!(pal.colors[2].g < 200, "green should not be neon");
        assert!(pal.colors[6].b < 220, "cyan should not be neon");
        assert_eq!(pal.background, TermColor::new(0x1c, 0x1b, 0x19));
    }

    #[test]
    fn load_theme_spec_caffeine() {
        let pal = load_theme_spec("caffeine");
        assert_eq!(pal.foreground, TermPalette::caffeine().foreground);
    }

    #[test]
    fn load_theme_spec_file() {
        let dir = std::env::temp_dir().join(format!("pw-theme-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("mine.conf");
        std::fs::write(&conf, "background #111111\ncolor2 #445533\n").unwrap();
        let pal = load_theme_spec(&conf.display().to_string());
        assert_eq!(pal.background, TermColor::new(0x11, 0x11, 0x11));
        assert_eq!(pal.colors[2], TermColor::new(0x44, 0x55, 0x33));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
