//! What kind of file an import candidate is, and how to turn its bytes into text.

use std::fs::File;
use std::io::Read;
use std::path::Path;

/// How a readable file is converted into a note body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Markdown, possibly with frontmatter and HTML fragments.
    Markdown,
    /// Plain text; cleaned like Markdown.
    Text,
    /// A whole HTML document.
    Html,
    Rtf,
    /// Data or code, kept verbatim in a fenced code block with this language.
    Code(&'static str),
}

const MARKDOWN: &[&str] = &["md", "markdown", "mdown", "mkd", "mdwn"];
const TEXT: &[&str] = &["txt", "text", "nfo", "me", "1st", "asc"];
const HTML: &[&str] = &["html", "htm", "xhtml", "shtml"];
const CODE: &[(&str, &str)] = &[
    ("csv", "csv"),
    ("tsv", "tsv"),
    ("json", "json"),
    ("xml", "xml"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("toml", "toml"),
    ("ini", "ini"),
    ("cfg", "ini"),
    ("conf", ""),
    ("log", ""),
    ("css", "css"),
    ("js", "js"),
    ("ts", "ts"),
    ("py", "python"),
    ("sh", "sh"),
    ("bat", "bat"),
    ("cmd", "bat"),
    ("ps1", "powershell"),
    ("sql", "sql"),
    ("c", "c"),
    ("h", "c"),
    ("cpp", "cpp"),
    ("cs", "csharp"),
    ("java", "java"),
    ("rb", "ruby"),
    ("php", "php"),
    ("rs", "rust"),
    ("go", "go"),
    ("pl", "perl"),
    ("vbs", "vb"),
    ("bas", "vb"),
    ("tex", "latex"),
];

/// Formats that are never text, so they are not even opened.
const BINARY: &[&str] = &[
    "pdf", "doc", "docx", "dot", "dotx", "docm", "odt", "ott", "wpd", "wps", "pages", "xls", "xlsx", "xlsm", "ods",
    "numbers", "ppt", "pptx", "odp", "key", "png", "jpg", "jpeg", "gif", "bmp", "tif", "tiff", "webp", "heic", "ico",
    "svg", "psd", "mp3", "wav", "ogg", "flac", "m4a", "wma", "mp4", "mov", "avi", "mkv", "wmv", "webm", "zip", "rar",
    "7z", "gz", "tgz", "bz2", "tar", "exe", "dll", "msi", "iso", "epub", "mobi", "azw", "db", "sqlite", "mdb", "accdb",
    "ttf", "otf", "woff", "woff2", "eot", "swf", "class", "jar", "pst", "msg",
];

/// File signatures of binary formats that may hide behind a text extension.
const MAGIC: &[&[u8]] = &[
    b"%PDF",
    b"PK\x03\x04",                       // zip: docx, odt, epub, …
    b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1", // OLE: doc, xls, msg, …
    b"\x89PNG",
    b"\xFF\xD8\xFF", // JPEG
    b"GIF8",
    b"MZ", // executables
];

/// How many bytes are looked at to tell text from binary.
const SNIFF_BYTES: usize = 8192;

/// The lowercased extension, or `""`.
pub fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Whether the extension is one people write notes in: Markdown, text, HTML or RTF.
pub fn is_document(path: &Path) -> bool {
    let ext = extension(path);
    [MARKDOWN, TEXT, HTML, &["rtf"]].iter().any(|list| list.contains(&ext.as_str()))
}

/// Sniffs a file. `None` means it is not text (or not a format we can read yet).
pub fn detect(path: &Path) -> std::io::Result<Option<Format>> {
    let ext = extension(path);
    if BINARY.contains(&ext.as_str()) {
        return Ok(None);
    }
    let mut head = Vec::with_capacity(SNIFF_BYTES);
    File::open(path)?.take(SNIFF_BYTES as u64).read_to_end(&mut head)?;
    if !looks_like_text(&head) {
        return Ok(None);
    }
    Ok(Some(match ext.as_str() {
        e if MARKDOWN.contains(&e) => Format::Markdown,
        e if HTML.contains(&e) => Format::Html,
        "rtf" => Format::Rtf,
        e => match CODE.iter().find(|(x, _)| *x == e) {
            Some((_, lang)) => Format::Code(lang),
            None if TEXT.contains(&e) => Format::Text,
            // Unknown or no extension: go by the content.
            None => sniff_content(&head),
        },
    }))
}

fn sniff_content(head: &[u8]) -> Format {
    let start = String::from_utf8_lossy(&head[..head.len().min(512)]).trim_start_matches('\u{feff}').trim_start().to_lowercase();
    if start.starts_with("{\\rtf") {
        Format::Rtf
    } else if start.starts_with("<!doctype html") || start.starts_with("<html") {
        Format::Html
    } else {
        Format::Text
    }
}

fn looks_like_text(head: &[u8]) -> bool {
    if head.starts_with(&[0xFF, 0xFE]) || head.starts_with(&[0xFE, 0xFF]) {
        return true; // UTF-16 with a byte order mark
    }
    if MAGIC.iter().any(|m| head.starts_with(m)) || head.contains(&0) {
        return false;
    }
    // Control characters other than whitespace (and the odd form feed or escape) mean binary.
    let control = head.iter().filter(|&&b| b < 0x20 && !matches!(b, b'\t' | b'\n' | b'\r' | 0x0C | 0x1B)).count();
    control * 100 <= head.len()
}

/// Decodes text: UTF-8 or UTF-16 (by byte order mark), otherwise Windows-1252, which
/// also covers Latin-1. Line endings become `\n`. Returns a warning for the fallback.
pub fn decode(bytes: Vec<u8>) -> (String, Option<String>) {
    let (text, warning) = if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        (utf16(rest, u16::from_le_bytes), None)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        (utf16(rest, u16::from_be_bytes), None)
    } else {
        match String::from_utf8(bytes) {
            Ok(s) => (s, None),
            Err(e) => (
                e.into_bytes().iter().map(|&b| cp1252(b)).collect(),
                Some("not valid UTF-8; read as Windows-1252".to_string()),
            ),
        }
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    (text.replace("\r\n", "\n").replace('\r', "\n"), warning)
}

fn utf16(bytes: &[u8], word: fn([u8; 2]) -> u16) -> String {
    let words = bytes.as_chunks::<2>().0.iter().map(|&c| word(c));
    char::decode_utf16(words).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
}

/// A Windows-1252 byte as a char. Bytes that 1252 leaves undefined map like Latin-1.
pub fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match b {
        0x80..=0x9F => HIGH[(b - 0x80) as usize],
        _ => b as char,
    }
}

