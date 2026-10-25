//! Importing a folder of notes (an Obsidian vault, or an old archive).
//!
//! Best effort: every file that turns out to be text is imported. Markdown and plain
//! text are cleaned (HTML, hard wraps), HTML and RTF documents are converted first,
//! and data or code files are kept verbatim in a code block. Each note then goes
//! through the label rules and gets frontmatter for what they found. Binary formats
//! (Word, PDF, images, …) are counted in the report for a later version.
//!
//! A dry run does all of that and reports
//! what would happen; a real run also writes the notes in batches. The source folder
//! is only ever read.

pub mod clean;
mod dates;
pub mod formats;
mod rtf;
pub mod rules;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use serde::Serialize;

use crate::db::{self, NewNote};
use crate::markdown;
use crate::session::{Session, SessionError};
use formats::Format;
use rules::{AUTHOR_EMAIL_KEY, CompiledRule, RuleInput};

pub use dates::parse_loose_date;

/// Files read and converted per round. Their notes are written in as many
/// transactions as [`INSERT_BUDGET`] takes.
pub const BATCH_SIZE: usize = 2_000;
/// How long a write transaction keeps inserting. Committing then takes about as long
/// again (the full-text index writes out what it buffered), so a transaction holds the
/// vault about 1 s: the longest the UI waits for it during an import.
pub const INSERT_BUDGET: Duration = Duration::from_millis(500);
/// Gap between write transactions. The vault mutex isn't fair: without a gap the import
/// takes it back before a waiting UI command wakes.
const WRITE_PAUSE: Duration = Duration::from_millis(5);
/// Bigger "text" files are almost certainly not notes (logs, data dumps).
pub const MAX_TEXT_BYTES: u64 = 10 * 1024 * 1024;
/// The report's name for the `<page>_files` folders browsers save next to a web page.
const PAGE_ASSETS: &str = "web page assets";
/// The report lists this many warnings, skipped files and folders at most; the
/// `*_count` fields have the totals. Uncapped, re-previewing 100k files that are
/// already in the vault gave a 14 MB report.
const MAX_REPORTED: usize = 2000;
const SAMPLE_COUNT: usize = 20;
const SAMPLE_MAX_BYTES: usize = 6000;

#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("could not read the source folder: {0}")]
    Source(std::io::Error),
}

impl From<db::Error> for ImportError {
    fn from(e: db::Error) -> Self {
        Self::Session(e.into())
    }
}

/// A file ready to be written as a note.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// Relative to the source folder, with `/` separators.
    pub path: String,
    pub folder: String,
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub created_from: CreatedFrom,
    pub updated_at: i64,
    pub warnings: Vec<String>,
    /// The decoded text before any change, for dry-run samples.
    pub original: String,
    /// The file's lowercased extension, `""` if none.
    pub extension: String,
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("could not read the file: {0}")]
    Io(#[from] std::io::Error),
    /// Not text, or a format that can't be read yet.
    #[error("format not supported yet")]
    Unsupported,
    #[error("too large to import as text ({:.1} MB)", *.0 as f64 / 1_048_576.0)]
    TooLarge(u64),
}

/// Where a note's creation date came from, in order of precedence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreatedFrom {
    Rule,
    Frontmatter,
    FileName,
    Modified,
}

