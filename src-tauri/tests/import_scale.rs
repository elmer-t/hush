//! Import benchmark: where the time goes when importing a large folder.
//!
//! `cargo test --release --test import_scale -- --ignored --nocapture`
//! - `HUSH_IMPORT_FILES`: files to generate (default 100,000).
//! - `HUSH_IMPORT_SOURCE`: keep the generated folder here and reuse it.
//! - `HUSH_IMPORT_BASE`: notes already in the vault the import runs into (default 0),
//!   copies of the generated notes under `Base <n>` folders.
//! - `HUSH_IMPORT_BASE_VAULT`: keep that vault here and reuse it (each run imports
//!   into a copy).

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hush_lib::db::{NewNote, Vault};
use hush_lib::import::rules::Rule;
use hush_lib::import::{self, Prepared, clean, formats};
use hush_lib::session::Session;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const WORDS: &[&str] = &[
    "the", "of", "and", "to", "in", "is", "that", "it", "was", "for", "on", "are", "with", "as", "they", "be", "at",
    "one", "have", "this", "from", "or", "had", "by", "word", "but", "what", "some", "we", "can", "out", "other",
    "were", "all", "there", "when", "up", "use", "your", "how", "said", "an", "each", "she", "which", "do", "their",
    "time", "if", "will", "way", "about", "many", "then", "them", "write", "would", "like", "so", "these", "her",
    "long", "make", "thing", "see", "him", "two", "has", "look", "more", "day", "could", "go", "come", "did",
    "number", "sound", "no", "most", "people", "my", "over", "know", "water", "than", "call", "first", "who", "may",
    "down", "side", "been", "now", "find", "harbour", "letter", "garden", "meeting", "travel", "winter", "kitchen",
];

/// Mail-style records like the old online archive: From/Date/Subject headers and a
/// body hard-wrapped at 72 columns, 100–1,500 words. One in ten is Markdown with
/// frontmatter instead.
fn generate(root: &Path, files: usize) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let folders = (files / 50).max(1);
    for i in 0..files {
        let folder = root.join(format!("archive/{:03}/box {}", i % 37, rng.below(folders)));
        fs::create_dir_all(&folder).unwrap();
        let words = 100 + (1_400.0 * (rng.below(1000) as f64 / 1000.0).powi(6)) as usize;
        let mut text = String::with_capacity(words * 7 + 200);
        let md = i % 10 == 0;
        if md {
            text.push_str(&format!("---\ntags: [t{}]\ncreated: 2011-0{}-1{}\n---\n", i % 50, 1 + i % 9, i % 10));
        } else {
            text.push_str(&format!(
                "From: Person {} <person{}@example.org>\nDate: {} Mar 20{:02} 10:{:02}\nSubject: {} {}\n\n",
                i % 300,
                i % 300,
                1 + i % 28,
                i % 20,
                i % 60,
                WORDS[rng.below(WORDS.len())],
                WORDS[rng.below(WORDS.len())]
            ));
        }
        let mut col = 0;
        for w in 0..words {
            let word = WORDS[rng.below(WORDS.len())];
            if col + word.len() > 72 {
                text.push('\n');
                col = 0;
            }
            text.push_str(word);
            text.push(if w % 15 == 14 { '.' } else { ' ' });
            col += word.len() + 1;
            if w % 120 == 119 {
                text.push_str("\n\n");
                col = 0;
            }
        }
        let name = format!("{} {} {i}.{}", WORDS[rng.below(WORDS.len())], if i % 7 == 0 { "jr" } else { "note" }, if md { "md" } else { "txt" });
        fs::write(folder.join(name), text).unwrap();
    }
}

