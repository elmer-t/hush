//! Turns imported text into clean Markdown: HTML is converted block by block, and
//! hard-wrapped paragraphs are joined back into single lines.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use htmd::HtmlToMarkdown;
use htmd::options::{BulletListMarker, HrStyle, Options};
use regex::Regex;

/// A paragraph line shorter than this ended where the writer pressed Enter, not
/// where an editor wrapped it, so the next line is not joined to it.
pub const UNWRAP_MIN_LINE: usize = 50;

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(/?)([a-zA-Z][a-zA-Z0-9]*)(?:\s[^<>]*)?/?>|<!--").unwrap());
static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(?:[a-zA-Z][a-zA-Z0-9]{1,31}|#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6});").unwrap()
});
/// The backslash escapes the converter adds to text that only looks like Markdown.
static ESCAPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\\([\\`*_{}\[\]()#+\-.!|<>~=])").unwrap());
static BLANK_RUNS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());
static SPACE_RUNS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\S) {2,}(\S)").unwrap());
/// Stands in for a line break while a paragraph goes through the HTML parser, which
/// would collapse it (a private-use character, so it can't clash with real text).
const LINE_BREAK: char = '\u{e000}';

/// Element names that count as HTML; anything else in angle brackets (`<jan@example.org>`,
/// `<3`) is text.
const HTML_TAGS: &[&str] = &[
    "a",
    "abbr",
    "acronym",
    "address",
    "area",
    "article",
    "aside",
    "b",
    "base",
    "basefont",
    "big",
    "blink",
    "blockquote",
    "body",
    "br",
    "button",
    "caption",
    "center",
    "cite",
    "code",
    "col",
    "colgroup",
    "dd",
    "del",
    "details",
    "dfn",
    "dir",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "font",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "marquee",
    "menu",
    "meta",
    "nav",
    "nobr",
    "noscript",
    "object",
    "ol",
    "option",
    "p",
    "param",
    "pre",
    "q",
    "s",
    "samp",
    "script",
    "section",
    "select",
    "small",
    "source",
    "span",
    "strike",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "tt",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// Tags that structure a whole block; a block holding one is converted as one piece of HTML.
const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "body",
    "br",
    "center",
    "dd",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "li",
    "main",
    "nav",
    "ol",
    "p",
    "pre",
    "script",
    "section",
    "style",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "ul",
];

/// Tags with a Markdown equivalent. Others are dropped (keeping their text) and reported.
const CONVERTED_TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "body",
    "br",
    "code",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "hr",
    "html",
    "i",
    "img",
    "li",
    "ol",
    "p",
    "pre",
    "strong",
    "ul",
];

/// Removed together with their content.
const SKIPPED_TAGS: &[&str] = &[
    "script", "style", "title", "meta", "link", "iframe", "object", "embed", "form", "noscript",
    "head",
];

thread_local! {
    static CONVERTER: HtmlToMarkdown = HtmlToMarkdown::builder()
        .skip_tags(SKIPPED_TAGS.to_vec())
        .options(Options {
            bullet_list_marker: BulletListMarker::Dash,
            ul_bullet_spacing: 1,
            ol_number_spacing: 1,
            hr_style: HrStyle::Dashes,
            ..Default::default()
        })
        .build();
}

/// Cleans the content of one file (frontmatter excluded). Returns the Markdown and
/// any warnings.
pub fn clean(content: &str) -> (String, Vec<String>) {
    let mut dropped = BTreeSet::new();
    let mut out = String::with_capacity(content.len());
    for segment in segments(content) {
        match segment {
            Segment::Verbatim(text) => out.push_str(text),
            Segment::Block(text) => out.push_str(&clean_block(text, &mut dropped)),
        }
    }
    let out = unwrap(&out.replace('\u{a0}', " "));
    let out = BLANK_RUNS.replace_all(out.trim_matches('\n'), "\n\n");
    let lines: Vec<&str> = out.lines().collect();
    let mut out = String::with_capacity(out.len());
    for (i, line) in lines.iter().enumerate() {
        // A hard break before a blank line (or at the end) breaks nothing.
        let next_blank = lines.get(i + 1).is_none_or(|l| l.trim().is_empty());
        out.push_str(if next_blank {
            line.trim_end()
        } else {
            trim_end_keep_break(line)
        });
        out.push('\n');
    }
    if out.trim().is_empty() {
        out.clear();
    }
    (out, dropped_warning(dropped))
}

/// Converts a whole HTML document (an `.html` file) to Markdown, to be cleaned like
/// any other text afterwards. Returns warnings about dropped markup.
pub fn html_document(html: &str) -> (String, Vec<String>) {
    let mut dropped = BTreeSet::new();
    note_dropped(&tags_in(html), &mut dropped);
    (convert(html), dropped_warning(dropped))
}

fn note_dropped(tags: &[String], dropped: &mut BTreeSet<String>) {
    for tag in tags {
        if !CONVERTED_TAGS.contains(&tag.as_str()) {
            dropped.insert(if tag == "!--" {
                "<!-- comment -->".to_string()
            } else {
                format!("<{tag}>")
            });
        }
    }
}

fn dropped_warning(dropped: BTreeSet<String>) -> Vec<String> {
    if dropped.is_empty() {
        return Vec::new();
    }
    let tags: Vec<String> = dropped.into_iter().collect();
    vec![format!("dropped HTML markup: {}", tags.join(", "))]
}

enum Segment<'a> {
    /// Fenced code and blank lines, copied as they are.
    Verbatim(&'a str),
    /// Lines between blank lines.
    Block(&'a str),
}

/// Splits on blank lines, keeping fenced code blocks whole. Every byte of `text`
/// ends up in exactly one segment.
fn segments<'a>(text: &'a str) -> Vec<Segment<'a>> {
    let mut segs = Vec::new();
    let mut fence: Option<&str> = None;
    let mut start = 0;
    let mut block_start: Option<usize> = None;
    let mut pos = 0;
    let flush = |segs: &mut Vec<Segment<'a>>, from: &mut Option<usize>, to: usize| {
        if let Some(s) = from.take() {
            segs.push(Segment::Block(&text[s..to]));
        }
    };
    for line in text.split_inclusive('\n') {
        let end = pos + line.len();
        if let Some(marker) = fence {
            if is_fence(line) == Some(marker) {
                fence = None;
                segs.push(Segment::Verbatim(&text[start..end]));
            }
        } else if let Some(marker) = is_fence(line) {
            flush(&mut segs, &mut block_start, pos);
            fence = Some(marker);
            start = pos;
        } else if line.trim().is_empty() {
            flush(&mut segs, &mut block_start, pos);
            segs.push(Segment::Verbatim(&text[pos..end]));
        } else if block_start.is_none() {
            block_start = Some(pos);
        }
        pos = end;
    }
    if fence.is_some() {
        segs.push(Segment::Verbatim(&text[start..]));
    }
    flush(&mut segs, &mut block_start, text.len());
    segs
}

fn is_fence(line: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if line.len() - t.len() > 3 {
        None
    } else if t.starts_with("```") {
        Some("```")
    } else if t.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

/// HTML tag names in `text`, lowercased; `!--` for comments.
fn tags_in(text: &str) -> Vec<String> {
    TAG.captures_iter(text)
        .filter_map(|c| match c.get(2) {
            Some(name) => {
                let name = name.as_str().to_lowercase();
                HTML_TAGS.contains(&name.as_str()).then_some(name)
            }
            None => Some("!--".to_string()),
        })
        .collect()
}

fn clean_block(block: &str, dropped: &mut BTreeSet<String>) -> String {
    if block
        .lines()
        .all(|l| l.starts_with('\t') || l.starts_with("    "))
    {
        // Indented code.
        return block.to_string();
    }
    let tags = tags_in(block);
    note_dropped(&tags, dropped);
    if tags
        .iter()
        .any(|t| t == "!--" || BLOCK_TAGS.contains(&t.as_str()))
    {
        // Structural HTML: convert as one piece, so tags left open close at the block's end.
        let trailing = &block[block.trim_end_matches(['\n', '\r']).len()..];
        let md = convert(block);
        return if md.is_empty() {
            String::new()
        } else {
            format!("{md}{trailing}")
        };
    }
    if tags.is_empty() && !ENTITY.is_match(block) {
        return block.to_string();
    }
    if !tags.is_empty() && block.lines().all(is_paragraph_line) {
        // A paragraph with inline markup: convert as one piece, so markup may span lines,
        // but keep the line breaks for `unwrap` to judge.
        let trailing = &block[block.trim_end_matches(['\n', '\r']).len()..];
        let md = convert(&block.trim_end().replace('\n', &LINE_BREAK.to_string()));
        let lines: Vec<&str> = md.split(LINE_BREAK).map(str::trim).collect();
        return format!("{}{trailing}", lines.join("\n"));
    }
    // Inline markup only: convert line by line so Markdown lists and line breaks survive.
    block
        .split_inclusive('\n')
        .map(|line| {
            let body = line.trim_end_matches(['\n', '\r']);
            if tags_in(body).is_empty() && !ENTITY.is_match(body) {
                return line.to_string();
            }
            let indent = &body[..body.len() - body.trim_start().len()];
            let hard_break = if body.ends_with("  ") { "  " } else { "" };
            format!(
                "{indent}{}{hard_break}{}",
                convert(body.trim()),
                &line[body.len()..]
            )
        })
        .collect()
}

/// Converts HTML to Markdown, undoing the escapes of Markdown-like text and tidying
/// the whitespace HTML would have collapsed.
fn convert(html: &str) -> String {
    let md = CONVERTER.with(|c| c.convert(html)).unwrap_or_default();
    let md = ESCAPE.replace_all(&md, "$1").replace('\u{a0}', " ");
    // Twice, as matches can't overlap ("a  b  c").
    let md = SPACE_RUNS.replace_all(&md, "$1 $2");
    let md = SPACE_RUNS.replace_all(&md, "$1 $2");
    let mut out = String::with_capacity(md.len());
    let mut after_break = false;
    for line in md.split('\n') {
        // After a `<br>`, the line break in the source survives as a leading space.
        let line = if after_break { line.trim_start() } else { line };
        after_break = line.ends_with("  ") || (after_break && line.is_empty());
        out.push_str(line);
        out.push('\n');
    }
    out.pop();
    out
}

/// Joins hard-wrapped paragraph lines. Headings, list items, quotes, tables, code and
/// lines ending in a hard break (two spaces or `\`) are left alone, and a line is only
/// joined to one that is at least [`UNWRAP_MIN_LINE`] characters long.
fn unwrap(text: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut fence: Option<&str> = None;
    // The previous physical line, when the current output line is a paragraph that may grow.
    let mut prev: Option<&str> = None;
    for line in text.split('\n') {
        if let Some(marker) = fence {
            fence = (is_fence(line) != Some(marker)).then_some(marker);
            out.push(line.to_string());
            continue;
        }
        if let Some(marker) = is_fence(line) {
            fence = Some(marker);
            prev = None;
            out.push(line.to_string());
            continue;
        }
        let joinable = prev.is_some_and(|p| {
            p.chars().count() >= UNWRAP_MIN_LINE && !p.ends_with("  ") && !p.ends_with('\\')
        }) && is_paragraph_line(line);
        match (joinable, out.last_mut()) {
            (true, Some(last)) => {
                last.push(' ');
                last.push_str(line.trim());
            }
            _ => out.push(line.to_string()),
        }
        prev = is_paragraph_line(line).then_some(line);
    }
    out.join("\n")
}

/// A line of plain paragraph text, as opposed to blank lines and other block syntax.
fn is_paragraph_line(line: &str) -> bool {
    static LIST_ITEM: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^\s*(?:[-*+]|\d{1,9}[.)])(?:\s|$)").unwrap());
    static RULE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s{0,3}(?:(?:-\s*){3,}|(?:\*\s*){3,}|(?:_\s*){3,}|=+)\s*$").unwrap()
    });
    let t = line.trim_start();
    !(t.is_empty()
        || line.starts_with('\t')
        || line.len() - t.len() >= 4
        || t.starts_with(['#', '>', '|'])
        || LIST_ITEM.is_match(line)
        || RULE.is_match(line))
}

