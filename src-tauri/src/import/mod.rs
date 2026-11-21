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
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
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
/// Batches read ahead of the one being written. Bounds memory at any import size.
const QUEUED_BATCHES: usize = 2;
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
    if formats::never_text(rel) {
        return Err(PrepareError::Unsupported);
    }
    // Opened once: on Windows every open also waits for the virus scanner.
    let mut file = File::open(root.join(rel))?;
    let meta = file.metadata()?;
    if meta.len() > MAX_TEXT_BYTES {
        // Only the start, to tell a big text file from a binary one.
        let mut head = Vec::with_capacity(formats::SNIFF_BYTES);
        (&mut file).take(formats::SNIFF_BYTES as u64).read_to_end(&mut head)?;
        formats::sniff(rel, &head).ok_or(PrepareError::Unsupported)?;
        return Err(PrepareError::TooLarge(meta.len()));
    }
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut bytes)?;
    drop(file);
    let format = formats::sniff(rel, &bytes).ok_or(PrepareError::Unsupported)?;
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or_else(db::now_ms, |d| d.as_millis() as i64);
    let (original, encoding_warning) = formats::decode(bytes);
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
    let mut writer = Writer::new(session, &vault_path, target, dry_run, cancel);
    writer.report.existing_in_target = existing_in_target;
    writer.report.files = total + scan.page_assets;
    if scan.page_assets > 0 {
        writer.unsupported.insert(PAGE_ASSETS.into(), scan.page_assets);
    }
    progress(Progress { done: 0, total });

    if !dry_run {
        on_vault(session, &vault_path, |v| v.set_bulk_writes(true))?;
    }
    // Files are read and converted on worker threads, a few batches ahead of the
    // writes; results come back in file order, so the report is the same as reading
    // them one by one.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let stop = AtomicBool::new(false);
    let stopped = || cancel.load(Ordering::Relaxed) || stop.load(Ordering::Relaxed);
    let (ready, prepared) = mpsc::sync_channel(QUEUED_BATCHES);
    let written = std::thread::scope(|s| {
        s.spawn(|| prepare_all(source, &scan.files, &rules, threads, &stopped, ready));
        let written = writer.write_all(&scan.files, prepared, &mut progress);
        stop.store(true, Ordering::Relaxed);
        written
    });
    if !dry_run {
        // Also after a failure; if the vault locked meanwhile, opening it again resets it.
        let _ = on_vault(session, &vault_path, |v| v.set_bulk_writes(false));
    }
    written?;

    let (mut report, imported_ids) = writer.finish();
    if !dry_run && !imported_ids.is_empty() {
        report.ambiguous_links = on_vault(session, &vault_path, |v| v.ambiguous_links(&imported_ids))?
            .into_iter()
            .map(|(note, target)| AmbiguousLink { note, target })
            .collect();
    }
    Ok(report)
}

/// What [`prepare_batch`] returns: one result per file, in file order.
type PreparedBatch = Vec<Result<Prepared, PrepareError>>;

/// Prepares `files` a batch at a time and sends each batch to `ready`, until `stopped`
/// or the receiver is gone.
fn prepare_all(
    source: &Path,
    files: &[PathBuf],
    rules: &[CompiledRule],
    threads: usize,
    stopped: &(dyn Fn() -> bool + Sync),
    ready: mpsc::SyncSender<PreparedBatch>,
) {
    for chunk in files.chunks(BATCH_SIZE) {
        let Some(results) = prepare_batch(source, chunk, rules, threads, stopped) else { break };
        if ready.send(results).is_err() {
            break;
        }
    }
}

/// Writes the prepared batches of one import and builds its report.
struct Writer<'a> {
    session: &'a Session,
    vault_path: &'a Path,
    target: String,
    dry_run: bool,
    cancel: &'a AtomicBool,
    report: ImportReport,
    folders: BTreeMap<String, usize>,
    skipped: Skipped,
    unsupported: BTreeMap<String, usize>,
    imported_ids: Vec<i64>,
    /// (folder, title) of every note this import creates, lowercased, to keep them apart.
    planned: HashSet<(String, String)>,
}

