//! The real embedder: a bundled ONNX model run in-process with ONNX Runtime. The model
//! and tokenizer files are checked against their pinned SHA-256 and loaded from memory.
//! On Windows a model made for the GPU can run through DirectML ([`Device::Gpu`]).

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use tokenizers::{Encoding, PostProcessor, Token, Tokenizer};

use super::{
    EmbedError, Embedder, FirstChunk, ModelDescriptor, TokenPooling, normalize, pool_chunks,
    read_verified,
};

/// A note longer than this many chunks is embedded from its start only, so one huge
/// note can't keep the job busy for minutes.
const MAX_CHUNKS: usize = 64;
/// Chunks run through the model together: of one note, or of several notes when the
/// model allows it ([`ModelDescriptor::batch_across_notes`]). 8 was also the fastest
/// batch on DirectML, ahead of 32 and 64 (#105).
const CHUNK_BATCH: usize = 8;
/// Embedded once when a model loads on the GPU: DirectML can register and then fail on
/// the first run (an old system DirectML.dll does), and that should fall back to the CPU
/// before any note waits on it.
const GPU_PROBE: &str = "Hush checks that the GPU can run the model.";

/// Where a model runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Device {
    /// ONNX Runtime's CPU execution provider.
    Cpu,
    /// DirectML on the GPU Windows considers the fastest (Windows only).
    Gpu,
}

pub struct OnnxEmbedder {
    descriptor: &'static ModelDescriptor,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
    /// Whether the model takes `token_type_ids` (BERT exports usually do).
    token_types: bool,
    device: Device,
}

impl OnnxEmbedder {
    /// Loads `descriptor`'s files from `dir`, refusing any whose checksum doesn't match.
    /// The model runs on half the cores.
    pub fn load(dir: &Path, descriptor: &'static ModelDescriptor) -> Result<Self, EmbedError> {
        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        Self::load_with_threads(dir, descriptor, threads)
    }

    /// [`OnnxEmbedder::load`], running the model on `threads` cores.
    pub fn load_with_threads(
        dir: &Path,
        descriptor: &'static ModelDescriptor,
        threads: usize,
    ) -> Result<Self, EmbedError> {
        Self::load_on(dir, descriptor, threads, Device::Cpu)
    }

    /// Loads `descriptor`'s files from `dir` to run on `device`: on the CPU with
    /// `threads` cores, or on the GPU, where it also embeds a probe text so a GPU that
    /// registers but can't run the model fails here.
    pub fn load_on(
        dir: &Path,
        descriptor: &'static ModelDescriptor,
        threads: usize,
        device: Device,
    ) -> Result<Self, EmbedError> {
        let tokenizer_bytes = read_verified(dir, &descriptor.tokenizer)?;
        let model_bytes = read_verified(dir, &descriptor.model)?;

        let mut tokenizer = Tokenizer::from_bytes(&tokenizer_bytes).map_err(|e| EmbedError::Load(e.to_string()))?;
        tokenizer.with_truncation(None).map_err(|e| EmbedError::Load(e.to_string()))?;
        tokenizer.with_padding(None);

        fn load(e: impl std::fmt::Display) -> EmbedError {
            EmbedError::Load(e.to_string())
        }
        let mut builder = Session::builder(environment()?).map_err(load)?;
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(load)?;
        builder = match device {
            Device::Cpu => builder
                .with_intra_threads(threads.max(1))
                .map_err(load)?
                .with_inter_threads(1)
                .map_err(load)?,
            // DirectML needs both off.
            Device::Gpu => directml(
                builder
                    .with_memory_pattern(false)
                    .map_err(load)?
                    .with_parallel_execution(false)
                    .map_err(load)?,
            )?,
        };
        let session = builder.commit_from_memory(&model_bytes).map_err(load)?;
        let token_types = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");
        let embedder = Self {
            descriptor,
            tokenizer,
            session: Mutex::new(session),
            token_types,
            device,
        };
        if device == Device::Gpu {
            embedder
                .embed(GPU_PROBE)
                .map_err(|e| EmbedError::Load(format!("the GPU couldn't run the model: {e}")))?;
        }
        Ok(embedder)
    }

    /// Where the model runs.
    pub fn device(&self) -> Device {
        self.device
    }

    /// The token count of each chunk `text` is split into (special tokens included), as
    /// a document. For the scale benchmark.
    #[doc(hidden)]
    pub fn chunk_lengths(&self, text: &str) -> Result<Vec<usize>, EmbedError> {
        Ok(self.chunks(text, "")?.iter().map(Encoding::len).collect())
    }

