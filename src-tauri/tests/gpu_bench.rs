//! Issue #105 prototype: bge-small-en-v1.5 on the CPU (int8, fp32) and on the GPU through
//! DirectML (fp16, fp32), on 300- and 3,000-word English notes cut from
//! `docs/decisions.md`. Prints chunks/s and notes/s per configuration.
//!
//! `HUSH_BENCH_MODELS` names a folder holding `model.onnx` (fp32), `model_fp16.onnx` and
//! `tokenizer.json` from `Xenova/bge-small-en-v1.5`; the int8 model is the bundled one
//! (`npm run fetch-model`). Run in release:
//!
//! `cargo test --release --manifest-path src-tauri/Cargo.toml --test gpu_bench -- --ignored --nocapture`

use std::path::{Path, PathBuf};
use std::time::Instant;

use hush_lib::embed::{self, ChunkPooling};
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::{GraphOptimizationLevel, SessionBuilder};
use ort::value::Tensor;
use tokenizers::{Encoding, PostProcessor, Token, Tokenizer};

const MAX_TOKENS: usize = 512;
const MAX_CHUNKS: usize = 64;
/// Chunks of one note run together, as in `embed/onnx.rs`.
const CHUNK_BATCH: usize = 8;
const DIMS: usize = 384;
const SHORT_NOTES: usize = 96;
const LONG_NOTES: usize = 16;

type Vectors = Vec<Vec<f32>>;

#[derive(Clone, Copy, PartialEq)]
enum Device {
    Cpu,
    DirectMl,
    /// DirectML on the NPU (Intel AI Boost), every run padded to [`MAX_TOKENS`].
    Npu,
}

struct Model {
    tokenizer: Tokenizer,
    session: Session,
    token_types: bool,
    /// Pads every run to this many tokens (fixed shapes for the NPU).
    pad_to: Option<usize>,
}

fn environment() -> &'static Environment {
    static ENV: std::sync::OnceLock<Environment> = std::sync::OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("hush-bench").with_telemetry(false).build().unwrap())
}

fn s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Model {
    fn load(model: &Path, tokenizer: &Path, device: Device, level: GraphOptimizationLevel) -> Result<Self, String> {
        let mut tokenizer = Tokenizer::from_file(tokenizer).map_err(s)?;
        tokenizer.with_truncation(None).map_err(s)?;
        tokenizer.with_padding(None);
        let bytes = std::fs::read(model).map_err(|e| format!("{}: {e}", model.display()))?;
        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        let mut b = Session::builder(environment()).map_err(s)?;
        b = b.with_optimization_level(level).map_err(s)?;
        match device {
            Device::Cpu => {
                b = b.with_intra_threads(threads).map_err(s)?;
                b = b.with_inter_threads(1).map_err(s)?;
            }
            Device::DirectMl | Device::Npu => {
                b = b.with_memory_pattern(false).map_err(s)?;
                b = b.with_parallel_execution(false).map_err(s)?;
                b = directml(b, device == Device::Npu)?;
            }
        }
        let session = b.commit_from_memory(&bytes).map_err(s)?;
        let token_types = session.inputs().iter().any(|i| i.name() == "token_type_ids");
        let pad_to = (device == Device::Npu).then_some(MAX_TOKENS);
        Ok(Self { tokenizer, session, token_types, pad_to })
    }

    /// Token windows of at most [`MAX_TOKENS`] (special tokens included), as `embed/onnx.rs`.
    fn chunks(&self, text: &str) -> Vec<Encoding> {
        let ids = self.tokenizer.encode(text, false).unwrap().get_ids().to_vec();
        let special = self.tokenizer.get_post_processor().map_or(0, |p| p.added_tokens(false));
        ids.chunks(MAX_TOKENS - special)
            .take(MAX_CHUNKS)
            .map(|chunk| {
                let tokens = chunk.iter().map(|&id| Token::new(id, String::new(), (0, 0))).collect();
                self.tokenizer.post_process(Encoding::from_tokens(tokens, 0), None, true).unwrap()
            })
            .collect()
    }

    /// One unit-length CLS vector per chunk, padded to the longest chunk of the run.
    fn run(&mut self, chunks: &[&Encoding]) -> Result<Vectors, String> {
        let rows = chunks.len();
        let len = chunks.iter().map(|c| c.len()).max().unwrap_or(0).max(self.pad_to.unwrap_or(0));
        let mut ids = vec![0i64; rows * len];
        let mut mask = vec![0i64; rows * len];
        let types = vec![0i64; rows * len];
        for (r, chunk) in chunks.iter().enumerate() {
            for (c, (&id, &m)) in chunk.get_ids().iter().zip(chunk.get_attention_mask()).enumerate() {
                ids[r * len + c] = i64::from(id);
                mask[r * len + c] = i64::from(m);
            }
        }
        let shape = [rows as i64, len as i64];
        let mut inputs = vec![
            ("input_ids", Tensor::from_array((shape, ids)).map_err(s)?.into_dyn()),
            ("attention_mask", Tensor::from_array((shape, mask)).map_err(s)?.into_dyn()),
        ];
        if self.token_types {
            inputs.push(("token_type_ids", Tensor::from_array((shape, types)).map_err(s)?.into_dyn()));
        }
        let outputs = self.session.run(inputs).map_err(s)?;
        let out = &outputs[0];
        let cls = |shape: &[i64], row: usize| {
            let seq = shape[1] as usize;
            assert_eq!(shape[2] as usize, DIMS);
            (row * seq * DIMS)..(row * seq * DIMS + DIMS)
        };
        let mut vectors: Vectors = match out.try_extract_tensor::<f32>() {
            Ok((sh, data)) => (0..rows).map(|r| data[cls(sh, r)].to_vec()).collect(),
            Err(_) => {
                let (sh, data) = out.try_extract_tensor::<half::f16>().map_err(s)?;
                (0..rows).map(|r| data[cls(sh, r)].iter().map(|x| x.to_f32()).collect()).collect()
            }
        };
        vectors.iter_mut().for_each(|v| embed::normalize(v));
        Ok(vectors)
    }

    /// Today's path: each note on its own, its chunks [`CHUNK_BATCH`] at a time.
    fn per_note(&mut self, notes: &[Vec<Encoding>]) -> Result<Vectors, String> {
        let mut out = Vec::with_capacity(notes.len());
        for chunks in notes {
            let mut vectors = Vec::new();
            for run in chunks.chunks(CHUNK_BATCH) {
                vectors.extend(self.run(&run.iter().collect::<Vec<_>>())?);
            }
            out.push(embed::pool_chunks(ChunkPooling::Mean, &vectors));
        }
        Ok(out)
    }

    /// Chunks of consecutive notes run `batch` at a time, then pooled per note.
    fn across_notes(&mut self, notes: &[Vec<Encoding>], batch: usize) -> Result<Vectors, String> {
        let all: Vec<(usize, &Encoding)> =
            notes.iter().enumerate().flat_map(|(n, chunks)| chunks.iter().map(move |c| (n, c))).collect();
        let mut per_note: Vec<Vectors> = vec![Vec::new(); notes.len()];
        for run in all.chunks(batch) {
            let vectors = self.run(&run.iter().map(|(_, c)| *c).collect::<Vec<_>>())?;
            for ((n, _), v) in run.iter().zip(vectors) {
                per_note[*n].push(v);
            }
        }
        Ok(per_note.iter().map(|v| embed::pool_chunks(ChunkPooling::Mean, v)).collect())
    }
}

