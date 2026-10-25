//! Import benchmark: where the time goes when importing a large folder.
//!
//! `cargo test --release --test import_scale -- --ignored --nocapture`
//! - `HUSH_IMPORT_FILES`: files to generate (default 100,000).
//! - `HUSH_IMPORT_SOURCE`: keep the generated folder here and reuse it.

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use hush_lib::db::NewNote;
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

    // Writes alone, on prepared notes, by batch of 500.
    {
        let (_dir, mut vault) = common::vault();
        let start = Instant::now();
        let mut times = Vec::new();
        for chunk in prepared.chunks(import::BATCH_SIZE) {
            let s = Instant::now();
            let notes: Vec<NewNote> = chunk
                .iter()
                .map(|p| NewNote { folder: &p.folder, title: &p.title, body: &p.body, created_at: p.created_at, updated_at: p.updated_at })
                .collect();
            for outcome in vault.import_notes(&notes).unwrap() {
                let _ = outcome;
            }
            times.push(s.elapsed());
        }
        let n = times.len();
        println!(
            "write {} notes: {} (first batch {} ms, last batch {} ms)",
            prepared.len(),
            secs(start.elapsed()),
            times[0].as_millis(),
            times[n - 1].as_millis()
        );
        let ids: Vec<i64> = (1..=prepared.len() as i64).collect();
        let s = Instant::now();
        let ambiguous = vault.ambiguous_links(&ids).unwrap();
        println!("ambiguous_links over imported ids: {} ({} found)", secs(s.elapsed()), ambiguous.len());
    }
    drop(prepared);

    // The whole flow as the app runs it: preview, then import, then preview again.
    let (dir, vault) = common::vault();
    let session = Session::default();
    session.unlock(vault, dir.path().join("vault.hush")).unwrap();
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
    let start = Instant::now();
    let report = import::run(&session, &source, "Import", false, &AtomicBool::new(false), |_| {}).unwrap();
    println!("run: import {}, {} imported, {}", secs(start.elapsed()), report.imported, rss_mb());
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
