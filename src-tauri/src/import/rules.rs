//! User-configurable label/attribute rules, run on each file during import.
//!
//! Every rule matches against the original text of its source. Single-valued targets
//! (author, email, topic, date) take the first match of the first rule that matches;
//! tags and attributes collect every match. Stripping happens after all rules ran.

use std::ops::Range;

use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};

use super::dates::parse_loose_date;

/// Compiled patterns are capped so a pathological rule can't eat memory.
const REGEX_SIZE_LIMIT: usize = 1 << 20;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rule {
    pub name: String,
    pub enabled: bool,
    pub source: Source,
    /// How many lines [`Source::FirstLines`] looks at.
    pub lines: u32,
    pub pattern: String,
    pub target: Target,
    /// Value mapping, matched case-insensitively, e.g. `jr → journal`.
    pub map: Vec<(String, String)>,
    /// Removes the matched text from the body (or, for the file name, from the title).
    pub strip: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// The file name without extension.
    FileName,
    /// The raw YAML frontmatter.
    Frontmatter,
    /// The first [`Rule::lines`] lines after the frontmatter.
    FirstLines,
    /// Everything after the frontmatter.
    Body,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileName => "fileName",
            Self::Frontmatter => "frontmatter",
            Self::FirstLines => "firstLines",
            Self::Body => "body",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        [Self::FileName, Self::Frontmatter, Self::FirstLines, Self::Body].into_iter().find(|v| v.as_str() == s)
    }
}

/// Serialized as `tag`, `author`, `author_email`, `topic`, `created_at` or `attribute:<key>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum Target {
    Tag,
    Author,
    AuthorEmail,
    Topic,
    CreatedAt,
    Attribute(String),
}

pub const AUTHOR_EMAIL_KEY: &str = "author_email";

impl TryFrom<String> for Target {
    type Error = String;

    fn try_from(s: String) -> Result<Self, String> {
        Ok(match s.as_str() {
            "tag" => Self::Tag,
            "author" => Self::Author,
            "author_email" => Self::AuthorEmail,
            "topic" => Self::Topic,
            "created_at" => Self::CreatedAt,
            _ => match s.strip_prefix("attribute:") {
                Some(key) => Self::Attribute(key.to_string()),
                None => return Err(format!("unknown rule target '{s}'")),
            },
        })
    }
}

impl From<Target> for String {
    fn from(t: Target) -> Self {
        match t {
            Target::Tag => "tag".into(),
            Target::Author => "author".into(),
            Target::AuthorEmail => AUTHOR_EMAIL_KEY.into(),
            Target::Topic => "topic".into(),
            Target::CreatedAt => "created_at".into(),
            Target::Attribute(key) => format!("attribute:{key}"),
        }
    }
}

#[derive(Debug)]
pub struct CompiledRule {
    rule: Rule,
    regex: Regex,
}

impl Rule {
    /// The rules a new vault starts with: the mail-style headers of the old online
    /// archive, and one label rule. Frontmatter needs no rule; it is read as is.
    pub fn defaults() -> Vec<Rule> {
        let rule = |name: &str, source, lines, pattern: &str, target, map: &[(&str, &str)]| Rule {
            name: name.into(),
            enabled: true,
            source,
            lines,
            pattern: pattern.into(),
            target,
            map: map.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
            strip: true,
        };
        vec![
            rule(
                "Author from From: line",
                Source::FirstLines,
                10,
                r"^From:[ \t]*([^<>@\n]+?)[ \t]*(?:<[^>\n]*>)?[ \t]*$",
                Target::Author,
                &[],
            ),
            rule(
                "Email from From: line",
                Source::FirstLines,
                10,
                r"^From:.*?<?([^\s<>]+@[^\s<>]+?)>?[ \t]*$",
                Target::AuthorEmail,
                &[],
            ),
            rule("Date from Date: line", Source::FirstLines, 10, r"^Date:[ \t]*(.+)$", Target::CreatedAt, &[]),
            rule(
                "Subject from Subject: line",
                Source::FirstLines,
                10,
                r"^Subject:[ \t]*(.+)$",
                Target::Attribute("subject".into()),
                &[],
            ),
            rule(
                "Label codes in file name",
                Source::FileName,
                0,
                r"(?i)\b(jr|dr)\b",
                Target::Tag,
                &[("jr", "journal"), ("dr", "dream")],
            ),
        ]
    }

