//! Cognitive vitals classifier: logprobs → [`CognitiveVitals`].
//!
//! Maps token-level logprobs from an LLM completion into a structured vitals reading.
//! Uses entropy and per-token logprob statistics — no external model, no atlas data,
//! no network calls. Designed to work on the first N tokens of a completion.
//!
//! # Category heuristics
//!
//! We classify into [`CognitivePhase`] by combining:
//! - **Mean logprob** (higher = more confident; typical range: -1.5 to -0.05)
//! - **Entropy estimate** from `top_logprobs` (lower = more peaked = higher certainty)
//! - **Vocabulary markers** — hedge tokens, refusal tokens, high-entropy token ids
//!
//! Thresholds are calibrated to our use cases; they are deliberately conservative and
//! should be tuned once we have real production data via graph memory analytics.
//!
//! # Provider notes
//! - OpenAI: `logprobs=true, top_logprobs=5` on the request; see `drivers/openai.rs`.
//! - OpenRouter: passthrough to the underlying model's logprob support.
//! - All other providers: return `None`; this module is not called.

use openfang_types::vitals::{CognitivePhase, CognitiveVitals, VitalsGate};

/// Maximum tokens to sample for classification (efficiency cap).
const MAX_SAMPLE_TOKENS: usize = 24;

/// A single token's logprob entry as parsed from the provider response.
#[derive(Debug, Clone)]
pub struct TokenLogprob {
    /// The token text.
    pub token: String,
    /// Log probability of this token under the model (natural log scale, ≤ 0.0).
    pub logprob: f32,
    /// Alternative tokens and their logprobs at this position (top-K from provider).
    pub top_alternatives: Vec<(String, f32)>,
}

/// Classify a sequence of token logprobs into [`CognitiveVitals`].
///
/// Returns `None` if the slice is empty or contains no usable data.
/// Never panics.
pub fn classify(tokens: &[TokenLogprob]) -> Option<CognitiveVitals> {
    if tokens.is_empty() {
        return None;
    }

    let sample = &tokens[..tokens.len().min(MAX_SAMPLE_TOKENS)];
    let n = sample.len() as f32;

    // --- Basic statistics ---
    let mean_logprob: f32 = sample.iter().map(|t| t.logprob).sum::<f32>() / n;

    // Entropy estimate: average over positions of H = -sum(p * log(p)) across top alternatives.
    // When top alternatives are unavailable for a position, use a fallback from logprob alone.
    let entropy: f32 = sample
        .iter()
        .map(position_entropy)
        .sum::<f32>()
        / n;

    // Trust: inverse normalisation of entropy and mean logprob combined.
    // High trust = low entropy + logprob close to 0.
    let trust = compute_trust(mean_logprob, entropy);

    // --- Category classification ---
    let phase = classify_phase(sample, mean_logprob, entropy);
    let phase_str = format!("{}:{:.2}", phase.as_str(), trust);

    // --- Gate derivation ---
    let gate = derive_gate(phase, trust, entropy);

    Some(CognitiveVitals {
        gate,
        phase: phase_str,
        trust,
        mean_logprob,
        entropy,
        sample_tokens: sample.len() as u32,
    })
}

/// Entropy at a single token position from its top alternatives.
fn position_entropy(tok: &TokenLogprob) -> f32 {
    // If we have top alternatives, compute proper entropy from them.
    if !tok.top_alternatives.is_empty() {
        // Convert logprobs → probabilities, compute H = -Σ p·ln(p).
        let log_sum = log_sum_exp(
            std::iter::once(tok.logprob)
                .chain(tok.top_alternatives.iter().map(|(_, lp)| *lp)),
        );
        let entropy: f32 = std::iter::once(tok.logprob)
            .chain(tok.top_alternatives.iter().map(|(_, lp)| *lp))
            .map(|lp| {
                let p = (lp - log_sum).exp();
                if p > 1e-9 { -p * lp } else { 0.0 }
            })
            .sum();
        return entropy.max(0.0);
    }
    // Fallback: single-token entropy proxy from logprob alone.
    // If the model is very certain (logprob ≈ 0), entropy is low.
    let p = tok.logprob.exp().clamp(1e-9, 1.0);
    -p * tok.logprob
}

