//! Scale benchmark (issue #45): a synthetic vault of up to 1M notes over 50,000 folders,
//! measuring what the scale fixes are judged by. Ignored, so it only runs on request:
//!
//! ```text
//! cargo test --release --manifest-path src-tauri/Cargo.toml --test scale -- --ignored --nocapture
//! ```
//!
//! - `HUSH_SCALE_NOTES`: notes to generate (default 1,000,000; 100,000 for a CI-sized run).
//!   Folders are `notes / 20`, capped at 50,000.
//! - `HUSH_SCALE_VAULT`: keep the generated, indexed vault at this path and reuse it on the
//!   next run (generation takes a while at 1M). Without it, a temp dir is used.
//!
//! Embedding uses a fake 384-dimension embedder (words hashed into buckets), so the
//! numbers are the job's bookkeeping, not inference; the real model's throughput is
//! measured separately on 1,000 notes when `npm run fetch-model` has been run.

mod common;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hush_lib::db::{NewNote, NoteFilter, ReadFilter, SearchMode, SearchQuery, Vault};
use hush_lib::discover::{self, Options};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::onnx::OnnxEmbedder;
use hush_lib::embed::{self, EmbedError, Embedder, related};
use hush_lib::meaning::{self, QueryModels};
use hush_lib::session::Session;

const MODEL: &str = "fake-384";
const DIMS: usize = 384;
/// Bumped when the generator changes, so a kept vault from an older one isn't reused.
const GENERATOR: &str = "scale-v1";
const GENERATOR_SETTING: &str = "scale_benchmark";
const IMPORT_BATCH: usize = 500;
const MAX_FOLDERS: usize = 50_000;
const MAX_DEPTH: usize = 5;
const TOPICS: usize = 500;
const TOPIC_WORDS: usize = 40;
/// Share of a body's words drawn from its topic, so notes form groups by meaning.
const TOPIC_SHARE: f64 = 0.25;
/// The job's poll interval in the app (`commands/embeddings.rs`).
const POLL: Duration = Duration::from_secs(1);
const SAVES: usize = 3;
const LOOKUPS: usize = 50;
const SEARCH_RUNS: usize = 5;
const MODEL_SAMPLE: usize = 1_000;

/// SplitMix64: small, deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Rank in `0..n` with probability roughly proportional to `1 / (rank + 1)` (Zipf, s = 1),
    /// like word frequencies in text.
    fn zipf(&mut self, n: usize) -> usize {
        (((n as f64 + 1.0).powf(self.unit())) as usize).saturating_sub(1).min(n - 1)
    }
}

/// The most common English words first, so short prefixes (`t`, `th`, `the`) hit
/// frequent terms as they would in a real archive; then made-up words.
const COMMON: &[&str] = &[
    "the", "of", "and", "to", "a", "in", "is", "it", "you", "that", "he", "was", "for", "on", "are", "with", "as",
    "his", "they", "be", "at", "one", "have", "this", "from", "or", "had", "by", "not", "word", "but", "what", "some",
    "we", "can", "out", "other", "were", "all", "there", "when", "up", "use", "your", "how", "said", "an", "each",
    "she", "which", "do", "their", "time", "if", "will", "way", "about", "many", "then", "them", "would", "write",
    "like", "so", "these", "her", "long", "make", "thing", "see", "him", "two", "has", "look", "more", "day", "could",
    "go", "come", "did", "number", "sound", "no", "most", "people", "my", "over", "know", "water", "than", "call",
    "first", "who", "may", "down", "side", "been", "now", "find", "any", "new", "work", "part", "take", "get", "place",
    "made", "live", "where", "after", "back", "little", "only", "round", "man", "year", "came", "show", "every",
    "good", "me", "give", "our", "under", "name", "very", "through", "just", "form", "sentence", "great", "think",
    "say", "help", "low", "line", "differ", "turn", "cause", "much", "mean", "before", "move", "right", "boy", "old",
    "too", "same", "tell", "does", "set", "three", "want", "air", "well", "also", "play", "small", "end", "put",
    "home", "read", "hand", "port", "large", "spell", "add", "even", "land", "here", "must", "big", "high", "such",
    "follow", "act", "why", "ask", "men", "change", "went", "light", "kind", "off", "need", "house", "picture", "try",
    "again", "animal", "point", "mother", "world", "near", "build", "self", "earth", "father", "head", "stand", "own",
    "page", "should", "country", "found", "answer", "school", "grow", "study", "still", "learn", "plant", "cover",
    "food", "sun", "four", "between", "state", "keep", "eye", "never", "last", "let", "thought", "city", "tree",
    "cross", "farm", "hard", "start", "might", "story", "saw", "far", "sea", "draw", "left", "late", "run", "while",
    "press", "close", "night", "real", "life", "few", "north",
];
const VOCABULARY: usize = 30_000;