/// Wraps text in a fence longer than any backtick run inside it.
pub fn fenced(text: &str, lang: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    let text = text.trim_end_matches('\n');
    format!("{fence}{lang}\n{text}\n{fence}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding() {
        assert_eq!(decode(b"caf\xc3\xa9\r\nx".to_vec()), ("café\nx".into(), None));
        let (text, warning) = decode(b"\x93caf\xe9\x94".to_vec());
        assert_eq!(text, "“café”");
        assert!(warning.is_some());
        assert_eq!(decode(b"\xFF\xFEh\0i\0\r\0\n\0".to_vec()).0, "hi\n");
        assert_eq!(decode(b"\xFE\xFF\0h\0i".to_vec()).0, "hi");
        assert_eq!(decode(b"\xEF\xBB\xBFbom".to_vec()).0, "bom");
    }

    #[test]
    fn text_or_binary() {
        assert!(looks_like_text(b"plain\ttext\n"));
        assert!(looks_like_text(b"\xFF\xFEa\0"));
        assert!(!looks_like_text(b"%PDF-1.4\n"));
        assert!(!looks_like_text(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1rest"));
        assert!(!looks_like_text(b"abc\0def"));
        assert!(!looks_like_text(b"\x01\x02\x03\x04ab"));
    }

    #[test]
    fn content_sniffing() {
        assert_eq!(sniff_content(b"{\\rtf1\\ansi"), Format::Rtf);
        assert_eq!(sniff_content(b"\n  <!DOCTYPE HTML>"), Format::Html);
        assert_eq!(sniff_content(b"Dear Jan,"), Format::Text);
    }

    #[test]
    fn fences_outrun_backticks() {
        assert_eq!(fenced("a,b\n", "csv"), "```csv\na,b\n```\n");
        assert_eq!(fenced("x ```` y", ""), "`````\nx ```` y\n`````\n");
    }
}
