/// Integration tests for the Rust-side persona evolution pass (Gap J).
///
/// These tests verify:
/// 1. `persona_turn_evolution_env_enabled()` is on by default (no env var needed)
/// 2. Opt-out via `AINL_PERSONA_EVOLUTION=0` disables evolution
/// 3. `PersonaEvolutionHook::evolve_from_turn` with a mismatch agent_id returns Err (non-fatal)
/// 4. Empty `TurnOutcome` (no tools, no delegation) produces Ok without panic
///
/// Note: tests that require direct access to the SQLite graph store (writing persona nodes,
/// reading `evolution_cycle`) use the in-module `#[cfg(all(test, ...))]` tests inside
/// `persona_evolution.rs` which have `pub(crate)` access. Integration tests here cover the
/// public API surface only.
use openfang_runtime::persona_evolution::{
    persona_turn_evolution_env_enabled, PersonaEvolutionHook, TurnOutcome,
};

/// Mutex protecting env-var mutations for serial access within this test binary.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_env<T>(key: &str, val: Option<&str>, f: impl FnOnce() -> T) -> T {
    let original = std::env::var(key).ok();
    match val {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    let result = f();
    match &original {
        Some(v) => std::env::set_var(key, v),
        None => std::env::remove_var(key),
    }
    result
}

/// Evolution must be enabled by default (env var absent) when the feature is compiled in.
#[test]
fn test_evolution_enabled_by_default() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let result = with_env("AINL_PERSONA_EVOLUTION", None, persona_turn_evolution_env_enabled);
    assert!(result, "persona evolution must be on by default when env var is absent");
}

/// Opt-out via AINL_PERSONA_EVOLUTION=0 must disable evolution at runtime.
#[test]
fn test_env_opt_out_zero_disables_evolution() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let result = with_env("AINL_PERSONA_EVOLUTION", Some("0"), persona_turn_evolution_env_enabled);
    assert!(!result, "AINL_PERSONA_EVOLUTION=0 should disable evolution");
}

/// Opt-out via AINL_PERSONA_EVOLUTION=false must also disable.
#[test]
fn test_env_opt_out_false_string_disables_evolution() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let result = with_env("AINL_PERSONA_EVOLUTION", Some("false"), persona_turn_evolution_env_enabled);
    assert!(!result, "AINL_PERSONA_EVOLUTION=false should disable evolution");
}

/// An arbitrary non-falsy value (e.g. "1") must keep evolution enabled.
#[test]
fn test_env_truthy_value_keeps_evolution_enabled() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let result = with_env("AINL_PERSONA_EVOLUTION", Some("1"), persona_turn_evolution_env_enabled);
    assert!(result, "AINL_PERSONA_EVOLUTION=1 should keep evolution enabled");
}

/// evolve_from_turn with a mismatched agent_id must return Err and not panic.
/// This simulates the non-fatal failure path (the daemon continues after a warning).
#[cfg(feature = "ainl-persona-evolution")]
#[tokio::test]
async fn test_persona_evolution_nonfatal_on_agent_id_mismatch() {
    // Set env before acquiring async resources, then release lock before awaiting.
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("AINL_PERSONA_EVOLUTION");
    } // lock released here — safe to await below

    let agent_id = format!("nonfatal-agent-{}", std::process::id());
    let writer = openfang_runtime::graph_memory_writer::GraphMemoryWriter::open(&format!(
        "{}-test",
        agent_id
    ))
    .unwrap_or_else(|_| {
        panic!("GraphMemoryWriter::open failed — ensure writable home directory");
    });

    let turn = TurnOutcome {
        tool_calls: vec!["shell_exec".into()],
        delegation_to: None,
    };

    let result = PersonaEvolutionHook::evolve_from_turn(&writer, "WRONG-agent-mismatch", &turn).await;
    assert!(result.is_err(), "agent_id mismatch should return Err; got Ok");
}

/// TurnOutcome with no tool calls and no delegation must return Ok without panic.
/// This is the "noop when no signals" path.
#[cfg(feature = "ainl-persona-evolution")]
#[tokio::test]
async fn test_persona_evolution_noop_no_signals_no_panic() {
    {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        std::env::remove_var("AINL_PERSONA_EVOLUTION");
    } // lock released before await

    let agent_id = format!("noop-no-signals-{}", std::process::id());
    let writer =
        openfang_runtime::graph_memory_writer::GraphMemoryWriter::open(&agent_id).unwrap_or_else(
            |_| panic!("GraphMemoryWriter::open failed — ensure writable home directory"),
        );

    let turn = TurnOutcome {
        tool_calls: vec![],
        delegation_to: None,
    };

    let result = PersonaEvolutionHook::evolve_from_turn(&writer, &agent_id, &turn).await;
    assert!(result.is_ok(), "empty turn should produce Ok, not {result:?}");
}
