//! Local embeddings: every note gets a vector computed in-process by a bundled model and
//! stored in the vault (see `docs/decisions.md`, AI classification). Nothing here
//! touches the network or writes outside the vault.
//!
//! Everything model-specific lives in a [`ModelDescriptor`]; no other code assumes a
//! vector length or chunk size.

pub mod job;
pub mod onnx;
pub mod related;

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::markdown;

/// The model a vault embeds with until its `embedding_model` setting says otherwise.
pub const DEFAULT_MODEL: &str = BGE_SMALL_EN_V15_Q8.id;

/// Every model Hush knows. Only the default ships with the app for now.
pub const MODELS: &[ModelDescriptor] = &[BGE_SMALL_EN_V15_Q8];

/// `BAAI/bge-small-en-v1.5` (MIT), int8-quantized ONNX export from `Xenova/bge-small-en-v1.5`.
const BGE_SMALL_EN_V15_Q8: ModelDescriptor = ModelDescriptor {
    id: "bge-small-en-v1.5-q8",
    model: ModelFile {
        name: "bge-small-en-v1.5-q8.onnx",
        url: "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/onnx/model_quantized.onnx",
        sha256: "6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4",
    },
    tokenizer: ModelFile {
        name: "bge-small-en-v1.5-q8.tokenizer.json",
        url: "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/tokenizer.json",
        sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    },
    dims: 384,
    max_tokens: 512,
    token_pooling: TokenPooling::Cls,
    chunk_pooling: ChunkPooling::Mean,
    prefix: "",
    suggestion_threshold: 0.75,
};

/// One model: its files, the shape of its vectors and how text is fed to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelDescriptor {
    /// Stored with every vector, and in the vault's model settings.
    pub id: &'static str,
    pub model: ModelFile,
    pub tokenizer: ModelFile,
    /// Length of each vector.
    pub dims: usize,
    /// Most tokens the model takes at once, special tokens included; longer notes are
    /// split into chunks of this size.
    pub max_tokens: usize,
    /// How a chunk's token vectors become one chunk vector.
    pub token_pooling: TokenPooling,
    /// How chunk vectors become the note vector.
    pub chunk_pooling: ChunkPooling,
    /// Prepended to every chunk (some models want a task prefix).
    pub prefix: &'static str,
    /// Default similarity above which a tag is suggested.
    pub suggestion_threshold: f32,
}

/// A bundled file, fetched at build time and checked again before it is loaded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelFile {
    /// File name in the app's `models` resource folder.
    pub name: &'static str,
    /// Where the build step fetches it from. Never used at runtime.
    pub url: &'static str,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TokenPooling {
    /// The first token's (`[CLS]`) vector.
    Cls,
    /// The mean of the token vectors, padding left out.
    Mean,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChunkPooling {
    /// The L2-normalised mean of the L2-normalised chunk vectors.
    Mean,
}

pub fn descriptor(id: &str) -> Option<&'static ModelDescriptor> {
    MODELS.iter().find(|m| m.id == id)
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("unknown embedding model “{0}”")]
    UnknownModel(String),
    #[error("model file {0} is missing")]
    Missing(PathBuf),
    #[error("model file {0} failed its checksum; not loading it")]
    Checksum(PathBuf),
    #[error("could not read model file {path}: {source}")]
    Read { path: PathBuf, source: std::io::Error },
    #[error("could not load the model: {0}")]
    Load(String),
    #[error("could not embed a note: {0}")]
    Inference(String),
}

/// Turns note text into a vector. The real model sits behind this so DB and job tests
/// can use a fast, deterministic fake.
pub trait Embedder: Send + Sync {
    /// The model ID stored with each vector.
    fn model_id(&self) -> &str;
    /// One unit-length vector for the whole text, however long.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
}

/// The text embedded for a note: title and body, without frontmatter (it holds the
/// tags, which would leak the labels into what later classifiers learn from). `None`
/// when the body has nothing to embed.
pub fn note_text(title: &str, body: &str) -> Option<Zeroizing<String>> {
    let content = match markdown::split_frontmatter(body) {
        Some((_, offset)) => &body[offset..],
        None => body,
    };
    let content = content.trim();
    if content.is_empty() {
        return None;
    }
    Some(Zeroizing::new(format!("{}\n\n{content}", title.trim())))
}

