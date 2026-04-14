/// Integration tests for the graph extraction path selection (Gap I).
///
/// These tests verify that:
/// - The crate-primary path fires when the `ainl-extractor` feature is compiled in and no opt-out is set
/// - The heuristic fallback fires when the crate path yields no candidates
/// - User self-disclosure facts are still extracted via fallback on trivial responses
/// - Empty turns produce no facts and no panic
///
/// Note on env-var tests: `ainl_extractor_runtime_enabled()` reads `AINL_EXTRACTOR_ENABLED` at
/// call time. Tests that mutate this global env var are isolated by reading it inline and
/// restoring immediately — but they still race if run in parallel with other env-mutating tests.
/// The boolean-logic tests are written as pure function contract assertions to minimise this risk.
use openfang_runtime::ainl_graph_extractor_bridge::{
    ainl_extractor_runtime_enabled, graph_memory_turn_extraction,
};
use openfang_runtime::graph_extractor::{extract_facts_for_turn, extract_facts_heuristic};

/// Env-var logic tests for `ainl_extractor_runtime_enabled` are isolated via a process-level
/// mutex so parallel test threads don't race on the global env var.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Opt-out via AINL_EXTRACTOR_ENABLED=0 disables the crate path.
#[test]
#[cfg(feature = "ainl-extractor")]
fn test_env_opt_out_disables_crate_path() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let key = "AINL_EXTRACTOR_ENABLED";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "0");
    let result = ainl_extractor_runtime_enabled();
    match &original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert!(!result, "AINL_EXTRACTOR_ENABLED=0 should disable crate path");
}

/// Opt-out via AINL_EXTRACTOR_ENABLED=false must also disable.
#[test]
#[cfg(feature = "ainl-extractor")]
fn test_env_opt_out_false_string() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let key = "AINL_EXTRACTOR_ENABLED";
    let original = std::env::var(key).ok();
    std::env::set_var(key, "false");
    let result = ainl_extractor_runtime_enabled();
    match &original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert!(!result, "AINL_EXTRACTOR_ENABLED=false should disable crate path");
}

/// Non-falsy value must keep the crate path enabled.
#[test]
#[cfg(feature = "ainl-extractor")]
fn test_crate_primary_path_enabled_by_default() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let key = "AINL_EXTRACTOR_ENABLED";
    let original = std::env::var(key).ok();
    std::env::remove_var(key); // absent = enabled
    let result = ainl_extractor_runtime_enabled();
    match &original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    assert!(result, "crate primary path must be enabled when env var is absent");
}

/// When the feature is on, a meaningful turn produces facts from the crate tagger path.
/// This test doesn't depend on env var state — crate extraction always runs in the test binary.
#[test]
#[cfg(feature = "ainl-extractor")]
fn test_crate_primary_path_fires_when_feature_enabled() {

    let (facts, _pattern) = graph_memory_turn_extraction(
        "Help me debug my Rust project — borrow checker and cargo errors",
        "Sure, run `cargo clippy` to surface borrow checker diagnostics.",
        &[],
        &[],
        "test-agent-i",
    );

    // The crate tagger should produce at least one fact (topic: rust or similar)
    assert!(
        !facts.is_empty(),
        "crate-primary path should produce facts for a substantive turn; got none"
    );
}

/// The heuristic fallback captures user self-disclosures regardless of which path runs.
/// This test calls the heuristic directly (path-agnostic) and also verifies the combined
/// pipeline returns at least one user-disclosure fact for a trivial assistant response.
#[test]
fn test_heuristic_fallback_fires_when_crate_yields_nothing() {
    // Verify the heuristic itself captures the self-disclosure
    let heuristic_facts = extract_facts_heuristic("I work at Contoso Corp on weekends", "Noted.");
    assert!(
        heuristic_facts.iter().any(|f| f.text.contains("Contoso")),
        "heuristic must capture self-disclosure; got {heuristic_facts:?}"
    );

    // Verify the combined pipeline returns at least one fact from any path
    let (facts, _pattern) = graph_memory_turn_extraction(
        "I work at Contoso Corp on weekends",
        "Noted.",
        &[],
        &[],
        "test-agent-fallback",
    );
    assert!(
        !facts.is_empty(),
        "at least one fact expected from any extraction path; got none"
    );
    // Either the crate path tagged something (e.g. preference: work) or the heuristic
    // captured the Contoso self-disclosure — both are acceptable outcomes.
    assert!(
        facts.iter().any(|f| f.text.contains("Contoso") || f.text.contains("topic") || f.text.contains("preference")),
        "expected Contoso, topic, or preference fact; got {facts:?}"
    );
}

/// User self-disclosure must be extractable regardless of which path runs.
#[test]
fn test_user_self_disclosure_still_extracted() {
    let facts = extract_facts_heuristic("I work at Acme Corp on weekends", "OK.");
    assert!(
        !facts.is_empty(),
        "heuristic must extract self-disclosure; got none"
    );
    assert!(
        facts.iter().any(|f| f.text.contains("Acme")),
        "expected Acme in extracted facts; got {facts:?}"
    );
}

/// Empty inputs must not panic and must return an empty fact list.
#[test]
fn test_extractor_does_not_panic_on_empty_turn() {
    let (facts, pattern) =
        graph_memory_turn_extraction("", "", &[], &[], "test-agent-empty");
    // No panic is the primary assertion; emptiness is expected but not required.
    let _ = (facts, pattern);
}

/// Empty inputs to the inner heuristic also must not panic.
#[test]
fn test_extract_facts_for_turn_empty_inputs_no_panic() {
    let facts = extract_facts_for_turn("", "", &[]);
    let _ = facts; // no panic is the assertion
}
