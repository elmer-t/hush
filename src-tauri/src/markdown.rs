//! Pure parsing of a note body: YAML frontmatter, inline `#tags` and `[[wikilinks]]`.
//!
//! The body is the single source of truth; everything the database indexes about a
//! note is derived from what this module returns.

use std::collections::HashSet;
use std::ops::Range;
use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use regex::Regex;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::{Yaml, YamlLoader};

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s(,;])#([\p{L}\p{N}_\-/]+)").unwrap());
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+)\]\]").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]+`").unwrap());
/// Bare URLs (`https://…`, `www.…`), whose `#fragment` isn't a tag.
static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(?:[a-z][a-z0-9+.\-]*://|www\.)\S+").unwrap());
/// HTML tags with their attributes, and `<style>` blocks, as HTML and Evernote exports
/// leave them: `<font color="#333333">`, `style="color: #fff"`.
static HTML: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<style\b[\s\S]*?</style\s*>|</?[a-z][a-z0-9-]*(?:\s[^<>]*)?/?>").unwrap());
static LETTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\p{L}").unwrap());
static ORDINAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9]+(?:st|nd|rd|th)$").unwrap());
static HEX_COLOUR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:[0-9a-fA-F]{3,4}|[0-9a-fA-F]{6}|[0-9a-fA-F]{8})$").unwrap());

/// Frontmatter longer than this isn't parsed: it is kept but not read, like broken YAML.
pub const MAX_FRONTMATTER_BYTES: usize = 64 * 1024;

/// Bumped whenever the tag rules change ([`is_tag`] or what [`parse`] skips), so the
/// background re-derive pass (`crate::retag`) runs once more on every vault.
pub const TAG_RULES_VERSION: u32 = 1;