fn vocabulary() -> Vec<String> {
    const ONSETS: &[&str] = &["b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "st", "tr", "pl", "gr", "sh"];
    const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ou", "ee"];
    const CODAS: &[&str] = &["", "n", "r", "s", "t", "l", "nd", "st"];
    let mut words: Vec<String> = COMMON.iter().map(|w| w.to_string()).collect();
    let mut seen: std::collections::HashSet<String> = words.iter().cloned().collect();
    let mut rng = Rng(7);
    while words.len() < VOCABULARY {
        let syllables = 2 + rng.below(3);
        let word: String = (0..syllables)
            .map(|_| {
                format!("{}{}{}", ONSETS[rng.below(ONSETS.len())], VOWELS[rng.below(VOWELS.len())], CODAS[rng.below(CODAS.len())])
            })
            .collect();
        if seen.insert(word.clone()) {
            words.push(word);
        }
    }
    words
}

/// 50,000 folder paths (fewer for small vaults), up to 5 levels deep: each folder hangs
/// off the root or off an earlier folder that isn't at the maximum depth yet.
fn folders(count: usize, words: &[String], rng: &mut Rng) -> Vec<String> {
    let mut folders: Vec<(String, usize)> = Vec::with_capacity(count);
    for i in 0..count {
        let name = format!("{} {i}", words[COMMON.len() + rng.below(2_000)]);
        let parent = if folders.is_empty() || rng.unit() < 0.02 { None } else { Some(rng.below(folders.len())) };
        match parent.map(|p| &folders[p]) {
            Some((path, depth)) if *depth < MAX_DEPTH => {
                let entry = (format!("{path}/{name}"), depth + 1);
                folders.push(entry);
            }
            _ => folders.push((name, 1)),
        }
    }
    folders.into_iter().map(|(p, _)| p).collect()
}

struct Generator {
    words: Vec<String>,
    topics: Vec<Vec<usize>>,
    folders: Vec<String>,
    rng: Rng,
}

struct Generated {
    folder: usize,
    title: String,
    body: String,
    created: i64,
    updated: i64,
}

impl Generator {
    fn new(notes: usize) -> Self {
        let words = vocabulary();
        let mut rng = Rng(45);
        let topics = (0..TOPICS)
            .map(|_| (0..TOPIC_WORDS).map(|_| 1_000 + rng.below(VOCABULARY - 1_000)).collect())
            .collect();
        let folders = folders((notes / 20).clamp(1, MAX_FOLDERS), &words, &mut rng);
        Self { words, topics, folders, rng }
    }

    /// Note `i`: 100–1,500 words, skewed short (mean ≈ 300), a quarter of them from the
    /// note's topic; frontmatter with 0–3 tags.
    fn note(&mut self, i: usize) -> Generated {
        let rng = &mut self.rng;
        let topic = rng.below(TOPICS);
        let count = 100 + (1_400.0 * rng.unit().powi(6)) as usize;
        let mut body = String::with_capacity(count * 7 + 64);
        let tags = rng.below(4);
        if tags > 0 {
            let names: Vec<String> = (0..tags).map(|t| format!("t{}", (topic + t * 97) % 200)).collect();
            body.push_str(&format!("---\ntags: [{}]\n---\n", names.join(", ")));
        }
        for w in 0..count {
            let word = if rng.unit() < TOPIC_SHARE {
                &self.words[self.topics[topic][rng.below(TOPIC_WORDS)]]
            } else {
                &self.words[rng.zipf(VOCABULARY)]
            };
            body.push_str(word);
            body.push_str(match w % 60 {
                59 => ".\n",
                n if n % 12 == 11 => ". ",
                _ => " ",
            });
        }
        let title = format!("{} {} {i}", self.words[self.topics[topic][0]], self.words[COMMON.len() + rng.below(5_000)]);
        // 2005–2026, in ms.
        let created = 1_104_537_600_000 + (rng.unit() * 6.6e11) as i64;
        let updated = created + (rng.unit() * 3e10) as i64;
        let folder = rng.below(self.folders.len());
        Generated { folder, title, body, created, updated }
    }
}

/// Fast, deterministic: words hashed into `DIMS` buckets.
struct Fake;

impl Embedder for Fake {
    fn model_id(&self) -> &str {
        MODEL
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        let mut v = vec![0.0; DIMS];
        for word in text.split_whitespace() {
            let h = word.bytes().fold(2166136261u32, |h, b| (h ^ u32::from(b)).wrapping_mul(16777619));
            v[h as usize % DIMS] += 1.0;
        }
        v[0] += 0.01;
        embed::normalize(&mut v);
        Ok(v)
    }
}

/// The fake, counting passes: the job asks for its embedder once at the start of each.
#[derive(Default)]
struct Counting {
    passes: AtomicUsize,
    started: Mutex<Option<Instant>>,
}

impl Counting {
    fn passes(&self) -> usize {
        self.passes.load(Ordering::SeqCst)
    }

    fn last_start(&self) -> Option<Instant> {
        *self.started.lock().unwrap()
    }
}

impl EmbedderSource for Counting {
    fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        self.passes.fetch_add(1, Ordering::SeqCst);
        *self.started.lock().unwrap() = Some(Instant::now());
        Ok(Arc::new(Fake))
    }
}

