//! Embedding-free content classifier for **adaptive eco** mode selection.
//!
//! Hosts may combine this with `CompressionProfile` defaults, telemetry, and user prefs.

use crate::EfficientMode;

/// Outcome of [`recommend_mode_for_content`].
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveRecommendation {
    pub mode: EfficientMode,
    /// Heuristic confidence in \[0, 1\].
    pub confidence: f32,
    pub reasons: Vec<&'static str>,
}

fn est_tokens(text: &str) -> usize {
    text.len() / 4 + 1
}

/// Rough signal for code / URL / opcode-heavy prompts (higher → keep safer mode).
fn structure_density(text: &str) -> f32 {
    let len = text.len().max(1) as f32;
    let mut score = 0.0_f32;
    score += text.matches("```").count() as f32 * 0.08;
    score += text.matches("http://").count() as f32 * 0.04;
    score += text.matches("https://").count() as f32 * 0.04;
    score += text.matches("::").count() as f32 * 0.01;
    score += text.matches("R ").count() as f32 * 0.02;
    score += text.matches(".ainl").count() as f32 * 0.03;
    (score / len * 600.0).clamp(0.0, 1.0)
}

/// Recommend an [`EfficientMode`] from raw prompt text (mirrors short-prompt passthrough threshold in [`crate::compress`]).
#[must_use]
pub fn recommend_mode_for_content(text: &str) -> AdaptiveRecommendation {
    let tokens = est_tokens(text);
    let density = structure_density(text);
    let mut reasons: Vec<&'static str> = Vec::new();

    if tokens < 80 {
        reasons.push("below_compression_floor");
        return AdaptiveRecommendation {
            mode: EfficientMode::Off,
            confidence: 0.88,
            reasons,
        };
    }

    if density > 0.32 {
        reasons.push("structured_or_code_heavy");
        return AdaptiveRecommendation {
            mode: EfficientMode::Balanced,
            confidence: 0.72,
            reasons,
        };
    }

    if tokens > 420 {
        reasons.push("long_prose_budget_pressure");
        return AdaptiveRecommendation {
            mode: EfficientMode::Aggressive,
            confidence: 0.58,
            reasons,
        };
    }

    reasons.push("general_prompt");
    AdaptiveRecommendation {
        mode: EfficientMode::Balanced,
        confidence: 0.62,
        reasons,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_is_off() {
        let r = recommend_mode_for_content("Hello.");
        assert_eq!(r.mode, EfficientMode::Off);
    }

    #[test]
    fn code_fence_prefers_balanced() {
        let filler = "y".repeat(400);
        let r = recommend_mode_for_content(&(filler + "\n```rust\nfn main(){}\n```"));
        assert_eq!(r.mode, EfficientMode::Balanced);
    }
}
