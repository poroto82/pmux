//! Markdown for the View pane (HTML via WebView, plus an AST for tests).
//!
//! spec.md is outline-style CommonMark: `12. Sessions` is an ordered list item,
//! ASCII trees live in paragraphs (soft breaks). We promote singleton ordered
//! lists / P-0N lines to headings before rendering HTML.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};

/// CommonMark → HTML, after outline fixes for spec.md-style docs.
pub fn markdown_to_html(src: &str) -> String {
    let atx = outline_to_atx(&normalize_outline(src));
    let opts = Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(&atx, opts);
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, parser);
    body
}

fn outline_to_atx(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len() + 64);
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if is_principle_heading(trimmed) || is_subsection_heading(trimmed) {
            out.push_str("### ");
            out.push_str(trimmed);
            out.push('\n');
            continue;
        }
        if is_numbered_line(line) {
            let title = line
                .trim_start()
                .split_once(". ")
                .map(|(_, t)| t)
                .unwrap_or("");
            let next = lines.get(i + 1).copied().unwrap_or("").trim_start();
            let section = !title.is_empty()
                && title.len() <= 90
                && !matches!(title.chars().last(), Some('.' | ';' | ','))
                && !is_numbered_line(next)
                && !next.starts_with("- ")
                && !next.starts_with("* ")
                && !next.starts_with("+ ");
            if section {
                out.push_str("## ");
                out.push_str(line.trim());
                out.push('\n');
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdRun {
    pub text: String,
    pub strong: bool,
    pub emphasis: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdBlock {
    Heading {
        level: u8,
        number: Option<u64>,
        runs: Vec<MdRun>,
    },
    Para {
        runs: Vec<MdRun>,
        mono: bool,
    },
    Code(String),
    Item {
        number: Option<u64>,
        task: Option<bool>,
        runs: Vec<MdRun>,
    },
    Quote {
        runs: Vec<MdRun>,
    },
    Image {
        alt: String,
        dest: String,
    },
    Rule,
}

#[derive(Default, Clone)]
struct Flags {
    strong: bool,
    emphasis: bool,
    strike: bool,
    link: Option<String>,
}

struct ListState {
    ordered: bool,
    next: u64,
    items: Vec<BufItem>,
}

struct BufItem {
    number: Option<u64>,
    task: Option<bool>,
    runs: Vec<MdRun>,
    had_nested: bool,
}

pub fn parse(src: &str) -> Vec<MdBlock> {
    let src = normalize_outline(src);
    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let parser = Parser::new_ext(&src, opts);

    let mut out = Vec::new();
    let mut runs: Vec<MdRun> = Vec::new();
    let mut flags = Flags::default();
    let mut in_code = false;
    let mut code_buf = String::new();
    let mut heading_level = 1u8;
    let mut in_heading = false;
    let mut in_quote = false;
    let mut in_image = false;
    let mut img_dest = String::new();
    let mut img_alt = String::new();
    let mut item_active = false;
    let mut item_number: Option<u64> = None;
    let mut item_task: Option<bool> = None;
    let mut item_had_nested = false;
    let mut lists: Vec<ListState> = Vec::new();

    for ev in parser {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_para(&mut out, &mut runs, in_quote);
                in_heading = true;
                heading_level = heading_u8(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                out.push(MdBlock::Heading {
                    level: heading_level,
                    number: None,
                    runs: std::mem::take(&mut runs),
                });
                in_heading = false;
            }
            Event::Start(Tag::Paragraph) => {
                if item_active && !runs.is_empty() {
                    push_text(&mut runs, &flags, "\n");
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if !in_heading && !item_active {
                    flush_para(&mut out, &mut runs, in_quote);
                }
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush_para(&mut out, &mut runs, in_quote);
                in_code = true;
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                out.push(MdBlock::Code(std::mem::take(&mut code_buf)));
                in_code = false;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush_para(&mut out, &mut runs, in_quote);
                in_quote = true;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush_para(&mut out, &mut runs, true);
                in_quote = false;
            }
            Event::Start(Tag::List(start)) => {
                if item_active {
                    item_had_nested = true;
                }
                lists.push(ListState {
                    ordered: start.is_some(),
                    next: start.unwrap_or(1),
                    items: Vec::new(),
                });
            }
            Event::End(TagEnd::List(_)) => {
                if let Some(list) = lists.pop() {
                    flush_list(&mut out, list, lists.is_empty());
                }
            }
            Event::Start(Tag::Item) => {
                item_number = lists.last_mut().and_then(|l| {
                    if l.ordered {
                        let n = l.next;
                        l.next += 1;
                        Some(n)
                    } else {
                        None
                    }
                });
                item_active = true;
                item_had_nested = false;
                item_task = None;
                runs.clear();
            }
            Event::End(TagEnd::Item) => {
                if let Some(list) = lists.last_mut() {
                    list.items.push(BufItem {
                        number: item_number.take(),
                        task: item_task.take(),
                        runs: std::mem::take(&mut runs),
                        had_nested: item_had_nested,
                    });
                }
                item_active = false;
            }
            Event::TaskListMarker(checked) => {
                item_task = Some(checked);
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                in_image = true;
                img_dest = dest_url.to_string();
                img_alt.clear();
            }
            Event::End(TagEnd::Image) => {
                flush_para(&mut out, &mut runs, in_quote);
                out.push(MdBlock::Image {
                    alt: std::mem::take(&mut img_alt),
                    dest: std::mem::take(&mut img_dest),
                });
                in_image = false;
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                flags.link = Some(dest_url.to_string());
            }
            Event::End(TagEnd::Link) => {
                flags.link = None;
            }
            Event::Start(Tag::Strong) => flags.strong = true,
            Event::End(TagEnd::Strong) => flags.strong = false,
            Event::Start(Tag::Emphasis) => flags.emphasis = true,
            Event::End(TagEnd::Emphasis) => flags.emphasis = false,
            Event::Start(Tag::Strikethrough) => flags.strike = true,
            Event::End(TagEnd::Strikethrough) => flags.strike = false,
            Event::Rule => {
                flush_para(&mut out, &mut runs, in_quote);
                out.push(MdBlock::Rule);
            }
            Event::Text(t) => {
                if in_image {
                    img_alt.push_str(&t);
                } else if in_code {
                    code_buf.push_str(&t);
                } else {
                    push_text(&mut runs, &flags, &t);
                }
            }
            Event::Code(t) => {
                if in_image {
                    img_alt.push_str(&t);
                } else if in_code {
                    code_buf.push_str(&t);
                } else {
                    push_code(&mut runs, &t);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if in_code {
                    code_buf.push('\n');
                } else if in_image {
                    img_alt.push(' ');
                } else {
                    push_text(&mut runs, &flags, "\n");
                }
            }
            _ => {}
        }
    }
    flush_para(&mut out, &mut runs, in_quote);
    while let Some(list) = lists.pop() {
        flush_list(&mut out, list, lists.is_empty());
    }
    out
}

pub fn flatten(runs: &[MdRun]) -> String {
    runs.iter().map(|r| r.text.as_str()).collect()
}

fn heading_u8(level: pulldown_cmark::HeadingLevel) -> u8 {
    use pulldown_cmark::HeadingLevel::*;
    match level {
        H1 => 1,
        H2 => 2,
        H3 => 3,
        H4 => 4,
        H5 => 5,
        H6 => 6,
    }
}

fn push_text(runs: &mut Vec<MdRun>, flags: &Flags, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if !last.code
            && last.strong == flags.strong
            && last.emphasis == flags.emphasis
            && last.strike == flags.strike
            && last.link == flags.link
        {
            last.text.push_str(text);
            return;
        }
    }
    runs.push(MdRun {
        text: text.to_string(),
        strong: flags.strong,
        emphasis: flags.emphasis,
        strike: flags.strike,
        code: false,
        link: flags.link.clone(),
    });
}

fn push_code(runs: &mut Vec<MdRun>, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut() {
        if last.code && last.link.is_none() {
            last.text.push_str(text);
            return;
        }
    }
    runs.push(MdRun {
        text: text.to_string(),
        strong: false,
        emphasis: false,
        strike: false,
        code: true,
        link: None,
    });
}

fn flush_para(out: &mut Vec<MdBlock>, runs: &mut Vec<MdRun>, in_quote: bool) {
    if runs.is_empty() {
        return;
    }
    let r = std::mem::take(runs);
    if in_quote {
        out.push(MdBlock::Quote { runs: r });
        return;
    }
    let flat = flatten(&r);
    let trimmed = flat.trim();
    if !trimmed.contains('\n')
        && (is_principle_heading(trimmed) || is_subsection_heading(trimmed))
    {
        out.push(MdBlock::Heading {
            level: 3,
            number: None,
            runs: r,
        });
        return;
    }
    out.push(MdBlock::Para {
        mono: looks_mono(&flat),
        runs: r,
    });
}

fn flush_list(out: &mut Vec<MdBlock>, list: ListState, top_level: bool) {
    let promote = top_level
        && list.ordered
        && list.items.len() == 1
        && !list.items[0].had_nested
        && is_section_heading(&list.items[0].runs);

    if promote {
        let item = list.items.into_iter().next().unwrap();
        let mut runs = Vec::new();
        if let Some(n) = item.number {
            runs.push(MdRun {
                text: format!("{n}. "),
                strong: true,
                emphasis: false,
                strike: false,
                code: false,
                link: None,
            });
        }
        runs.extend(item.runs);
        out.push(MdBlock::Heading {
            level: 2,
            number: item.number,
            runs,
        });
        return;
    }

    for item in list.items {
        out.push(MdBlock::Item {
            number: item.number,
            task: item.task,
            runs: item.runs,
        });
    }
}

fn is_section_heading(runs: &[MdRun]) -> bool {
    let t = flatten(runs);
    let t = t.trim();
    if t.is_empty() || t.len() > 90 || t.contains('\n') {
        return false;
    }
    !matches!(t.chars().last(), Some('.' | ';' | ','))
}

/// spec.md writes `12. Sessions` with no blank line after the previous block.
/// CommonMark only lets `1.` interrupt a paragraph, so we insert the blank.
fn normalize_outline(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = String::with_capacity(src.len() + 64);
    for (i, line) in lines.iter().enumerate() {
        if i > 0 && needs_break_before(line, lines[i - 1]) {
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn needs_break_before(line: &str, prev: &str) -> bool {
    if prev.trim().is_empty() {
        return false;
    }
    let s = line.trim_start();
    if is_numbered_line(s) {
        return !is_list_line(prev);
    }
    is_principle_heading(s) || is_subsection_heading(s)
}

fn is_numbered_line(line: &str) -> bool {
    let s = line.trim_start();
    let Some((num, rest)) = s.split_once(". ") else {
        return false;
    };
    !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) && !rest.is_empty()
}

fn is_list_line(line: &str) -> bool {
    let s = line.trim_start();
    s.starts_with("- ") || s.starts_with("* ") || s.starts_with("+ ") || is_numbered_line(s)
}

fn is_subsection_heading(s: &str) -> bool {
    let Some((nums, title)) = s.split_once(' ') else {
        return false;
    };
    if title.is_empty() || s.len() > 90 {
        return false;
    }
    let parts: Vec<&str> = nums.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn is_principle_heading(s: &str) -> bool {
    let Some((code, _)) = s
        .split_once(" — ")
        .or_else(|| s.split_once(" – "))
        .or_else(|| s.split_once(" - "))
    else {
        return false;
    };
    let mut parts = code.split('-');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(prefix), Some(num), None) => {
            !prefix.is_empty()
                && prefix.len() <= 4
                && prefix.chars().all(|c| c.is_ascii_uppercase())
                && !num.is_empty()
                && num.chars().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

fn looks_mono(s: &str) -> bool {
    const BOX: &[char] = &['├', '└', '│', '┌', '┐', '┘', '┴', '┬', '┤', '─'];
    if s.chars().any(|c| BOX.contains(&c)) {
        return true;
    }
    if (s.contains("trait ") || s.contains("impl ") || s.contains("fn ")) && s.contains('{') {
        return true;
    }
    s.lines().any(|l| l.starts_with("    ") && l.len() > 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atx_heading_and_inline() {
        let blocks = parse("# Hello\n\nthis is **bold** and `code`\n");
        match &blocks[0] {
            MdBlock::Heading { level, runs, .. } => {
                assert_eq!(*level, 1);
                assert_eq!(flatten(runs), "Hello");
            }
            other => panic!("{other:?}"),
        }
        match &blocks[1] {
            MdBlock::Para { runs, .. } => {
                assert!(runs.iter().any(|r| r.strong && r.text.contains("bold")));
                assert!(runs.iter().any(|r| r.code && r.text.contains("code")));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tight_ordered_list_stays_list() {
        let blocks = parse("1. foo\n2. bar\n3. baz\n");
        let items: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                MdBlock::Item { number, runs, .. } => Some((*number, flatten(runs))),
                _ => None,
            })
            .collect();
        assert_eq!(
            items,
            vec![
                (Some(1), "foo".into()),
                (Some(2), "bar".into()),
                (Some(3), "baz".into()),
            ]
        );
        assert!(!blocks.iter().any(|b| matches!(b, MdBlock::Heading { .. })));
    }

    #[test]
    fn singleton_ordered_item_becomes_heading() {
        let src = "1. Objetivo\n\nConstruir una app.\n\n2. Principios\n\nTexto.\n";
        let blocks = parse(src);
        let headings: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                MdBlock::Heading { number, runs, .. } => Some((*number, flatten(runs))),
                _ => None,
            })
            .collect();
        assert_eq!(headings[0], (Some(1), "1. Objetivo".into()));
        assert_eq!(headings[1], (Some(2), "2. Principios".into()));
    }

    #[test]
    fn numbered_section_without_blank_line() {
        let src = "    Shell\n12. Sessions\n\nUna sesión.\n";
        let blocks = parse(src);
        let headings: Vec<_> = blocks
            .iter()
            .filter_map(|b| match b {
                MdBlock::Heading { runs, .. } => Some(flatten(runs)),
                _ => None,
            })
            .collect();
        assert!(
            headings.iter().any(|h| h.contains("12. Sessions")),
            "{headings:?}"
        );
    }

    #[test]
    fn ascii_tree_keeps_newlines() {
        let src = "La unidad:\n\nWorkspace\n    └── Layout Tree\n          ├── Split\n          └── Component\n";
        let blocks = parse(src);
        let found = blocks.iter().any(|b| match b {
            MdBlock::Para { runs, mono } => {
                let t = flatten(runs);
                *mono && t.contains('\n') && t.contains('└') && t.contains("Workspace")
            }
            _ => false,
        });
        assert!(found, "{blocks:?}");
    }

    #[test]
    fn principle_line_is_subheading() {
        let blocks = parse("P-01 — El terminal no es el núcleo\n\nEl terminal es un componente.\n");
        match &blocks[0] {
            MdBlock::Heading { level, runs, .. } => {
                assert_eq!(*level, 3);
                assert!(flatten(runs).contains("P-01"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fenced_code_preserves_newlines() {
        let blocks = parse("```\nfn main() {\n    println!(\"hi\");\n}\n```\n");
        match &blocks[0] {
            MdBlock::Code(s) => {
                assert!(s.contains('\n'));
                assert!(s.contains("fn main()"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn spec_md_sections_and_trees() {
        let src = include_str!("../spec.md");
        let blocks = parse(src);
        let headings: Vec<String> = blocks
            .iter()
            .filter_map(|b| match b {
                MdBlock::Heading { runs, .. } => Some(flatten(runs)),
                _ => None,
            })
            .collect();
        assert!(
            headings.iter().any(|h| h.contains("1. Objetivo")),
            "got headings: {:?}",
            headings.iter().take(8).collect::<Vec<_>>()
        );
        assert!(headings.iter().any(|h| h.contains("12. Sessions")));
        assert!(headings.iter().any(|h| h.contains("P-01")));
        assert!(headings.iter().any(|h| h.contains("4.1")));

        let tree = blocks.iter().any(|b| match b {
            MdBlock::Para { runs, mono } => {
                let t = flatten(runs);
                *mono && t.contains('\n') && (t.contains('├') || t.contains('└'))
            }
            MdBlock::Code(s) => s.contains('├') || s.contains('└'),
            _ => false,
        });
        assert!(tree, "expected ascii tree with newlines");
    }

    #[test]
    fn spec_md_html_has_section_headings() {
        let html = markdown_to_html(include_str!("../spec.md"));
        assert!(html.contains("<h2>"), "got: {}", &html[..html.len().min(400)]);
        assert!(html.contains("Objetivo"));
        assert!(html.contains("Sessions"));
        assert!(html.contains("P-01"));
    }
}
