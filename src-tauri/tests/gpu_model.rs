//! Issue #105: the fp16 GPU model through the app's own loader and batching, on the GPU
//! and on the CPU (its fallback), on 3,000-word English notes cut from `docs/decisions.md`.
//! Prints the adapters, notes/s per device and the cosine between the two devices.
//!
//! Needs `bge-small-en-v1.5-fp16.onnx` and `bge-small-en-v1.5-fp16.tokenizer.json` in
//! `HUSH_GPU_MODELS`, or else in the app's local data `models` folder. Run in release:
//!
//! `cargo test --release --manifest-path src-tauri/Cargo.toml --test gpu_model -- --ignored --nocapture`
//!
//! The second test indexes a vault with the embedding job on the GPU. It is a temporary
//! one unless `HUSH_KEEP_VAULT` names a new `.hush` file to keep (passphrase
//! [`common::PASS`]).

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use hush_lib::db::Vault;
use hush_lib::embed::job::{self, BATCH, EmbedderSource};
use hush_lib::embed::onnx::{Device, OnnxEmbedder};
use hush_lib::embed::{self, EmbedError, Embedder, ModelDescriptor, gpu};
use hush_lib::session::Session;

const NOTES: usize = 50;
const WORDS: usize = 3_000;
const MODEL: &str = "bge-small-en-v1.5-fp16";

/// The model's folder and descriptor, or `None` (and a note why) when its files are missing.
fn model() -> Option<(PathBuf, &'static ModelDescriptor)> {
    let dir = std::env::var_os("HUSH_GPU_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("com.redheadit.hush")
                .join("models")
        });
    let d = embed::descriptor(MODEL).unwrap();
    if !dir.join(d.model.name).exists() || !dir.join(d.tokenizer.name).exists() {
        println!("SKIPPED: the fp16 model files aren't in {}", dir.display());
        return None;
    }
    Some((dir, d))
}

fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1))
}

/// `NOTES` titles and bodies of `WORDS` words, cut from `docs/decisions.md`.
fn bodies() -> Vec<(String, String)> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/decisions.md");
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<&str> = text.split_whitespace().collect();
    (0..NOTES)
        .map(|i| {
            let start = (i * 997) % all.len();
            let body: Vec<&str> = (0..WORDS).map(|w| all[(start + w) % all.len()]).collect();
            (format!("Note {i}"), body.join(" "))
        })
        .collect()
}

fn notes() -> Vec<String> {
    bodies()
        .iter()
        .map(|(title, body)| embed::note_text(title, body).unwrap().to_string())
        .collect()
}

/// As the job indexes: first chunks [`BATCH`] notes at a time, then whole notes as the
/// second phase takes them ([`Embedder::embed_group`]: a group of notes on the GPU, #111).
fn job_path(model: &OnnxEmbedder, texts: &[&str]) -> (Vec<Vec<f32>>, f64, f64) {
    let start = Instant::now();
    for page in texts.chunks(BATCH) {
        model.embed_first_batch(page, &|| false).unwrap();
    }
    let first = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let mut vectors = Vec::with_capacity(texts.len());
    while vectors.len() < texts.len() {
        vectors.extend(model.embed_group(&texts[vectors.len()..], &|| false).unwrap());
    }
    (vectors, first, start.elapsed().as_secs_f64())
}

/// Whole notes [`BATCH`] at a time, so the model can run chunks of several notes together.
fn batched(model: &OnnxEmbedder, texts: &[&str]) -> (Vec<Vec<f32>>, f64) {
    let start = Instant::now();
    let vectors = texts
        .chunks(BATCH)
        .flat_map(|page| model.embed_batch(page, &|| false).unwrap())
        .collect();
    (vectors, start.elapsed().as_secs_f64())
}

