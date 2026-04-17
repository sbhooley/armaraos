//! Optional bridge to **`ainl-semantic-tagger`** for graph-memory node tags.
//!
//! When the `ainl-tagger` Cargo feature is off, [`SemanticTaggerBridge::tag_fact`] and
//! [`SemanticTaggerBridge::tag_episode`] are no-ops (empty vectors). When the feature is on,
//! tagging runs only if **`AINL_TAGGER_ENABLED=1`** at node-write time (see crate README).

/// Returns true when the `ainl-tagger` feature is enabled **and** `AINL_TAGGER_ENABLED` is set to `1`.
#[inline]
pub fn tagger_writes_enabled() -> bool {
    #[cfg(not(feature = "ainl-tagger"))]
    {
        false
    }
    #[cfg(feature = "ainl-tagger")]
    {
        std::env::var("AINL_TAGGER_ENABLED")
            .map(|v| v.trim() == "1")
            .unwrap_or(false)
    }
}

/// Thin facade over the optional `ainl-semantic-tagger` dependency (same pattern as graph-extractor).
pub struct SemanticTaggerBridge;

impl SemanticTaggerBridge {
    /// Category tags for a single fact string (namespaced `topic:…`, `preference:…`, etc.).
    pub fn tag_fact(fact: &str) -> Vec<String> {
        #[cfg(not(feature = "ainl-tagger"))]
        {
            let _ = fact;
            Vec::new()
        }
        #[cfg(feature = "ainl-tagger")]
        {
            if !tagger_writes_enabled() {
                Vec::new()
            } else {
                ainl_semantic_tagger::tag_turn(fact, None, &[])
                    .into_iter()
                    .map(|t| t.to_canonical_string())
                    .collect()
            }
        }
    }

    /// Tags derived from the episode’s tool-call sequence (tool namespace).
    pub fn tag_episode(tool_calls: &[String]) -> Vec<String> {
        #[cfg(not(feature = "ainl-tagger"))]
        {
            let _ = tool_calls;
            Vec::new()
        }
        #[cfg(feature = "ainl-tagger")]
        {
            if !tagger_writes_enabled() {
                Vec::new()
            } else {
                ainl_semantic_tagger::tag_tool_names(tool_calls)
                    .into_iter()
                    .map(|t| t.to_canonical_string())
                    .collect()
            }
        }
    }
}

#[cfg(all(test, feature = "ainl-tagger"))]
mod tests {
    use super::*;

    #[test]
    fn test_tag_fact_returns_strings() {
        std::env::set_var("AINL_TAGGER_ENABLED", "1");
        let tags = SemanticTaggerBridge::tag_fact("I need help with rust serde and async traits");
        assert!(
            tags.iter()
                .any(|t| t.contains("rust") || t.starts_with("topic:")),
            "expected topic-style tags, got {tags:?}"
        );
        std::env::remove_var("AINL_TAGGER_ENABLED");
    }

    #[test]
    fn test_tag_episode_from_tool_sequence() {
        std::env::set_var("AINL_TAGGER_ENABLED", "1");
        let tools = vec!["web_search".to_string(), "file_read".to_string()];
        let tags = SemanticTaggerBridge::tag_episode(&tools);
        assert!(
            tags.iter()
                .any(|t| t.contains("search_web") || t.contains("file_read")),
            "expected tool-derived tags, got {tags:?}"
        );
        std::env::remove_var("AINL_TAGGER_ENABLED");
    }
}
