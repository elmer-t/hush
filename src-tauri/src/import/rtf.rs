//! Best-effort RTF to plain text: paragraphs, line breaks, special characters and
//! Unicode survive; formatting, tables of fonts and colours, pictures and other
//! embedded data are dropped.

use std::iter::Peekable;
use std::str::Chars;

use super::formats::cp1252;

/// Groups starting with these control words hold no readable text.
const SKIPPED_DESTINATIONS: &[&str] = &[
    "fonttbl", "colortbl", "stylesheet", "info", "pict", "object", "header", "headerl", "headerr", "headerf",
    "footer", "footerl", "footerr", "footerf", "fldinst", "themedata", "colorschememapping", "latentstyles",
    "datastore", "xmlnstbl", "listtable", "listoverridetable", "rsidtbl", "generator", "mmathpr", "pgdsctbl",
    "filetbl", "revtbl", "bkmkstart", "bkmkend", "operator", "author", "company",
];

#[derive(Clone, Copy)]
struct Group {
    skip: bool,
    /// How many fallback characters follow a `\u` escape.
    uc: usize,
}

/// The text a control word stands for, if it stands for any.
fn word_text(word: &str) -> Option<&'static str> {
    match word {
        "tab" | "cell" => Some(" "),
        "emdash" => Some("—"),
        "endash" => Some("–"),
        "lquote" => Some("‘"),
        "rquote" => Some("’"),
        "ldblquote" => Some("“"),
        "rdblquote" => Some("”"),
        "bullet" => Some("•"),
        _ => None,
    }
}

struct Parser<'a> {
    chars: Peekable<Chars<'a>>,
    out: String,
    stack: Vec<Group>,
    group: Group,
    /// Fallback characters still to skip after a `\u` escape.
    skip_chars: usize,
    /// Whether the current group has seen anything yet (destinations come first).
    group_start: bool,
}

impl<'a> Parser<'a> {
    fn new(rtf: &'a str) -> Self {
        Self {
            chars: rtf.chars().peekable(),
            out: String::with_capacity(rtf.len() / 2),
            stack: Vec::new(),
            group: Group { skip: false, uc: 1 },
            skip_chars: 0,
            group_start: false,
        }
    }

    fn run(mut self) -> String {
        while let Some(c) = self.chars.next() {
            match c {
                '{' => self.open_group(),
                '}' => self.close_group(),
                '\\' => {
                    if !self.control() {
                        break;
                    }
                }
                // Line breaks in RTF source are not content.
                '\r' | '\n' => {}
                c => self.text(c),
            }
        }
        self.out
    }

    /// Appends visible text, unless it is a fallback character after a `\u` escape.
    fn emit(&mut self, s: &str) {
        if self.skip_chars > 0 {
            self.skip_chars -= 1;
        } else {
            self.emit_always(s);
        }
    }

    /// Appends visible text that never counts as a fallback character.
    fn emit_always(&mut self, s: &str) {
        if !self.group.skip {
            self.out.push_str(s);
        }
    }

    /// Whether this is the first thing in its group; either way the group has started.
    fn take_group_start(&mut self) -> bool {
        std::mem::replace(&mut self.group_start, false)
    }

    fn open_group(&mut self) {
        self.stack.push(self.group);
        self.group_start = true;
        self.skip_chars = 0;
    }

    fn close_group(&mut self) {
        self.group = self.stack.pop().unwrap_or(self.group);
        self.group_start = false;
        self.skip_chars = 0;
    }

    fn text(&mut self, c: char) {
        self.group_start = false;
        self.emit(c.encode_utf8(&mut [0; 4]));
    }

    /// Handles what follows a backslash. `false` when the input ends there.
    fn control(&mut self) -> bool {
        let Some(&next) = self.chars.peek() else { return false };
        if next.is_ascii_alphabetic() {
            self.control_word();
        } else {
            self.chars.next();
            self.control_symbol(next);
        }
        true
    }

    fn take_while(&mut self, keep: impl Fn(char) -> bool) -> String {
        let mut s = String::new();
        while let Some(c) = self.chars.next_if(|&c| keep(c)) {
            s.push(c);
        }
        s
    }

    fn control_word(&mut self) {
        let word = self.take_while(|c| c.is_ascii_alphabetic());
        let mut param = String::new();
        if self.chars.next_if_eq(&'-').is_some() {
            param.push('-');
        }
        param.push_str(&self.take_while(|c| c.is_ascii_digit()));
        self.chars.next_if_eq(&' ');
        let first = self.take_group_start();
        if first && SKIPPED_DESTINATIONS.contains(&word.to_lowercase().as_str()) {
            self.group.skip = true;
        } else {
            self.word_action(&word, param.parse().ok());
        }
    }

    fn word_action(&mut self, word: &str, n: Option<i32>) {
        match word {
            "par" | "sect" | "page" => self.emit_always("\n\n"),
            "line" => self.emit_always("  \n"),
            "row" => self.emit_always("\n"),
            "uc" => self.group.uc = n.unwrap_or(1).max(0) as usize,
            "u" => {
                if let Some(n) = n {
                    self.unicode(n);
                }
            }
            _ => {
                if let Some(s) = word_text(word) {
                    self.emit(s);
                }
            }
        }
    }

