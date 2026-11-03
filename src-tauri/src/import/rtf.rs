//! Best-effort RTF to plain text: paragraphs, line breaks, special characters and
//! Unicode survive; formatting, tables of fonts and colours, pictures and other
//! embedded data are dropped.

use super::formats::cp1252;

/// Groups starting with these control words hold no readable text.
const SKIPPED_DESTINATIONS: &[&str] = &[
    "fonttbl",
    "colortbl",
    "stylesheet",
    "info",
    "pict",
    "object",
    "header",
    "headerl",
    "headerr",
    "headerf",
    "footer",
    "footerl",
    "footerr",
    "footerf",
    "fldinst",
    "themedata",
    "colorschememapping",
    "latentstyles",
    "datastore",
    "xmlnstbl",
    "listtable",
    "listoverridetable",
    "rsidtbl",
    "generator",
    "mmathpr",
    "pgdsctbl",
    "filetbl",
    "revtbl",
    "bkmkstart",
    "bkmkend",
    "operator",
    "author",
    "company",
];

#[derive(Clone, Copy)]
struct Group {
    skip: bool,
    /// How many fallback characters follow a `\u` escape.
    uc: usize,
}

/// Appends visible text, unless it is a fallback character after a `\u` escape.
fn emit(out: &mut String, group: &Group, skip_chars: &mut usize, s: &str) {
    if *skip_chars > 0 {
        *skip_chars -= 1;
    } else if !group.skip {
        out.push_str(s);
    }
}

/// `None` unless `rtf` starts like an RTF document.
pub fn to_text(rtf: &str) -> Option<String> {
    if !rtf.trim_start().starts_with("{\\rtf") {
        return None;
    }
    let mut out = String::with_capacity(rtf.len() / 2);
    let mut stack: Vec<Group> = Vec::new();
    let mut group = Group { skip: false, uc: 1 };
    // Fallback characters still to skip after a `\u` escape.
    let mut skip_chars = 0usize;
    // Whether the current group has seen anything yet (destinations come first).
    let mut group_start = false;
    let mut chars = rtf.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '{' => {
                stack.push(group);
                group_start = true;
                skip_chars = 0;
            }
            '}' => {
                group = stack.pop().unwrap_or(group);
                group_start = false;
                skip_chars = 0;
            }
            '\\' => {
                let Some(&next) = chars.peek() else { break };
                if next.is_ascii_alphabetic() {
                    let mut word = String::new();
                    while let Some(&ch) = chars.peek().filter(|c| c.is_ascii_alphabetic()) {
                        word.push(ch);
                        chars.next();
                    }
                    let mut param = String::new();
                    if chars.peek() == Some(&'-') {
                        param.push('-');
                        chars.next();
                    }
                    while let Some(&ch) = chars.peek().filter(|c| c.is_ascii_digit()) {
                        param.push(ch);
                        chars.next();
                    }
                    if chars.peek() == Some(&' ') {
                        chars.next();
                    }
                    let n: Option<i32> = param.parse().ok();
                    let first = std::mem::replace(&mut group_start, false);
                    if first && SKIPPED_DESTINATIONS.contains(&word.to_lowercase().as_str()) {
                        group.skip = true;
                        continue;
                    }
                    match word.as_str() {
                        "par" | "sect" | "page" => emit(&mut out, &group, &mut 0, "\n\n"),
                        "line" => emit(&mut out, &group, &mut 0, "  \n"),
                        "row" => emit(&mut out, &group, &mut 0, "\n"),
                        "tab" | "cell" => emit(&mut out, &group, &mut skip_chars, " "),
                        "emdash" => emit(&mut out, &group, &mut skip_chars, "—"),
                        "endash" => emit(&mut out, &group, &mut skip_chars, "–"),
                        "lquote" => emit(&mut out, &group, &mut skip_chars, "‘"),
                        "rquote" => emit(&mut out, &group, &mut skip_chars, "’"),
                        "ldblquote" => emit(&mut out, &group, &mut skip_chars, "“"),
                        "rdblquote" => emit(&mut out, &group, &mut skip_chars, "”"),
                        "bullet" => emit(&mut out, &group, &mut skip_chars, "•"),
                        "uc" => group.uc = n.unwrap_or(1).max(0) as usize,
                        "u" => {
                            if let Some(n) = n {
                                let code = if n < 0 { n + 65536 } else { n } as u32;
                                let ch =
                                    char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER);
                                emit(&mut out, &group, &mut 0, &ch.to_string());
                                skip_chars = group.uc;
                            }
                        }
                        _ => {}
                    }
                } else {
                    chars.next();
                    let first = std::mem::replace(&mut group_start, false);
                    match next {
                        '*' if first => group.skip = true,
                        '\'' => {
                            let hex: String = chars.by_ref().take(2).collect();
                            if let Ok(b) = u8::from_str_radix(&hex, 16) {
                                emit(&mut out, &group, &mut skip_chars, &cp1252(b).to_string());
                            }
                        }
                        '~' => emit(&mut out, &group, &mut skip_chars, " "),
                        '_' => emit(&mut out, &group, &mut skip_chars, "-"),
                        '\n' | '\r' => emit(&mut out, &group, &mut 0, "\n\n"),
                        '\\' | '{' | '}' => {
                            emit(&mut out, &group, &mut skip_chars, &next.to_string())
                        }
                        _ => {} // optional hyphen and other symbols
                    }
                }
            }
            // Line breaks in RTF source are not content.
            '\r' | '\n' => {}
            c => {
                group_start = false;
                emit(&mut out, &group, &mut skip_chars, &c.to_string());
            }
        }
    }

    let lines: Vec<&str> = out
        .split('\n')
        .map(|l| if l.trim().is_empty() { "" } else { l })
        .collect();
    let mut text = lines.join("\n");
    while text.contains("\n\n\n") {
        text = text.replace("\n\n\n", "\n\n");
    }
    Some(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::to_text;

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
}