impl QueryModels for Counting {
    fn query_embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(Arc::new(Fake))
    }

    fn query_loaded(&self, _model: &str) -> bool {
        true
    }
}

/// Resident memory from `/proc/self/status` in kB (`VmRSS` now, `VmHWM` peak). Linux only.
fn memory_kb(field: &str) -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with(field))?;
    line.split_whitespace().nth(1)?.parse().ok()
}

/// Resets the peak (`VmHWM`) to the current RSS. Linux only.
fn reset_peak() -> bool {
    std::fs::write("/proc/self/clear_refs", "5").is_ok()
}

fn mb(kb: u64) -> String {
    format!("{:.0} MB", kb as f64 / 1024.0)
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn secs(d: Duration) -> String {
    format!("{:.2} s", d.as_secs_f64())
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn machine() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|c| c.lines().find(|l| l.starts_with("model name")).map(|l| l.split(':').nth(1).unwrap_or("").trim().to_string()))
        .or_else(|| std::env::var("PROCESSOR_IDENTIFIER").ok())
        .unwrap_or_else(|| "unknown CPU".into());
    let ram = memory_from_meminfo().map_or("unknown RAM".into(), |kb| format!("{:.1} GB RAM", kb as f64 / 1024.0 / 1024.0));
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    format!("{} {}, {cpu}, {cores} cores available, {ram}", std::env::consts::OS, std::env::consts::ARCH)
}

fn memory_from_meminfo() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    info.lines().find(|l| l.starts_with("MemTotal"))?.split_whitespace().nth(1)?.parse().ok()
}

/// Locks the session when dropped, so a failed measurement stops the job thread
/// instead of leaving the test hanging.
struct LockOnDrop<'a>(&'a Session);

impl Drop for LockOnDrop<'_> {
    fn drop(&mut self) {
        self.0.lock();
    }
}

