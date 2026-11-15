//! Note-opening benchmark: what the backend does when the user switches notes, on a
//! synthetic vault shaped like a linked notes archive. Ignored, so it only runs on request:
//!
//! ```text
//! cargo test --release --manifest-path src-tauri/Cargo.toml --test open_note -- --ignored --nocapture
//! ```
//!
//! - `HUSH_OPEN_NOTES`: notes to generate (default and maximum 10,000).
//! - `HUSH_OPEN_VAULT`: keep the generated, indexed vault at this path and reuse it.
//!
//! A switch is the commands the frontend sends for it, in its order: `open_note` (which
//! the editor waits for), then the sidebar's recent list and the details panel's tag
//! suggestions, backlinks, outgoing links and related notes. They all take the one
//! vault, so a slow panel query of the note just left delays the next note's
//! `open_note`: "next open" is what a quick second click waits for.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use hush_lib::db::{NewNote, Vault};
use hush_lib::embed::job::{self, EmbedderSource};
use hush_lib::embed::{self, EmbedError, Embedder, related};
use hush_lib::markdown;
use hush_lib::session::Session;

const MODEL: &str = "fake-384";
const DIMS: usize = 384;
const MAX_NOTES: usize = 10_000;
/// Bumped when the generator changes, so a kept vault from an older one isn't reused.
const GENERATOR: &str = "open-v1";
const GENERATOR_SETTING: &str = "open_benchmark";
const IMPORT_BATCH: usize = 500;
const HUBS: usize = 5;
const TAGS: usize = 40;
const SAMPLES: usize = 40;

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

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn word(rng: &mut Rng) -> String {
    const ONSETS: &[&str] = &["b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "st", "tr", "pl", "gr", "sh"];
    const VOWELS: &[&str] = &["a", "e", "i", "o", "u", "ai", "ou", "ee"];
    const CODAS: &[&str] = &["", "n", "r", "s", "t", "l", "nd", "st"];
    (0..1 + rng.below(3))
        .map(|_| format!("{}{}{}", ONSETS[rng.below(ONSETS.len())], VOWELS[rng.below(VOWELS.len())], CODAS[rng.below(CODAS.len())]))
        .collect()
}

/// Note `i` (0-based) of `notes`. The first [`HUBS`] are hubs: a person or project
/// note that many others link to (hub 0 from 15% of the vault, each next from half as
/// many). One in 100 is titled `Notes`, in its own folder, as a yearly journal has.
fn title(i: usize) -> String {
    if i < HUBS {
        format!("Hub {i}")
    } else if i % 100 == 99 {
        "Notes".into()
    } else {
        format!("Note {i:05}")
    }
}

fn folder(i: usize, notes: usize) -> String {
    let folders = (notes / 50).max(1);
    // The `Notes` notes each get a folder of their own: a folder holds a title once.
    let f = if i % 100 == 99 { (i / 100) % folders } else { i % folders };
    format!("area {}/year {}/folder {f}", f % 7, 2000 + f % 25)
}

/// 100-400 words for most notes, 5,000 for one in 20, 60,000 (about 400 kB) for one in
/// 200; frontmatter tags, inline tags, 0-6 links to other notes, links to the hubs and
/// to a `Notes` note (ambiguous unless written with its folder).
fn body(i: usize, notes: usize, rng: &mut Rng) -> String {
    let words = match i {
        _ if i % 200 == 7 => 60_000,
        _ if i % 20 == 3 => 5_000,
        _ => 100 + rng.below(300),
    };
    let mut body = String::with_capacity(words * 7 + 256);
    let tags = rng.below(4);
    if tags > 0 {
        let names: Vec<String> = (0..tags).map(|t| format!("t{}", (i + t * 13) % TAGS)).collect();
        body.push_str(&format!("---\ntags: [{}]\nauthor: Author {}\n---\n", names.join(", "), i % 3));
    }
    let mut links: Vec<String> = (0..rng.below(7)).map(|_| format!("[[{}]]", title(HUBS + rng.below(notes - HUBS)))).collect();
    for hub in 0..HUBS {
        if rng.unit() < 0.15 / f64::from(1u32 << hub) {
            links.push(format!("[[Hub {hub}|the hub]]"));
        }
    }
    if rng.unit() < 0.03 {
        links.push("[[Notes]]".into());
    }
    if rng.unit() < 0.03 {
        links.push(format!("[[folder {}/Notes]]", rng.below((notes / 50).max(1))));
    }
    if rng.unit() < 0.05 {
        links.push(format!("[[Missing {}]]", rng.below(50)));
    }
    for w in 0..words {
        body.push_str(&word(rng));
        // The links are spread over the text.
        if !links.is_empty() && w % (words / links.len()).max(1) == 5 {
            body.push(' ');
            body.push_str(&links.pop().unwrap());
        }
        body.push_str(match w % 60 {
            59 => ".\n\n",
            n if n % 12 == 11 => ". ",
            _ => " ",
        });
    }
    for link in links {
        body.push_str(&link);
        body.push(' ');
    }
    if rng.unit() < 0.3 {
        body.push_str(&format!("\n#t{}\n", rng.below(TAGS)));
    }
    body
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

struct Source;

impl EmbedderSource for Source {
    fn embedder(&self, _model: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(Arc::new(Fake))
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
    let mut rng = Rng(193);
    let start = Instant::now();
    let mut i = 0;
    while i < notes {
        let batch: Vec<(String, String, String)> =
            (i..(i + IMPORT_BATCH).min(notes)).map(|n| (folder(n, notes), title(n), body(n, notes, &mut rng))).collect();
        let new: Vec<NewNote> = batch
            .iter()
            .enumerate()
            .map(|(n, (folder, title, body))| NewNote {
                folder,
                title,
                body,
                created_at: 1_104_537_600_000 + (i + n) as i64 * 60_000_000,
                updated_at: 1_104_537_600_000 + (i + n) as i64 * 60_000_000,
            })
            .collect();
        for outcome in vault.import_notes(&new).unwrap() {
            outcome.unwrap();
        }
        i += batch.len();
    }
    // Tags with a description are suggested for every note that opens.
    for t in 0..TAGS {
        vault.set_tag_description(&format!("t{t}"), &format!("notes about {} and {}", word(&mut rng), word(&mut rng))).unwrap();
    }
    println!("generated {notes} notes in {:.1} s", start.elapsed().as_secs_f64());
    vault.set_setting(GENERATOR_SETTING, &marker).unwrap();
    vault
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1e3)
}

/// One switch to note `id`, as the frontend sends it: each step's time.
#[derive(Default, Clone, Copy)]
struct Switch {
    open: Duration,
    recent: Duration,
    suggestions: Duration,
    backlinks: Duration,
    outgoing: Duration,
    related: Duration,
}

impl Switch {
    /// Everything after `open_note`: what a click on another note right away waits for.
    fn panels(&self) -> Duration {
        self.recent + self.suggestions + self.backlinks + self.outgoing + self.related
    }
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, Duration) {
    let start = Instant::now();
    let out = f();
    (out, start.elapsed())
}

fn switch(session: &Session, id: i64) -> Switch {
    let mut s = Switch::default();
    s.open = timed(|| {
        session
            .with_vault(|v| {
                let note = v.note(id)?;
                let meta = markdown::parse(&note.body);
                v.record_opened(id)?;
                // As the command does: the note goes to the webview as JSON.
                Ok(serde_json::to_string(&(note, meta.tags)).unwrap().len())
            })
            .unwrap()
    })
    .1;
    s.recent = timed(|| session.with_vault(|v| v.recent_opened_notes()).unwrap()).1;
    s.suggestions = timed(|| session.with_vault(|v| v.tag_suggestions(id)).unwrap()).1;
    s.backlinks = timed(|| session.with_vault(|v| v.backlinks(id)).unwrap()).1;
    s.outgoing = timed(|| session.with_vault(|v| v.outgoing_links(id)).unwrap()).1;
    s.related = timed(|| related::related_notes(session, id, 10).unwrap()).1;
    s
}

fn report(label: &str, mut runs: Vec<Switch>) {
    let pick = |runs: &mut Vec<Switch>, f: fn(&Switch) -> Duration| {
        runs.sort_by_key(f);
        (f(&runs[runs.len() / 2]), f(runs.last().unwrap()))
    };
    let n = runs.len();
    let (open, open_worst) = pick(&mut runs, |s| s.open);
    let (panels, panels_worst) = pick(&mut runs, |s| s.panels());
    let (recent, _) = pick(&mut runs, |s| s.recent);
    let (suggestions, _) = pick(&mut runs, |s| s.suggestions);
    let (backlinks, backlinks_worst) = pick(&mut runs, |s| s.backlinks);
    let (outgoing, _) = pick(&mut runs, |s| s.outgoing);
    let (related, related_worst) = pick(&mut runs, |s| s.related);
    println!(
        "{label} ({n} switches)\n  open_note: median {}, worst {}\n  next open waits for the panels: median {}, worst {}\n    recent {}, tag suggestions {}, backlinks {} (worst {}), outgoing {}, related {} (worst {})",
        ms(open),
        ms(open_worst),
        ms(panels),
        ms(panels_worst),
        ms(recent),
        ms(suggestions),
        ms(backlinks),
        ms(backlinks_worst),
        ms(outgoing),
        ms(related),
        ms(related_worst),
    );
}

#[test]
#[ignore = "benchmark: about a minute; run with --ignored --nocapture"]
fn open_note_benchmark() {
    let notes: usize = std::env::var("HUSH_OPEN_NOTES").ok().and_then(|n| n.replace('_', "").parse().ok()).unwrap_or(MAX_NOTES);
    assert!((1_000..=MAX_NOTES).contains(&notes), "1,000 to {MAX_NOTES} notes");
    let temp = tempfile::tempdir().unwrap();
    let path: PathBuf = std::env::var_os("HUSH_OPEN_VAULT").map_or_else(|| temp.path().join("open.hush"), PathBuf::from);
    let vault = prepare(&path, notes);
    let session = Session::default();
    let g = session.unlock(vault, path.clone()).unwrap();
    let start = Instant::now();
    while job::pass(&session, g, &Source, &mut |_| {}).unwrap().stale > 0 {}
    println!("indexed (or checked) with the fake embedder: {:.1} s", start.elapsed().as_secs_f64());
    println!("vault file: {:.0} MB", std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) as f64 / 1e6);

    let mut rng = Rng(7);
    let id = |i: usize| i as i64 + 1;
    let ordinary: Vec<i64> = (0..SAMPLES).map(|_| id(HUBS + rng.below(notes - HUBS))).filter(|i| (i - 1) % 20 != 3 && (i - 1) % 200 != 7).collect();
    let long: Vec<i64> = (0..SAMPLES).map(|_| id(rng.below(notes / 200) * 200 + 7)).collect();
    let duplicates: Vec<i64> = (0..SAMPLES).map(|_| id(rng.below(notes / 100) * 100 + 99)).collect();
    println!();
    println!("== Hush note opening: {notes} notes ==");
    // Warm the page cache and the statement cache, as a vault in use has them.
    for &i in ordinary.iter().take(5) {
        switch(&session, i);
    }
    report("ordinary notes (100-400 words)", ordinary.iter().map(|&i| switch(&session, i)).collect());
    report("long notes (60,000 words, about 400 kB)", long.iter().map(|&i| switch(&session, i)).collect());
    report("notes titled Notes (100 of them, linked ambiguously)", duplicates.iter().map(|&i| switch(&session, i)).collect());
    for hub in 0..HUBS {
        let backlinks = session.with_vault(|v| v.backlinks(id(hub))).unwrap().len();
        report(&format!("hub {hub} ({backlinks} backlinks)"), (0..5).map(|_| switch(&session, id(hub))).collect());
    }
    session.lock();
}