#[cfg(windows)]
fn directml(b: SessionBuilder, npu: bool) -> Result<SessionBuilder, String> {
    use ort::ep::directml::DeviceFilter;
    let filter = if npu { DeviceFilter::Npu } else { DeviceFilter::Gpu };
    b.with_execution_providers([ort::ep::DirectML::default().with_device_filter(filter).build().error_on_failure()])
        .map_err(|e| format!("DirectML failed to register: {e}"))
}

#[cfg(not(windows))]
fn directml(_: SessionBuilder, _: bool) -> Result<SessionBuilder, String> {
    Err("DirectML is Windows-only".into())
}

/// `count` notes of `words` words each, cut from `docs/decisions.md` at spread-out offsets.
fn notes(words: usize, count: usize) -> Vec<String> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/decisions.md");
    let text = std::fs::read_to_string(path).unwrap();
    let all: Vec<&str> = text.split_whitespace().collect();
    (0..count)
        .map(|i| {
            let start = (i * 997) % all.len();
            let body: Vec<&str> = (0..words).map(|w| all[(start + w) % all.len()]).collect();
            embed::note_text(&format!("Note {i}"), &body.join(" ")).unwrap().to_string()
        })
        .collect()
}

struct Row {
    config: String,
    words: usize,
    notes: usize,
    chunks: usize,
    secs: f64,
}

impl Row {
    fn line(&self) -> String {
        format!(
            "| {} | {} | {} | {} | {:.1} | {:.1} | {:.2} |",
            self.config,
            self.words,
            self.notes,
            self.chunks,
            self.secs,
            self.chunks as f64 / self.secs,
            self.notes as f64 / self.secs
        )
    }
}

fn timed(f: impl FnOnce() -> Result<Vectors, String>) -> Result<(Vectors, f64), String> {
    let start = Instant::now();
    let v = f()?;
    Ok((v, start.elapsed().as_secs_f64()))
}

fn cosines(a: &[Vec<f32>], b: &[Vec<f32>]) -> (f32, f32) {
    let c: Vec<f32> = a.iter().zip(b).map(|(x, y)| embed::cosine(x, y)).collect();
    (c.iter().copied().fold(f32::MAX, f32::min), c.iter().sum::<f32>() / c.len() as f32)
}

