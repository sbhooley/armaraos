//! Formality heuristics (migrated verbatim from `ainl-graph-extractor` persona signals).

use crate::tag::{SemanticTag, TagNamespace};

fn formality_score(user: &str) -> f32 {
    let words: Vec<&str> = user
        .split_whitespace()
        .filter(|w| w.chars().any(|c| c.is_alphanumeric()))
        .collect();
    if words.is_empty() {
        return 0.5;
    }
    let n = words.len() as f32;
    let avg_word_len = words.iter().map(|w| w.len()).sum::<usize>() as f32 / n;
    let lower = user.to_lowercase();
    let slang_hits = ["gonna", "wanna", "gotta", "lol", "cool", "yeah", "yo "]
        .iter()
        .filter(|c| lower.contains(*c))
        .count() as f32;
    let slang_density = (slang_hits / n.max(1.0)).min(1.0);
    let contraction_hits = [
        "n't ", "'nt ", "'re ", "'ve ", "'ll ", " i'm", "i'm ", "i’m ",
    ]
    .iter()
    .filter(|c| lower.contains(*c))
    .count() as f32;
    let contraction_density = (contraction_hits / n.max(1.0)).min(1.0);
    let punct =
        user.matches(|c: char| c.is_ascii_punctuation()).count() as f32 / user.len().max(1) as f32;
    let formal = (avg_word_len / 11.0).min(1.0) * 0.38
        + (punct * 10.0).min(1.0) * 0.22
        + (1.0 - (contraction_density * 5.0).min(1.0)) * 0.22
        + (1.0 - (slang_density * 4.0).min(1.0)) * 0.18;
    formal.clamp(0.0, 1.0)
}

/// Returns `tone:formal` or `tone:informal` when the score is outside the ambiguous band
/// `(0.38, 0.62)`; otherwise `None`. Confidence is always `0.75` on a match.
pub fn infer_formality(text: &str) -> Option<SemanticTag> {
    let score = formality_score(text);
    if score <= 0.38 {
        Some(SemanticTag {
            namespace: TagNamespace::Tone,
            value: "informal".to_string(),
            confidence: 0.75,
        })
    } else if score >= 0.62 {
        Some(SemanticTag {
            namespace: TagNamespace::Tone,
            value: "formal".to_string(),
            confidence: 0.75,
        })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formality_score_informal_fixture() {
        let s = formality_score("yo gonna grab some food lol yeah");
        assert!(s < 0.38, "score={s}");
    }

    #[test]
    fn infer_formality_informal() {
        let t = infer_formality("yo gonna grab some food lol yeah").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Tone);
        assert_eq!(t.value, "informal");
    }
}