/// Whether `name` (without the `#`) can be a tag, wherever it was written: inline, in
/// the frontmatter or by an import rule. It needs a letter and at least two characters,
/// and isn't an ordinal (`1st`) or a hex colour with a digit in it (`1e1e1e`, `f00`);
/// all-letter ones (`cafe`, `bad`) can be words and are kept.
pub fn is_tag(name: &str) -> bool {
    name.chars().nth(1).is_some()
        && LETTER.is_match(name)
        && !ORDINAL.is_match(name)
        && !(HEX_COLOUR.is_match(name) && name.contains(|c: char| c.is_ascii_digit()))
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parsed {
    pub author: Option<String>,
    pub topic: Option<String>,
    /// Frontmatter `rating`, a whole number 1–5. Any other value is kept as an attribute.
    pub rating: Option<u8>,
    /// Unix millis from frontmatter `created` (or `date`).
    pub created_at: Option<i64>,
    pub tags: Vec<String>,
    /// Every value of the frontmatter `tags`, as written: the frontmatter part of `tags`
    /// plus any value [`is_tag`] rejects, so rewriting the list never drops one.
    pub frontmatter_tags: Vec<String>,
    /// Every other frontmatter key, lowercased; list values become one pair per item.
    pub attributes: Vec<(String, String)>,
    pub links: Vec<WikiLink>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WikiLink {
    /// Folder part of the target, `""` for a bare title.
    pub path: String,
    pub title: String,
    pub heading: Option<String>,
    pub alias: Option<String>,
    /// Byte range of the target text (between `[[` and `#`, `|` or `]]`) in the body.
    pub target_range: Range<usize>,
}

pub fn parse(body: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let content_start = match split_frontmatter(body) {
        Some((yaml, end)) => {
            apply_frontmatter(yaml, &mut parsed);
            end
        }
        None => 0,
    };
    // Sorted and merged once, so each link is a binary search, not a scan (#151).
    let code = merge_ranges(code_ranges(body, content_start));
    let content = &body[content_start..];

    let skipped = not_tag_ranges(content, code.iter().map(|r| r.start - content_start..r.end - content_start));
    // A set, not `push_unique`: a note with many distinct tags was quadratic too.
    let mut seen_tags: HashSet<String> = parsed.tags.iter().cloned().collect();
    for caps in TAG.captures_iter(content) {
        let m = caps.get(1).unwrap();
        let tag = m.as_str().trim_end_matches('/');
        let hash = m.start() - 1;
        if in_ranges(&skipped, hash) || !is_tag(tag) || content[..hash].ends_with("](") || is_css_value(content, tag, m.end())
        {
            continue;
        }
        if !seen_tags.contains(tag) {
            seen_tags.insert(tag.to_string());
            parsed.tags.push(tag.to_string());
        }
    }

    for caps in WIKILINK.captures_iter(content) {
        let whole = caps.get(0).unwrap();
        // Embeds (`![[...]]`) are out of scope with attachments.
        if in_ranges(&code, content_start + whole.start()) || !caps[1].is_empty() {
            continue;
        }
        let inner = caps.get(2).unwrap();
        let (target_part, alias) = match inner.as_str().split_once('|') {
            Some((t, a)) => (t, Some(a.trim().to_string())),
            None => (inner.as_str(), None),
        };
        let (target, heading) = match target_part.split_once('#') {
            Some((t, h)) => (t, Some(h.trim().to_string())),
            None => (target_part, None),
        };
        let trimmed = target.trim();
        if trimmed.is_empty() {
            continue;
        }
        let start = content_start + inner.start() + (target.len() - target.trim_start().len());
        let (path, title) = split_target(trimmed);
        parsed.links.push(WikiLink {
            path,
            title,
            heading,
            alias,
            target_range: start..start + trimmed.len(),
        });
    }
    parsed
}

/// Splits a link target such as `archive/2019/Title.md` into `("archive/2019", "Title")`.
pub fn split_target(target: &str) -> (String, String) {
    let target = target.trim();
    let target = target.strip_suffix(".md").unwrap_or(target);
    match target.rsplit_once(['/', '\\']) {
        Some((path, title)) => (normalize_folder(path), title.trim().to_string()),
        None => (String::new(), target.to_string()),
    }
}

/// `/a\b//c/` → `a/b/c`; the root folder is `""`.
pub fn normalize_folder(folder: &str) -> String {
    folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Returns the YAML text and the byte offset where the content after the frontmatter starts.
pub fn split_frontmatter(body: &str) -> Option<(&str, usize)> {
    let rest = body.strip_prefix("---\n").or_else(|| body.strip_prefix("---\r\n"))?;
    let offset = body.len() - rest.len();
    let mut pos = 0;
    for line in rest.split_inclusive('\n') {
        let t = line.trim_end_matches(['\r', '\n']);
        if t == "---" || t == "..." {
            return Some((&rest[..pos], offset + pos + line.len()));
        }
        pos += line.len();
    }
    None
}

/// Whether frontmatter YAML can be read: empty, or a mapping.
pub fn frontmatter_is_valid(yaml: &str) -> bool {
    match load_frontmatter(yaml) {
        Some(docs) => matches!(docs.first(), None | Some(Yaml::Hash(_))),
        None => false,
    }
}

/// Loads frontmatter YAML, or `None` when it is broken, longer than
/// [`MAX_FRONTMATTER_BYTES`] or uses an alias (`*name`). The loader expands every alias
/// by copying the anchored node, so a few hundred bytes of nested aliases take minutes
/// and gigabytes (#150); Hush only reads flat keys, where aliases have no use.
fn load_frontmatter(yaml: &str) -> Option<Vec<Yaml>> {
    if yaml.len() > MAX_FRONTMATTER_BYTES {
        return None;
    }
    let mut events = Parser::new_from_str(yaml);
    loop {
        match events.next_token().ok()?.0 {
            Event::Alias(_) => return None,
            Event::StreamEnd => break,
            _ => {}
        }
    }
    YamlLoader::load_from_str(yaml).ok()
}

/// Sets the top-level frontmatter `key` to `values`, or removes it when `values` is empty.
/// Creates the frontmatter block when needed and drops it once it is empty; every other
/// line is left exactly as written.
pub fn set_frontmatter_field(body: &str, key: &str, values: &[String]) -> String {
    let crlf = body.starts_with("---\r\n") || (!body.starts_with("---\n") && body.contains("\r\n"));
    let eol = if crlf { "\r\n" } else { "\n" };
    let Some((yaml, end)) = split_frontmatter(body) else {
        if values.is_empty() {
            return body.to_string();
        }
        return format!("---{eol}{}---{eol}{body}", render_field(key, values, eol));
    };
    let yaml_start = if body.starts_with("---\r\n") { 5 } else { 4 };
    let lines: Vec<&str> = yaml.split_inclusive('\n').collect();

    let mut new_yaml = String::new();
    match lines.iter().position(|l| top_level_key(l).is_some_and(|k| k.eq_ignore_ascii_case(key))) {
        Some(start) => {
            let end = start + 1 + lines[start + 1..].iter().take_while(|l| l.starts_with([' ', '\t', '-'])).count();
            let written_key = top_level_key(lines[start]).unwrap_or(key);
            new_yaml.extend(lines[..start].iter().copied());
            if !values.is_empty() {
                new_yaml.push_str(&render_field(written_key, values, eol));
            }
            new_yaml.extend(lines[end..].iter().copied());
        }
        None => {
            new_yaml.push_str(yaml);
            if !values.is_empty() {
                if !new_yaml.is_empty() && !new_yaml.ends_with('\n') {
                    new_yaml.push_str(eol);
                }
                new_yaml.push_str(&render_field(key, values, eol));
            }
        }
    }

    let rest = &body[end..];
    if new_yaml.trim().is_empty() {
        return rest.to_string();
    }
    let open = &body[..yaml_start];
    let close = &body[yaml_start + yaml.len()..end];
    format!("{open}{new_yaml}{close}{rest}")
}

/// The key of a `key: value` line at the top level of the frontmatter.
fn top_level_key(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '-', '#']) {
        return None;
    }
    line.split_once(':').map(|(k, _)| k.trim()).filter(|k| !k.is_empty())
}

/// `tags` is always a list; other keys are a plain value when there is only one.
fn render_field(key: &str, values: &[String], eol: &str) -> String {
    let rendered: Vec<String> = values.iter().map(|v| yaml_scalar(v)).collect();
    if rendered.len() == 1 && !key.eq_ignore_ascii_case("tags") {
        format!("{key}: {}{eol}", rendered[0])
    } else {
        format!("{key}: [{}]{eol}", rendered.join(", "))
    }
}

/// A YAML scalar that reads back as the same string, quoted only when it has to be.
fn yaml_scalar(value: &str) -> String {
    let value = value.replace(['\r', '\n'], " ");
    let value = value.trim();
    let reserved = matches!(value.to_lowercase().as_str(), "" | "~" | "null" | "true" | "false");
    let plain = !reserved
        && !value.starts_with(|c: char| "-?:,[]{}#&*!|>'\"%@`".contains(c))
        && !value.contains([',', '[', ']', '{', '}'])
        && !value.contains(": ")
        && !value.contains(" #")
        && !value.ends_with(':');
    if plain {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Parses `2019-03-04`, `2019-03-04 10:30[:00]`, `2019-03-04T10:30[:00]` or RFC 3339 into
/// unix millis. Dates without an offset are taken as UTC.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.timestamp_millis());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(dt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(dt.and_utc().timestamp_millis());
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis())
}

fn apply_frontmatter(yaml: &str, parsed: &mut Parsed) {
    let Some(docs) = load_frontmatter(yaml) else {
        return;
    };
    let Some(Yaml::Hash(map)) = docs.into_iter().next() else {
        return;
    };
    for (k, v) in map {
        let Some(key) = scalar(&k) else { continue };
        let key = key.to_lowercase();
        let values = values(&v);
        match key.as_str() {
            "tags" | "tag" => {
                for tag in values.iter().flat_map(|v| v.split([',', ' '])) {
                    let tag = tag.trim().trim_start_matches('#');
                    if !tag.is_empty() {
                        push_unique(&mut parsed.frontmatter_tags, tag);
                    }
                    if is_tag(tag) {
                        push_unique(&mut parsed.tags, tag);
                    }
                }
            }
            "author" => parsed.author = values.into_iter().next(),
            "topic" => parsed.topic = values.into_iter().next(),
            "rating" => match rating(&v) {
                Some(r) => parsed.rating = Some(r),
                None => parsed.attributes.extend(values.into_iter().map(|v| (key.clone(), v))),
            },
            "created" => parsed.created_at = values.first().and_then(|s| parse_date(s)).or(parsed.created_at),
            "date" => {
                if parsed.created_at.is_none() {
                    parsed.created_at = values.first().and_then(|s| parse_date(s));
                }
            }
            _ => parsed.attributes.extend(values.into_iter().map(|v| (key.clone(), v))),
        }
    }
}

/// A rating is a single whole number from 1 to 5 (`4` or `"4"`); `4.5`, `five`, `0`,
/// `6` or a list are not.
fn rating(y: &Yaml) -> Option<u8> {
    match y {
        Yaml::Integer(i) => u8::try_from(*i).ok(),
        Yaml::String(s) if s.trim().chars().all(|c| c.is_ascii_digit()) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|r| (1..=5).contains(r))
}

fn scalar(y: &Yaml) -> Option<String> {
    let s = match y {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(i) => i.to_string(),
        Yaml::Boolean(b) => b.to_string(),
        _ => return None,
    };
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

fn values(y: &Yaml) -> Vec<String> {
    match y {
        Yaml::Array(items) => items.iter().filter_map(scalar).collect(),
        other => scalar(other).into_iter().collect(),
    }
}

/// Byte ranges of fenced code blocks and inline code spans, where tags and links don't count.
fn code_ranges(body: &str, from: usize) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut fence: Option<(usize, &str)> = None;
    let mut pos = from;
    for line in body[from..].split_inclusive('\n') {
        let t = line.trim_start();
        let marker = if line.len() - t.len() > 3 {
            None
        } else if t.starts_with("```") {
            Some("```")
        } else if t.starts_with("~~~") {
            Some("~~~")
        } else {
            None
        };
        match (fence, marker) {
            (None, Some(m)) => fence = Some((pos, m)),
            (Some((start, open)), Some(m)) if open == m => {
                ranges.push(start..pos + line.len());
                fence = None;
            }
            _ => {}
        }
        pos += line.len();
    }
    if let Some((start, _)) = fence {
        ranges.push(start..body.len());
    }
    ranges.extend(INLINE_CODE.find_iter(&body[from..]).map(|m| from + m.start()..from + m.end()));
    ranges
}

/// Byte ranges of `content` where a `#word` is never a tag, sorted and merged: `code`
/// (fenced and inline), indented code blocks, bare URLs and HTML tags.
fn not_tag_ranges(content: &str, code: impl Iterator<Item = Range<usize>>) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = code.collect();
    ranges.extend(indented_code(content));
    ranges.extend(URL.find_iter(content).map(|m| m.range()));
    // Most notes have no HTML: skip the scan.
    if content.contains('<') {
        ranges.extend(HTML.find_iter(content).map(|m| m.range()));
    }
    merge_ranges(ranges)
}

