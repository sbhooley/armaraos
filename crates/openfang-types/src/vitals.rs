//! Cognitive vitals — a structured signal derived from LLM token-level logprobs.
//!
//! Vitals are **always optional and fail-open**: if a provider does not return logprobs,
//! or if classification fails for any reason, `vitals` on a `CompletionResponse` is `None`
//! and all downstream consumers treat it as if no signal is available.
//!
//! # Design
//! - `gate` is the coarse routing signal: `Pass` / `Warn` / `Fail`.
//! - `phase` is the fine-grained cognitive category with a confidence score.
//! - `trust` is a scalar summary in [0, 1] combining entropy and phase confidence.
//!
//! Downstream consumers (persona evolution, graph memory, AINL frame) all read from
//! `EpisodicNode::vitals_gate` / `vitals_phase` / `vitals_trust`, not from this struct
//! directly — this struct is the in-memory transport; SQLite columns are the durable form.

use serde::{Deserialize, Serialize};

/// The six cognitive categories the classifier can assign.
///
/// Inspired by Styxx's categories but calibrated to our use cases —
/// thresholds are defined in `openfang-runtime::vitals_classifier`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitivePhase {
    /// The model is working through structured reasoning (low entropy, confident tokens).
    Reasoning,
    /// The model is retrieving and reciting known facts (low entropy, high confidence).
    Retrieval,
    /// The model is expressing uncertainty or declining (hedge tokens dominant).
    Refusal,
    /// The model is in open-ended / generative mode (higher entropy, diverse vocabulary).
    Creative,
    /// High variance logprobs suggesting fabrication or low-grounding (hallucination risk).
    Hallucination,
    /// Anomalous token distribution consistent with adversarial injection or jailbreak attempt.
    Adversarial,
}

impl CognitivePhase {
    /// Canonical lowercase string representation (matches serialization).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Reasoning => "reasoning",
            Self::Retrieval => "retrieval",
            Self::Refusal => "refusal",
            Self::Creative => "creative",
            Self::Hallucination => "hallucination",
            Self::Adversarial => "adversarial",
        }
    }
}

/// Coarse routing gate derived from vitals — the signal downstream policy acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VitalsGate {
    /// Normal operation — no intervention needed.
    #[default]
    Pass,
    /// Elevated risk — log, annotate memory, optionally alert.
    Warn,
    /// High risk — flag for review, suppress output in strict contexts.
    Fail,
}

impl VitalsGate {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

impl std::fmt::Display for VitalsGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Cognitive vitals for a single LLM completion.
///
/// Computed from token logprobs in `openfang-runtime::vitals_classifier`.
/// Always `None` when the provider does not supply logprobs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveVitals {
    /// Coarse routing gate.
    pub gate: VitalsGate,
    /// Classified cognitive phase with confidence, e.g. `"reasoning:0.69"`.
    pub phase: String,
    /// Scalar trust score in \[0, 1\]. Higher = more confident / lower entropy.
    pub trust: f32,
    /// Mean token logprob across the sampled window (negative; closer to 0 = more confident).
    pub mean_logprob: f32,
    /// Token-level entropy estimate (nats). Lower = more peaked distribution.
    pub entropy: f32,
    /// Number of tokens sampled for classification.
    pub sample_tokens: u32,
}

impl CognitiveVitals {
    /// Human-readable one-liner, e.g. `"reasoning:0.69 | pass | trust=0.87"`.
    pub fn summary(&self) -> String {
        format!(
            "{} | {} | trust={:.2}",
            self.phase,
            self.gate.as_str(),
            self.trust
        )
    }

    /// Returns `true` if this vitals reading indicates elevated risk (Warn or Fail gate).
    pub fn is_elevated(&self) -> bool {
        matches!(self.gate, VitalsGate::Warn | VitalsGate::Fail)
    }
}
