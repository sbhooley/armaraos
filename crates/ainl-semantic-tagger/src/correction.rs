//! Correction / behavior phrase detection (migrated from `ainl-graph-extractor`).

use crate::tag::{SemanticTag, TagNamespace};
use regex_lite::Regex;
use std::sync::OnceLock;

/// Compiled regex bundle for correction extraction.
pub struct CorrectionRegexes {
    pub stop_ing: Regex,
    pub dont_verb: Regex,
    pub you_keep_ing: Regex,
    pub told_not_to: Regex,
    pub asked_not_to: Regex,
    pub dont_want: Regex,
}

/// Lazily compiled correction regexes (identical patterns to legacy `persona_signals`).
pub fn correction_regexes() -> &'static CorrectionRegexes {
    static RES: OnceLock<CorrectionRegexes> = OnceLock::new();
    RES.get_or_init(|| CorrectionRegexes {
        stop_ing: Regex::new(r"(?i)\bstop\s+([a-z][a-z]+ing)\b").expect("regex"),
        dont_verb: Regex::new(r"(?i)\bdon['’]?t\s+([a-z][a-z0-9]*(?:\s+[a-z][a-z0-9]*){0,8})\b")
            .expect("regex"),
        you_keep_ing: Regex::new(r"(?i)\byou\s+keep\s+([a-z][a-z]+ing(?:\s+[a-z][a-z]+){0,4})\b")
            .expect("regex"),
        told_not_to: Regex::new(r"(?i)\bi\s+told\s+you\s+not\s+to\s+([a-z][^\n.!?]{1,48})")
            .expect("regex"),
        asked_not_to: Regex::new(r"(?i)\bi\s+asked\s+you\s+not\s+to\s+([a-z][^\n.!?]{1,48})")
            .expect("regex"),
        dont_want: Regex::new(r"(?i)\bi\s+don['’]?t\s+want\s+you\s+to\s+([a-z][^\n.!?]{1,48})")
            .expect("regex"),
    })
}

const CORRECTION_TRIGGERS: &[&str] = &[
    "don't do that",
    "don't use",
    "stop doing",
    "you keep",
    "i told you",
    "i said",
    "please stop",
    "i asked you not to",
    "why do you keep",
    "stop saying",
    "quit doing",
    "i don't want you to",
];

/// Exact trimmed lowercase rejections (noise / non-behavioral triggers).
const PHRASE_REJECTIONS: &[&str] = &[
    "stop",
    "don't",
    "i said so",
    "don't do that",
    "don't do that.",
];

fn normalize_behavior_phrase(s: &str) -> Option<String> {
    let t = s
        .trim()
        .trim_matches(|c: char| c == '.' || c == '!' || c == '?');
    if t.len() < 4 {
        return None;
    }
    let tl = t.to_lowercase();
    if tl == "do that" || tl == "that" || tl == "it" || tl == "so" {
        return None;
    }
    if !t.chars().any(|c| c.is_alphabetic()) {
        return None;
    }
    Some(t.to_string())
}

fn map_normalized_phrase(phrase: &str) -> SemanticTag {
    let pl = phrase.to_lowercase();
    if pl.contains("bullet") {
        return SemanticTag {
            namespace: TagNamespace::Correction,
            value: "avoid_bullets".to_string(),
            confidence: 0.85,
        };
    }
    if pl.contains("caveat") {
        return SemanticTag {
            namespace: TagNamespace::Behavior,
            value: "adding_caveats".to_string(),
            confidence: 0.85,
        };
    }
    if pl.contains("over-explain")
        || pl.contains("overexplain")
        || (pl.contains("over") && pl.contains("explain"))
    {
        return SemanticTag {
            namespace: TagNamespace::Behavior,
            value: "overexplaining".to_string(),
            confidence: 0.85,
        };
    }
    if pl.contains("emoji") {
        return SemanticTag {
            namespace: TagNamespace::Correction,
            value: "avoid_emojis".to_string(),
            confidence: 0.85,
        };
    }
    let slug = phrase
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    SemanticTag {
        namespace: TagNamespace::Correction,
        value: if slug.is_empty() {
            "unspecified".to_string()
        } else {
            slug
        },
        confidence: 0.85,
    }
}

fn correction_behavior_phrase(user: &str) -> Option<String> {
    let lower = user.to_lowercase();
    let triggered = CORRECTION_TRIGGERS.iter().any(|t| lower.contains(*t));
    if !triggered {
        return None;
    }
    let trimmed = lower.trim();
    if PHRASE_REJECTIONS.contains(&trimmed) {
        return None;
    }

    let re = correction_regexes();
    if let Some(c) = re.stop_ing.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                return Some(b);
            }
        }
    }
    if let Some(c) = re.you_keep_ing.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                return Some(b);
            }
        }
    }
    if let Some(c) = re.told_not_to.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                return Some(b);
            }
        }
    }
    if let Some(c) = re.asked_not_to.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                return Some(b);
            }
        }
    }
    if let Some(c) = re.dont_want.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                return Some(b);
            }
        }
    }
    if let Some(c) = re.dont_verb.captures(user) {
        if let Some(m) = c.get(1) {
            if let Some(b) = normalize_behavior_phrase(m.as_str()) {
                let bl = b.to_lowercase();
                if bl == "do that" || bl == "that" {
                    return None;
                }
                return Some(b);
            }
        }
    }
    None
}

/// Stage-1 triggers + stage-2 regex normalization, mapped to canonical [`SemanticTag`] values.
pub fn extract_correction_behavior(text: &str) -> Option<SemanticTag> {
    let phrase = correction_behavior_phrase(text)?;
    Some(map_normalized_phrase(&phrase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correction_dont_use_bullets() {
        let t = extract_correction_behavior("don't use bullet points").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Correction);
        assert_eq!(t.value, "avoid_bullets");
    }

    #[test]
    fn correction_you_keep_caveats() {
        let t = extract_correction_behavior("you keep adding caveats").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Behavior);
        assert_eq!(t.value, "adding_caveats");
    }

    #[test]
    fn correction_told_emojis() {
        let t = extract_correction_behavior("I told you not to use emojis").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Correction);
        assert_eq!(t.value, "avoid_emojis");
    }

    #[test]
    fn correction_stop_alone() {
        assert!(extract_correction_behavior("stop").is_none());
    }

    #[test]
    fn correction_i_said_so() {
        assert!(extract_correction_behavior("I said so").is_none());
    }

    #[test]
    fn correction_dont_do_that_no_behavior() {
        assert!(extract_correction_behavior("don't do that").is_none());
    }
}