impl<'a> Writer<'a> {
    fn new(session: &'a Session, vault_path: &'a Path, target: String, dry_run: bool, cancel: &'a AtomicBool) -> Self {
        let report = ImportReport { dry_run, target: target.clone(), ..Default::default() };
        Self {
            session,
            vault_path,
            target,
            dry_run,
            cancel,
            report,
            folders: BTreeMap::new(),
            skipped: Skipped::default(),
            unsupported: BTreeMap::new(),
            imported_ids: Vec::new(),
            planned: HashSet::new(),
        }
    }

    /// Writes `files` batch by batch as the workers deliver them, until done or cancelled.
    fn write_all(
        &mut self,
        files: &[PathBuf],
        prepared: mpsc::Receiver<PreparedBatch>,
        progress: &mut impl FnMut(Progress),
    ) -> Result<(), ImportError> {
        let total = files.len();
        for (n, chunk) in files.chunks(BATCH_SIZE).enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                self.report.cancelled = true;
                break;
            }
            // The workers only stop early when cancelled.
            let Ok(results) = prepared.recv() else {
                self.report.cancelled = true;
                break;
            };
            self.write_batch(chunk, results)?;
            if self.report.cancelled {
                break;
            }
            progress(Progress { done: (n * BATCH_SIZE + chunk.len()).min(total), total });
        }
        Ok(())
    }

    /// Writes (or, in a dry run, checks) one batch and adds it to the report.
    fn write_batch(&mut self, chunk: &[PathBuf], results: PreparedBatch) -> Result<(), ImportError> {
        let batch = self.plan(chunk, results);
        let outcomes = if self.dry_run { self.check_all(&batch)? } else { self.insert_all(&batch)? };
        // Only the files written: a cancel can stop partway through a batch.
        for p in &batch[..outcomes.len()] {
            self.add_warnings(p);
        }
        for (p, outcome) in batch.iter().zip(outcomes) {
            match outcome {
                Ok(id) => self.add_imported(p, id),
                Err(e) => self.add_skipped(p, e),
            }
        }
        Ok(())
    }

    /// The files to write, placed under the target with unique titles. Files that can't
    /// be imported go to the report.
    fn plan(&mut self, chunk: &[PathBuf], results: PreparedBatch) -> Vec<Prepared> {
        let mut batch = Vec::with_capacity(chunk.len());
        for (rel, result) in chunk.iter().zip(results) {
            match result {
                Ok(mut p) => {
                    // After `prepare`, so the default topic stays the source-relative top folder.
                    p.folder = target_folder(&self.target, &p.folder);
                    give_unique_title(&mut p, &mut self.planned);
                    batch.push(p);
                }
                Err(PrepareError::Unsupported) => {
                    let ext = formats::extension(rel);
                    let format = if ext.is_empty() { "no extension".to_string() } else { format!(".{ext}") };
                    *self.unsupported.entry(format).or_default() += 1;
                }
                Err(e) => self.skipped.push(
                    false,
                    FileMessage { path: rel.to_string_lossy().replace('\\', "/"), message: e.to_string() },
                ),
            }
        }
        batch
    }

    /// Dry run: whether each note would be written, without writing anything.
    fn check_all(&self, batch: &[Prepared]) -> Result<Vec<db::Result<i64>>, ImportError> {
        Ok(on_vault(self.session, self.vault_path, |v| {
            let mut outcomes = Vec::with_capacity(batch.len());
            for p in batch {
                let taken = v.note_exists(&p.folder, &p.title)?;
                outcomes.push(if taken { Err(db::Error::TitleTaken) } else { Ok(0) });
            }
            Ok(outcomes)
        })?)
    }

    /// Writes the notes in transactions of at most [`INSERT_BUDGET`], with a pause
    /// between them. Stops early when cancelled.
    fn insert_all(&mut self, batch: &[Prepared]) -> Result<Vec<db::Result<i64>>, ImportError> {
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
                if self.cancel.load(Ordering::Relaxed) {
                    self.report.cancelled = true;
                    break;
                }
            }
            let rest = &notes[outcomes.len()..];
            outcomes.extend(on_vault(self.session, self.vault_path, |v| v.import_notes_within(rest, INSERT_BUDGET))?);
        }
        Ok(outcomes)
    }

    fn add_warnings(&mut self, p: &Prepared) {
        for w in &p.warnings {
            self.report.warning_count += 1;
            if self.report.warnings.len() < MAX_REPORTED {
                self.report.warnings.push(FileMessage { path: p.path.clone(), message: w.clone() });
            }
        }
    }

    fn add_imported(&mut self, p: &Prepared, id: i64) {
        self.report.imported += 1;
        *self.folders.entry(p.folder.clone()).or_default() += 1;
        self.imported_ids.push(id);
        if self.dry_run && self.report.samples.len() < SAMPLE_COUNT && p.body != p.original {
            self.report.samples.push(Sample {
                path: p.path.clone(),
                folder: p.folder.clone(),
                before: truncate(&p.original),
                after: truncate(&p.body),
            });
        }
    }

    fn add_skipped(&mut self, p: &Prepared, e: db::Error) {
        let exists = matches!(e, db::Error::TitleTaken);
        let message = if exists {
            self.report.already_exist += 1;
            let folder = if p.folder.is_empty() { "the vault root".to_string() } else { format!("'{}'", p.folder) };
            format!("a note titled '{}' already exists in {folder}", p.title)
        } else {
            e.to_string()
        };
        self.skipped.push(exists, FileMessage { path: p.path.clone(), message });
    }

    /// The report, and the ids of the notes written.
    fn finish(self) -> (ImportReport, Vec<i64>) {
        let mut report = self.report;
        let mut unsupported: Vec<FormatCount> =
            self.unsupported.into_iter().map(|(format, count)| FormatCount { format, count }).collect();
        unsupported.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.format.cmp(&b.format)));
        report.unsupported = unsupported;
        report.skipped_count = self.skipped.count;
        report.skipped = self.skipped.into_list();
        report.folder_count = self.folders.len();
        report.folders = largest_folders(self.folders);
        (report, self.imported_ids)
    }
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