fn secs(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

fn rss_mb() -> String {
    let status = fs::read_to_string("/proc/self/status").unwrap_or_default();
    let field = |name: &str| {
        status.lines().find(|l| l.starts_with(name)).and_then(|l| l.split_whitespace().nth(1)?.parse::<u64>().ok()).unwrap_or(0)
    };
    format!("rss {} MB, peak {} MB", field("VmRSS:") / 1024, field("VmHWM:") / 1024)
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

/// Median, 99th percentile and max, in ms.
fn spread(mut times: Vec<Duration>) -> String {
    if times.is_empty() {
        return "none".into();
    }
    times.sort();
    format!("median {} ms, p99 {} ms, max {} ms", ms(times[times.len() / 2]), ms(times[times.len() * 99 / 100]), ms(times[times.len() - 1]))
}

/// A vault holding `notes` copies of `prepared` (under `Base <n>/…`), kept at `keep`
/// when given. Returns the temp dir holding the copy to import into, and its path.
fn base_vault(prepared: &[Prepared], notes: usize, keep: Option<PathBuf>) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = keep.unwrap_or_else(|| dir.path().join("base.hush"));
    if !path.exists() {
        let start = Instant::now();
        let mut vault = Vault::create(&path, common::PASS).unwrap();
        let mut done = 0;
        for round in 1.. {
            let folders: Vec<String> = prepared.iter().map(|p| format!("Base {round}/{}", p.folder)).collect();
            for (chunk, folders) in prepared.chunks(import::BATCH_SIZE).zip(folders.chunks(import::BATCH_SIZE)) {
                let take = chunk.len().min(notes - done);
                let new: Vec<NewNote> = chunk[..take]
                    .iter()
                    .zip(folders)
                    .map(|(p, f)| NewNote { folder: f, title: &p.title, body: &p.body, created_at: p.created_at, updated_at: p.updated_at })
                    .collect();
                for outcome in vault.import_notes(&new).unwrap() {
                    outcome.unwrap();
                }
                done += take;
                if done == notes {
                    break;
                }
            }
            println!("  base vault: {done} notes, {}", secs(start.elapsed()));
            if done == notes {
                break;
            }
        }
        vault.close().unwrap();
    }
    let copy = dir.path().join("vault.hush");
    if copy != path {
        fs::copy(&path, &copy).unwrap();
    }
    let size = fs::metadata(&copy).map(|m| m.len()).unwrap_or(0);
    println!("base vault: {notes} notes, {:.2} GB", size as f64 / 1e9);
    (dir, copy)
}

fn drop_caches() -> bool {
    unsafe { libc_sync() };
    fs::write("/proc/sys/vm/drop_caches", "3").is_ok()
}

unsafe extern "C" {
    #[link_name = "sync"]
    fn libc_sync();
}

#[test]
#[ignore = "import benchmark: minutes; run with --ignored --nocapture"]
fn import_benchmark() {
    let files: usize = std::env::var("HUSH_IMPORT_FILES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(100_000);
    let temp = tempfile::tempdir().unwrap();
    let source: PathBuf = std::env::var_os("HUSH_IMPORT_SOURCE").map_or_else(|| temp.path().join("source"), PathBuf::from);
    if !source.exists() {
        let start = Instant::now();
        generate(&source, files);
        println!("generated {files} files in {}", secs(start.elapsed()));
    }
    let rules: Vec<_> = Rule::defaults().iter().map(|r| r.compile().unwrap()).collect();

    // Scan, cold and warm.
    let cold = drop_caches();
    let start = Instant::now();
    let scan = import::scan(&source).unwrap();
    println!("scan ({}): {} files, {}", if cold { "cold" } else { "warm" }, scan.files.len(), secs(start.elapsed()));
    let start = Instant::now();
    import::scan(&source).unwrap();
    println!("scan (warm): {}", secs(start.elapsed()));

    // Prepare, one thread, by stage on the warm cache.
    let mut t_detect = Duration::ZERO;
    let mut t_read = Duration::ZERO;
    let mut t_clean = Duration::ZERO;
    for rel in &scan.files {
        let full = source.join(rel);
        let s = Instant::now();
        formats::detect(&full).unwrap();
        t_detect += s.elapsed();
        let s = Instant::now();
        let (text, _) = formats::decode(fs::read(&full).unwrap());
        t_read += s.elapsed();
        let s = Instant::now();
        clean::clean(&text);
        t_clean += s.elapsed();
    }
    println!("stages, 1 thread: detect {}, read+decode {}, clean {}", secs(t_detect), secs(t_read), secs(t_clean));

    let cold = drop_caches();
    let start = Instant::now();
    let mut prepared: Vec<Prepared> = Vec::with_capacity(scan.files.len());
    for rel in &scan.files {
        prepared.push(import::prepare(&source, rel, &rules).unwrap());
    }
    println!("prepare, 1 thread ({}): {}", if cold { "cold" } else { "warm" }, secs(start.elapsed()));
    let start = Instant::now();
    for rel in &scan.files {
        import::prepare(&source, rel, &rules).unwrap();
    }
    println!("prepare, 1 thread (warm): {}", secs(start.elapsed()));
    let bytes: usize = prepared.iter().map(|p| p.body.len() + p.original.len()).sum();
    println!("prepared held in memory: {} MB (body + original), {}", bytes >> 20, rss_mb());

    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    for cold in [true, false] {
        let cold = cold && drop_caches();
        let start = Instant::now();
        let chunk = scan.files.len().div_ceil(threads);
        std::thread::scope(|s| {
            for part in scan.files.chunks(chunk) {
                let (source, rules) = (&source, &rules);
                s.spawn(move || {
                    for rel in part {
                        import::prepare(source, rel, rules).unwrap();
                    }
                });
            }
        });
        println!("prepare, {threads} threads ({}): {}", if cold { "cold" } else { "warm" }, secs(start.elapsed()));
    }

    // Writes alone, on prepared notes, as `import::run` does them.
    write_only(&prepared);

    let base: usize = std::env::var("HUSH_IMPORT_BASE").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(0);
    let (_dir, vault_path) = if base > 0 {
        base_vault(&prepared, base, std::env::var_os("HUSH_IMPORT_BASE_VAULT").map(PathBuf::from))
    } else {
        let (dir, vault) = common::vault();
        vault.close().unwrap();
        let path = dir.path().join("vault.hush");
        (dir, path)
    };
    drop(prepared);

    // The whole flow as the app runs it: preview, then import, then preview again.
    let session = Session::default();
    session.unlock(Vault::open(&vault_path, common::PASS).unwrap(), vault_path.clone()).unwrap();
    let cold = drop_caches();
    let start = Instant::now();
    let preview = import::run(&session, &source, "Import", true, &AtomicBool::new(false), |_| {}).unwrap();
    println!(
        "run: preview ({}) {}, {} would be imported, samples {}, {}",
        if cold { "cold" } else { "warm" },
        secs(start.elapsed()),
        preview.imported,
        preview.samples.len(),
        rss_mb()
    );
    let json = serde_json::to_string(&preview).unwrap();
    println!("preview report JSON: {} KB", json.len() >> 10);
    // Meanwhile a stand-in for the UI asks for the vault every 20 ms, as typing in
    // search would, and records how long it waited.
    let start = Instant::now();
    let finished = AtomicBool::new(false);
    let (report, waits) = std::thread::scope(|s| {
        let probe = s.spawn(|| {
            let mut waits = Vec::new();
            while !finished.load(Ordering::Relaxed) {
                let s = Instant::now();
                session.with_vault(|_| Ok(())).unwrap();
                waits.push(s.elapsed());
                std::thread::sleep(Duration::from_millis(20));
            }
            waits
        });
        let report = import::run(&session, &source, "Import", false, &AtomicBool::new(false), |_| {}).unwrap();
        finished.store(true, Ordering::Relaxed);
        (report, probe.join().unwrap())
    });
    println!("run: import {}, {} imported, {}", secs(start.elapsed()), report.imported, rss_mb());
    println!("UI wait for the vault during import: {}", spread(waits));
    let dirty = session.with_vault(|v| v.embedding_queue_len()).unwrap();
    println!("embedding queue after import: {dirty}");
    let start = Instant::now();
    let again = import::run(&session, &source, "Import", true, &AtomicBool::new(false), |_| {}).unwrap();
    let json = serde_json::to_string(&again).unwrap();
    println!(
        "run: preview again {}, {} skipped (already exist), report JSON {} KB",
        secs(start.elapsed()),
        again.already_exist,
        json.len() >> 10
    );
}

fn write_only(prepared: &[Prepared]) {
    let (_dir, mut vault) = common::vault();
    vault.set_bulk_writes(true).unwrap();
    let start = Instant::now();
    let mut holds = Vec::new();
    for chunk in prepared.chunks(import::BATCH_SIZE) {
        let notes: Vec<NewNote> = chunk
            .iter()
            .map(|p| NewNote { folder: &p.folder, title: &p.title, body: &p.body, created_at: p.created_at, updated_at: p.updated_at })
            .collect();
        let mut written = 0;
        while written < notes.len() {
            let s = Instant::now();
            written += vault.import_notes_within(&notes[written..], import::INSERT_BUDGET).unwrap().len();
            holds.push(s.elapsed());
        }
    }
    println!("write {} notes: {} in {} transactions ({})", prepared.len(), secs(start.elapsed()), holds.len(), spread(holds));
    vault.set_bulk_writes(false).unwrap();
    let ids: Vec<i64> = (1..=prepared.len() as i64).collect();
    let s = Instant::now();
    let ambiguous = vault.ambiguous_links(&ids).unwrap();
    println!("ambiguous_links over imported ids: {} ({} found)", secs(s.elapsed()), ambiguous.len());
}