/// Loads `model` and checks it runs one note; on DirectML, lower optimisation levels are
/// tried when a fused node fails on the GPU.
fn load_working(name: &str, model: &Path, tokenizer: &Path, device: Device, probe: &str) -> Option<Model> {
    let levels: &[(GraphOptimizationLevel, &str)] = match device {
        Device::Cpu => &[(GraphOptimizationLevel::Level3, "Level3")],
        Device::DirectMl | Device::Npu => &[
            (GraphOptimizationLevel::Level3, "Level3"),
            (GraphOptimizationLevel::Level2, "Level2"),
            (GraphOptimizationLevel::Level1, "Level1"),
            (GraphOptimizationLevel::Disable, "Disable"),
        ],
    };
    for (level, label) in levels {
        let start = Instant::now();
        let mut m = match Model::load(model, tokenizer, device, *level) {
            Ok(m) => m,
            Err(e) => {
                println!("{name} [{label}]: FAILED to load: {e}");
                continue;
            }
        };
        let chunks = vec![m.chunks(probe)];
        match m.per_note(&chunks) {
            Ok(_) => {
                println!("{name}: works with optimisation {label} (load + first run {:.1}s)", start.elapsed().as_secs_f64());
                return Some(m);
            }
            Err(e) => println!("{name} [{label}]: FAILED to run: {}", e.lines().next().unwrap_or("")),
        }
    }
    None
}

#[test]
#[ignore = "needs the model files and a GPU; prints measurements"]
fn measure_cpu_and_directml_throughput() {
    let bench = PathBuf::from(std::env::var("HUSH_BENCH_MODELS").expect("set HUSH_BENCH_MODELS"));
    let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
    let tokenizer = bench.join("tokenizer.json");
    let configs = [
        ("int8 CPU", bundled.join("bge-small-en-v1.5-q8.onnx"), Device::Cpu),
        ("fp32 CPU", bench.join("model.onnx"), Device::Cpu),
        ("fp16 DirectML", bench.join("model_fp16.onnx"), Device::DirectMl),
        ("fp32 DirectML", bench.join("model.onnx"), Device::DirectMl),
        ("fp16 NPU", bench.join("model_fp16.onnx"), Device::Npu),
        ("fp32 NPU", bench.join("model.onnx"), Device::Npu),
    ];
    let sets = [(300, notes(300, SHORT_NOTES)), (3_000, notes(3_000, LONG_NOTES))];
    let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
    println!("CPU sessions use {threads} intra-op threads");

    let mut rows = Vec::new();
    let mut reference: Vec<(&str, usize, Vectors)> = Vec::new();
    for (name, model_path, device) in &configs {
        let Some(mut model) = load_working(name, model_path, &tokenizer, *device, &sets[1].1[0]) else {
            println!("{name}: SKIPPED, no optimisation level runs");
            continue;
        };
        for (words, texts) in &sets {
            let chunked: Vec<Vec<Encoding>> = texts.iter().map(|t| model.chunks(t)).collect();
            let chunks: usize = chunked.iter().map(Vec::len).sum();
            // Warm up on a few notes (DirectML prepares kernels for new shapes on first use).
            let single = model.per_note(&chunked[..4]).and_then(|_| timed(|| model.per_note(&chunked)));
            let single = match single {
                Ok((v, secs)) => {
                    let row = Row { config: format!("{name}, per note"), words: *words, notes: texts.len(), chunks, secs };
                    println!("{}", row.line());
                    rows.push(row);
                    v
                }
                Err(e) => {
                    println!("{name} {words} words per note: FAILED: {e}");
                    continue;
                }
            };
            if *device != Device::Cpu {
                for batch in [8, 32, 64] {
                    let warm = chunked.len().min(8);
                    match model.across_notes(&chunked[..warm], batch).and_then(|_| timed(|| model.across_notes(&chunked, batch))) {
                        Ok((batched, secs)) => {
                            let row = Row {
                                config: format!("{name}, {batch} chunks across notes"),
                                words: *words,
                                notes: texts.len(),
                                chunks,
                                secs,
                            };
                            let (min, _) = cosines(&single, &batched);
                            println!("{}  (min cosine alone vs batched {min:.6})", row.line());
                            rows.push(row);
                        }
                        Err(e) => println!("{name} {words} words batch {batch}: FAILED: {e}"),
                    }
                }
            }
            reference.push((name, *words, single));
        }
    }

    println!();
    println!("| Configuration | Words/note | Notes | Chunks | Seconds | Chunks/s | Notes/s |");
    println!("|---|---|---|---|---|---|---|");
    for r in &rows {
        println!("{}", r.line());
    }
    println!();
    for words in [300, 3_000] {
        let get = |n: &str| reference.iter().find(|(c, w, _)| *c == n && *w == words).map(|(_, _, v)| v);
        for (a, b) in [
            ("int8 CPU", "fp32 CPU"),
            ("int8 CPU", "fp16 DirectML"),
            ("int8 CPU", "fp32 DirectML"),
            ("fp32 CPU", "fp16 DirectML"),
            ("fp32 CPU", "fp32 DirectML"),
            ("fp32 CPU", "fp16 NPU"),
            ("fp32 CPU", "fp32 NPU"),
            ("int8 CPU", "fp16 NPU"),
        ] {
            if let (Some(x), Some(y)) = (get(a), get(b)) {
                let (min, mean) = cosines(x, y);
                println!("{words} words: {a} vs {b}: min cosine {min:.6}, mean {mean:.6}");
            }
        }
    }
}