#[derive(Debug)]
pub struct Scan {
    /// Every candidate file, relative to the source folder, sorted. Hidden files and
    /// folders are left out.
    pub files: Vec<PathBuf>,
    /// Files in `<page>_files` folders next to a saved web page: styles, scripts and
    /// images that belong to the page rather than being notes. Such a folder with
    /// documents in it (Markdown, text, HTML, RTF) is scanned like any other.
    pub page_assets: usize,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub dry_run: bool,
    /// The vault folder the notes go under (normalised); `""` for the vault root.
    pub target: String,
    /// Notes already in the target folder and its subfolders before this run, so the
    /// import merges into them. Always 0 for the vault root.
    pub existing_in_target: usize,
    pub files: usize,
    /// Notes written, or that would be written in a dry run.
    pub imported: usize,
    /// Files that weren't imported, in source order, at most [`MAX_REPORTED`]. Files
    /// that couldn't be read are listed before those that already exist, so a few
    /// errors aren't lost among thousands of notes imported earlier.
    pub skipped: Vec<FileMessage>,
    pub skipped_count: usize,
    /// How many of the skipped files were left out because a note with that folder and
    /// title already exists.
    pub already_exist: usize,
    /// The folders notes were (or would be) written to, with how many, sorted. At most
    /// [`MAX_REPORTED`]: the ones with the most notes.
    pub folders: Vec<FolderCount>,
    pub folder_count: usize,
    pub warnings: Vec<FileMessage>,
    pub warning_count: usize,
    pub ambiguous_links: Vec<AmbiguousLink>,
    /// Before/after pairs of changed files (dry run only).
    pub samples: Vec<Sample>,
    /// Files left out because their format can't be read yet, by format, most common first.
    pub unsupported: Vec<FormatCount>,
    pub cancelled: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FormatCount {
    /// `.doc`, `no extension`, …
    pub format: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FolderCount {
    pub folder: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FileMessage {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AmbiguousLink {
    /// The linking note, as `folder/title`.
    pub note: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Sample {
    pub path: String,
    /// The note's folder in the vault, target prefix included.
    pub folder: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

/// Finds the files to import, skipping hidden files and folders (such as `.obsidian`).
pub fn scan(root: &Path) -> std::io::Result<Scan> {
    fn walk(root: &Path, dir: &Path, scan: &mut Scan, assets: bool) -> std::io::Result<()> {
        let mut entries = fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        let names: HashSet<String> = entries.iter().map(|e| e.file_name().to_string_lossy().to_lowercase()).collect();
        for entry in &entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if assets || !is_page_assets(&name, &names) {
                    walk(root, &path, scan, assets)?;
                    continue;
                }
                // Browsers fill `<page>_files` with styles, scripts and images. One that
                // holds documents is an ordinary folder that happens to have that name.
                let mut inner = Scan { files: Vec::new(), page_assets: 0 };
                walk(root, &path, &mut inner, false)?;
                if inner.files.iter().any(|f| formats::is_document(f)) {
                    scan.files.extend(inner.files);
                    scan.page_assets += inner.page_assets;
                } else {
                    scan.page_assets += inner.files.len() + inner.page_assets;
                }
            } else if assets {
                scan.page_assets += 1;
            } else {
                scan.files.push(path.strip_prefix(root).unwrap_or(&path).to_path_buf());
            }
        }
        Ok(())
    }
    let mut scan = Scan { files: Vec::new(), page_assets: 0 };
    walk(root, root, &mut scan, false)?;
    Ok(scan)
}

/// `page_files` next to `page.html` (or `.htm`), as browsers save a complete page.
/// [`scan`] still imports it when it holds documents.
fn is_page_assets(dir: &str, siblings: &HashSet<String>) -> bool {
    let Some(page) = dir.strip_suffix("_files") else { return false };
    let page = page.to_lowercase();
    ["html", "htm", "xhtml", "shtml"].iter().any(|ext| siblings.contains(&format!("{page}.{ext}")))
}

/// Reads and converts one file. Pure apart from reading the file.
pub fn prepare(root: &Path, rel: &Path, rules: &[CompiledRule]) -> Result<Prepared, PrepareError> {
    let full = root.join(rel);
    let format = formats::detect(&full)?.ok_or(PrepareError::Unsupported)?;
    let meta = fs::metadata(&full)?;
    if meta.len() > MAX_TEXT_BYTES {
        return Err(PrepareError::TooLarge(meta.len()));
    }
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or_else(db::now_ms, |d| d.as_millis() as i64);
    let (original, encoding_warning) = formats::decode(fs::read(&full)?);
    let mut warnings: Vec<String> = encoding_warning.into_iter().collect();
    let text = match format {
        Format::Markdown | Format::Text | Format::Code(_) => original.clone(),
        Format::Html => {
            let (md, w) = clean::html_document(&original);
            warnings.extend(w);
            md
        }
        Format::Rtf => rtf::to_text(&original).unwrap_or_else(|| {
            warnings.push("not a valid RTF document; imported as text".into());
            original.clone()
        }),
    };
    let has_frontmatter = matches!(format, Format::Markdown | Format::Text);

    let path = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
    let folder = markdown::normalize_folder(&rel.parent().map(|p| p.to_string_lossy()).unwrap_or_default());
    let stem = rel.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();

    let (yaml, content_start) = match markdown::split_frontmatter(&text).filter(|_| has_frontmatter) {
        Some((yaml, end)) => {
            if !markdown::frontmatter_is_valid(yaml) {
                warnings.push("frontmatter is not valid YAML; it was kept but not read".into());
            }
            (yaml, end)
        }
        None => ("", 0),
    };
    let found = rules::apply(rules, &RuleInput { file_name: &stem, frontmatter: yaml, content: &text[content_start..] });
    warnings.extend(found.warnings.iter().cloned());

    let title = sanitize_title(if found.file_name.is_empty() { &stem } else { &found.file_name }, &mut warnings);
    let content = match format {
        Format::Code(lang) => formats::fenced(&found.content, lang),
        _ => {
            let (content, clean_warnings) = clean::clean(&found.content);
            warnings.extend(clean_warnings);
            content
        }
    };

    let mut body = format!("{}{content}", &text[..content_start]);
    let parsed = markdown::parse(&body);
    let mut set = |key: &str, values: &[String]| body = markdown::set_frontmatter_field(&body, key, values);
    if let Some(author) = &found.author {
        set("author", std::slice::from_ref(author));
    }
    if let Some(email) = &found.author_email {
        set(AUTHOR_EMAIL_KEY, std::slice::from_ref(email));
    }
    let topic = found.topic.clone().or_else(|| match &parsed.topic {
        Some(_) => None,
        None => folder.split('/').next().filter(|f| !f.is_empty()).map(str::to_string),
    });
    if let Some(topic) = topic {
        set("topic", &[topic]);
    }
    if !found.tags.is_empty() {
        let mut tags = parsed.frontmatter_tags.clone();
        for tag in &found.tags {
            if !tags.contains(tag) {
                tags.push(tag.clone());
            }
        }
        set("tags", &tags);
    }
    for (key, values) in &found.attributes {
        set(key, values);
    }
    let (created_at, created_from) = if let Some(ms) = found.created_at {
        set("created", &[dates::format_date(ms)]);
        (ms, CreatedFrom::Rule)
    } else if let Some(ms) = parsed.created_at {
        (ms, CreatedFrom::Frontmatter)
    } else if let Some(ms) = dates::date_in_name(&stem) {
        (ms, CreatedFrom::FileName)
    } else {
        (modified, CreatedFrom::Modified)
    };

    Ok(Prepared {
        path,
        folder,
        title,
        body,
        created_at,
        created_from,
        updated_at: modified,
        warnings,
        original,
        extension: formats::extension(rel),
    })
}

/// Replaces characters titles can't hold (they would break links or paths).
fn sanitize_title(title: &str, warnings: &mut Vec<String>) -> String {
    let clean: String = title
        .chars()
        .map(|c| match c {
            '[' => '(',
            ']' => ')',
            '/' | '\\' | '|' => '-',
            '#' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = if clean.is_empty() { "Untitled".to_string() } else { clean };
    if clean != title {
        warnings.push(format!("title changed to '{clean}': titles can't contain [ ] # | / \\"));
    }
    clean
}

/// The import target suggested for a source folder: the folder's own name, so
/// `D:\Archive\OldVault` goes under `OldVault`. `""` (the vault root) for a drive root.
/// Splits on both separators, so it gives the same answer on every platform.
pub fn suggest_target(source: &str) -> String {
    let name = source.split(['/', '\\']).map(str::trim).rfind(|s| !s.is_empty()).unwrap_or("");
    if name.ends_with(':') { String::new() } else { name.to_string() }
}

/// The vault folder of an imported file: `target` followed by its folder relative to
/// the source. An empty target is the vault root.
pub fn target_folder(target: &str, relative: &str) -> String {
    markdown::normalize_folder(&format!("{target}/{relative}"))
}

/// Imports (or, with `dry_run`, previews importing) the files under `source` into the
/// open vault under the folder `target` (`""` for the root), using the vault's rules.
/// Writes happen in transactions of at most [`BATCH_SIZE`] notes or [`INSERT_BUDGET`],
/// with the vault set up for bulk writes meanwhile; `cancel` is checked between
/// transactions.
pub fn run(
    session: &Session,
    source: &Path,
    target: &str,
    dry_run: bool,
    cancel: &AtomicBool,
    mut progress: impl FnMut(Progress),
) -> Result<ImportReport, ImportError> {
    let vault_path = session.path().ok_or(SessionError::Locked)?;
    let rules = on_vault(session, &vault_path, |v| v.import_rules())?;
    let rules = rules
        .iter()
        .map(|r| r.compile().map_err(db::Error::InvalidRule))
        .collect::<Result<Vec<_>, _>>()?;
    let scan = scan(source).map_err(ImportError::Source)?;
    let total = scan.files.len();
    let target = markdown::normalize_folder(target);
    let existing_in_target =
        if target.is_empty() { 0 } else { on_vault(session, &vault_path, |v| v.count_in_folder(&target))? };
    let mut report = ImportReport {
        dry_run,
        target: target.clone(),
        existing_in_target,
        files: total + scan.page_assets,
        ..Default::default()
    };
    let mut folders: BTreeMap<String, usize> = BTreeMap::new();
    let mut skipped = Skipped::default();
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    if scan.page_assets > 0 {
        unsupported.insert(PAGE_ASSETS.into(), scan.page_assets);
    }
    let mut imported_ids = Vec::new();
    // (folder, title) of every note this import creates, lowercased, to keep them apart.
    let mut planned: HashSet<(String, String)> = HashSet::new();
    progress(Progress { done: 0, total });

    if !dry_run {
        on_vault(session, &vault_path, |v| v.set_bulk_writes(true))?;
    }
    let mut write_all = || -> Result<(), ImportError> {
        for (n, chunk) in scan.files.chunks(BATCH_SIZE).enumerate() {
            if cancel.load(Ordering::Relaxed) {
                report.cancelled = true;
                break;
            }
            let mut batch = Vec::with_capacity(chunk.len());
            for rel in chunk {
                match prepare(source, rel, &rules) {
                    Ok(mut p) => {
                        // After `prepare`, so the default topic stays the source-relative top folder.
                        p.folder = target_folder(&target, &p.folder);
                        give_unique_title(&mut p, &mut planned);
                        batch.push(p);
                    }
                    Err(PrepareError::Unsupported) => {
                        let ext = formats::extension(rel);
                        let format = if ext.is_empty() { "no extension".to_string() } else { format!(".{ext}") };
                        *unsupported.entry(format).or_default() += 1;
                    }
                    Err(e) => skipped.push(false, FileMessage {
                        path: rel.to_string_lossy().replace('\\', "/"),
                        message: e.to_string(),
                    }),
                }
            }

            let outcomes: Vec<db::Result<i64>> = if dry_run {
                on_vault(session, &vault_path, |v| {
                    let mut outcomes = Vec::with_capacity(batch.len());
                    for p in &batch {
                        let taken = v.note_exists(&p.folder, &p.title)?;
                        outcomes.push(if taken { Err(db::Error::TitleTaken) } else { Ok(0) });
                    }
                    Ok(outcomes)
                })?
            } else {
                let notes: Vec<NewNote> = batch
                    .iter()
                    .map(|p| NewNote {
                        folder: &p.folder,
                        title: &p.title,
                        body: &p.body,
                        created_at: p.created_at,
                        updated_at: p.updated_at,
                    })
                    .collect();
                let mut outcomes = Vec::with_capacity(notes.len());
                while outcomes.len() < notes.len() {
                    if !outcomes.is_empty() {
                        std::thread::sleep(WRITE_PAUSE);
                        if cancel.load(Ordering::Relaxed) {
                            report.cancelled = true;
                            break;
                        }
                    }
                    let rest = &notes[outcomes.len()..];
                    outcomes.extend(on_vault(session, &vault_path, |v| v.import_notes_within(rest, INSERT_BUDGET))?);
                }
                outcomes
            };

            // Only the files written: a cancel can stop partway through a batch.
            for p in &batch[..outcomes.len()] {
                for w in &p.warnings {
                    report.warning_count += 1;
                    if report.warnings.len() < MAX_REPORTED {
                        report.warnings.push(FileMessage { path: p.path.clone(), message: w.clone() });
                    }
                }
            }
            for (p, outcome) in batch.iter().zip(outcomes) {
                match outcome {
                    Ok(id) => {
                        report.imported += 1;
                        *folders.entry(p.folder.clone()).or_default() += 1;
                        imported_ids.push(id);
                        if dry_run && report.samples.len() < SAMPLE_COUNT && p.body != p.original {
                            report.samples.push(Sample {
                                path: p.path.clone(),
                                folder: p.folder.clone(),
                                before: truncate(&p.original),
                                after: truncate(&p.body),
                            });
                        }
                    }
                    Err(e) => {
                        let exists = matches!(e, db::Error::TitleTaken);
                        let message = match e {
                            db::Error::TitleTaken => {
                                report.already_exist += 1;
                                let folder = if p.folder.is_empty() { "the vault root".to_string() } else { format!("'{}'", p.folder) };
                                format!("a note titled '{}' already exists in {folder}", p.title)
                            }
                            e => e.to_string(),
                        };
                        skipped.push(exists, FileMessage { path: p.path.clone(), message });
                    }
                }
            }
            if report.cancelled {
                break;
            }
            progress(Progress { done: (n * BATCH_SIZE + chunk.len()).min(total), total });
        }
        Ok(())
    };
    let written = write_all();
    if !dry_run {
        // Also after a failure; if the vault locked meanwhile, opening it again resets it.
        let _ = on_vault(session, &vault_path, |v| v.set_bulk_writes(false));
    }
    written?;

    let mut unsupported: Vec<FormatCount> =
        unsupported.into_iter().map(|(format, count)| FormatCount { format, count }).collect();
    unsupported.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.format.cmp(&b.format)));
    report.unsupported = unsupported;
    report.skipped_count = skipped.count;
    report.skipped = skipped.into_list();
    report.folder_count = folders.len();
    report.folders = largest_folders(folders);

    if !dry_run && !imported_ids.is_empty() {
        report.ambiguous_links = on_vault(session, &vault_path, |v| v.ambiguous_links(&imported_ids))?
            .into_iter()
            .map(|(note, target)| AmbiguousLink { note, target })
            .collect();
    }
    Ok(report)
}

/// The skipped files listed in the report: up to [`MAX_REPORTED`] that couldn't be read,
/// then as many that already exist as fit, numbered so they go back into source order.
#[derive(Default)]
struct Skipped {
    count: usize,
    failed: Vec<(usize, FileMessage)>,
    exist: Vec<(usize, FileMessage)>,
}

impl Skipped {
    fn push(&mut self, exists: bool, message: FileMessage) {
        let list = if exists { &mut self.exist } else { &mut self.failed };
        if list.len() < MAX_REPORTED {
            list.push((self.count, message));
        }
        self.count += 1;
    }

    fn into_list(mut self) -> Vec<FileMessage> {
        self.exist.truncate(MAX_REPORTED - self.failed.len());
        let mut all = self.failed;
        all.append(&mut self.exist);
        all.sort_unstable_by_key(|(n, _)| *n);
        all.into_iter().map(|(_, m)| m).collect()
    }
}

/// The [`MAX_REPORTED`] folders with the most notes (ties by name), sorted by name.
fn largest_folders(folders: BTreeMap<String, usize>) -> Vec<FolderCount> {
    let mut all: Vec<FolderCount> = folders.into_iter().map(|(folder, count)| FolderCount { folder, count }).collect();
    if all.len() > MAX_REPORTED {
        // Stable, so equal counts keep name order.
        all.sort_by(|a, b| b.count.cmp(&a.count));
        all.truncate(MAX_REPORTED);
        all.sort_by(|a, b| a.folder.cmp(&b.folder));
    }
    all
}

/// Files with the same name in one folder (`letter.txt`, `letter.rtf`) would get the
/// same title; later ones get their extension added (`letter (rtf)`). Clashes with
/// notes already in the vault are left alone: those files are skipped, so importing
/// a folder again only adds what is new.
fn give_unique_title(p: &mut Prepared, planned: &mut HashSet<(String, String)>) {
    let folder = p.folder.to_lowercase();
    if planned.insert((folder.clone(), p.title.to_lowercase())) {
        return;
    }
    let base = if p.extension.is_empty() { p.title.clone() } else { format!("{} ({})", p.title, p.extension) };
    let mut title = base.clone();
    let mut n = 2;
    while !planned.insert((folder.clone(), title.to_lowercase())) {
        title = format!("{base} {n}");
        n += 1;
    }
    p.warnings.push(format!("renamed to '{title}': another file in this folder has the same name"));
    p.title = title;
}

/// Runs `f` on the open vault if it is still the one the import started in, so a vault
/// unlocked after a lock mid-import is never written to.
fn on_vault<T>(
    session: &Session,
    vault_path: &Path,
    f: impl FnOnce(&mut db::Vault) -> db::Result<T>,
) -> Result<T, SessionError> {
    if session.path().as_deref() != Some(vault_path) {
        return Err(SessionError::Locked);
    }
    session.with_vault(f)
}

fn truncate(s: &str) -> String {
    if s.len() <= SAMPLE_MAX_BYTES {
        return s.to_string();
    }
    let mut end = SAMPLE_MAX_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(n: usize) -> FileMessage {
        FileMessage { path: format!("f{n:05}"), message: String::new() }
    }

    #[test]
    fn skipped_files_are_capped_with_unreadable_ones_kept_first() {
        let mut skipped = Skipped::default();
        for n in 0..10_000 {
            // Every 100th file couldn't be read; the rest already exist.
            skipped.push(n % 100 != 0, message(n));
        }
        assert_eq!(skipped.count, 10_000);
        let list = skipped.into_list();
        assert_eq!(list.len(), MAX_REPORTED);
        let failed: Vec<&str> = list.iter().map(|m| m.path.as_str()).filter(|p| p.ends_with("00")).collect();
        assert_eq!(failed.len(), 100, "every unreadable file is listed");
        assert_eq!(failed.last(), Some(&"f09900"));
        assert!(list.windows(2).all(|w| w[0].path < w[1].path), "source order");
    }

    #[test]
    fn only_the_largest_folders_are_listed_sorted_by_name() {
        let folders: BTreeMap<String, usize> =
            (0..MAX_REPORTED + 500).map(|n| (format!("d{n:05}"), if n % 2 == 0 { 2 } else { 1 })).collect();
        let listed = largest_folders(folders);
        assert_eq!(listed.len(), MAX_REPORTED);
        // All 1,250 two-note folders, then the one-note folders that come first by name.
        assert_eq!(listed.iter().filter(|f| f.count == 2).count(), (MAX_REPORTED + 500) / 2);
        assert!(listed.iter().any(|f| f.folder == "d00001"));
        assert!(!listed.iter().any(|f| f.folder == "d02499"));
        assert!(listed.windows(2).all(|w| w[0].folder < w[1].folder));
    }
}