    /// Checks the rule and compiles its pattern. Patterns are Rust `regex` syntax and
    /// run in linear time; `^`/`$` match at line ends.
    pub fn compile(&self) -> Result<CompiledRule, String> {
        let name = if self.name.trim().is_empty() { "unnamed rule" } else { self.name.trim() };
        let fail = |msg: String| Err(format!("rule '{name}': {msg}"));
        if self.pattern.is_empty() {
            return fail("the pattern is empty".into());
        }
        if self.source == Source::FirstLines && self.lines == 0 {
            return fail("the number of lines must be at least 1".into());
        }
        if let Target::Attribute(key) = &self.target
            && !valid_key(key)
        {
            return fail(format!("attribute name '{key}' may only contain letters, digits, - and _"));
        }
        match RegexBuilder::new(&self.pattern).multi_line(true).size_limit(REGEX_SIZE_LIMIT).build() {
            Ok(regex) => Ok(CompiledRule { rule: self.clone(), regex }),
            Err(e) => fail(format!("invalid pattern: {e}")),
        }
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// The texts rules look at.
pub struct RuleInput<'a> {
    pub file_name: &'a str,
    pub frontmatter: &'a str,
    pub content: &'a str,
}

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleOutput {
    pub tags: Vec<String>,
    pub author: Option<String>,
    pub author_email: Option<String>,
    pub topic: Option<String>,
    pub created_at: Option<i64>,
    /// In rule order; values of the same key are merged.
    pub attributes: Vec<(String, Vec<String>)>,
    /// The file name with stripped matches removed.
    pub file_name: String,
    /// The content with stripped matches removed.
    pub content: String,
    pub warnings: Vec<String>,
}

pub fn apply(rules: &[CompiledRule], input: &RuleInput) -> RuleOutput {
    let mut out = RuleOutput::default();
    let mut strip_name: Vec<Range<usize>> = Vec::new();
    let mut strip_content: Vec<Range<usize>> = Vec::new();

    for CompiledRule { rule, regex } in rules.iter().filter(|r| r.rule.enabled) {
        let text = match rule.source {
            Source::FileName => input.file_name,
            Source::Frontmatter => input.frontmatter,
            Source::FirstLines => first_lines(input.content, rule.lines as usize),
            Source::Body => input.content,
        };
        let multi = matches!(rule.target, Target::Tag | Target::Attribute(_));
        for caps in regex.captures_iter(text) {
            let whole = caps.get(0).unwrap();
            let raw = caps.get(1).unwrap_or(whole).as_str().trim();
            let value = map_value(&rule.map, raw);
            if value.is_empty() {
                continue;
            }
            let used = match &rule.target {
                Target::Tag => {
                    let tags = tags_of(&value);
                    if tags.is_empty() {
                        out.warnings.push(format!("rule '{}': '{value}' is not a valid tag", rule.name));
                    }
                    for tag in tags {
                        if !out.tags.contains(&tag) {
                            out.tags.push(tag);
                        }
                    }
                    true
                }
                Target::Author => set_once(&mut out.author, value),
                Target::AuthorEmail => set_once(&mut out.author_email, value),
                Target::Topic => set_once(&mut out.topic, value),
                Target::CreatedAt => match parse_loose_date(&value) {
                    Some(ms) => set_once(&mut out.created_at, ms),
                    None => {
                        out.warnings.push(format!("rule '{}': '{value}' is not a date", rule.name));
                        false
                    }
                },
                Target::Attribute(key) => {
                    let key = key.to_lowercase();
                    match out.attributes.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, values)) => push_unique(values, value),
                        None => out.attributes.push((key, vec![value])),
                    }
                    true
                }
            };
            if used && rule.strip {
                match rule.source {
                    Source::FileName => strip_name.push(whole.range()),
                    Source::FirstLines | Source::Body => strip_content.push(whole.range()),
                    Source::Frontmatter => {}
                }
            }
            if !multi {
                break;
            }
        }
    }

    out.file_name = squeeze(&remove_ranges(input.file_name, strip_name, false));
    out.content = remove_ranges(input.content, strip_content, true);
    out
}