#[test]
#[ignore = "needs the fp16 model files and a GPU; prints measurements"]
fn measure_the_fp16_model_on_the_gpu_and_the_cpu() {
    let adapters = gpu::adapters();
    println!("adapters: {adapters:?}");
    println!("best: {:?}", gpu::best(&adapters));
    let Some((dir, d)) = model() else { return };
    let texts = notes();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();

    let mut results = Vec::new();
    for device in [Device::Gpu, Device::Cpu] {
        let start = Instant::now();
        let model = match OnnxEmbedder::load_on(&dir, d, threads(), device) {
            Ok(m) => m,
            Err(e) => {
                println!("{device:?}: FAILED to load: {e}");
                continue;
            }
        };
        println!(
            "{device:?}: loaded in {:.1}s, runs on {:?}",
            start.elapsed().as_secs_f64(),
            model.device()
        );
        model.embed_batch(&texts[..2], &|| false).unwrap();
        // Tokenizing is inside every timing below; this is its share.
        let start = Instant::now();
        let chunks: usize = texts
            .iter()
            .map(|t| model.chunk_lengths(t).unwrap().len())
            .sum();
        let tokenize = start.elapsed().as_secs_f64();
        println!("{device:?}: tokenizing {NOTES} notes ({chunks} chunks) takes {tokenize:.1}s");
        let (job_vectors, first, rest) = job_path(&model, &texts);
        println!(
            "{device:?}, job path: first chunks {:.1}s ({:.1} notes/s), rest {:.1}s; {NOTES} notes in {:.1}s = {:.2} notes/s",
            first,
            NOTES as f64 / first,
            rest,
            first + rest,
            NOTES as f64 / (first + rest)
        );
        // The second phase before #111: one note per call.
        let start = Instant::now();
        let single: Vec<Vec<f32>> = texts.iter().map(|t| model.embed_batch(&[t], &|| false).unwrap().remove(0)).collect();
        let secs = start.elapsed().as_secs_f64();
        let min = job_vectors.iter().zip(&single).map(|(a, b)| embed::cosine(a, b)).fold(f32::MAX, f32::min);
        println!("{device:?}, one note per call: {secs:.1}s = {:.2} notes/s; min cosine vs job path {min:.6}", NOTES as f64 / secs);
        assert!(min >= 0.9999, "grouping changed the vectors: {min}");
        let (vectors, secs) = batched(&model, &texts);
        println!(
            "{device:?}, {BATCH} whole notes per call: {secs:.1}s = {:.2} notes/s",
            NOTES as f64 / secs
        );
        let min = job_vectors
            .iter()
            .zip(&vectors)
            .map(|(a, b)| embed::cosine(a, b))
            .fold(f32::MAX, f32::min);
        println!("{device:?}: min cosine job path vs {BATCH} per call {min:.6}");
        results.push((device, vectors));
    }
    if let [(_, gpu), (_, cpu)] = &results[..] {
        let c: Vec<f32> = gpu
            .iter()
            .zip(cpu)
            .map(|(a, b)| embed::cosine(a, b))
            .collect();
        let min = c.iter().copied().fold(f32::MAX, f32::min);
        println!(
            "GPU vs CPU: min cosine {min:.6}, mean {:.6}",
            c.iter().sum::<f32>() / c.len() as f32
        );
        assert!(
            min >= 0.999,
            "the GPU's vectors differ from the CPU's: {min}"
        );
    }
}

/// Hands the job one model, loaded once.
struct Loaded(Arc<dyn Embedder>);

impl EmbedderSource for Loaded {
    fn embedder(&self, _: &str) -> Result<Arc<dyn Embedder>, EmbedError> {
        Ok(self.0.clone())
    }
}

#[test]
#[ignore = "needs the fp16 model files and a GPU; prints measurements"]
fn index_a_vault_on_the_gpu() {
    let Some((dir, d)) = model() else { return };
    let temp = tempfile::tempdir().unwrap();
    let path = std::env::var_os("HUSH_KEEP_VAULT")
        .map(PathBuf::from)
        .unwrap_or_else(|| temp.path().join("gpu-test.hush"));
    assert!(!path.exists(), "{} exists already", path.display());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut vault = Vault::create(&path, common::PASS).unwrap();
    vault.set_embedding_model(MODEL).unwrap();
    for (title, body) in bodies() {
        vault.create_note("", &title, &body).unwrap();
    }
    let session = Session::default();
    let generation = session.unlock(vault, path.clone()).unwrap();

    let model = OnnxEmbedder::load_on(&dir, d, threads(), Device::Gpu).unwrap();
    println!("model {MODEL} runs on {:?}", model.device());
    let source = Loaded(Arc::new(model));
    let start = Instant::now();
    for n in 1.. {
        let pass = job::pass(&session, generation, &source, &mut |_| {}).unwrap();
        let queued = session
            .with_vault_quiet(generation, |v| v.embedding_queue_len())
            .unwrap();
        println!(
            "pass {n}: {} stale, {queued} queued, {:.1}s so far",
            pass.stale,
            start.elapsed().as_secs_f64()
        );
        if pass.stale == 0 && queued == 0 {
            break;
        }
        assert!(n < 10, "indexing never finished");
    }
    let secs = start.elapsed().as_secs_f64();
    let indexed = session.with_vault(|v| v.embedding_count(MODEL)).unwrap();
    println!(
        "job: {indexed} of {NOTES} notes in {secs:.1}s = {:.2} notes/s",
        NOTES as f64 / secs
    );
    assert_eq!(indexed, NOTES);
    session.lock();
    println!("vault: {}", path.display());
}