    /// Splits `text` into chunks of at most `max_tokens` tokens, each with the model's
    /// special tokens and `prefix`.
    fn chunks(&self, text: &str, prefix: &str) -> Result<Vec<Encoding>, EmbedError> {
        let err = |e: tokenizers::Error| EmbedError::Inference(e.to_string());
        let ids = self.tokenizer.encode(text, false).map_err(err)?.get_ids().to_vec();
        let prefix = if prefix.is_empty() {
            Vec::new()
        } else {
            self.tokenizer.encode(prefix, false).map_err(err)?.get_ids().to_vec()
        };
        let special = self.tokenizer.get_post_processor().map_or(0, |p| p.added_tokens(false));
        let window = self.descriptor.max_tokens.saturating_sub(special + prefix.len()).max(1);
        ids.chunks(window)
            .take(MAX_CHUNKS)
            .map(|chunk| {
                let tokens = prefix.iter().chain(chunk).map(|&id| Token::new(id, String::new(), (0, 0))).collect();
                self.tokenizer.post_process(Encoding::from_tokens(tokens, 0), None, true).map_err(err)
            })
            .collect()
    }

    /// One unit-length vector per chunk.
    fn run(&self, chunks: &[Encoding]) -> Result<Vec<Vec<f32>>, EmbedError> {
        let err = |e: ort::Error| EmbedError::Inference(e.to_string());
        let rows = chunks.len();
        let len = chunks.iter().map(Encoding::len).max().unwrap_or(0);
        let mut ids = vec![0i64; rows * len];
        let mut mask = vec![0i64; rows * len];
        let mut types = vec![0i64; rows * len];
        for (r, chunk) in chunks.iter().enumerate() {
            for (c, ((&id, &m), &t)) in
                chunk.get_ids().iter().zip(chunk.get_attention_mask()).zip(chunk.get_type_ids()).enumerate()
            {
                ids[r * len + c] = i64::from(id);
                mask[r * len + c] = i64::from(m);
                types[r * len + c] = i64::from(t);
            }
        }
        let shape = [rows as i64, len as i64];
        let mut inputs = vec![
            ("input_ids", Tensor::from_array((shape, ids)).map_err(err)?.into_dyn()),
            ("attention_mask", Tensor::from_array((shape, mask.clone())).map_err(err)?.into_dyn()),
        ];
        if self.token_types {
            inputs.push(("token_type_ids", Tensor::from_array((shape, types)).map_err(err)?.into_dyn()));
        }

        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let outputs = session.run(inputs).map_err(err)?;
        // fp16 exports hand back float16.
        let (out_shape, hidden): (Vec<i64>, std::borrow::Cow<[f32]>) = match outputs[0]
            .try_extract_tensor::<f32>()
        {
            Ok((shape, data)) => (shape.to_vec(), data.into()),
            Err(_) => {
                let (shape, data) = outputs[0].try_extract_tensor::<half::f16>().map_err(err)?;
                (
                    shape.to_vec(),
                    data.iter().map(|x| x.to_f32()).collect::<Vec<_>>().into(),
                )
            }
        };
        let dims = self.descriptor.dims;
        if out_shape.len() != 3 || out_shape[0] as usize != rows || out_shape[2] as usize != dims {
            return Err(EmbedError::Inference(format!("unexpected model output shape {out_shape:?}")));
        }
        let seq = out_shape[1] as usize;
        let token = |r: usize, t: usize| &hidden[(r * seq + t) * dims..(r * seq + t + 1) * dims];
        let vectors = (0..rows)
            .map(|r| {
                let mut v = match self.descriptor.token_pooling {
                    TokenPooling::Cls => token(r, 0).to_vec(),
                    TokenPooling::Mean => {
                        let mut sum = vec![0.0; dims];
                        let mut n = 0.0;
                        for t in 0..seq.min(len) {
                            if mask[r * len + t] != 0 {
                                sum.iter_mut().zip(token(r, t)).for_each(|(s, x)| *s += x);
                                n += 1.0;
                            }
                        }
                        sum.iter_mut().for_each(|s| *s /= f32::max(n, 1.0));
                        sum
                    }
                };
                normalize(&mut v);
                v
            })
            .collect();
        Ok(vectors)
    }
}

impl Embedder for OnnxEmbedder {
    fn model_id(&self) -> &str {
        self.descriptor.id
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed_with(text, self.descriptor.prefix)
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        self.embed_with(text, self.descriptor.query_prefix)
    }

