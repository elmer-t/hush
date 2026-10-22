//! Pure parsing of a note body: YAML frontmatter, inline `#tags` and `[[wikilinks]]`.
//!
//! The body is the single source of truth; everything the database indexes about a
//! note is derived from what this module returns.

use std::ops::Range;
use std::sync::LazyLock;

use chrono::{DateTime, NaiveDate, NaiveDateTime};
use regex::Regex;
use yaml_rust2::{Yaml, YamlLoader};

static TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[\s(,;])#([\p{L}\p{N}_\-/]+)").unwrap());
static WIKILINK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(!?)\[\[([^\[\]\n]+)\]\]").unwrap());
static INLINE_CODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`[^`\n]+`").unwrap());

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parsed {
    pub author: Option<String>,
    pub topic: Option<String>,
    /// Unix millis from frontmatter `created` (or `date`).
    pub created_at: Option<i64>,
    pub tags: Vec<String>,
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
    let code = code_ranges(body, content_start);
    let in_code = |pos: usize| code.iter().any(|r| r.contains(&pos));
    let content = &body[content_start..];

    for caps in TAG.captures_iter(content) {
        let m = caps.get(1).unwrap();
        let tag = m.as_str().trim_end_matches('/');
        // "#123" is an issue number, not a tag.
        if in_code(content_start + m.start()) || tag.is_empty() || tag.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        push_unique(&mut parsed.tags, tag);
    }

    for caps in WIKILINK.captures_iter(content) {
        let whole = caps.get(0).unwrap();
        // Embeds (`![[...]]`) are out of scope with attachments.
        if in_code(content_start + whole.start()) || !caps[1].is_empty() {
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
    let Ok(docs) = YamlLoader::load_from_str(yaml) else {
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
                        push_unique(&mut parsed.tags, tag);
                    }
                }
            }
            "author" => parsed.author = values.into_iter().next(),
            "topic" => parsed.topic = values.into_iter().next(),
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

fn push_unique(list: &mut Vec<String>, value: &str) {
    let lower = value.to_lowercase();
    if !list.iter().any(|v| v.to_lowercase() == lower) {
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
    fn comma_separated_tag_string() {
        assert_eq!(parse("---\ntags: \"a, #b\"\n---\n").tags, vec!["a", "b"]);
    }

    #[test]
    fn broken_frontmatter_is_ignored() {
        let p = parse("---\nauthor: [unclosed\n---\n#ok");
        assert_eq!(p.author, None);
        assert_eq!(p.tags, vec!["ok"]);
    }

    #[test]
    fn inline_tags() {
        let p = parse("# Heading\n#one and #two/sub, (#three) #123 a#notatag http://x/#anchor #One");
        assert_eq!(p.tags, vec!["one", "two/sub", "three"]);
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
