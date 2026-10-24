//! The real embedder: a bundled ONNX model run in-process with ONNX Runtime. The model
//! and tokenizer files are checked against their pinned SHA-256 and loaded from memory.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use ort::value::Tensor;
use tokenizers::{Encoding, PostProcessor, Token, Tokenizer};

use super::{EmbedError, Embedder, ModelDescriptor, TokenPooling, normalize, pool_chunks, read_verified};

/// A note longer than this many chunks is embedded from its start only, so one huge
/// note can't keep the job busy for minutes.
const MAX_CHUNKS: usize = 64;
/// Chunks run through the model together.
const CHUNK_BATCH: usize = 8;

pub struct OnnxEmbedder {
    descriptor: &'static ModelDescriptor,
    tokenizer: Tokenizer,
    session: Mutex<Session>,
    /// Whether the model takes `token_type_ids` (BERT exports usually do).
    token_types: bool,
}

impl OnnxEmbedder {
    /// Loads `descriptor`'s files from `dir`, refusing any whose checksum doesn't match.
    pub fn load(dir: &Path, descriptor: &'static ModelDescriptor) -> Result<Self, EmbedError> {
        let tokenizer_bytes = read_verified(dir, &descriptor.tokenizer)?;
        let model_bytes = read_verified(dir, &descriptor.model)?;

        let mut tokenizer = Tokenizer::from_bytes(&tokenizer_bytes).map_err(|e| EmbedError::Load(e.to_string()))?;
        tokenizer.with_truncation(None).map_err(|e| EmbedError::Load(e.to_string()))?;
        tokenizer.with_padding(None);

        let threads = std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).max(1));
        let session = Session::builder(environment()?)
            .and_then(|b| Ok(b.with_optimization_level(GraphOptimizationLevel::Level3)?))
            .and_then(|b| Ok(b.with_intra_threads(threads)?))
            .and_then(|b| Ok(b.with_inter_threads(1)?))
            .and_then(|mut b| b.commit_from_memory(&model_bytes))
            .map_err(|e| EmbedError::Load(e.to_string()))?;
        let token_types = session.inputs().iter().any(|i| i.name() == "token_type_ids");
        Ok(Self { descriptor, tokenizer, session: Mutex::new(session), token_types })
    }

    /// Splits `text` into chunks of at most `max_tokens` tokens, each with the model's
    /// special tokens and the descriptor's prefix.
    fn chunks(&self, text: &str) -> Result<Vec<Encoding>, EmbedError> {
        let err = |e: tokenizers::Error| EmbedError::Inference(e.to_string());
        let ids = self.tokenizer.encode(text, false).map_err(err)?.get_ids().to_vec();
        let prefix = if self.descriptor.prefix.is_empty() {
            Vec::new()
        } else {
            self.tokenizer.encode(self.descriptor.prefix, false).map_err(err)?.get_ids().to_vec()
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
        let (out_shape, hidden) = outputs[0].try_extract_tensor::<f32>().map_err(err)?;
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
        let chunks = self.chunks(text)?;
        let mut vectors = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(CHUNK_BATCH) {
            vectors.extend(self.run(batch)?);
        }
        Ok(pool_chunks(self.descriptor.chunk_pooling, &vectors))
    }
}

/// ONNX Runtime allows one environment per process; it is made on first use, with
/// telemetry off.
fn environment() -> Result<&'static Environment, EmbedError> {
    static ENV: OnceLock<Result<Environment, String>> = OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("hush").with_telemetry(false).build().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| EmbedError::Load(e.clone()))
}
