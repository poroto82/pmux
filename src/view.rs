//! File / URL preview. UI hosts a native WebView (wry); this module decides what to load.

use std::path::{Path, PathBuf};

use crate::component::{
    Component, ComponentInput, ComponentState, RenderOutput, StyledLine, TextColor,
};
use crate::markdown;

pub const VIEW: &str = "view";

const MAX_TEXT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone)]
pub enum ViewContent {
    Empty,
    Markdown { text: String, base_dir: PathBuf },
    /// Local file the WebView can open directly (html, pdf, image, svg, …).
    File(PathBuf),
    Url(String),
    PlainText { text: String, ext: String },
    Error(String),
}

/// What the native WebView should show this frame.
#[derive(Debug, Clone)]
pub enum ViewNav {
    None,
    Html { key: String, html: String },
    Url { key: String, url: String },
    Message(String),
}

pub struct ViewComponent {
    name: String,
    path: Option<PathBuf>,
    path_edit: String,
    content: ViewContent,
}

impl ViewComponent {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            path: None,
            path_edit: String::new(),
            content: ViewContent::Empty,
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn loaded_path_str(&self) -> Option<String> {
        self.path.as_ref().map(|p| p.display().to_string())
    }

    pub fn path_edit(&self) -> &str {
        &self.path_edit
    }

    pub fn path_edit_mut(&mut self) -> &mut String {
        &mut self.path_edit
    }

    pub fn content(&self) -> &ViewContent {
        &self.content
    }

    pub fn load(&mut self, path: impl AsRef<Path>) -> Result<(), String> {
        let raw = path.as_ref();
        self.path_edit = raw.display().to_string();
        let s = raw.to_string_lossy();
        if s.starts_with("http://") || s.starts_with("https://") {
            self.path = Some(PathBuf::from(s.as_ref()));
            self.content = ViewContent::Url(s.into_owned());
            return Ok(());
        }
        let path = raw;
        self.path = Some(path.to_path_buf());
        self.content = load_path(path);
        match &self.content {
            ViewContent::Error(e) => Err(e.clone()),
            _ => Ok(()),
        }
    }

    pub fn nav(&self) -> ViewNav {
        match &self.content {
            ViewContent::Empty => ViewNav::Message(
                "Drop a file, Load a path, or from a terminal:\npwctl view README.md\npwctl view http://localhost:5173".into(),
            ),
            ViewContent::Error(e) => ViewNav::Message(e.clone()),
            ViewContent::Url(url) => ViewNav::Url {
                key: url.clone(),
                url: url.clone(),
            },
            ViewContent::File(path) => match file_url(path) {
                Ok(url) => ViewNav::Url {
                    key: url.clone(),
                    url,
                },
                Err(e) => ViewNav::Message(e),
            },
            ViewContent::Markdown { text, base_dir } => {
                let body = markdown::markdown_to_html(text);
                let title = self
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("preview");
                let html = wrap_preview_html(&body, title, Some(base_dir));
                ViewNav::Html {
                    key: self.loaded_path_str().unwrap_or_else(|| title.into()),
                    html,
                }
            }
            ViewContent::PlainText { text, ext } => {
                let escaped = escape_html(text);
                let body = format!("<pre><code>{escaped}</code></pre>");
                let title = self
                    .path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or(ext);
                let base = self.path.as_ref().and_then(|p| p.parent());
                let html = wrap_preview_html(&body, title, base);
                ViewNav::Html {
                    key: self.loaded_path_str().unwrap_or_else(|| title.into()),
                    html,
                }
            }
        }
    }
}

fn load_path(path: &Path) -> ViewContent {
    if !path.exists() {
        return ViewContent::Error(format!("not found: {}", path.display()));
    }
    if path.is_dir() {
        let index = path.join("index.html");
        if index.is_file() {
            return ViewContent::File(index);
        }
        return ViewContent::Error(format!(
            "directory (no index.html): {}",
            path.display()
        ));
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "md" | "markdown" | "mdown" => match std::fs::read_to_string(path) {
            Ok(text) => ViewContent::Markdown {
                text,
                base_dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            },
            Err(e) => ViewContent::Error(e.to_string()),
        },
        "html" | "htm" | "pdf" | "svg" | "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        | "ico" => ViewContent::File(path.to_path_buf()),
        _ => load_plain(path, &ext),
    }
}

fn load_plain(path: &Path, ext: &str) -> ViewContent {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => return ViewContent::Error(e.to_string()),
    };
    if meta.len() > MAX_TEXT_BYTES {
        return ViewContent::Error(format!("file too large for text preview ({ext})"));
    }
    match std::fs::read_to_string(path) {
        Ok(text) => ViewContent::PlainText {
            text,
            ext: ext.to_string(),
        },
        Err(_) => ViewContent::Error(format!("binary or unknown type .{ext}")),
    }
}

pub fn file_url(path: &Path) -> Result<String, String> {
    let abs = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    url::Url::from_file_path(&abs)
        .map(|u| u.to_string())
        .map_err(|_| format!("invalid file path: {}", abs.display()))
}