/// Sorts `ranges` and merges overlapping ones, for [`in_ranges`].
fn merge_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for r in ranges {
        match merged.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => merged.push(r),
        }
    }
    merged
}

fn in_ranges(sorted: &[Range<usize>], pos: usize) -> bool {
    let i = sorted.partition_point(|r| r.end <= pos);
    sorted.get(i).is_some_and(|r| r.contains(&pos))
}

/// Indented code blocks: lines indented by a tab or four spaces that follow a blank
/// line (or another such line). An indented list item, or a line inside a list or a
/// paragraph, isn't code.
fn indented_code(content: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let (mut pos, mut after_blank, mut after_code, mut in_list) = (0, true, false, false);
    for line in content.split_inclusive('\n') {
        let text = line.trim_end_matches(['\r', '\n']);
        let start = pos;
        pos += line.len();
        let trimmed = text.trim_start();
        if trimmed.is_empty() {
            after_blank = true;
            continue;
        }
        let indented = text.starts_with('\t') || text.starts_with("    ");
        let list_item = is_list_item(trimmed);
        let code = indented && !list_item && !in_list && (after_blank || after_code);
        if code {
            ranges.push(start..pos);
        } else if list_item {
            in_list = true;
        } else if !indented {
            in_list = false;
        }
        after_code = code;
        after_blank = false;
    }
    ranges
}

