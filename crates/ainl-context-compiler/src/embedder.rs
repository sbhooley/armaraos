//! Tier 2 embedding surface (M3).
//!
//! M1 ships the *trait* (so callers can already opt in via `with_embedder`); the M3 milestone
//! adds a concrete adapter (e.g. `OnnxMiniLMEmbedder`) and the rerank step in the orchestrator.
//! With no embedder injected, the orchestrator stays at Tier 0 / Tier 1.

use std::error::Error;
use std::fmt;

/// Errors an [`Embedder`] implementation may return.
#[derive(Debug)]
pub enum EmbedderError {
    /// Network / IO error.
    Transport(String),
    /// Model not loaded.
    ModelMissing,
    /// Catch-all.
    Other(String),
}

impl fmt::Display for EmbedderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) => write!(f, "embedder transport: {m}"),
            Self::ModelMissing => f.write_str("embedder model not loaded"),
            Self::Other(m) => write!(f, "embedder: {m}"),
        }
    }
}

impl Error for EmbedderError {}

/// Pluggable embedding backend (M3).
///
/// Implementations should return a fixed-dimension `Vec<f32>` per text; the orchestrator
/// computes cosine similarity between the latest user query and each segment to rerank.
///
/// Marked `Send + Sync` so a single embedder instance can be shared via `Arc`.
pub trait Embedder: Send + Sync {
    /// Embed a single text. Returns a fixed-dimension vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbedderError>;

    /// Embed a batch (default impl loops; backends should override for efficiency).
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }
}

/// Cosine similarity between two equal-length vectors. Returns 0.0 on length mismatch.
#[must_use]
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_identical_is_one() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_orthogonal_is_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!(cosine(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn cosine_mismatched_lengths_returns_zero() {
        let a = vec![1.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert_eq!(cosine(&a, &b), 0.0);
    }
}