/// Trims trailing whitespace but keeps a two-space hard break.
fn trim_end_keep_break(line: &str) -> &str {
    let trimmed = line.trim_end();
    if !trimmed.is_empty() && line[trimmed.len()..].starts_with("  ") {
        &line[..trimmed.len() + 2]
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cleaned(s: &str) -> String {
        clean(s).0
    }

    #[test]
    fn plain_markdown_is_untouched() {
        let md = "# Title\n\n- a\n- b\n\n> quote\n\nShort line\nanother short line\n";
        assert_eq!(cleaned(md), md);
    }

    #[test]
    fn hard_wraps_are_joined() {
        let long = "This line is long enough that an editor would have wrapped it here";
        assert_eq!(
            cleaned(&format!("{long}\nand here.\n")),
            format!("{long} and here.\n")
        );
        assert_eq!(
            cleaned("Dear Jan,\nHow are you?\n"),
            "Dear Jan,\nHow are you?\n"
        );
        assert_eq!(
            cleaned(&format!("{long}  \nkept\n")),
            format!("{long}  \nkept\n")
        );
        assert_eq!(
            cleaned(&format!("{long}\\\nkept\n")),
            format!("{long}\\\nkept\n")
        );
        assert_eq!(
            cleaned(&format!("{long}\n- item\n")),
            format!("{long}\n- item\n")
        );
        assert_eq!(
            cleaned(&format!("- {long}\n  continued\n")),
            format!("- {long}\n  continued\n")
        );
        assert_eq!(cleaned(&format!("{long}\n===\n")), format!("{long}\n===\n"));
    }

    #[test]
    fn code_is_untouched() {
        let long = "This line is long enough that an editor would have wrapped it here";
        let md =
            format!("```\n<b>x</b>\n\n{long}\nnext\n```\n\n    <b>indented</b> {long}\n    next\n");
        assert_eq!(cleaned(&md), md);
    }

    #[test]
    fn structural_html_is_converted_per_block() {
        assert_eq!(
            cleaned("<p>One\ntwo <b>bold</b><br>three</p>\n"),
            "One two **bold**  \nthree\n"
        );
        assert_eq!(cleaned("<ul><li>a<li>b</ul>"), "- a\n- b\n");
        assert_eq!(
            cleaned("<p>unclosed <i>italic\n\nnext block</i>\n"),
            "unclosed *italic*\n\nnext block\n"
        );
        assert_eq!(cleaned("<script>x()</script>\n\nkept\n"), "kept\n");
    }

    #[test]
    fn inline_html_keeps_markdown_structure() {
        assert_eq!(
            cleaned(
                "Some **md** and <i>html</i> with [[A link]] and #tag\n- item <b>b</b>\n  - nested &amp; more\n"
            ),
            "Some **md** and *html* with [[A link]] and #tag\n- item **b**\n  - nested & more\n"
        );
    }

    #[test]
    fn entities_and_lookalikes() {
        assert_eq!(
            cleaned("Tom &amp; Jerry&nbsp;&mdash; 1. &lt;3\n"),
            "Tom & Jerry — 1. <3\n"
        );
        assert_eq!(
            cleaned("From: Jan <jan@example.org>\n"),
            "From: Jan <jan@example.org>\n"
        );
        assert_eq!(cleaned("a <3 b > c\n"), "a <3 b > c\n");
    }

    #[test]
    fn dropped_markup_is_reported() {
        let (md, warnings) =
            clean("<font color=red>Red <u>x</u></font>\n\n<!-- c -->\n\n<b>ok</b>");
        assert_eq!(md, "Red x\n\n**ok**\n");
        assert_eq!(
            warnings,
            ["dropped HTML markup: <!-- comment -->, <font>, <u>"]
        );
    }

    #[test]
    fn html_whitespace_is_tidied() {
        assert_eq!(cleaned("<p>a<br>\nb<br><br>\nc</p>"), "a  \nb\n\nc\n");
        assert_eq!(cleaned("<i>x</i>&nbsp; y  z<br>"), "*x* y z\n");
    }

    #[test]
    fn inline_markup_may_span_paragraph_lines() {
        let long = "This line is long enough that an editor would have wrapped it here";
        assert_eq!(
            cleaned(&format!("<i>{long}\nand here</i>\n")),
            format!("*{long} and here*\n")
        );
        assert_eq!(
            cleaned("<b>Dear</b> Jan,\nHow are you?\n"),
            "**Dear** Jan,\nHow are you?\n"
        );
    }

    #[test]
    fn blank_runs_collapse() {
        assert_eq!(cleaned("\n\na\n\n\n\nb\n\n"), "a\n\nb\n");
        assert_eq!(cleaned(""), "");
    }
}