/// The first `n` lines of `text`, including their line breaks.
fn first_lines(text: &str, n: usize) -> &str {
    let end = text.split_inclusive('\n').take(n).map(str::len).sum();
    &text[..end]
}

/// An exact match wins, so `jr` and `Jr` can map to different values; otherwise case is ignored.
fn map_value(map: &[(String, String)], value: &str) -> String {
    map.iter()
        .find(|(from, _)| from.trim() == value)
        .or_else(|| map.iter().find(|(from, _)| from.trim().to_lowercase() == value.to_lowercase()))
        .map_or(value, |(_, to)| to.trim())
        .to_string()
}

/// Splits a value like `"a, #b c"` into tags, replacing characters tags can't hold.
fn tags_of(value: &str) -> Vec<String> {
    value
        .split([',', ';', ' ', '\t'])
        .map(|t| {
            let t: String = t
                .trim()
                .trim_start_matches('#')
                .chars()
                .map(|c| if c.is_alphanumeric() || matches!(c, '_' | '-' | '/') { c } else { '-' })
                .collect();
            t.trim_matches(['-', '/']).to_string()
        })
        .filter(|t| !t.is_empty() && !t.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

fn set_once<T>(slot: &mut Option<T>, value: T) -> bool {
    if slot.is_some() {
        return false;
    }
    *slot = Some(value);
    true
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.iter().any(|v| v.to_lowercase() == value.to_lowercase()) {
        list.push(value);
    }
}

/// Removes `ranges` from `text`. With `whole_lines`, a match that is all there is on its
/// line takes the line (and its line break) with it.
fn remove_ranges(text: &str, mut ranges: Vec<Range<usize>>, whole_lines: bool) -> String {
    if whole_lines {
        for r in &mut ranges {
            let line_start = text[..r.start].rfind('\n').map_or(0, |i| i + 1);
            let line_end = text[r.end..].find('\n').map_or(text.len(), |i| r.end + i);
            if text[line_start..r.start].trim().is_empty() && text[r.end..line_end].trim().is_empty() {
                *r = line_start..(line_end + 1).min(text.len());
            }
        }
    }
    ranges.sort_by_key(|r| r.start);
    let mut out = String::with_capacity(text.len());
    let mut pos = 0;
    for r in ranges {
        if r.start > pos {
            out.push_str(&text[pos..r.start]);
        }
        pos = pos.max(r.end);
    }
    out.push_str(&text[pos..]);
    out
}

/// Collapses runs of whitespace and trims, for titles left over after stripping.
fn squeeze(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").trim_matches(['-', '_', ' ']).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(source: Source, pattern: &str, target: Target) -> Rule {
        Rule {
            name: "r".into(),
            enabled: true,
            source,
            lines: 2,
            pattern: pattern.into(),
            target,
            map: vec![],
            strip: false,
        }
    }

    fn run(rules: &[Rule], file_name: &str, frontmatter: &str, content: &str) -> RuleOutput {
        let compiled: Vec<_> = rules.iter().map(|r| r.compile().unwrap()).collect();
        apply(&compiled, &RuleInput { file_name, frontmatter, content })
    }

    #[test]
    fn defaults_compile_and_round_trip_through_json() {
        for r in Rule::defaults() {
            r.compile().unwrap();
            let json = serde_json::to_string(&r).unwrap();
            assert_eq!(serde_json::from_str::<Rule>(&json).unwrap(), r);
        }
    }

    #[test]
    fn invalid_rules_are_rejected() {
        assert!(rule(Source::Body, "(", Target::Tag).compile().unwrap_err().contains("invalid pattern"));
        assert!(rule(Source::Body, "", Target::Tag).compile().is_err());
        assert!(rule(Source::Body, "x", Target::Attribute("a b".into())).compile().is_err());
        assert!(Rule { lines: 0, ..rule(Source::FirstLines, "x", Target::Tag) }.compile().is_err());
        assert!(serde_json::from_str::<Target>("\"nope\"").is_err());
    }

    #[test]
    fn label_codes_map_to_tags_and_strip_from_the_file_name() {
        let r = Rule {
            map: vec![("JR".into(), "journal".into())],
            strip: true,
            ..rule(Source::FileName, r"(?i)\b(jr|dr)\b", Target::Tag)
        };
        let out = run(&[r], "2004-03-12 jr Walk dr", "", "");
        assert_eq!(out.tags, ["journal", "dr"]);
        assert_eq!(out.file_name, "2004-03-12 Walk");
    }

    #[test]
    fn map_prefers_an_exact_case_match() {
        let r = Rule {
            map: vec![("jr".into(), "journal".into()), ("Jr".into(), "junior".into()), ("jR".into(), "jury".into())],
            ..rule(Source::Body, r"\b(jr|Jr|jR)\b", Target::Tag)
        };
        assert_eq!(run(&[r], "", "", "jR Jr jr JR").tags, ["jury", "junior", "journal"]);
    }

    #[test]
    fn first_lines_only_sees_its_lines_and_strips_whole_lines() {
        let r = Rule { strip: true, ..rule(Source::FirstLines, r"^Author: (.+)$", Target::Author) };
        let out = run(std::slice::from_ref(&r), "", "", "Author: Ada\nAuthor: Bob\nbody");
        assert_eq!(out.author.as_deref(), Some("Ada"));
        assert_eq!(out.content, "Author: Bob\nbody", "single-valued targets use the first match only");
        let out = run(&[r], "", "", "a\nb\nAuthor: Late");
        assert_eq!(out.author, None);
    }

    #[test]
    fn strip_inside_a_line_keeps_the_rest() {
        let r = Rule { strip: true, ..rule(Source::Body, r"\[mood: (\w+)\]", Target::Attribute("Mood".into())) };
        let out = run(&[r], "", "", "Fine day [mood: calm] and [mood: tired]\n[mood: calm]\nend");
        assert_eq!(out.attributes, [("mood".to_string(), vec!["calm".to_string(), "tired".to_string()])]);
        assert_eq!(out.content, "Fine day  and \nend");
    }

    #[test]
    fn dates_and_bad_values_warn() {
        let date = rule(Source::Frontmatter, r"(?m)^when: (.+)$", Target::CreatedAt);
        let out = run(std::slice::from_ref(&date), "", "when: yesterday\nwhen: 2004-03-12", "");
        assert_eq!(out.created_at, None, "the first match is used even when it fails");
        assert_eq!(out.warnings, ["rule 'r': 'yesterday' is not a date"]);
        let out = run(&[date], "", "when: Fri, 12 Mar 2004 07:45:00 +0100", "");
        assert_eq!(out.created_at, Some(1_079_073_900_000));
        let tag = rule(Source::Body, r"tags: (.+)", Target::Tag);
        assert_eq!(run(std::slice::from_ref(&tag), "", "", "tags: a, #b c/d").tags, ["a", "b", "c/d"]);
        assert_eq!(run(&[tag], "", "", "tags: !!").warnings, ["rule 'r': '!!' is not a valid tag"]);
    }

    #[test]
    fn earlier_rules_win_and_disabled_rules_are_skipped() {
        let a = rule(Source::Body, r"A: (\w+)", Target::Topic);
        let b = rule(Source::Body, r"B: (\w+)", Target::Topic);
        assert_eq!(run(&[a.clone(), b.clone()], "", "", "B: two A: one").topic.as_deref(), Some("one"));
        let a = Rule { enabled: false, ..a };
        assert_eq!(run(&[a, b], "", "", "B: two A: one").topic.as_deref(), Some("two"));
    }

    #[test]
    fn mail_header_defaults() {
        let rules = Rule::defaults();
        let out = run(
            &rules,
            "2004-03-12 jr Walk",
            "",
            "From: Jan de Vries <jan@example.org>\nDate: 2004-03-12\nSubject: Walk\n\nBody\n",
        );
        assert_eq!(out.author.as_deref(), Some("Jan de Vries"));
        assert_eq!(out.author_email.as_deref(), Some("jan@example.org"));
        assert_eq!(out.attributes, [("subject".to_string(), vec!["Walk".to_string()])]);
        assert_eq!(out.tags, ["journal"]);
        assert_eq!(out.file_name, "2004-03-12 Walk");
        assert_eq!(out.content, "\nBody\n");
        let bare = run(&rules, "x", "", "From: jan@example.org\n");
        assert_eq!((bare.author, bare.author_email.as_deref()), (None, Some("jan@example.org")));
    }
}