    /// `\u` takes a signed 16-bit value; the fallback characters after it are skipped.
    fn unicode(&mut self, n: i32) {
        let code = if n < 0 { n + 65536 } else { n } as u32;
        let ch = char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER);
        self.emit_always(ch.encode_utf8(&mut [0; 4]));
        self.skip_chars = self.group.uc;
    }

    fn control_symbol(&mut self, symbol: char) {
        let first = self.take_group_start();
        match symbol {
            '*' if first => self.group.skip = true,
            '\'' => self.hex_escape(),
            '~' => self.emit(" "),
            '_' => self.emit("-"),
            '\n' | '\r' => self.emit_always("\n\n"),
            '\\' | '{' | '}' => self.emit(symbol.encode_utf8(&mut [0; 4])),
            _ => {} // optional hyphen and other symbols
        }
    }

    fn hex_escape(&mut self) {
        let hex: String = self.chars.by_ref().take(2).collect();
        if let Ok(b) = u8::from_str_radix(&hex, 16) {
            self.emit(&cp1252(b).to_string());
        }
    }
}

/// Blank lines hold nothing, and no more than one in a row stays.
fn tidy_blank_lines(text: &str) -> String {
    let lines: Vec<&str> = text.split('\n').map(|l| if l.trim().is_empty() { "" } else { l }).collect();
    let mut text = lines.join("\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    text.trim().to_string()
}

/// `None` unless `rtf` starts like an RTF document.
pub fn to_text(rtf: &str) -> Option<String> {
    if !rtf.trim_start().starts_with("{\\rtf") {
        return None;
    }
    Some(tidy_blank_lines(&Parser::new(rtf).run()))
}

#[cfg(test)]
mod tests {
    use super::{tidy_blank_lines, to_text};

    #[test]
    fn paragraphs_and_characters() {
        let rtf = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss Arial;}}{\colortbl;\red0\green0\blue0;}
{\*\generator Riched20 10.0;}{\info{\author Oma}}\viewkind4\uc1\pard\f0\fs20 Dear Jan,\par
\par
The caf\'e9 is open \endash  and \b busy\b0 .\line Second line\par
It\rquote s \u8364?5 \{braces\}\par
}";
        assert_eq!(
            to_text(rtf).unwrap(),
            "Dear Jan,\n\nThe café is open – and busy.  \nSecond line\n\nIt’s €5 {braces}"
        );
    }

    #[test]
    fn unicode_fallbacks_are_skipped() {
        assert_eq!(to_text(r"{\rtf1\uc2 a\u252\'fc\'fcb}").unwrap(), "aüb");
        assert_eq!(to_text(r"{\rtf1 x{\*\unknowndest secret}y}").unwrap(), "xy");
    }

    #[test]
    fn not_rtf() {
        assert_eq!(to_text("plain"), None);
    }

    #[test]
    fn breaks_and_separators() {
        assert_eq!(to_text(r"{\rtf1 a\tab b\cell c\row d\page e\sect f}").unwrap(), "a b c\nd\n\ne\n\nf");
    }

    #[test]
    fn typographic_characters() {
        let rtf = r"{\rtf1 \lquote a\rquote \ldblquote b\rdblquote \bullet \emdash \endash }";
        assert_eq!(to_text(rtf).unwrap(), "‘a’“b”•—–");
    }

    #[test]
    fn control_symbols() {
        assert_eq!(to_text(r"{\rtf1 a\~b\_c\-d\*e}").unwrap(), "a b-cde");
        assert_eq!(to_text("{\\rtf1 a\\\nb}").unwrap(), "a\n\nb");
    }

    #[test]
    fn source_line_breaks_are_not_content() {
        assert_eq!(to_text("{\\rtf1 a\nb\r\nc}").unwrap(), "abc");
    }

    #[test]
    fn input_ending_after_a_backslash() {
        assert_eq!(to_text(r"{\rtf1 abc\").unwrap(), "abc");
    }

    #[test]
    fn skipped_destinations() {
        assert_eq!(to_text(r"{\rtf1 a{\FONTTBL hidden}b}").unwrap(), "ab");
        assert_eq!(to_text(r"{\rtf1 a{\*\foo {\bar nested} hidden}b}").unwrap(), "ab");
    }

    #[test]
    fn unicode_escapes() {
        assert_eq!(to_text(r"{\rtf1 a\u-28672?b}").unwrap(), "a\u{9000}b");
        assert_eq!(to_text(r"{\rtf1 a\u-10000?b}").unwrap(), "a\u{FFFD}b");
        assert_eq!(to_text(r"{\rtf1 a\u b}").unwrap(), "ab");
    }

    #[test]
    fn fallback_count_follows_uc() {
        assert_eq!(to_text(r"{\rtf1\uc0 \u252ü}").unwrap(), "üü");
        assert_eq!(to_text(r"{\rtf1\uc-1 \u252ü}").unwrap(), "üü");
        assert_eq!(to_text(r"{\rtf1\uc \u252?x}").unwrap(), "üx");
    }

    #[test]
    fn bad_hex_escape_is_dropped() {
        assert_eq!(to_text(r"{\rtf1 a\'zzb}").unwrap(), "ab");
    }

    #[test]
    fn unbalanced_closing_brace() {
        assert_eq!(to_text(r"{\rtf1 a}}b}").unwrap(), "ab");
    }

    #[test]
    fn blank_lines_collapse() {
        assert_eq!(to_text(r"{\rtf1 a\par \par \par \par b\par   \par c}").unwrap(), "a\n\nb\n\nc");
        assert_eq!(tidy_blank_lines("a\n  \n\n\nb\n"), "a\n\nb");
    }
}