/// Waits until `f` holds, checking every 10 ms.
fn wait_until(limit: Duration, mut f: impl FnMut() -> bool) -> Duration {
    let start = Instant::now();
    while !f() {
        assert!(start.elapsed() < limit, "gave up after {limit:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
    start.elapsed()
}

/// Waits until the running job has started no pass for `quiet`.
fn wait_idle(source: &Counting, quiet: Duration) {
    loop {
        let since = source.last_start().map_or(quiet, |t| t.elapsed());
        if since >= quiet {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Creates (or reuses) the vault and fills it with `notes` synthetic notes.
fn prepare(path: &Path, notes: usize) -> Vault {
    let marker = format!("{GENERATOR}:{notes}");
    if path.exists() {
        let vault = Vault::open(path, common::PASS).unwrap();
        assert_eq!(
            vault.setting(GENERATOR_SETTING).unwrap().as_deref(),
            Some(marker.as_str()),
            "{} was made by another generator or size; delete it",
            path.display()
        );
        println!("reusing {}", path.display());
        return vault;
    }
    let mut vault = Vault::create(path, common::PASS).unwrap();
    vault.set_embedding_model(MODEL).unwrap();
    let mut generator = Generator::new(notes);
    println!("generating {notes} notes in {} folders", generator.folders.len());
    let start = Instant::now();
    let mut words = 0usize;
    let mut i = 0;
    while i < notes {
        let batch: Vec<Generated> = (i..(i + IMPORT_BATCH).min(notes)).map(|n| generator.note(n)).collect();
        words += batch.iter().map(|g| g.body.split_whitespace().count()).sum::<usize>();
        let new: Vec<NewNote> = batch
            .iter()
            .map(|g| NewNote {
                folder: &generator.folders[g.folder],
                title: &g.title,
                body: &g.body,
                created_at: g.created,
                updated_at: g.updated,
            })
            .collect();
        for outcome in vault.import_notes(&new).unwrap() {
            outcome.unwrap();
        }
        i += batch.len();
        if i % 100_000 == 0 || i == notes {
            println!("  {i} notes, {}", secs(start.elapsed()));
        }
    }
    println!("generated: {} ({} words per note on average)", secs(start.elapsed()), words / notes);
    vault.set_setting(GENERATOR_SETTING, &marker).unwrap();
    vault
}

#[test]
#[ignore = "scale benchmark: minutes to hours; run with --ignored --nocapture"]
fn scale_benchmark() {
    let notes: usize = std::env::var("HUSH_SCALE_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(1_000_000);
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = std::env::var_os("HUSH_SCALE_VAULT").map_or_else(|| temp.path().join("scale.hush"), PathBuf::from);
    println!("machine: {}", machine());

    let vault = prepare(&path, notes);
    let session = Session::default();
    let g = session.unlock(vault, path.clone()).unwrap();
    let ids = session.with_vault(|v| v.count_in_folder("")).unwrap();
    assert_eq!(ids, notes);

    // Index the whole vault (or check a kept one), as the job's first passes after unlock do.
    let source = Counting::default();
    let start = Instant::now();
    loop {
        let pass = job::pass(&session, g, &source, &mut |_| {}).unwrap();
        if pass.stale == 0 {
            break;
        }
        println!("  pass embedded {} notes", pass.stale);
    }
    println!("indexing with the fake embedder: {}", secs(start.elapsed()));
    let file = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    println!("vault file: {:.2} GB", file as f64 / 1e9);

    // 2. A pass that finds nothing to do: what every change costs the idle job.
    let mut idle = Vec::new();
    for _ in 0..3 {
        let start = Instant::now();
        assert_eq!(job::pass(&session, g, &source, &mut |_| {}).unwrap().stale, 0);
        idle.push(start.elapsed());
    }
    let idle_pass = median(idle);

    // 1, 2. Saves with the job running as in the app.
    let quiet = idle_pass * 3 + POLL * 3;
    let mut rng = Rng(46);
    let run = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let mut latencies = Vec::new();
    let mut passes_per_save = Vec::new();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, POLL, |_| {}));
        let _stop = LockOnDrop(&session);
        wait_idle(&source, quiet);
        for n in 0..SAVES {
            let id = 1 + rng.below(notes) as i64;
            let before = session.with_vault(|v| v.note_embedding(id)).unwrap();
            let passes = source.passes();
            // Unique per run, so a kept vault saved by an earlier run still changes.
            let text = format!("A new text for the benchmark, save {n} of run {run}, about lighthouses.");
            session.with_vault(|v| v.update_body(id, &text)).unwrap();
            latencies.push(wait_until(Duration::from_secs(3_600), || {
                session.with_vault_quiet(g, |v| v.note_embedding(id)).unwrap() != before
            }));
            wait_idle(&source, quiet);
            passes_per_save.push(source.passes() - passes);
        }
        session.lock();
    });

    // 3, 5. Unlock to related notes, and peak memory on the way.
    let probe = 1 + rng.below(notes) as i64;
    let baseline = memory_kb("VmRSS:");
    let peak_reset = reset_peak();
    let start = Instant::now();
    let vault = Vault::open(&path, common::PASS).unwrap();
    let g = session.unlock(vault, path.clone()).unwrap();
    let source = Counting::default();
    let mut unlock_to_related = Duration::ZERO;
    let mut lookups = Vec::new();
    let mut searches: Vec<(String, Duration, Duration, usize)> = Vec::new();
    let mut discover_time = Duration::ZERO;
    let mut discover_peak = None;
    let mut discovered = (0, 0);
    let mut meaning_runs: Vec<(String, Duration, Duration, usize)> = Vec::new();
    let mut allowed_ids: Vec<(String, Duration, usize)> = Vec::new();
    let mut peak = None;
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, POLL, |_| {}));
        let _stop = LockOnDrop(&session);
        wait_until(Duration::from_secs(3_600), || matches!(related::related_notes(&session, probe, 10), Ok(Some(_))));
        unlock_to_related = start.elapsed();
        peak = memory_kb("VmHWM:");
        wait_idle(&source, quiet);

        // 4. Related notes for notes picked at random.
        for _ in 0..LOOKUPS {
            let id = 1 + rng.below(notes) as i64;
            let start = Instant::now();
            related::related_notes(&session, id, 10).unwrap();
            lookups.push(start.elapsed());
        }

        // 7. Search as the user types: 1-, 2- and 3-character prefixes and two words.
        let words = vocabulary();
        let two = format!("{} {}", words[30], words[1_200]);
        for text in ["t", "th", "the", two.as_str()] {
            let query = SearchQuery { text: text.into(), ..Default::default() };
            let mut runs = Vec::new();
            let mut hits = 0;
            for _ in 0..SEARCH_RUNS {
                let start = Instant::now();
                hits = session.with_vault(|v| v.search(&query)).unwrap().len();
                runs.push(start.elapsed());
            }
            let worst = *runs.iter().max().unwrap();
            searches.push((text.to_string(), median(runs), worst, hits));
        }

        // 8. Discover tags with the default number of groups.
        let before = memory_kb("VmRSS:");
        reset_peak();
        let start = Instant::now();
        let d = discover::run(&session, g, &Options::default(), &|| false).unwrap();
        discover_time = start.elapsed();
        discover_peak = memory_kb("VmHWM:").zip(before).map(|(p, b)| p.saturating_sub(b));
        discovered = (d.notes, d.groups.len());

        // 9. Search by meaning (#146): the fake embeds the query, so this is the index
        // scan, the exact rerank, the words side and the fusion; the real model's query
        // embedding is measured below. Vault-wide, then in a folder holding about 10%
        // of the notes (the top-level folder closest to that).
        let folders = session.with_vault(|v| v.folders()).unwrap();
        let mut tops: Vec<&str> = folders.iter().map(|f| f.split('/').next().unwrap()).collect();
        tops.sort_unstable();
        tops.dedup();
        let tenth = tops
            .iter()
            .map(|f| (session.with_vault(|v| v.count_in_folder(f)).unwrap(), *f))
            .min_by_key(|(n, _)| n.abs_diff(notes / 10))
            .unwrap();
        let meaning_query = |folder: Option<&str>, read: Option<ReadFilter>| SearchQuery {
            text: two.clone(),
            mode: SearchMode::Meaning,
            folder: folder.map(Into::into),
            read_status: read,
            ..Default::default()
        };
        let cases = [
            ("vault-wide".to_string(), meaning_query(None, None)),
            (format!("folder {:?} ({} notes, {:.0}%)", tenth.1, tenth.0, tenth.0 as f64 * 100.0 / notes as f64), meaning_query(Some(tenth.1), None)),
        ];
        for (label, query) in &cases {
            let mut runs = Vec::new();
            let mut hits = 0;
            for _ in 0..SEARCH_RUNS {
                let start = Instant::now();
                let page = meaning::search(&session, &source, query).unwrap().unwrap();
                runs.push(start.elapsed());
                assert!(page.by_meaning, "{:?}", page.words_only);
                hits = page.hits.len();
            }
            let worst = *runs.iter().max().unwrap();
            meaning_runs.push((label.clone(), median(runs), worst, hits));
        }
        // The allowed-id query alone, for the folder and for a filter nearly every note passes.
        for (label, query) in [
            (format!("folder {:?}", tenth.1), meaning_query(Some(tenth.1), None)),
            ("read status \"not finished\" (nearly every note)".to_string(), meaning_query(None, Some(ReadFilter::NotFinished))),
        ] {
            let mut runs = Vec::new();
            let mut count = 0;
            for _ in 0..SEARCH_RUNS {
                let start = Instant::now();
                count = match session.with_vault(|v| v.search_filter_ids(&query)).unwrap().unwrap() {
                    NoteFilter::Only(ids) => ids.len(),
                    NoteFilter::Except(ids) => ids.len(),
                };
                runs.push(start.elapsed());
            }
            allowed_ids.push((label, median(runs), count));
        }
        session.lock();
    });
    let query_embedding = real_query_embedding();

    // 6. The real model, if its files are there.
    let throughput = real_model_throughput(&path, notes);

    lookups.sort();
    println!();
    println!("== Hush scale benchmark: {notes} notes, {} folders ==", (notes / 20).clamp(1, MAX_FOLDERS));
    println!("machine: {}", machine());
    println!("1. save-to-vector latency: {} (each: {})", secs(median(latencies.clone())), latencies.iter().map(|d| secs(*d)).collect::<Vec<_>>().join(", "));
    println!("2. idle job: {} full passes per save ({passes_per_save:?}); a pass with nothing to do takes {}", passes_per_save.iter().max().unwrap(), secs(idle_pass));
    println!("3. unlock-to-related latency: {} (open + KDF + first pass + index load)", secs(unlock_to_related));
    println!("4. related_notes: median {}, worst {} over {LOOKUPS} lookups", ms(lookups[LOOKUPS / 2]), ms(lookups[LOOKUPS - 1]));
    match (peak_reset, peak, baseline) {
        (true, Some(p), Some(b)) => println!("5. peak RSS during unlock + index load: {} ({} above the {} before unlock)", mb(p), mb(p.saturating_sub(b)), mb(b)),
        _ => println!("5. peak RSS: not measured on this OS (needs /proc/self/clear_refs)"),
    }
    match throughput {
        Some(t) => println!(
            "6. real model: {:.1} notes/s ({} notes in {}); {:.2} chunks and {:.0} tokens per note",
            t.notes as f64 / t.time.as_secs_f64(),
            t.notes,
            secs(t.time),
            t.chunks as f64 / t.notes as f64,
            t.tokens as f64 / t.notes as f64,
        ),
        None => println!("6. real model: SKIPPED (model files missing; run `npm run fetch-model`)"),
    }
    for (text, median, worst, hits) in &searches {
        println!("7. search {text:?}: median {}, worst {} ({hits} hits shown)", ms(*median), ms(*worst));
    }
    println!(
        "8. discover tags: {} for {} notes into {} groups{}",
        secs(discover_time),
        discovered.0,
        discovered.1,
        discover_peak.map_or(String::new(), |p| format!(", peak RSS {} above the start", mb(p)))
    );
    for (label, median, worst, hits) in &meaning_runs {
        println!("9. meaning search, {label}: median {}, worst {} ({hits} hits shown; query embedding not included)", ms(*median), ms(*worst));
    }
    for (label, median, count) in &allowed_ids {
        println!("9. allowed ids for {label}: median {} ({count} ids listed)", ms(*median));
    }
    match query_embedding {
        Some((median, worst)) => println!("9. real model query embedding (own 2-thread CPU session): median {}, worst {}", ms(median), ms(worst)),
        None => println!("9. real model query embedding: SKIPPED (model files missing; run `npm run fetch-model`)"),
    }
}

