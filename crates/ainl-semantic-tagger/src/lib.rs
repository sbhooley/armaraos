//! Deterministic, heuristic-only semantic tagging for AINL / ArmaraOS.
//!
//! No ML, embeddings, or graph-store dependency.

mod correction;
mod preference;
mod tag;
mod tone;
mod tool;
mod topic;

pub use correction::{correction_regexes, extract_correction_behavior, CorrectionRegexes};
pub use preference::{infer_brevity_preference, tag_user_message};
pub use tag::{
    quantize_confidence, SemanticTag, TagNamespace, BEHAVIOR_ADDING_CAVEATS,
    BEHAVIOR_OVEREXPLAINING, CORRECTION_AVOID_BULLETS, CORRECTION_AVOID_EMOJIS, PREFERENCE_BREVITY,
    PREFERENCE_DETAIL, PREFERENCE_DIRECTNESS, PREFERENCE_EXAMPLES, TONE_FORMAL, TONE_INFORMAL,
};
pub use tone::infer_formality;
pub use tool::tag_tool_names;
pub use topic::infer_topic_tags;

use std::collections::HashSet;

/// Tags user + assistant text and tool metadata; deduplicates by `(namespace, value)`.
pub fn tag_turn(user: &str, assistant: Option<&str>, tools: &[String]) -> Vec<SemanticTag> {
    let mut combined = user.to_string();
    if let Some(a) = assistant {
        if !combined.is_empty() && !a.is_empty() {
            combined.push(' ');
        }
        combined.push_str(a);
    }

    let mut out: Vec<SemanticTag> = Vec::new();
    out.extend(infer_topic_tags(&combined));
    out.extend(tag_user_message(user));
    if let Some(t) = infer_formality(&combined) {
        out.push(t);
    }
    if let Some(t) = extract_correction_behavior(user) {
        out.push(t);
    }
    if let Some(a) = assistant {
        if let Some(t) = extract_correction_behavior(a) {
            out.push(t);
        }
    }
    out.extend(tag_tool_names(tools));

    let mut seen: HashSet<(TagNamespace, String)> = HashSet::new();
    out.into_iter()
        .filter(|t| seen.insert((t.namespace, t.value.clone())))
        .collect()
}
