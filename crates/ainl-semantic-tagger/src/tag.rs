//! Core semantic tag types and canonical string constants.

use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SemanticTag {
    pub namespace: TagNamespace,
    pub value: String,
    pub confidence: f32,
}

/// Quantize `confidence` to \[0, 100\] for equality and hashing (~two decimal places of resolution).
/// Non-finite values (`NaN`, ±∞) map to **0** so they cannot poison [`Hash`] implementations.
#[inline]
pub fn quantize_confidence(c: f32) -> u8 {
    if !c.is_finite() {
        return 0;
    }
    (c.clamp(0.0, 1.0) * 100.0).round() as u8
}

impl PartialEq for SemanticTag {
    fn eq(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.value == other.value
            && quantize_confidence(self.confidence) == quantize_confidence(other.confidence)
    }
}

impl Eq for SemanticTag {}

impl Hash for SemanticTag {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.namespace.hash(state);
        self.value.hash(state);
        quantize_confidence(self.confidence).hash(state);
    }
}

impl SemanticTag {
    pub fn to_canonical_string(&self) -> String {
        format!("{}:{}", self.namespace.prefix(), self.value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum TagNamespace {
    Topic,
    Preference,
    Correction,
    Behavior,
    Task,
    Tone,
    Tool,
    Domain,
}

impl TagNamespace {
    pub fn prefix(&self) -> &'static str {
        match self {
            Self::Topic => "topic",
            Self::Preference => "preference",
            Self::Correction => "correction",
            Self::Behavior => "behavior",
            Self::Task => "task",
            Self::Tone => "tone",
            Self::Tool => "tool",
            Self::Domain => "domain",
        }
    }
}

pub const PREFERENCE_BREVITY: &str = "preference:brevity";
pub const PREFERENCE_DETAIL: &str = "preference:detail";
pub const PREFERENCE_EXAMPLES: &str = "preference:examples";
pub const PREFERENCE_DIRECTNESS: &str = "preference:directness";
pub const TONE_FORMAL: &str = "tone:formal";
pub const TONE_INFORMAL: &str = "tone:informal";
pub const CORRECTION_AVOID_BULLETS: &str = "correction:avoid_bullets";
pub const CORRECTION_AVOID_EMOJIS: &str = "correction:avoid_emojis";
pub const BEHAVIOR_ADDING_CAVEATS: &str = "behavior:adding_caveats";
pub const BEHAVIOR_OVEREXPLAINING: &str = "behavior:overexplaining";

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;

    fn hash_one(tag: &SemanticTag) -> u64 {
        let mut h = DefaultHasher::new();
        tag.hash(&mut h);
        h.finish()
    }

    #[test]
    fn eq_and_hash_quantize_confidence() {
        let a = SemanticTag {
            namespace: TagNamespace::Topic,
            value: "rust".into(),
            confidence: 0.849,
        };
        let b = SemanticTag {
            namespace: TagNamespace::Topic,
            value: "rust".into(),
            confidence: 0.851,
        };
        assert_eq!(a, b);
        assert_eq!(hash_one(&a), hash_one(&b));
    }

    #[test]
    fn nan_confidence_hashes_like_zero_quantized() {
        let a = SemanticTag {
            namespace: TagNamespace::Tool,
            value: "x".into(),
            confidence: f32::NAN,
        };
        let b = SemanticTag {
            namespace: TagNamespace::Tool,
            value: "x".into(),
            confidence: 0.0,
        };
        assert_eq!(a, b);
        assert_eq!(hash_one(&a), hash_one(&b));
    }

    #[test]
    fn non_finite_confidence_quantizes_to_zero() {
        assert_eq!(quantize_confidence(f32::NAN), 0);
        assert_eq!(quantize_confidence(f32::INFINITY), 0);
        assert_eq!(quantize_confidence(f32::NEG_INFINITY), 0);
    }

    #[test]
    fn distinct_quantized_confidence_not_equal() {
        let low = SemanticTag {
            namespace: TagNamespace::Topic,
            value: "t".into(),
            confidence: 0.004,
        };
        let high = SemanticTag {
            namespace: TagNamespace::Topic,
            value: "t".into(),
            confidence: 0.006,
        };
        assert_eq!(quantize_confidence(0.004), 0);
        assert_eq!(quantize_confidence(0.006), 1);
        assert_ne!(low, high);
        assert_ne!(hash_one(&low), hash_one(&high));
    }
}
