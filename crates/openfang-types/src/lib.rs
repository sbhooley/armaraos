//! Core types and traits for the OpenFang Agent Operating System.
//!
//! This crate defines all shared data structures used across the OpenFang kernel,
//! runtime, memory substrate, and wire protocol. It contains no business logic.

pub mod adaptive_eco;
pub mod agent;
pub mod approval;
pub mod capability;
pub mod comms;
pub mod config;
pub mod error;
pub mod event;
pub mod learning_frame;
pub mod manifest_signing;
pub mod media;
pub mod memory;
pub mod message;
pub mod model_catalog;
pub mod orchestration;
pub mod orchestration_trace;
pub mod runtime_limits;
pub mod scheduler;
pub mod serde_compat;
pub mod taint;
pub mod task_queue;
pub mod tool;
pub mod tool_compat;
pub mod turn_constraints;
pub mod vitals;
pub mod webhook;

/// Safely truncate a string to at most `max_bytes`, never splitting a UTF-8 char.
pub fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_str_ascii() {
        assert_eq!(truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn truncate_str_chinese() {
        // Each Chinese character is 3 bytes
        let s = "\u{4F60}\u{597D}\u{4E16}\u{754C}"; // 你好世界
        assert_eq!(truncate_str(s, 6), "\u{4F60}\u{597D}"); // 你好
        assert_eq!(truncate_str(s, 7), "\u{4F60}\u{597D}"); // still 你好 (7 is mid-char)
        assert_eq!(truncate_str(s, 9), "\u{4F60}\u{597D}\u{4E16}"); // 你好世
    }

    #[test]
    fn truncate_str_emoji() {
        let s = "hi\u{1F600}there"; // hi😀there — emoji is 4 bytes
        assert_eq!(truncate_str(s, 3), "hi"); // 3 is mid-emoji
        assert_eq!(truncate_str(s, 6), "hi\u{1F600}"); // after emoji
    }

    #[test]
    fn truncate_str_em_dash() {
        // Em dash (—) is 3 bytes (0xE2 0x80 0x94) — the exact char that caused
        // production panics in kernel.rs and session.rs (issue #104)
        let s = "Here is a summary — with details";
        assert_eq!(truncate_str(s, 19), "Here is a summary ");
        assert_eq!(truncate_str(s, 20), "Here is a summary ");
        assert_eq!(truncate_str(s, 21), "Here is a summary \u{2014}");
    }

    #[test]
    fn truncate_str_no_truncation() {
        assert_eq!(truncate_str("short", 100), "short");
    }

    #[test]
    fn truncate_str_empty() {
        assert_eq!(truncate_str("", 10), "");
    }

    /// Phase 0 gate: `openfang_types::vitals` must remain the same types as `ainl_contracts::vitals`.
    #[test]
    fn vitals_types_are_canonical_ainl_contracts_reexports() {
        use std::any::TypeId;

        use crate::vitals::{CognitivePhase, CognitiveVitals, VitalsGate};
        use ainl_contracts::vitals as ac;

        assert_eq!(
            TypeId::of::<CognitiveVitals>(),
            TypeId::of::<ac::CognitiveVitals>()
        );
        assert_eq!(TypeId::of::<VitalsGate>(), TypeId::of::<ac::VitalsGate>());
        assert_eq!(
            TypeId::of::<CognitivePhase>(),
            TypeId::of::<ac::CognitivePhase>()
        );
    }

    #[test]
    fn vitals_json_roundtrip_matches_ainl_contracts_wire_format() {
        use crate::vitals::{CognitiveVitals, VitalsGate};
        use ainl_contracts::vitals::CognitiveVitals as AcVitals;

        let v = CognitiveVitals {
            gate: VitalsGate::Pass,
            phase: "reasoning:0.71".into(),
            trust: 0.9,
            mean_logprob: -0.1,
            entropy: 0.2,
            sample_tokens: 16,
        };
        let json_of = serde_json::to_string(&v).unwrap();
        let v2: AcVitals = serde_json::from_str(&json_of).unwrap();
        let json_ac = serde_json::to_string(&v2).unwrap();
        assert_eq!(json_of, json_ac);
        assert_eq!(v.gate, v2.gate);
        assert_eq!(v.trust, v2.trust);
    }
}
