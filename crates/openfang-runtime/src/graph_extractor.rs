//! Lightweight post-turn extraction: derives Semantic and Procedural graph
//! nodes from completed agent turns without requiring AINL programs.
//!
//! Extraction is intentionally heuristic and non-LLM (no extra API call):
//! fast regex + structural analysis over the assistant response text.

use regex::Regex;
use std::sync::OnceLock;

/// A fact extracted from an assistant response.
#[derive(Debug, Clone)]
pub struct ExtractedFact {
    pub text: String,
    pub confidence: f32,
}

/// A tool sequence pattern extracted from a completed turn.
#[derive(Debug, Clone)]
pub struct ExtractedPattern {
    pub name: String,
    pub tool_sequence: Vec<String>,
    pub confidence: f32,
}

/// Extract semantic facts from an assistant response + user message.
///
/// Rules (heuristic, no LLM call):
/// 1. Named entity mentions with a clear assertion ("X is Y", "X was Y",
///    "X means Y", "X refers to Y") — confidence 0.7
/// 2. Explicit memory-worthy statements the user made about themselves
///    ("I work at", "I prefer", "I always", "my X is") — confidence 0.85
/// 3. Tool results that were summarized into facts by the agent
///    ("found that", "the result is", "according to", "retrieved:") — confidence 0.6
/// 4. Decisions or outcomes stated by the agent
///    ("I've set", "I've created", "I've saved", "completed:", "done:") — confidence 0.75
///
/// Returns up to 6 facts per turn to avoid graph bloat.
pub fn extract_facts(user_message: &str, assistant_response: &str) -> Vec<ExtractedFact> {
    let mut facts: Vec<ExtractedFact> = Vec::new();

    // Rule 2: user self-disclosures (highest confidence — explicit user-stated facts)
    static USER_FACT_RE: OnceLock<Regex> = OnceLock::new();
    let user_re = USER_FACT_RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(i (?:work at|am a|am an|prefer|always|use|live in|am based in|my \w+ is|my name is)|my \w+ is\b)"
        ).unwrap()
    });
    for m in user_re.find_iter(user_message) {
        // Extract the surrounding sentence (up to 120 chars from match start)
        let start = m.start();
        let end = (m.end() + 100).min(user_message.len());
        let sentence = user_message[start..end]
            .split(['.', '\n', '!', '?'])
            .next()
            .unwrap_or("")
            .trim()
            .to_string();
        if sentence.len() > 10 {
            facts.push(ExtractedFact {
                text: format!("User said: {sentence}"),
                confidence: 0.85,
            });
        }
        if facts.len() >= 2 {
            break;
        }
    }

    // Rule 1: entity assertions in assistant response
    static ASSERTION_RE: OnceLock<Regex> = OnceLock::new();
    let assert_re = ASSERTION_RE.get_or_init(|| {
        Regex::new(
            r"(?i)\b(\w[\w\s]{2,30})\s+(?:is|was|are|were|means|refers to|stands for)\s+([^.!?\n]{10,100})"
        ).unwrap()
    });
    for cap in assert_re.captures_iter(assistant_response).take(3) {
        let subject = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        let predicate = cap.get(2).map(|m| m.as_str().trim()).unwrap_or("");
        if subject.split_whitespace().count() <= 5 && predicate.len() > 8 {
            facts.push(ExtractedFact {
                text: format!("{subject} is {predicate}"),
                confidence: 0.65,
            });
        }
        if facts.len() >= 5 {
            break;
        }
    }

    // Rule 4: agent completion statements
    static COMPLETION_RE: OnceLock<Regex> = OnceLock::new();
    let completion_re = COMPLETION_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:i(?:'ve| have) (?:created|saved|set|written|updated|configured|added|built|deployed)|completed:|done:|result:|output:)\s*([^.!?\n]{15,120})"
        ).unwrap()
    });
    for cap in completion_re.captures_iter(assistant_response).take(2) {
        let action = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if action.len() > 12 {
            facts.push(ExtractedFact {
                text: format!("Agent action: {action}"),
                confidence: 0.72,
            });
        }
        if facts.len() >= 6 {
            break;
        }
    }

    // Rule 3: tool result summaries
    static TOOL_RESULT_RE: OnceLock<Regex> = OnceLock::new();
    let tool_re = TOOL_RESULT_RE.get_or_init(|| {
        Regex::new(
            r"(?i)(?:found that|the result is|according to|retrieved:|search results show|the output is)\s*([^.!?\n]{15,120})"
        ).unwrap()
    });
    for cap in tool_re.captures_iter(assistant_response).take(2) {
        let finding = cap.get(1).map(|m| m.as_str().trim()).unwrap_or("");
        if finding.len() > 12 {
            facts.push(ExtractedFact {
                text: format!("Finding: {finding}"),
                confidence: 0.60,
            });
        }
        if facts.len() >= 6 {
            break;
        }
    }

    facts.truncate(6);
    facts
}

/// Extract a procedural pattern if the tool sequence for this turn
/// matches a known high-value workflow.
///
/// Returns Some(pattern) if the sequence is worth storing as a
/// Procedural node, None otherwise.
///
/// Current patterns detected:
/// - Research workflow: web_search + (summarize|memory_write) → "web_research"
/// - Code workflow: code_exec + file_write → "code_and_save"
/// - Delegation workflow: agent_send|agent_spawn (any) → "agent_delegation"
/// - Data pipeline: file_read + code_exec + file_write → "data_pipeline"
pub fn extract_pattern(tool_sequence: &[String]) -> Option<ExtractedPattern> {
    if tool_sequence.is_empty() {
        return None;
    }

    let tools: Vec<&str> = tool_sequence.iter().map(|s| s.as_str()).collect();

    // Delegation workflow
    if tools.iter().any(|t| {
        matches!(
            *t,
            "agent_send" | "agent_spawn" | "a2a_send" | "agent_delegate"
        )
    }) {
        return Some(ExtractedPattern {
            name: "agent_delegation".to_string(),
            tool_sequence: tool_sequence.to_vec(),
            confidence: 0.90,
        });
    }

    // Data pipeline
    if tools.contains(&"file_read")
        && tools.contains(&"code_exec")
        && tools.contains(&"file_write")
    {
        return Some(ExtractedPattern {
            name: "data_pipeline".to_string(),
            tool_sequence: tool_sequence.to_vec(),
            confidence: 0.85,
        });
    }

    // Research workflow
    if tools.contains(&"web_search") || tools.contains(&"web_fetch") {
        return Some(ExtractedPattern {
            name: "web_research".to_string(),
            tool_sequence: tool_sequence.to_vec(),
            confidence: 0.75,
        });
    }

    // Code workflow
    if tools.contains(&"code_exec") && tools.contains(&"file_write") {
        return Some(ExtractedPattern {
            name: "code_and_save".to_string(),
            tool_sequence: tool_sequence.to_vec(),
            confidence: 0.80,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_facts_user_disclosure() {
        let facts = extract_facts("I work at Acme Corp on weekends", "OK.");
        assert!(facts.iter().any(|f| f.text.contains("Acme")));
    }

    #[test]
    fn extract_pattern_web_research() {
        let p = extract_pattern(&["web_search".to_string(), "file_read".to_string()]);
        assert_eq!(p.as_ref().map(|x| x.name.as_str()), Some("web_research"));
    }

    #[test]
    fn extract_pattern_delegation_prefers_over_research() {
        let p = extract_pattern(&["web_search".to_string(), "agent_send".to_string()]);
        assert_eq!(p.as_ref().map(|x| x.name.as_str()), Some("agent_delegation"));
    }
}