/// How long the bundled model takes to embed a search query on a 2-thread CPU session
/// of its own, as the app's query session runs it; `None` without the files.
fn real_query_embedding() -> Option<(Duration, Duration)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let d = embed::descriptor(embed::DEFAULT_MODEL)?;
    if !dir.join(d.model.name).is_file() {
        return None;
    }
    let model = OnnxEmbedder::load_with_threads(&dir, d, 2).ok()?;
    model.embed_query("warm up").ok()?;
    let mut runs: Vec<Duration> = ["times I felt stuck at work", "ferry timetable", "budget review with the lighthouse keepers"]
        .iter()
        .cycle()
        .take(30)
        .map(|q| {
            let start = Instant::now();
            model.embed_query(q).unwrap();
            start.elapsed()
        })
        .collect();
    runs.sort();
    Some((runs[runs.len() / 2], runs[runs.len() - 1]))
}

struct Throughput {
    notes: usize,
    time: Duration,
    chunks: usize,
    tokens: usize,
}

/// The bundled model on the first 1,000 notes in the job's batches, and their size in
/// chunks and tokens; `None` without the files.
fn real_model_throughput(path: &Path, notes: usize) -> Option<Throughput> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let d = embed::descriptor(embed::DEFAULT_MODEL).unwrap();
    if !dir.join(d.model.name).is_file() || !dir.join(d.tokenizer.name).is_file() {
        return None;
    }
    let model = OnnxEmbedder::load(&dir, d).unwrap();
    let vault = Vault::open(path, common::PASS).unwrap();
    let ids: Vec<i64> = (1..=MODEL_SAMPLE.min(notes) as i64).collect();
    let texts = vault.embedding_texts(&ids).unwrap();
    drop(vault);
    let texts: Vec<&str> = texts.iter().map(|n| n.text.as_str()).collect();
    let (mut chunks, mut tokens) = (0, 0);
    for text in &texts {
        let lengths = model.chunk_lengths(text).unwrap();
        chunks += lengths.len();
        tokens += lengths.iter().sum::<usize>();
    }
    let start = Instant::now();
    for batch in texts.chunks(job::BATCH) {
        model.embed_batch(batch, &|| false).unwrap();
    }
    Some(Throughput { notes: texts.len(), time: start.elapsed(), chunks, tokens })
}

