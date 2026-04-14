//! `TurnWarning.phase` maps to per-phase graph extraction failures.

use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use ainl_runtime::{AinlRuntime, RuntimeConfig, TurnInput, TurnOutcome, TurnPhase, TurnStatus};
use serde_json::json;
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("turn_phase_granularity.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn rt_cfg(agent_id: &str) -> RuntimeConfig {
    RuntimeConfig {
        agent_id: agent_id.to_string(),
        extraction_interval: 1,
        max_steps: 100,
        ..Default::default()
    }
}

#[test]
fn test_pattern_persistence_failure_maps_to_correct_turn_phase() {
    let (_d, store) = open_store();
    let ag = "granularity-pattern";
    let mut ep = AinlMemoryNode::new_episode(
        Uuid::new_v4(),
        3_000_000_020,
        vec!["noop".into()],
        None,
        None,
    );
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_extractor_mut().test_inject_pattern_error = Some("injected pattern persistence".into());

    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(
        warnings.iter().any(|w| {
            w.phase == TurnPhase::PatternPersistence && w.error.contains("injected pattern")
        }),
        "{warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.phase == TurnPhase::ExtractionPass),
        "{warnings:?}"
    );
}

#[test]
fn test_persona_evolution_failure_maps_to_correct_turn_phase() {
    let (_d, store) = open_store();
    let ag = "granularity-persona";
    let mut ep = AinlMemoryNode::new_episode(
        Uuid::new_v4(),
        3_000_000_021,
        vec!["shell_exec".into()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_extractor_mut().test_inject_persona_error = Some("injected persona evolution".into());

    let out = rt
        .run_turn(TurnInput {
            user_message: "run tools".into(),
            tools_invoked: vec!["shell_exec".into()],
            ..Default::default()
        })
        .unwrap();

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(
        warnings.iter().any(|w| {
            w.phase == TurnPhase::PersonaEvolution && w.error.contains("injected persona")
        }),
        "{warnings:?}"
    );
    assert!(
        !warnings
            .iter()
            .any(|w| w.phase == TurnPhase::PatternPersistence),
        "{warnings:?}"
    );
}

#[test]
fn test_extract_and_persona_both_fail_two_warnings_in_outcome() {
    let (_d, store) = open_store();
    let ag = "granularity-dual";
    let mut ep = AinlMemoryNode::new_episode(
        Uuid::new_v4(),
        3_000_000_022,
        vec!["web_search".into()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_extractor_mut().test_inject_extract_error = Some("injected extract".into());
    rt.test_extractor_mut().test_inject_persona_error = Some("injected persona".into());

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec!["web_search".into()],
            ..Default::default()
        })
        .unwrap();

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(
        warnings
            .iter()
            .any(|w| w.phase == TurnPhase::ExtractionPass),
        "{warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.phase == TurnPhase::PersonaEvolution),
        "{warnings:?}"
    );
    assert!(warnings.len() >= 2, "{warnings:?}");
}

#[test]
fn test_clean_extraction_produces_complete_outcome_not_partial() {
    let (_d, store) = open_store();
    let ag = "granularity-clean";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_023, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();

    assert!(
        matches!(out, TurnOutcome::Complete(_)),
        "expected Complete, got {out:?}"
    );
    let r = out.result();
    assert_eq!(r.status, TurnStatus::Ok);
    assert!(r.extraction_report.is_some());
    assert!(
        r.extraction_report
            .as_ref()
            .is_some_and(|rep| !rep.has_errors()),
        "{:?}",
        r.extraction_report
    );
}