/// `- x`, `* x`, `+ x`, `1. x`, `1) x`.
fn is_list_item(line: &str) -> bool {
    let rest = line.trim_start_matches(|c: char| c.is_ascii_digit());
    let marker = if rest.len() < line.len() { rest.strip_prefix(['.', ')']) } else { line.strip_prefix(['-', '*', '+']) };
    marker.is_some_and(|r| r.is_empty() || r.starts_with([' ', '\t']))
}

/// A hex colour in a CSS declaration (`color: #fff;`), followed by `;` or `}`.
fn is_css_value(content: &str, tag: &str, end: usize) -> bool {
    HEX_COLOUR.is_match(tag) && content[end..].trim_start_matches([' ', '\t']).starts_with([';', '}'])
}

/// Tags are case-sensitive: `jr` and `Jr` are both kept.
fn push_unique(list: &mut Vec<String>, value: &str) {
    if !list.iter().any(|v| v == value) {
        list.push(value.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_fields_and_attributes() {
        let p = parse(
            "---\nauthor: Ada\ntopic: Journal\ntags: [jr, Dream]\ncreated: 2019-03-04\nemail: ada@example.com\nmood: [calm, tired]\n---\nBody #jr",
        );
        assert_eq!(p.author.as_deref(), Some("Ada"));
        assert_eq!(p.topic.as_deref(), Some("Journal"));
        assert_eq!(p.tags, vec!["jr", "Dream"]);
        assert_eq!(p.created_at, parse_date("2019-03-04"));
        assert_eq!(
            p.attributes,
            vec![
                ("email".into(), "ada@example.com".into()),
                ("mood".into(), "calm".into()),
                ("mood".into(), "tired".into())
            ]
        );
    }

    #[test]
    fn rating_is_a_whole_number_from_1_to_5() {
        for (yaml, want) in [("4", Some(4)), ("1", Some(1)), ("5", Some(5)), ("\"3\"", Some(3)), (" 2 ", Some(2))] {
            let p = parse(&format!("---\nrating: {yaml}\n---\n"));
            assert_eq!(p.rating, want, "{yaml}");
            assert!(p.attributes.is_empty(), "{yaml}: a valid rating isn't also an attribute");
        }
        for (yaml, kept) in [("4.5", vec!["4.5"]), ("five", vec!["five"]), ("0", vec!["0"]), ("6", vec!["6"]), ("-3", vec!["-3"]), ("[4]", vec!["4"]), ("[4, 5]", vec!["4", "5"])] {
            let p = parse(&format!("---\nrating: {yaml}\n---\n"));
            assert_eq!(p.rating, None, "{yaml}");
            let attrs: Vec<&str> = p.attributes.iter().filter(|(k, _)| k == "rating").map(|(_, v)| v.as_str()).collect();
            assert_eq!(attrs, kept, "{yaml}: kept as an attribute");
        }
        assert_eq!(parse("---\nRating: 3\n---\n").rating, Some(3), "keys are case-insensitive");
        assert_eq!(parse("no frontmatter").rating, None);
    }

    #[test]
    fn comma_separated_tag_string() {
        assert_eq!(parse("---\ntags: \"aa, #bb\"\n---\n").tags, vec!["aa", "bb"]);
    }

    #[test]
    fn broken_frontmatter_is_ignored() {
        let p = parse("---\nauthor: [unclosed\n---\n#ok");
        assert_eq!(p.author, None);
        assert_eq!(p.tags, vec!["ok"]);
    }

    /// #150: `levels` of anchors, each a list of ten aliases to the previous one.
    pub(crate) fn alias_bomb(levels: usize) -> String {
        let mut yaml = "a0: &a0 [x,x,x,x,x,x,x,x,x,x]\n".to_string();
        for i in 1..levels {
            let refs = vec![format!("*a{}", i - 1); 10].join(",");
            yaml += &format!("a{i}: &a{i} [{refs}]\n");
        }
        yaml
    }

    #[test]
    fn aliases_make_frontmatter_invalid_without_expanding_them() {
        // Checked first, so a regression fails here instead of hanging on the bomb.
        // A single harmless alias is refused too: Hush only reads flat keys.
        assert!(!frontmatter_is_valid("a: &x 1\nb: *x\n"));
        assert_eq!(parse("---\nauthor: Ada\nb: &x 1\nc: *x\n---\n").author, None);
        let yaml = alias_bomb(9);
        let body = format!("---\nauthor: Ada\n{yaml}---\nBody #ok [[Link]]");
        let start = std::time::Instant::now();
        let p = parse(&body);
        assert!(!frontmatter_is_valid(&yaml));
        assert!(start.elapsed() < std::time::Duration::from_millis(100), "{:?}", start.elapsed());
        assert_eq!(p.author, None, "frontmatter with aliases isn't read");
        assert!(p.attributes.is_empty());
        assert_eq!(p.tags, vec!["ok"], "the body is still parsed");
        assert_eq!(p.links.len(), 1);
        // Anchors without aliases are harmless and still read.
        assert_eq!(parse("---\nauthor: &x Ada\n---\n").author.as_deref(), Some("Ada"));
    }

    #[test]
    fn oversized_frontmatter_is_not_read() {
        let filler = "x".repeat(MAX_FRONTMATTER_BYTES);
        let yaml = format!("author: Ada\nnote: {filler}\n");
        assert!(!frontmatter_is_valid(&yaml));
        assert_eq!(parse(&format!("---\n{yaml}---\n#ok")).author, None);
        assert_eq!(parse(&format!("---\n{yaml}---\n#ok")).tags, vec!["ok"]);
        let fits = format!("author: Ada\nnote: {}\n", "x".repeat(MAX_FRONTMATTER_BYTES - 100));
        assert!(frontmatter_is_valid(&fits));
        assert_eq!(parse(&format!("---\n{fits}---\n")).author.as_deref(), Some("Ada"));
    }

    #[test]
    fn inline_tags() {
        let p = parse("# Heading\n#one and #two/sub, (#three) #123 a#notatag http://x/#anchor #One");
        assert_eq!(p.tags, vec!["one", "two/sub", "three", "One"], "tags are case-sensitive");
    }

    fn tags(body: &str) -> Vec<String> {
        parse(body).tags
    }

    #[test]
    fn anchor_links_are_not_tags() {
        assert!(tags("[A](#a) | [Z](#z)").is_empty());
        assert!(tags("[Intro](#introduction) and [Next](#next-steps)").is_empty(), "longer anchors too");
    }

    #[test]
    fn css_colours_are_not_tags() {
        assert!(tags("color: #fff; border: #1e1e1e").is_empty());
        assert!(tags("#a0a0a0 and #1E1E1E and #f00 and #12ab").is_empty(), "hex colours with a digit");
        assert!(tags("<p style=\"color: #cafe\">x</p>").is_empty(), "inside an HTML tag");
    }

    #[test]
    fn indented_code_is_skipped() {
        assert_eq!(tags("Some C:\n\n    #include <stdio.h>\n    #define X 1\n\n#real"), ["real"]);
        assert!(tags("\tint x; #notatag\n").is_empty(), "a tab counts as indentation");
        assert_eq!(tags("- item #one\n    - sub #two\n\t- tab #three"), ["one", "two", "three"], "nested list items are not code");
        assert_eq!(tags("A paragraph\n    still the paragraph #kept"), ["kept"], "indentation can't interrupt a paragraph");
    }

    #[test]
    fn numbers_and_ordinals_are_not_tags() {
        assert!(tags("#1-3, #12/13, #1st #2nd #3rd #4th #1").is_empty());
        assert_eq!(tags("#2024-review #3d #v2"), ["2024-review", "3d", "v2"], "a letter beyond an ordinal suffix is enough");
    }

    #[test]
    fn single_characters_are_not_tags() {
        assert_eq!(tags("#a #Z #é #ok"), ["ok"]);
    }

    #[test]
    fn urls_and_html_attributes_are_skipped() {
        assert!(tags("see https://example.com/a(#frag) and www.example.com/x,#part").is_empty());
        assert!(tags("<a href=\"#top\" title=\"x #note\">up</a>").is_empty());
        assert_eq!(tags("<b>bold</b> #after"), ["after"], "text between HTML tags still counts");
    }

    #[test]
    fn real_tags_are_kept() {
        assert_eq!(
            tags("#work and #two/sub, (#three) #café #cafe #bad #fff"),
            ["work", "two/sub", "three", "café", "cafe", "bad", "fff"],
            "all-letter hex words are words"
        );
    }

    #[test]
    fn frontmatter_tags_follow_the_same_rules() {
        let p = parse("---\ntags: [1, a, ok, 1e1e1e, 2nd]\n---\n");
        assert_eq!(p.tags, ["ok"]);
        assert_eq!(p.frontmatter_tags, ["1", "a", "ok", "1e1e1e", "2nd"], "kept as written, so edits don't drop them");
        assert!(tags("---\ntag: \"#1, #b\"\n---\n").is_empty());
    }

    /// A callout marker (#184) is neither a link nor a tag; what is written in the callout still counts.
    #[test]
    fn callout_markers_are_not_links_or_tags() {
        for body in ["> [!warning] Check the passphrase\n> There is no recovery.", "> [!note]-\n> > [!tip]+ Inner", "> [!my-type]"] {
            let p = parse(body);
            assert!(p.links.is_empty(), "{body}");
            assert!(p.tags.is_empty(), "{body}");
        }
        let p = parse("> [!note] See [[Other]] #topic\n> and [[Second|alias]].");
        assert_eq!(p.links.iter().map(|l| l.title.as_str()).collect::<Vec<_>>(), ["Other", "Second"]);
        assert_eq!(p.tags, ["topic"]);
    }

    #[test]
    fn is_tag_rules() {
        for good in ["ok", "work", "two/sub", "café", "cafe", "bad", "2024-review", "x1", "Ab"] {
            assert!(is_tag(good), "{good}");
        }
        for bad in ["", "a", "1", "123", "1-3", "12/13", "1st", "22ND", "1e1e1e", "a0a0a0", "f00", "12ab", "--", "_/"] {
            assert!(!is_tag(bad), "{bad}");
        }
    }

    #[test]
    fn code_is_skipped() {
        let p = parse("```\n#nottag [[NotLink]]\n```\n`#nor [[this]]` #yes [[Yes]]");
        assert_eq!(p.tags, vec!["yes"]);
        assert_eq!(p.links.len(), 1);
        assert_eq!(p.links[0].title, "Yes");
    }

    #[test]
    fn wikilink_forms() {
        let body = "[[Plain]] [[a/b/Deep.md#Sec|Alias]] ![[embed.png]] [[ Spaced ]] [[#self]]";
        let p = parse(body);
        let got: Vec<_> = p
            .links
            .iter()
            .map(|l| (l.path.as_str(), l.title.as_str(), l.heading.as_deref(), l.alias.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                ("", "Plain", None, None),
                ("a/b", "Deep", Some("Sec"), Some("Alias")),
                ("", "Spaced", None, None)
            ]
        );
        assert_eq!(&body[p.links[1].target_range.clone()], "a/b/Deep.md");
        assert_eq!(&body[p.links[2].target_range.clone()], "Spaced");
    }

    #[test]
    fn link_ranges_account_for_frontmatter() {
        let body = "---\na: 1\n---\nsee [[X]]";
        let p = parse(body);
        assert_eq!(&body[p.links[0].target_range.clone()], "X");
    }

    /// #151: each link was checked against every code span, so 160,000 links next to
    /// 160,000 spans (1.6 MB) took 17 s in a release build (370 s in a debug build).
    /// The bound is for a debug build on a slow shared machine, where the fixed parse
    /// takes under 1 s.
    #[test]
    fn many_links_and_code_spans_parse_quickly() {
        let n = 160_000;
        let body = "`a` [[b]]\n".repeat(n);
        let start = std::time::Instant::now();
        let p = parse(&body);
        let elapsed = start.elapsed();
        assert_eq!(p.links.len(), n);
        assert!(p.links.iter().enumerate().all(|(i, l)| l.target_range == (i * 10 + 6..i * 10 + 7) && l.title == "b"));
        assert!(elapsed < std::time::Duration::from_secs(10), "{elapsed:?}");
    }

    #[test]
    fn links_in_nested_code_ranges_are_skipped() {
        // Inline spans inside a fence overlap it; links after both still count.
        let body = "```\n`x` [[In]] `y`\n```\n`z` [[Out]] `w` [[Out2]]\n~~~\n[[Tail]]";
        let titles: Vec<String> = parse(body).links.into_iter().map(|l| l.title).collect();
        assert_eq!(titles, vec!["Out", "Out2"]);
    }

    #[test]
    fn many_distinct_tags_parse_quickly() {
        let n = 100_000;
        let body: String = (0..n).map(|i| format!("#tag{i} ")).collect();
        let start = std::time::Instant::now();
        let p = parse(&body);
        let elapsed = start.elapsed();
        assert_eq!(p.tags.len(), n);
        assert_eq!(p.tags[n - 1], format!("tag{}", n - 1));
        assert_eq!(parse("#bb #aa #bb #AA").tags, vec!["bb", "aa", "AA"], "first-seen order, case-sensitive");
        assert!(elapsed < std::time::Duration::from_secs(10), "{elapsed:?}");
    }

    fn set(body: &str, key: &str, values: &[&str]) -> String {
        let values: Vec<String> = values.iter().map(|v| v.to_string()).collect();
        set_frontmatter_field(body, key, &values)
    }

    #[test]
    fn set_field_creates_frontmatter() {
        assert_eq!(set("Body", "author", &["Ada"]), "---\nauthor: Ada\n---\nBody");
        assert_eq!(set("Body", "author", &[]), "Body");
        assert_eq!(set("a\r\nb", "topic", &["T"]), "---\r\ntopic: T\r\n---\r\na\r\nb");
    }

    #[test]
    fn set_field_replaces_only_its_own_lines() {
        let body = "---\n# comment\nTags:\n  - a\n  - b\nauthor: Ada\n---\nBody #x";
        assert_eq!(
            set(body, "tags", &["a", "c"]),
            "---\n# comment\nTags: [a, c]\nauthor: Ada\n---\nBody #x"
        );
        assert_eq!(set(body, "author", &["Bea"]), "---\n# comment\nTags:\n  - a\n  - b\nauthor: Bea\n---\nBody #x");
        assert_eq!(set(body, "mood", &["calm"]), "---\n# comment\nTags:\n  - a\n  - b\nauthor: Ada\nmood: calm\n---\nBody #x");
    }

    #[test]
    fn set_field_removes_key_and_empty_frontmatter() {
        assert_eq!(set("---\nauthor: Ada\ntopic: T\n---\nBody", "author", &[]), "---\ntopic: T\n---\nBody");
        assert_eq!(set("---\nauthor: Ada\n---\nBody", "author", &[]), "Body");
    }

    #[test]
    fn set_field_values_round_trip() {
        let tricky = ["a: b", "#notcomment", "x, y", "say \"hi\"", "null", "2019", "back\\slash", "[x]"];
        for value in tricky {
            let body = set("", "note", &[value]);
            assert_eq!(parse(&body).attributes, vec![("note".to_string(), value.to_string())], "{body}");
        }
        let body = set("", "tags", &["a b", "c,d"]);
        assert_eq!(parse(&body).frontmatter_tags, vec!["a", "b", "c", "d"]);
        let body = set("", "tags", &["one"]);
        assert_eq!(body, "---\ntags: [one]\n---\n");
    }

    #[test]
    fn dates() {
        assert_eq!(parse_date("1970-01-02"), Some(86_400_000));
        assert_eq!(parse_date("1970-01-01 00:01"), Some(60_000));
        assert_eq!(parse_date("1970-01-01T01:00:00+01:00"), Some(0));
        assert_eq!(parse_date("yesterday"), None);
    }

    #[test]
    fn folders() {
        assert_eq!(normalize_folder("/a\\b//c/"), "a/b/c");
        assert_eq!(split_target("x/y/T.md"), ("x/y".into(), "T".into()));
    }
}