/// The generator itself, so a change to it shows up without running the benchmark.
/// The tag re-derive pass (#122) over the generated vault: the whole pass as unlock
/// runs it, and the longest it holds the vault for one page. Same environment variables
/// as [`scale_benchmark`]; the vault is marked as needing the pass first.
#[test]
#[ignore = "scale benchmark: minutes; run with --ignored --nocapture"]
fn retag_benchmark() {
    let notes: usize = std::env::var("HUSH_SCALE_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(1_000_000);
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = std::env::var_os("HUSH_SCALE_VAULT").map_or_else(|| temp.path().join("scale.hush"), PathBuf::from);
    println!("machine: {}", machine());
    let vault = prepare(&path, notes);
    vault.delete_setting(hush_lib::db::TAG_RULES_SETTING).unwrap();
    vault.delete_setting(hush_lib::db::TAG_RULES_PROGRESS_SETTING).unwrap();
    let session = Session::default();
    let g = session.unlock(vault, path.clone()).unwrap();

    // Page by page, timing each vault access.
    let mut pages = Vec::new();
    let mut after = 0;
    let start = Instant::now();
    loop {
        let t = Instant::now();
        let page = session.with_vault_quiet(g, |v| v.retag_page(after, hush_lib::retag::PAGE)).unwrap();
        pages.push(t.elapsed());
        match page.next {
            Some(next) => after = next,
            None => break,
        }
    }
    let end = Instant::now();
    session.with_vault_quiet(g, |v| v.finish_retag()).unwrap();
    let total = start.elapsed();
    let longest = pages.iter().max().copied().unwrap_or_default();
    println!(
        "retag pages: {} pages of {}, all {} (finish {}), median page {}, longest {}",
        pages.len(),
        hush_lib::retag::PAGE,
        secs(total),
        ms(end.elapsed()),
        ms(median(pages.clone())),
        ms(longest)
    );

    // The job itself, with its pauses, on the same vault marked stale again.
    session.with_vault(|v| v.delete_setting(hush_lib::db::TAG_RULES_SETTING)).unwrap();
    let start = Instant::now();
    let done = hush_lib::retag::run(&session, g, Duration::from_secs(1)).unwrap().unwrap();
    println!("retag job: {} notes checked, {} changed, {}", done.checked, done.changed, secs(start.elapsed()));
}

/// Listing folders for the folder tree (#129): from the `folders` table, so it should
/// follow the number of folders, not notes. The first call after opening reads cold
/// pages. Same environment variables as [`scale_benchmark`]; a kept vault from before
/// the table gets it on open, so the open time then includes the migration's backfill.
#[test]
#[ignore = "scale benchmark: minutes; run with --ignored --nocapture"]
fn folders_benchmark() {
    let notes: usize = std::env::var("HUSH_SCALE_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(1_000_000);
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = std::env::var_os("HUSH_SCALE_VAULT").map_or_else(|| temp.path().join("scale.hush"), PathBuf::from);
    println!("machine: {}", machine());
    prepare(&path, notes).close().unwrap();
    let start = Instant::now();
    let vault = Vault::open(&path, common::PASS).unwrap();
    println!("open (KDF, and the migration if the vault predates it): {}", ms(start.elapsed()));
    let mut times = Vec::new();
    let mut listed = 0;
    for _ in 0..5 {
        let t = Instant::now();
        listed = vault.folders().unwrap().len();
        times.push(t.elapsed());
    }
    println!(
        "folders(): {listed} folders, first {}, median {}",
        ms(times[0]),
        ms(median(times.clone()))
    );
    let folders_median = median(times);

    // Sizes for the tree (#133): the same rows with their stored byte totals, so the
    // same budget as the list itself, not a sum over note bodies.
    let mut times = Vec::new();
    let mut sizes = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        sizes = vault.folder_sizes().unwrap();
        times.push(t.elapsed());
    }
    let sizes_median = median(times.clone());
    let total: u64 = sizes.iter().map(|s| s.bytes).sum();
    println!(
        "folder_sizes(): {} folders, {total} bytes in all, first {}, median {}",
        sizes.len(),
        ms(times[0]),
        ms(sizes_median)
    );
    assert_eq!(sizes.len(), listed);
    assert!(
        sizes_median <= folders_median * 3 + Duration::from_millis(20),
        "folder_sizes() {} should cost about what folders() {} does",
        ms(sizes_median),
        ms(folders_median)
    );

    // A page of the biggest folder's notes, each with its size measured from the body.
    let biggest = sizes.iter().max_by_key(|s| s.bytes).map(|s| s.path.clone()).unwrap_or_default();
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        let page = vault.folder_notes(&biggest, 0, 500).unwrap();
        times.push(t.elapsed());
        assert!(page.iter().all(|n| n.size.is_some()));
    }
    println!("folder_notes() page of 500 with sizes in {biggest:?}: first {}, median {}", ms(times[0]), ms(median(times.clone())));
    assert!(median(times) < Duration::from_millis(250), "a tree page stays well under a frame budget's worth of waiting");
}