/// [`prepare`] on `threads` threads, in file order. `None` when `stopped` said so first.
fn prepare_batch(
    source: &Path,
    files: &[PathBuf],
    rules: &[CompiledRule],
    threads: usize,
    stopped: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<Result<Prepared, PrepareError>>> {
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, Result<Prepared, PrepareError>)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.min(files.len()))
            .map(|_| {
                s.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= files.len() || stopped() {
                            return done;
                        }
                        done.push((i, prepare(source, &files[i], rules)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap()).collect()
    });
    if results.len() < files.len() {
        return None;
    }
    results.sort_unstable_by_key(|(i, _)| *i);
    Some(results.into_iter().map(|(_, r)| r).collect())
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

    fn prepared(folder: &str, title: &str) -> Prepared {
        Prepared {
            path: format!("{folder}/{title}.md"),
            folder: folder.into(),
            title: title.into(),
            body: format!("changed {title}"),
            created_at: 0,
            created_from: CreatedFrom::Modified,
            updated_at: 0,
            warnings: Vec::new(),
            original: title.into(),
            extension: "md".into(),
        }
    }

    fn with_writer<T>(dry_run: bool, f: impl FnOnce(&mut Writer) -> T) -> T {
        let session = Session::default();
        let cancel = AtomicBool::new(false);
        f(&mut Writer::new(&session, Path::new("v"), "t".into(), dry_run, &cancel))
    }

    #[test]
    fn plan_counts_unsupported_files_and_reports_unreadable_ones() {
        let files: Vec<PathBuf> = ["a.md", "b.doc", "c", "d.md"].iter().map(PathBuf::from).collect();
        let results = vec![
            Ok(prepared("x", "a")),
            Err(PrepareError::Unsupported),
            Err(PrepareError::Unsupported),
            Err(PrepareError::TooLarge(11 * 1_048_576)),
        ];
        with_writer(false, |w| {
            let batch = w.plan(&files, results);
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].folder, "t/x");
            assert_eq!(w.unsupported.get(".doc"), Some(&1));
            assert_eq!(w.unsupported.get("no extension"), Some(&1));
            assert_eq!(w.skipped.count, 1);
            assert_eq!(std::mem::take(&mut w.skipped).into_list()[0].path, "d.md");
        });
    }

    #[test]
    fn warnings_are_counted_and_listed_up_to_the_cap() {
        with_writer(false, |w| {
            let mut p = prepared("", "a");
            p.warnings = vec!["one".into(), "two".into()];
            w.add_warnings(&p);
            assert_eq!(w.report.warning_count, 2);
            assert_eq!(w.report.warnings.len(), 2);
            w.report.warnings = (0..MAX_REPORTED).map(message).collect();
            w.add_warnings(&p);
            assert_eq!(w.report.warning_count, 4);
            assert_eq!(w.report.warnings.len(), MAX_REPORTED);
        });
    }

    #[test]
    fn only_a_dry_run_samples_changed_files() {
        let mut unchanged = prepared("", "same");
        unchanged.body = unchanged.original.clone();
        with_writer(true, |w| {
            w.add_imported(&prepared("f", "a"), 0);
            w.add_imported(&unchanged, 0);
            assert_eq!((w.report.imported, w.report.samples.len(), w.imported_ids.len()), (2, 1, 2));
            assert_eq!(w.folders.get("f"), Some(&1));
        });
        with_writer(false, |w| {
            w.add_imported(&prepared("f", "a"), 7);
            assert!(w.report.samples.is_empty());
            assert_eq!(w.imported_ids, [7]);
        });
    }

    #[test]
    fn a_skipped_note_says_why() {
        with_writer(false, |w| {
            w.add_skipped(&prepared("", "root"), db::Error::TitleTaken);
            w.add_skipped(&prepared("f", "in folder"), db::Error::TitleTaken);
            w.add_skipped(&prepared("f", "bad"), db::Error::InvalidTitle("bad".into()));
            assert_eq!(w.report.already_exist, 2);
            assert_eq!(w.skipped.count, 3);
            let messages: Vec<String> = std::mem::take(&mut w.skipped).into_list().into_iter().map(|m| m.message).collect();
            assert!(messages.contains(&"a note titled 'root' already exists in the vault root".to_string()));
            assert!(messages.contains(&"a note titled 'in folder' already exists in 'f'".to_string()));
            assert!(messages.contains(&"invalid title: bad".to_string()));
        });
    }

    #[test]
    fn a_lost_worker_counts_as_cancelled() {
        let (ready, prepared) = mpsc::sync_channel::<PreparedBatch>(1);
        drop(ready);
        with_writer(false, |w| {
            w.write_all(&[PathBuf::from("a.md")], prepared, &mut |_| {}).unwrap();
            assert!(w.report.cancelled);
        });
    }

    #[test]
    fn cancelling_before_the_first_batch_writes_nothing() {
        let session = Session::default();
        let cancel = AtomicBool::new(true);
        let mut w = Writer::new(&session, Path::new("v"), String::new(), false, &cancel);
        let (_ready, prepared) = mpsc::sync_channel::<PreparedBatch>(1);
        w.write_all(&[PathBuf::from("a.md")], prepared, &mut |_| panic!("no progress")).unwrap();
        assert!(w.report.cancelled);
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