/// Numerically stable log-sum-exp.
fn log_sum_exp(iter: impl Iterator<Item = f32>) -> f32 {
    let vals: Vec<f32> = iter.collect();
    if vals.is_empty() {
        return f32::NEG_INFINITY;
    }
    let max = vals.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    if max.is_infinite() {
        return max;
    }
    let sum: f32 = vals.iter().map(|&v| (v - max).exp()).sum();
    max + sum.ln()
}

/// Normalize mean_logprob and entropy into a [0, 1] trust score.
fn compute_trust(mean_logprob: f32, entropy: f32) -> f32 {
    // Mean logprob in [-5, 0]; clamp and invert (closer to 0 = higher trust).
    let logprob_score = (mean_logprob.clamp(-5.0, 0.0) / 5.0 + 1.0).clamp(0.0, 1.0);
    // Entropy in [0, ~3] nats for 5 alternatives; lower = higher trust.
    let entropy_score = (1.0 - (entropy / 3.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    // Weighted average: entropy is slightly more predictive.
    (0.4 * logprob_score + 0.6 * entropy_score).clamp(0.0, 1.0)
}

/// Known prompt-injection phrase starters — any single token that, when normalised, matches one
/// of these prefixes strongly suggests the model is outputting a prompt-injection pattern.
const ADVERSARIAL_LEAD_TOKENS: &[&str] = &[
    "ignore", "disregard", "forget", "override", "bypass",
    "jailbreak", "pretend", "roleplay",
];

/// Secondary words that, when appearing within 5 tokens of an adversarial lead, confirm injection.
const ADVERSARIAL_FOLLOW_TOKENS: &[&str] = &[
    "previous", "above", "prior", "instructions", "system",
    "prompt", "rules", "context", "constraints",
];

/// Classify the dominant cognitive phase from the token window.
fn classify_phase(tokens: &[TokenLogprob], mean_logprob: f32, entropy: f32) -> CognitivePhase {
    // --- Adversarial: anomalous entropy OR known injection vocabulary in the first tokens ---
    // Path 1: entropy spike at first token (high uncertainty about what comes next).
    if let Some(first) = tokens.first() {
        let first_entropy = position_entropy(first);
        if first_entropy > 2.5 && entropy > 2.0 {
            return CognitivePhase::Adversarial;
        }
    }
    // Path 2: injection n-gram detection.
    // A lead adversarial token ("ignore", "disregard", …) followed by at least one
    // follow token ("previous", "instructions", …) within the first 8 tokens is a
    // high-confidence indicator of a prompt injection pattern.
    let window = tokens.iter().take(8).collect::<Vec<_>>();
    let has_lead = window.iter().any(|t| {
        let lower = t.token.trim().to_ascii_lowercase();
        ADVERSARIAL_LEAD_TOKENS
            .iter()
            .any(|kw| lower.starts_with(kw))
    });
    if has_lead {
        let has_follow = window.iter().any(|t| {
            let lower = t.token.trim().to_ascii_lowercase();
            ADVERSARIAL_FOLLOW_TOKENS
                .iter()
                .any(|kw| lower.starts_with(kw))
        });
        if has_follow {
            return CognitivePhase::Adversarial;
        }
    }

    // --- Refusal: hedge/refusal tokens appear in the first 5 tokens ---
    let refusal_token_count = tokens
        .iter()
        .take(5)
        .filter(|t| is_refusal_token(&t.token))
        .count();
    if refusal_token_count >= 1 {
        return CognitivePhase::Refusal;
    }

    // --- Hallucination: moderate mean logprob + high entropy sustained ---
    // The model is generating plausible-sounding tokens with weak grounding.
    if mean_logprob < -2.0 && entropy > 1.8 {
        return CognitivePhase::Hallucination;
    }

    // --- Reasoning: low entropy, moderate-to-good confidence, structured tokens ---
    if entropy < 1.0 && mean_logprob > -1.5 {
        return CognitivePhase::Reasoning;
    }

    // --- Retrieval: very high confidence, low entropy (verbatim recall) ---
    if mean_logprob > -0.5 && entropy < 0.5 {
        return CognitivePhase::Retrieval;
    }

    // --- Creative: high entropy, acceptable confidence ---
    if entropy > 1.5 && mean_logprob > -2.5 {
        return CognitivePhase::Creative;
    }

    // Default: treat as reasoning (low-risk default).
    CognitivePhase::Reasoning
}

/// Derive the coarse gate from phase + trust.
fn derive_gate(phase: CognitivePhase, trust: f32, entropy: f32) -> VitalsGate {
    match phase {
        CognitivePhase::Adversarial => VitalsGate::Fail,
        CognitivePhase::Hallucination => {
            if trust < 0.35 || entropy > 2.2 {
                VitalsGate::Fail
            } else {
                VitalsGate::Warn
            }
        }
        CognitivePhase::Refusal => VitalsGate::Warn,
        CognitivePhase::Creative => {
            // Creative is expected high-entropy; only warn at extremes.
            if trust < 0.25 {
                VitalsGate::Warn
            } else {
                VitalsGate::Pass
            }
        }
        CognitivePhase::Reasoning | CognitivePhase::Retrieval => {
            if trust < 0.4 {
                VitalsGate::Warn
            } else {
                VitalsGate::Pass
            }
        }
    }
}

/// Returns true if the token text is a known refusal/hedge indicator.
fn is_refusal_token(token: &str) -> bool {
    let t = token.trim().to_ascii_lowercase();
    // Common refusal openings.
    matches!(
        t.as_str(),
        "i" | "sorry"
            | "unfortunately"
            | "i'm"
            | "i cannot"
            | "i can't"
            | "i'm unable"
            | "i am unable"
            | "apolog"
            | "regret"
    ) || t.contains("cannot") && t.len() < 20
        || t.starts_with("apolog")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tok(token: &str, logprob: f32, alts: &[(&str, f32)]) -> TokenLogprob {
        TokenLogprob {
            token: token.to_string(),
            logprob,
            top_alternatives: alts.iter().map(|(t, lp)| (t.to_string(), *lp)).collect(),
        }
    }

    #[test]
    fn empty_returns_none() {
        assert!(classify(&[]).is_none());
    }

    #[test]
    fn high_confidence_reasoning() {
        let tokens = vec![
            tok("The", -0.1, &[("A", -1.5), ("This", -1.8)]),
            tok(" answer", -0.2, &[(" result", -1.2), (" solution", -1.6)]),
            tok(" is", -0.05, &[(" was", -2.0), (" are", -2.3)]),
        ];
        let v = classify(&tokens).expect("should classify");
        assert_eq!(v.gate, VitalsGate::Pass);
        assert!(v.trust > 0.5, "trust={}", v.trust);
    }

    #[test]
    fn refusal_detected() {
        let tokens = vec![
            tok("Sorry", -0.3, &[("I", -0.8), ("Unfortunately", -1.2)]),
            tok(",", -0.1, &[]),
            tok(" I", -0.2, &[]),
        ];
        let v = classify(&tokens).expect("should classify");
        assert!(v.phase.starts_with("refusal"), "phase={}", v.phase);
        assert_eq!(v.gate, VitalsGate::Warn);
    }

    #[test]
    fn high_entropy_adversarial() {
        // Simulate anomalous first-token entropy by providing many near-equal alternatives.
        let alts: Vec<(&str, f32)> = (0..5).map(|i| ("x", -1.5 - i as f32 * 0.1)).collect();
        let tokens = vec![
            tok("Ignore", -0.8, &alts),
            tok(" previous", -0.9, &alts),
            tok(" instructions", -0.9, &alts),
        ];
        let v = classify(&tokens).expect("should classify");
        // With genuinely high entropy (many near-equal alternatives) this should Warn or Fail.
        assert!(
            matches!(v.gate, VitalsGate::Warn | VitalsGate::Fail),
            "gate={:?}",
            v.gate
        );
    }

    #[test]
    fn hallucination_high_entropy_low_logprob() {
        let alts: Vec<(&str, f32)> = (0..5).map(|i| ("y", -2.0 - i as f32 * 0.2)).collect();
        let tokens: Vec<TokenLogprob> = (0..6)
            .map(|_| tok("word", -2.5, &alts))
            .collect();
        let v = classify(&tokens).expect("should classify");
        assert!(
            matches!(v.gate, VitalsGate::Warn | VitalsGate::Fail),
            "gate={:?} phase={}",
            v.gate,
            v.phase
        );
    }

    #[test]
    fn summary_format() {
        let tokens = vec![tok("Hello", -0.2, &[("Hi", -1.0)])];
        let v = classify(&tokens).expect("should classify");
        let s = v.summary();
        assert!(s.contains("trust="), "summary={s}");
        assert!(s.contains('|'), "summary={s}");
    }
}