/// What the UI waits for while the background jobs start up after unlock: every job
/// the app starts on unlock runs (embedding job with the related-index rebuild and
/// safety net, tag training, tag re-derive), and the foreground opens a folder page
/// every 100 ms, timing how long it waits for the vault. Same environment variables as
/// [`scale_benchmark`], plus `HUSH_CONTENTION_SECS` (default 120): how long to watch.
#[test]
#[ignore = "scale benchmark: minutes; run with --ignored --nocapture"]
fn unlock_contention_benchmark() {
    let notes: usize = std::env::var("HUSH_SCALE_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(1_000_000);
    let watch = Duration::from_secs(std::env::var("HUSH_CONTENTION_SECS").ok().and_then(|n| n.parse().ok()).unwrap_or(120));
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = std::env::var_os("HUSH_SCALE_VAULT").map_or_else(|| temp.path().join("scale.hush"), PathBuf::from);
    println!("machine: {}", machine());
    let vault = prepare(&path, notes);
    let session = Session::default();
    let g = session.unlock(vault, path.clone()).unwrap();
    let source = Counting::default();
    let start = Instant::now();
    while job::pass(&session, g, &source, &mut |_| {}).unwrap().stale > 0 {}
    println!("indexed (or checked) with the fake embedder: {}", secs(start.elapsed()));
    let folder = session.with_vault(|v| v.folders()).unwrap().into_iter().nth(1).unwrap_or_default();
    // As after the #122 rules change: the re-derive pass runs on unlock.
    session.with_vault(|v| {
        v.delete_setting(hush_lib::db::TAG_RULES_SETTING)?;
        v.delete_setting(hush_lib::db::TAG_RULES_PROGRESS_SETTING)
    })
    .unwrap();
    session.lock();

    let start = Instant::now();
    let vault = Vault::open(&path, common::PASS).unwrap();
    let opened = start.elapsed();
    let g = session.unlock(vault, path.clone()).unwrap();
    println!("open {}, unlock {}", ms(opened), ms(start.elapsed()));
    let done = |what: &str, since: Instant| println!("  {what} done after {}", secs(since.elapsed()));
    let mut waits = Vec::new();
    let source = Counting::default();
    let unlocked = Instant::now();
    std::thread::scope(|s| {
        s.spawn(|| job::run(&session, g, &source, POLL, |_| {}));
        s.spawn(|| {
            if hush_lib::classify::job::pass(&session, g).is_ok() {
                done("tag training pass", unlocked);
            }
        });
        s.spawn(|| {
            if hush_lib::retag::run(&session, g, POLL).is_ok() {
                done("tag re-derive", unlocked);
            }
        });
        let _stop = LockOnDrop(&session);
        let mut related_seen = false;
        while unlocked.elapsed() < watch {
            let t = Instant::now();
            session.with_vault(|v| v.folder_notes(&folder, 0, 200)).unwrap();
            waits.push(t.elapsed());
            if !related_seen && session.related_index().as_ref().is_some_and(|i| i.len() > 0) {
                related_seen = true;
                done("related index rebuild", unlocked);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    });
    waits.sort();
    let pct = |p: f64| waits[((waits.len() - 1) as f64 * p) as usize];
    println!(
        "folder page over {}: {} calls, median {}, p95 {}, p99 {}, max {}, {} over 1 s",
        secs(watch),
        waits.len(),
        ms(pct(0.5)),
        ms(pct(0.95)),
        ms(pct(0.99)),
        ms(*waits.last().unwrap()),
        waits.iter().filter(|w| **w > Duration::from_secs(1)).count()
    );
}

#[test]
fn generator_is_deterministic_and_shaped_as_specified() {
    let mut a = Generator::new(20_000);
    let mut b = Generator::new(20_000);
    assert_eq!(a.folders.len(), 1_000);
    assert!(a.folders.iter().all(|f| f.matches('/').count() < MAX_DEPTH));
    assert!(a.folders.iter().any(|f| f.matches('/').count() == MAX_DEPTH - 1), "some folders are 5 levels deep");
    let mut words = 0;
    for i in 0..2_000 {
        let (x, y) = (a.note(i), b.note(i));
        assert_eq!((&x.title, &x.body, x.folder), (&y.title, &y.body, y.folder));
        let n = x.body.lines().filter(|l| !l.starts_with("---") && !l.starts_with("tags:")).map(|l| l.split_whitespace().count()).sum::<usize>();
        assert!((100..=1_500).contains(&n), "{n} words");
        words += n;
    }
    let mean = words / 2_000;
    assert!((250..=350).contains(&mean), "mean {mean} words");
}

/// Compacting a vault (#149): a generated vault loses 60% of its notes to the trash, the
/// trash is emptied, and the vault is compacted. Prints the sizes, the time and the peak
/// memory, then times an emptied trash on the compacted vault (incremental vacuum).
/// `HUSH_SCALE_NOTES` sets the size (default 100,000 notes, a few hundred MB); the vault
/// is always a fresh temp file, since the run deletes most of it.
#[test]
#[ignore = "scale benchmark: minutes; run with --ignored --nocapture"]
fn compact_benchmark() {
    let notes: usize = std::env::var("HUSH_SCALE_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(100_000);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("compact.hush");
    println!("machine: {}", machine());
    let mut vault = prepare(&path, notes);
    let ids: Vec<i64> = (1..=notes as i64).collect();
    let size = |p: &Path| {
        let wal = PathBuf::from(format!("{}-wal", p.display()));
        std::fs::metadata(p).map_or(0, |m| m.len()) + std::fs::metadata(wal).map_or(0, |m| m.len())
    };
    let gb = |b: u64| format!("{:.2} GB", b as f64 / 1e9);
    println!("generated vault: {}", gb(size(&path)));

    let start = Instant::now();
    let gone: Vec<i64> = ids.iter().copied().filter(|id| id % 5 < 3).collect();
    for chunk in gone.chunks(10_000) {
        vault.trash_notes(chunk).unwrap();
    }
    let emptied = vault.empty_trash().unwrap();
    println!("trashed and emptied {emptied} notes: {}", secs(start.elapsed()));
    let space = vault.space().unwrap();
    println!(
        "before: file {}, reclaimable {}, needs {} free, worth compacting: {}",
        gb(space.file_bytes),
        gb(space.reclaimable_bytes),
        gb(space.needed_bytes),
        space.worth_compacting
    );

    let peak_reset = reset_peak();
    let rss_before = memory_kb("VmRSS").unwrap_or(0);
    let start = Instant::now();
    let done = vault.compact(&hush_lib::db::available_space).unwrap();
    let took = start.elapsed();
    println!(
        "compact: {} -> {} in {} ({:.0} MB/s of the file before); RSS before {}, peak {}{}",
        gb(done.before_bytes),
        gb(done.after_bytes),
        secs(took),
        done.before_bytes as f64 / 1e6 / took.as_secs_f64(),
        mb(rss_before),
        memory_kb("VmHWM").map_or("?".into(), mb),
        if peak_reset { "" } else { " (peak since process start)" }
    );
    assert!(done.after_bytes < done.before_bytes);

    // On the compacted vault, an emptied trash gives the space back at once.
    let more: Vec<i64> = ids.iter().copied().filter(|id| id % 5 == 3).collect();
    let before = size(&path);
    for chunk in more.chunks(10_000) {
        vault.trash_notes(chunk).unwrap();
    }
    let start = Instant::now();
    let emptied = vault.empty_trash().unwrap();
    println!(
        "then empty_trash of {emptied} notes with incremental vacuum: {} -> {} in {}",
        gb(before),
        gb(size(&path)),
        secs(start.elapsed())
    );
    vault.close().unwrap();
    let start = Instant::now();
    let vault = Vault::open(&path, common::PASS).unwrap();
    println!("reopen: {}", ms(start.elapsed()));
    assert_eq!(vault.live_note_count().unwrap(), notes - gone.len() - more.len());
}
