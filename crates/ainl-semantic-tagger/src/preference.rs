//! Explicit user preference phrase detection.

use crate::tag::{SemanticTag, TagNamespace};

/// Legacy substring cues from `ainl-graph-extractor` `EXPLICIT_BREVITY` (kept for compatibility).
const LEGACY_BREVITY_KEYWORDS: &[&str] = &[
    "shorter",
    "brief",
    "concise",
    "too long",
    "tldr",
    "tl;dr",
    "summarize",
    "less detail",
    "get to the point",
    "keep it short",
];

const BREVITY_PHRASES: &[&str] = &[
    "keep it short",
    "be concise",
    "brief",
    "tl;dr",
    "don't over-explain",
    "do not over-explain",
];

const DETAIL_PHRASES: &[&str] = &[
    "more detail",
    "elaborate",
    "explain more",
    "go deeper",
    "in depth",
];

const EXAMPLES_PHRASES: &[&str] = &[
    "give me an example",
    "show me an example",
    "example please",
    "with examples",
];

const DIRECTNESS_PHRASES: &[&str] = &[
    "just tell me",
    "tell me directly",
    "get to the point",
    "bottom line",
];

fn lower_contains_any(hay: &str, needles: &[&str]) -> bool {
    let l = hay.to_lowercase();
    needles.iter().any(|n| l.contains(n))
}

fn push_if_new(out: &mut Vec<SemanticTag>, tag: SemanticTag) {
    let dup = out.iter().any(|t| t.namespace == tag.namespace && t.value == tag.value);
    if !dup {
        out.push(tag);
    }
}

/// Detects an explicit brevity preference (including legacy `EXPLICIT_BREVITY` substring cues).
pub fn infer_brevity_preference(user_text: &str) -> Option<SemanticTag> {
    tag_user_message(user_text).into_iter().find(|t| {
        t.namespace == TagNamespace::Preference && t.value == "brevity"
    })
}

/// Runs all preference detectors; multiple matches return multiple tags.
pub fn tag_user_message(text: &str) -> Vec<SemanticTag> {
    let mut out = Vec::new();
    let l = text.to_lowercase();

    if BREVITY_PHRASES.iter().any(|p| l.contains(p)) || LEGACY_BREVITY_KEYWORDS.iter().any(|k| l.contains(k)) {
        push_if_new(
            &mut out,
            SemanticTag {
                namespace: TagNamespace::Preference,
                value: "brevity".to_string(),
                confidence: 0.9,
            },
        );
    }
    if lower_contains_any(text, DETAIL_PHRASES) {
        push_if_new(
            &mut out,
            SemanticTag {
                namespace: TagNamespace::Preference,
                value: "detail".to_string(),
                confidence: 0.9,
            },
        );
    }
    if lower_contains_any(text, EXAMPLES_PHRASES) {
        push_if_new(
            &mut out,
            SemanticTag {
                namespace: TagNamespace::Preference,
                value: "examples".to_string(),
                confidence: 0.9,
            },
        );
    }
    if lower_contains_any(text, DIRECTNESS_PHRASES) {
        push_if_new(
            &mut out,
            SemanticTag {
                namespace: TagNamespace::Preference,
                value: "directness".to_string(),
                confidence: 0.85,
            },
        );
    }
    out
}