    fn embed_batch(&self, texts: &[&str], stop: &dyn Fn() -> bool) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.batch_with(texts, self.descriptor.prefix, stop)
    }

    /// Each text's first chunk, run as [`OnnxEmbedder::batch_with`] runs chunks; a
    /// one-chunk text gets exactly its full vector.
    fn embed_first_batch(
        &self,
        texts: &[&str],
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<FirstChunk>, EmbedError> {
        let mut firsts = Vec::with_capacity(texts.len());
        let mut complete = Vec::with_capacity(texts.len());
        for text in texts {
            let mut chunks = self.chunks(text, self.descriptor.prefix)?;
            complete.push(chunks.len() <= 1);
            chunks.truncate(1);
            firsts.push(chunks);
        }
        let vectors = self.run_chunks(&firsts, stop)?;
        Ok(vectors
            .iter()
            .zip(complete)
            .map(|(v, complete)| FirstChunk {
                vector: pool_chunks(self.descriptor.chunk_pooling, v),
                complete,
            })
            .collect())
    }
}

impl OnnxEmbedder {
    fn embed_with(&self, text: &str, prefix: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(self.batch_with(&[text], prefix, &|| false)?.pop().expect("one vector per text"))
    }

    /// Embeds `texts`, asking `stop` before each model run. With an int8 model each text
    /// runs on its own, its chunks [`CHUNK_BATCH`] at a time: the bundled int8 model
    /// quantises each run with one scale for the whole run, so a text's vector would
    /// depend on the texts it ran with (up to 0.04 per element, measured). A model that
    /// allows it ([`ModelDescriptor::batch_across_notes`]) runs the chunks of all texts
    /// [`CHUNK_BATCH`] at a time, which keeps the GPU busy with short notes.
    fn batch_with(
        &self,
        texts: &[&str],
        prefix: &str,
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<f32>>, EmbedError> {
        let chunked = texts
            .iter()
            .map(|t| self.chunks(t, prefix))
            .collect::<Result<Vec<_>, _>>()?;
        let vectors = self.run_chunks(&chunked, stop)?;
        Ok(vectors
            .iter()
            .map(|v| pool_chunks(self.descriptor.chunk_pooling, v))
            .collect())
    }

    /// Each text's chunk vectors, run as [`OnnxEmbedder::batch_with`] says.
    fn run_chunks(
        &self,
        texts: &[Vec<Encoding>],
        stop: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<Vec<f32>>>, EmbedError> {
        let mut out: Vec<Vec<Vec<f32>>> =
            texts.iter().map(|c| Vec::with_capacity(c.len())).collect();
        if self.descriptor.batch_across_notes {
            let all: Vec<(usize, &Encoding)> = texts
                .iter()
                .enumerate()
                .flat_map(|(n, chunks)| chunks.iter().map(move |c| (n, c)))
                .collect();
            for run in all.chunks(CHUNK_BATCH) {
                if stop() {
                    return Err(EmbedError::Stopped);
                }
                let encodings: Vec<Encoding> = run.iter().map(|(_, c)| (*c).clone()).collect();
                for ((n, _), v) in run.iter().zip(self.run(&encodings)?) {
                    out[*n].push(v);
                }
            }
        } else {
            for (n, chunks) in texts.iter().enumerate() {
                for run in chunks.chunks(CHUNK_BATCH) {
                    if stop() {
                        return Err(EmbedError::Stopped);
                    }
                    out[n].extend(self.run(run)?);
                }
            }
        }
        Ok(out)
    }
}

/// Adds DirectML to `builder`, failing if it can't register (no DirectX 12 GPU, or not
/// Windows). It asks for the high-performance GPU, so a laptop's discrete GPU wins over
/// its integrated one.
#[cfg(windows)]
fn directml(
    builder: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, EmbedError> {
    use ort::ep::directml::{DeviceFilter, PerformancePreference};
    let ep = ort::ep::DirectML::default()
        .with_device_filter(DeviceFilter::Gpu)
        .with_performance_preference(PerformancePreference::HighPerformance)
        .build()
        .error_on_failure();
    builder
        .with_execution_providers([ep])
        .map_err(|e| EmbedError::Load(format!("DirectML didn't start: {e}")))
}

#[cfg(not(windows))]
fn directml(
    _: ort::session::builder::SessionBuilder,
) -> Result<ort::session::builder::SessionBuilder, EmbedError> {
    Err(EmbedError::Load("the GPU is only used on Windows".into()))
}

/// ONNX Runtime allows one environment per process; it is made on first use, with
/// telemetry off.
fn environment() -> Result<&'static Environment, EmbedError> {
    static ENV: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("hush").with_telemetry(false).build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| EmbedError::Load(e.clone()))
}