pub fn content_hash(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

/// Little-endian `f32`s.
pub fn vector_to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

/// Scales `v` to unit length (a zero vector stays zero).
pub fn normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

/// Combines chunk vectors into the note vector.
pub fn pool_chunks(pooling: ChunkPooling, chunks: &[Vec<f32>]) -> Vec<f32> {
    match pooling {
        ChunkPooling::Mean => {
            let mut mean = vec![0.0; chunks.first().map_or(0, Vec::len)];
            for chunk in chunks {
                let mut c = chunk.clone();
                normalize(&mut c);
                mean.iter_mut().zip(&c).for_each(|(m, x)| *m += x);
            }
            normalize(&mut mean);
            mean
        }
    }
}

/// Cosine similarity; vectors of different lengths (different models) are unrelated.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// Reads a model file into memory and checks it against its pinned SHA-256. The
/// checked bytes are what gets loaded, so the file can't change in between.
pub fn read_verified(dir: &Path, file: &ModelFile) -> Result<Vec<u8>, EmbedError> {
    let path = dir.join(file.name);
    let bytes = std::fs::read(&path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => EmbedError::Missing(path.clone()),
        _ => EmbedError::Read { path: path.clone(), source: e },
    })?;
    let hash = Sha256::digest(&bytes);
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    if hex != file.sha256 {
        return Err(EmbedError::Checksum(path));
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_text_leaves_out_frontmatter_and_skips_empty_notes() {
        let body = "---\ntags: [a]\n---\nHello there\n";
        assert_eq!(note_text("T", body).as_deref().map(String::as_str), Some("T\n\nHello there"));
        assert!(note_text("T", "---\ntags: [a]\n---\n  \n").is_none());
        assert!(note_text("T", "").is_none());
        // Same text after a frontmatter-only change.
        let other = "---\ntags: [a, b]\nauthor: x\n---\nHello there\n";
        assert_eq!(content_hash(&note_text("T", body).unwrap()), content_hash(&note_text("T", other).unwrap()));
    }

    /// The build step fetches what the descriptors expect, with the same checksums.
    #[test]
    fn fetch_manifest_matches_descriptors() {
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../../models/manifest.json")).unwrap();
        let listed: Vec<(String, String, String)> = manifest["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| (f["name"].as_str().unwrap().into(), f["url"].as_str().unwrap().into(), f["sha256"].as_str().unwrap().into()))
            .collect();
        let expected: Vec<(String, String, String)> = MODELS
            .iter()
            .flat_map(|m| [m.model, m.tokenizer])
            .map(|f| (f.name.into(), f.url.into(), f.sha256.into()))
            .collect();
        assert_eq!(listed, expected);
    }

    #[test]
    fn vectors_round_trip_through_blobs() {
        let v = vec![1.0, -0.5, 3.25];
        assert_eq!(blob_to_vector(&vector_to_blob(&v)), v);
        assert_eq!(vector_to_blob(&[1.0]), [0, 0, 0x80, 0x3f]);
    }

    #[test]
    fn chunk_mean_is_unit_length() {
        let v = pool_chunks(ChunkPooling::Mean, &[vec![2.0, 0.0], vec![0.0, 1.0]]);
        assert!((v[0] - v[1]).abs() < 1e-6);
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
        assert!((v.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn wrong_checksum_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = ModelFile { name: "m.onnx", url: "", sha256: BGE_SMALL_EN_V15_Q8.model.sha256 };
        assert!(matches!(read_verified(dir.path(), &file), Err(EmbedError::Missing(_))));
        std::fs::write(dir.path().join("m.onnx"), b"not the model").unwrap();
        assert!(matches!(read_verified(dir.path(), &file), Err(EmbedError::Checksum(_))));
        std::fs::write(dir.path().join("m.onnx"), b"hello").unwrap();
        let ok = ModelFile { sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824", ..file };
        assert_eq!(read_verified(dir.path(), &ok).unwrap(), b"hello");
    }
}
