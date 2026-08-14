//! Terminal color palette.
//!
//! Loads `~/.config/kitty/kitty.conf` (+ `include`s) so ANSI 16 + default
//! fg/bg match Kitty. Fallback: generic dark 16-color.

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

/// Full terminal palette (ANSI 16 + defaults).
#[derive(Debug, Clone)]
pub struct TermPalette {
    pub colors: [TermColor; 16],
    pub foreground: TermColor,
    pub background: TermColor,
    pub cursor: TermColor,
    pub active_border: TermColor,
    pub inactive_border: TermColor,
}

impl TermPalette {
    pub fn fallback() -> Self {
        Self {
            colors: [
                TermColor::new(0, 0, 0),
                TermColor::new(204, 4, 3),
                TermColor::new(25, 203, 0),
                TermColor::new(206, 203, 0),
                TermColor::new(13, 115, 204),
                TermColor::new(203, 30, 209),
                TermColor::new(13, 205, 205),
                TermColor::new(221, 221, 221),
                TermColor::new(118, 118, 118),
                TermColor::new(242, 32, 31),
                TermColor::new(35, 253, 0),
                TermColor::new(255, 253, 0),
                TermColor::new(26, 143, 255),
                TermColor::new(253, 40, 255),
                TermColor::new(20, 255, 255),
                TermColor::new(255, 255, 255),
            ],
            foreground: TermColor::new(221, 221, 221),
            background: TermColor::new(0, 0, 0),
            cursor: TermColor::new(221, 221, 221),
            active_border: TermColor::new(42, 212, 163),
            inactive_border: TermColor::new(18, 18, 18),
        }
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

/// Process-wide palette (Kitty theme if present).
pub fn global() -> &'static TermPalette {
    PALETTE.get_or_init(|| {
        kitty_config_path()
            .and_then(|p| load_kitty_file(&p).ok())
            .unwrap_or_else(TermPalette::fallback)
    })
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
    let mut pal = TermPalette::fallback();
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
            "active_border_color" => pal.active_border = color,
            "inactive_border_color" => pal.inactive_border = color,
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
}
