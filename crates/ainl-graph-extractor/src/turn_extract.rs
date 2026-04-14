//! Turn-scoped semantic extraction for **graph-memory fact candidates** (no SQLite store).
//!
//! This complements [`crate::GraphExtractorTask::run_pass`], which walks a persisted
//! [`ainl_memory::SqliteGraphStore`] and returns [`crate::ExtractionReport`] (recurrence updates,
//! persona evolution, merged `RawSignal`s). That pass cannot run inside the agent loop without
//! store access and would duplicate persona side-effects.
//!
//! Here we reuse the same deterministic signal source used inside `extract_pass` — namely
//! [`ainl_semantic_tagger::tag_turn`] over user text, optional assistant text, and tool names —
//! and expose **non-tool, non-tone** [`SemanticTag`]s so callers can map them to lightweight
//! semantic rows (e.g. `ExtractedFact` in `openfang-runtime`).
//!
//! **Rationale for filtering:** episode nodes already list tools; tone tags are high-churn and
//! poor fits for durable "fact" rows compared to topic / preference / correction / behavior.
//!
//! Vitals: when [`TurnVitals`] are available they are surfaced as additional
//! [`TagNamespace::Behavior`] tags (e.g. `"vitals:reasoning:pass"`) so downstream consumers
//! can index or route on cognitive state without depending on `openfang-types` directly.

use ainl_semantic_tagger::{tag_turn, SemanticTag, TagNamespace};

/// Minimal vitals snapshot accepted by this crate (avoids a direct `openfang-types` dep).
///
/// Callers in `openfang-runtime` construct this from `openfang_types::vitals::CognitiveVitals`.
#[derive(Debug, Clone)]
pub struct TurnVitals {
    /// Gate string: "pass" / "warn" / "fail".
    pub gate: String,
    /// Phase string, e.g. "reasoning:0.69".
    pub phase: String,
    /// Trust score in [0, 1].
    pub trust: f32,
}

/// Semantic tags from one completed turn suitable for downstream fact extraction.
///
/// Excludes [`TagNamespace::Tool`] (tool list lives on the episode) and [`TagNamespace::Tone`]
/// (keeps graph fact rows focused on preferences, topics, and correction/behavior signals).
///
/// When `vitals` is `Some`, additional [`TagNamespace::Behavior`] tags are appended:
/// - `"vitals:<phase_kind>:<gate>"` — primary routing tag, e.g. `"vitals:reasoning:pass"`
/// - `"vitals:elevated"` — present only when gate is `"warn"` or `"fail"`
pub fn extract_turn_semantic_tags_for_memory(
    user_message: &str,
    assistant_response: Option<&str>,
    tools: &[String],
    vitals: Option<&TurnVitals>,
) -> Vec<SemanticTag> {
    let mut tags: Vec<SemanticTag> = tag_turn(user_message, assistant_response, tools)
        .into_iter()
        .filter(|t| !matches!(t.namespace, TagNamespace::Tool | TagNamespace::Tone))
        .collect();

    if let Some(v) = vitals {
        let phase_kind = v.phase.split(':').next().unwrap_or("unknown");
        tags.push(SemanticTag {
            namespace: TagNamespace::Behavior,
            value: format!("vitals:{}:{}", phase_kind, v.gate),
            confidence: v.trust,
        });
        if v.gate == "warn" || v.gate == "fail" {
            tags.push(SemanticTag {
                namespace: TagNamespace::Behavior,
                value: "vitals:elevated".to_string(),
                confidence: 1.0 - v.trust,
            });
        }
    }

    tags
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
            None,
        );
        assert!(
            tags.iter()
                .any(|t| t.namespace == TagNamespace::Topic && t.value == "rust"),
            "expected rust topic, got {tags:?}"
        );
    }

    #[test]
    fn turn_extract_filters_tools_and_tone() {
        let tags = extract_turn_semantic_tags_for_memory(
            "Hello",
            Some("Hi there"),
            &["bash".into()],
            None,
        );
        assert!(!tags.iter().any(|t| t.namespace == TagNamespace::Tool));
        assert!(!tags.iter().any(|t| t.namespace == TagNamespace::Tone));
    }

    #[test]
    fn turn_extract_vitals_pass_tag() {
        let v = TurnVitals {
            gate: "pass".to_string(),
            phase: "reasoning:0.72".to_string(),
            trust: 0.72,
        };
        let tags = extract_turn_semantic_tags_for_memory("Hello", None, &[], Some(&v));
        assert!(
            tags.iter()
                .any(|t| t.namespace == TagNamespace::Behavior
                    && t.value == "vitals:reasoning:pass"),
            "expected vitals:reasoning:pass tag"
        );
        assert!(
            !tags.iter().any(|t| t.value == "vitals:elevated"),
            "should not have elevated tag on pass"
        );
    }

    #[test]
    fn turn_extract_vitals_warn_elevated() {
        let v = TurnVitals {
            gate: "warn".to_string(),
            phase: "hallucination:0.40".to_string(),
            trust: 0.40,
        };
        let tags = extract_turn_semantic_tags_for_memory("tell me facts", None, &[], Some(&v));
        assert!(
            tags.iter().any(|t| t.value == "vitals:elevated"),
            "expected vitals:elevated tag on warn"
        );
    }
}