pub fn wrap_preview_html(body: &str, title: &str, base_dir: Option<&Path>) -> String {
    let title = escape_html(title);
    let base = base_dir
        .and_then(|d| url::Url::from_directory_path(d).ok())
        .map(|u| format!(r#"<base href="{}">"#, escape_html(u.as_str())))
        .unwrap_or_default();
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
{base}
<title>{title}</title>
<style>
:root {{ color-scheme: dark; }}
html, body {{
  margin: 0;
  background: #121212;
  color: #e8e8e8;
  font: 16px/1.55 -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
}}
body {{ padding: 20px 28px 64px; }}
h1, h2, h3, h4 {{ color: #fff; font-weight: 650; }}
h1 {{ font-size: 1.8rem; margin: 0.4em 0 0.5em; }}
h2 {{ font-size: 1.35rem; margin: 1.4em 0 0.45em; border-bottom: 1px solid #2a2a2a; padding-bottom: 0.25em; }}
h3 {{ font-size: 1.1rem; margin: 1.2em 0 0.35em; color: #2ad4a3; }}
a {{ color: #2ad4a3; }}
code, pre {{ font-family: ui-monospace, Menlo, "JetBrains Mono", monospace; font-size: 0.92em; }}
code {{ background: #1c1c1c; padding: 0.12em 0.4em; border-radius: 4px; }}
pre {{ background: #1c1c1c; padding: 12px 14px; overflow: auto; border-radius: 8px; }}
pre code {{ background: none; padding: 0; }}
blockquote {{ border-left: 3px solid #2ad4a3; margin: 0.8em 0; padding: 0.15em 1em; color: #aaa; }}
img, svg {{ max-width: 100%; height: auto; }}
table {{ border-collapse: collapse; }}
th, td {{ border: 1px solid #333; padding: 6px 10px; }}
hr {{ border: 0; border-top: 1px solid #2a2a2a; margin: 1.5em 0; }}
ul {{ padding-left: 1.3em; }}
li {{ margin: 0.2em 0; }}
</style>
</head>
<body>
{body}
</body>
</html>"#
    )
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

impl Component for ViewComponent {
    fn component_type(&self) -> &str {
        VIEW
    }
    fn display_name(&self) -> &str {
        &self.name
    }
    fn state(&self) -> ComponentState {
        ComponentState::Running
    }
    fn render(&self, _cols: usize, _lines: usize) -> RenderOutput {
        let label = match &self.content {
            ViewContent::Empty => "view — drop a file or pwctl view <path>".into(),
            ViewContent::Markdown { .. } => "markdown".into(),
            ViewContent::File(_) => "file".into(),
            ViewContent::Url(_) => "url".into(),
            ViewContent::PlainText { .. } => "text".into(),
            ViewContent::Error(e) => e.clone(),
        };
        RenderOutput::Lines {
            header: Some(StyledLine::single("view", TextColor::accent())),
            subheader: self
                .path
                .as_ref()
                .map(|p| StyledLine::single(p.display().to_string(), TextColor::dim())),
            lines: vec![StyledLine::single(label, TextColor::white())],
        }
    }
    fn input(&mut self, _input: ComponentInput) {}
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_markdown() {
        let dir = std::env::temp_dir().join(format!("pw-view-md-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.md");
        std::fs::write(&path, "# Hello\n\nworld").unwrap();

        let mut view = ViewComponent::new("preview");
        view.load(&path).unwrap();
        match view.content() {
            ViewContent::Markdown { text, .. } => assert!(text.contains("# Hello")),
            other => panic!("expected markdown, got {other:?}"),
        }
        match view.nav() {
            ViewNav::Html { html, .. } => {
                assert!(html.contains("<h1>") || html.contains("<h1 "));
                assert!(html.contains("Hello"));
            }
            other => panic!("expected html nav, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_png_is_file() {
        let dir = std::env::temp_dir().join(format!("pw-view-png-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.png");
        // 1x1 PNG
        let png = [
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x03, 0x00, 0x01, 0x00, 0x05, 0xFE,
            0xD4, 0xEF, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        std::fs::write(&path, png).unwrap();

        let mut view = ViewComponent::new("preview");
        view.load(&path).unwrap();
        assert!(matches!(view.content(), ViewContent::File(_)));
        match view.nav() {
            ViewNav::Url { url, .. } => assert!(url.starts_with("file:")),
            other => panic!("expected file url, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_errors() {
        let mut view = ViewComponent::new("preview");
        let err = view.load("/tmp/pw-definitely-missing-xyz.md").unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn http_url_is_supported() {
        let mut view = ViewComponent::new("preview");
        view.load("https://example.com").unwrap();
        assert!(matches!(view.content(), ViewContent::Url(_)));
        match view.nav() {
            ViewNav::Url { url, .. } => assert_eq!(url, "https://example.com"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn html_index_in_dir() {
        let dir = std::env::temp_dir().join(format!("pw-view-idx-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("index.html"), "<h1>plugin</h1>").unwrap();
        let mut view = ViewComponent::new("preview");
        view.load(&dir).unwrap();
        assert!(matches!(view.content(), ViewContent::File(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
