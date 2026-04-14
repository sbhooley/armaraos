//! Turn-scoped semantic extraction for **graph-memory fact candidates** (no SQLite store).
//!
//! This complements [`crate::GraphExtractorTask::run_pass`], which walks a persisted
//! [`ainl_memory::SqliteGraphStore`] and returns [`crate::ExtractionReport`] (recurrence updates,
//! persona evolution, merged `RawSignal`s). That pass cannot run inside the agent loop without
//! store access and would duplicate persona side-effects.
//!
//! Here we reuse the same deterministic signal source used inside `extract_pass` — namely
//! [`ainl_semantic_tagger::tag_turn`] over user text, optional assistant text, and tool names —
//! and expose **non–tool, non–tone** [`SemanticTag`]s so callers can map them to lightweight
//! semantic rows (e.g. `ExtractedFact` in `openfang-runtime`).
//!
//! **Rationale for filtering:** episode nodes already list tools; tone tags are high-churn and
//! poor fits for durable “fact” rows compared to topic / preference / correction / behavior.

use ainl_semantic_tagger::{tag_turn, SemanticTag, TagNamespace};

/// Semantic tags from one completed turn suitable for downstream fact extraction.
///
/// Excludes [`TagNamespace::Tool`] (tool list lives on the episode) and [`TagNamespace::Tone`]
/// (keeps graph fact rows focused on preferences, topics, and correction/behavior signals).
pub fn extract_turn_semantic_tags_for_memory(
    user_message: &str,
    assistant_response: Option<&str>,
    tools: &[String],
) -> Vec<SemanticTag> {
    tag_turn(user_message, assistant_response, tools)
        .into_iter()
        .filter(|t| !matches!(t.namespace, TagNamespace::Tool | TagNamespace::Tone))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_extract_includes_topic_rust() {
        let tags = extract_turn_semantic_tags_for_memory(
            "I need help with cargo, serde, and async fn in my rust project",
            Some("We can use tokio for that."),
            &[],
        );
        assert!(
            tags.iter()
                .any(|t| t.namespace == TagNamespace::Topic && t.value == "rust"),
            "expected rust topic, got {tags:?}"
        );
    }

    #[test]
    fn turn_extract_filters_tools_and_tone() {
        let tags =
            extract_turn_semantic_tags_for_memory("Hello", Some("Hi there"), &["bash".into()]);
        assert!(!tags.iter().any(|t| t.namespace == TagNamespace::Tool));
        assert!(!tags.iter().any(|t| t.namespace == TagNamespace::Tone));
    }
}
