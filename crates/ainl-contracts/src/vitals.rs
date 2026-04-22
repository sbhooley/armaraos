//! Cognitive vitals — structured signal derived from LLM token-level logprobs.
//!
//! Canonical location for cross-runtime use (`openfang-types::vitals` re-exports this module).
//! Vitals are **optional and fail-open** when providers omit logprobs.

use serde::{Deserialize, Serialize};

/// The six cognitive categories the classifier can assign.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitivePhase {
    Reasoning,
    Retrieval,
    Refusal,
    Creative,
    Hallucination,
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

/// Coarse routing gate derived from vitals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum VitalsGate {
    #[default]
    Pass,
    Warn,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveVitals {
    pub gate: VitalsGate,
    /// Classified cognitive phase with confidence, e.g. `"reasoning:0.69"`.
    pub phase: String,
    pub trust: f32,
    pub mean_logprob: f32,
    pub entropy: f32,
    pub sample_tokens: u32,
}

impl CognitiveVitals {
    pub fn summary(&self) -> String {
        format!(
            "{} | {} | trust={:.2}",
            self.phase,
            self.gate.as_str(),
            self.trust
        )
    }

    pub fn is_elevated(&self) -> bool {
        matches!(self.gate, VitalsGate::Warn | VitalsGate::Fail)
    }
}
