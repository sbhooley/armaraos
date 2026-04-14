//! `TurnOutcome` / non-fatal write semantics for [`ainl_runtime::AinlRuntime::run_turn`].

use std::borrow::Cow;
use std::collections::HashMap;

use ainl_memory::{
    AgentGraphSnapshot, AinlMemoryNode, AinlNodeType, GraphStore, SnapshotEdge, SqliteGraphStore,
    SNAPSHOT_SCHEMA_VERSION,
};
use ainl_runtime::{
    AinlRuntime, AinlRuntimeError, RuntimeConfig, TurnInput, TurnOutcome, TurnPhase, TurnStatus,
};
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("turn_outcome.db");
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
fn test_fitness_writeback_failure_produces_partial_success() {
    let (_d, store) = open_store();
    let ag = "fitness-fail";
    let mut proc = AinlMemoryNode::new_pattern("p1".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L_fit_fail".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["k".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_set_force_fitness_write_failure(true);
    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(
        warnings
            .iter()
            .any(|w| w.phase == TurnPhase::FitnessWriteBack),
        "{warnings:?}"
    );
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "L_fit_fail")
        .expect("patch row");
    assert!(!r.dispatched);
}

#[test]
fn test_extraction_pass_failure_produces_partial_success() {
    let (_d, store) = open_store();
    let ag = "extract-fail";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_set_force_extraction_failure(true);

    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec!["noop".into()],
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
    assert!(out.result().extraction_report.is_none());
}

#[test]
fn test_both_failures_accumulate_warnings() {
    let (_d, store) = open_store();
    let ag = "both-fail";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_001, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut proc = AinlMemoryNode::new_pattern("p2".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L_both".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["k".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.test_set_force_fitness_write_failure(true);
    rt.test_set_force_extraction_failure(true);
    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec!["noop".into()],
            frame,
            ..Default::default()
        })
        .unwrap();

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(warnings.iter().any(|w| w.phase == TurnPhase::FitnessWriteBack));
    assert!(warnings.iter().any(|w| w.phase == TurnPhase::ExtractionPass));
    assert!(warnings.len() >= 2, "{warnings:?}");
}

#[test]
fn test_hard_store_open_failure_still_returns_err() {
    let (_d, mut store) = open_store();
    let ag = "dang-agent";
    let mut a = AinlMemoryNode::new_episode(Uuid::new_v4(), 2_300_000_000, vec![], None, None);
    a.agent_id = ag.to_string();
    let aid = a.id;
    let ghost = Uuid::new_v4();
    let snap = AgentGraphSnapshot {
        agent_id: ag.to_string(),
        exported_at: chrono::Utc::now(),
        schema_version: Cow::Borrowed(SNAPSHOT_SCHEMA_VERSION),
        nodes: vec![a],
        edges: vec![SnapshotEdge {
            source_id: aid,
            target_id: ghost,
            edge_type: "broken".to_string(),
            weight: 1.0,
            metadata: None,
        }],
    };
    store.import_graph(&snap, true).expect("import dangling");

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    let err = rt
        .run_turn(TurnInput {
            user_message: "x".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap_err();
    let msg = match err {
        AinlRuntimeError::Message(m) => m,
        other => panic!("expected Message error, got {other:?}"),
    };
    assert!(
        msg.contains("graph validation failed before turn"),
        "unexpected: {msg}"
    );
}

#[test]
fn test_complete_turn_returns_complete_variant() {
    let (_d, store) = open_store();
    let ag = "complete-agent";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_002, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    let out = rt
        .run_turn(TurnInput {
            user_message: "ok".into(),
            tools_invoked: vec!["noop".into()],
            ..Default::default()
        })
        .unwrap();

    assert!(out.is_complete());
    assert_eq!(out.turn_status(), TurnStatus::Ok);
    assert!(out.warnings().is_empty());
    assert!(out.result().extraction_report.is_some());
}
