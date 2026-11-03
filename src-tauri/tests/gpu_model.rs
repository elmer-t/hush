//! Issue #105: the fp16 GPU model through the app's own loader and batching, on the GPU
//! and on the CPU (its fallback), on 3,000-word English notes cut from `docs/decisions.md`.
//! Prints the adapters, notes/s per device and the cosine between the two devices.
//!
//! Needs `bge-small-en-v1.5-fp16.onnx` and `bge-small-en-v1.5-fp16.tokenizer.json` in
//! `HUSH_GPU_MODELS`, or else in the app's local data `models` folder. Run in release:
//!
//! `cargo test --release --manifest-path src-tauri/Cargo.toml --test gpu_model -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::time::Instant;

use hush_lib::embed::job::BATCH;
use hush_lib::embed::onnx::{Device, OnnxEmbedder};
use hush_lib::embed::{self, Embedder, gpu};

const NOTES: usize = 50;
const WORDS: usize = 3_000;

fn model_dir() -> PathBuf {
    std::env::var_os("HUSH_GPU_MODELS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
                .join("com.redheadit.hush")
                .join("models")
        })
}

fn notes() -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/decisions.md");
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<&str> = text.split_whitespace().collect();
    (0..NOTES)
        .map(|i| {
            let start = (i * 997) % all.len();
            let body: Vec<&str> = (0..WORDS).map(|w| all[(start + w) % all.len()]).collect();
            embed::note_text(&format!("Note {i}"), &body.join(" "))
                .unwrap()
                .to_string()
        })
        .collect()
}

/// As the job indexes: first chunks [`BATCH`] notes at a time, then each note whole on its own.
fn job_path(model: &OnnxEmbedder, texts: &[&str]) -> (Vec<Vec<f32>>, f64, f64) {
    let start = Instant::now();
    for page in texts.chunks(BATCH) {
        model.embed_first_batch(page, &|| false).unwrap();
    }
    let first = start.elapsed().as_secs_f64();
    let start = Instant::now();
    let vectors = texts
        .iter()
        .map(|t| model.embed_batch(&[t], &|| false).unwrap().remove(0))
        .collect();
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

    let dir = model_dir();
    let d = embed::descriptor("bge-small-en-v1.5-fp16").unwrap();
    if !dir.join(d.model.name).exists() || !dir.join(d.tokenizer.name).exists() {
        println!("SKIPPED: the fp16 model files aren't in {}", dir.display());
        return;
    }
    let texts = notes();
    let texts: Vec<&str> = texts.iter().map(String::as_str).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));

    let mut results = Vec::new();
    for device in [Device::Gpu, Device::Cpu] {
        let start = Instant::now();
        let model = match OnnxEmbedder::load_on(&dir, d, threads, device) {
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
        println!("{device:?}: min cosine one note per call vs {BATCH} per call {min:.6}");
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
