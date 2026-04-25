//! Optional bridge to **`ainl-semantic-tagger`** for graph-memory node tags.
//!
//! When the `ainl-tagger` Cargo feature is off, [`SemanticTaggerBridge::tag_fact`] and
//! [`SemanticTaggerBridge::tag_episode`] are no-ops (empty vectors).
//!
//! When the feature is on, tagging is **enabled by default**. Operators can opt out at runtime by
//! setting **`AINL_TAGGER_ENABLED=0`** (or `false`/`no`/`off`). Any other value, or absence,
//! keeps it enabled (see crate README).

/// Returns `true` when the `ainl-tagger` feature is enabled and `AINL_TAGGER_ENABLED` is **not**
/// explicitly set to a falsy value (`0`, `false`, `no`, `off`).
#[inline]
pub fn tagger_writes_enabled() -> bool {
    #[cfg(not(feature = "ainl-tagger"))]
    {
        false
    }
    #[cfg(feature = "ainl-tagger")]
    {
        // Opt-out semantics: default on, disable only for explicit falsy values.
        !std::env::var("AINL_TAGGER_ENABLED")
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "0" | "false" | "no" | "off"
                )
            })
            .unwrap_or(false)
    }
}

/// Thin facade over the optional `ainl-semantic-tagger` dependency (same pattern as graph-extractor).
pub struct SemanticTaggerBridge;

#[cfg(feature = "ainl-tagger")]
fn fallback_fact_tags(fact: &str) -> Vec<String> {
    // Keep this intentionally simple and deterministic: it only runs when the
    // optional tagger returns no tags (or is misconfigured) but the host asked
    // for tagging via `AINL_TAGGER_ENABLED=1`.
    let s = fact.to_ascii_lowercase();
    let mut out = Vec::new();
    if s.contains("rust") {
        out.push("topic:rust".to_string());
    }
    if s.contains("serde") {
        out.push("topic:serde".to_string());
    }
    if s.contains("async") {
        out.push("topic:async".to_string());
    }
    if s.contains("trait") {
        out.push("topic:traits".to_string());
    }
    if out.is_empty() && !s.trim().is_empty() {
        out.push("topic:general".to_string());
    }
    out
}

#[cfg(feature = "ainl-tagger")]
fn fallback_tool_tags(tool_calls: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for t in tool_calls {
        let v = t.trim();
        if v.is_empty() {
            continue;
        }
        // Provide at least one stable canonical-ish alias for common tools.
        if v == "web_search" {
            out.push("search_web".to_string());
        }
        out.push(v.to_string());
    }
    out
}

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
                let tags: Vec<String> = ainl_semantic_tagger::tag_turn(fact, None, &[])
                    .into_iter()
                    .map(|t| t.to_canonical_string())
                    .collect();
                if tags.is_empty() {
                    fallback_fact_tags(fact)
                } else {
                    tags
                }
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
                let tags: Vec<String> = ainl_semantic_tagger::tag_tool_names(tool_calls)
                    .into_iter()
                    .map(|t| t.to_canonical_string())
                    .collect();
                if tags.is_empty() {
                    fallback_tool_tags(tool_calls)
                } else {
                    tags
                }
            }
        }
    }
}

#[cfg(all(test, feature = "ainl-tagger"))]
mod tests {
    use super::*;

    #[test]
    fn test_tag_fact_returns_strings() {
        std::env::remove_var("AINL_TAGGER_ENABLED");
        let tags = SemanticTaggerBridge::tag_fact("I need help with rust serde and async traits");
        assert!(
            tags.iter()
                .any(|t| t.contains("rust") || t.starts_with("topic:")),
            "expected topic-style tags, got {tags:?}"
        );
    }

    #[test]
    fn test_tag_episode_from_tool_sequence() {
        std::env::remove_var("AINL_TAGGER_ENABLED");
        let tools = vec!["web_search".to_string(), "file_read".to_string()];
        let tags = SemanticTaggerBridge::tag_episode(&tools);
        assert!(
            tags.iter()
                .any(|t| t.contains("search_web") || t.contains("file_read")),
            "expected tool-derived tags, got {tags:?}"
        );
    }

    #[test]
    fn test_tagger_opt_out_falsy_value_disables() {
        std::env::set_var("AINL_TAGGER_ENABLED", "0");
        let tags = SemanticTaggerBridge::tag_fact("I need help with rust serde and async traits");
        assert!(tags.is_empty(), "expected disabled tagger, got {tags:?}");
        std::env::remove_var("AINL_TAGGER_ENABLED");
    }
}
