//! Internal nested [`ainl_runtime::AinlRuntime::run_turn`] depth and [`AinlRuntimeError::DelegationDepthExceeded`].

use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AinlRuntime, AinlRuntimeError, RuntimeConfig, TurnInput, TurnStatus,
};
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("delegation_depth.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn rt_with_episode(agent_id: &str, max_depth: u32) -> (tempfile::TempDir, AinlRuntime) {
    let (dir, store) = open_store();
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = agent_id.into();
    store.write_node(&ep).unwrap();
    let cfg = RuntimeConfig {
        agent_id: agent_id.to_string(),
        max_delegation_depth: max_depth,
        extraction_interval: 0,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    let rt = AinlRuntime::new(cfg, store);
    assert!(rt.load_artifact().unwrap().validation.is_valid);
    (dir, rt)
}

#[test]
fn test_depth_exceeded_returns_hard_error() {
    let (_d, mut rt) = rt_with_episode("depth-hard", 3);
    rt.test_set_delegation_depth(3);
    let err = rt
        .run_turn(TurnInput::default())
        .expect_err("expected DelegationDepthExceeded");
    assert_eq!(
        err,
        AinlRuntimeError::DelegationDepthExceeded { depth: 3, max: 3 }
    );
}

#[test]
fn test_depth_within_limit_succeeds() {
    let (_d, mut rt) = rt_with_episode("depth-ok", 8);
    rt.test_set_delegation_depth(7);
    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .expect("turn within depth ceiling");
    assert!(out.is_complete());
    assert_eq!(out.turn_status(), TurnStatus::Ok);
    assert_eq!(rt.test_delegation_depth(), 7);
}

#[test]
fn test_depth_resets_after_turn_completes() {
    let (_d, mut rt) = rt_with_episode("depth-reset-ok", 8);
    assert_eq!(rt.test_delegation_depth(), 0);
    rt.run_turn(TurnInput {
        user_message: "a".into(),
        tools_invoked: vec![],
        ..Default::default()
    })
    .unwrap();
    assert_eq!(rt.test_delegation_depth(), 0);
}

#[test]
fn test_depth_resets_after_turn_errors() {
    let (dir, store) = open_store();
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = "depth-reset-err".into();
    store.write_node(&ep).unwrap();
    let cfg = RuntimeConfig {
        agent_id: String::new(),
        enable_graph_memory: true,
        max_delegation_depth: 8,
        extraction_interval: 0,
        max_steps: 50,
    };
    let mut rt = AinlRuntime::new(cfg, store);
    assert_eq!(rt.test_delegation_depth(), 0);
    let err = rt
        .run_turn(TurnInput::default())
        .expect_err("empty agent_id");
    assert!(
        matches!(err, AinlRuntimeError::Message(_)),
        "unexpected err: {err}"
    );
    assert_eq!(rt.test_delegation_depth(), 0);
    drop(dir);
}

#[test]
fn test_caller_supplied_depth_zero_still_enforced() {
    let (_d, mut rt) = rt_with_episode("depth-caller-zero", 1);
    rt.test_set_delegation_depth(1);
    let err = rt
        .run_turn(TurnInput {
            depth: 0,
            user_message: "x".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .expect_err("TurnInput.depth must not bypass internal depth");
    assert_eq!(
        err,
        AinlRuntimeError::DelegationDepthExceeded { depth: 1, max: 1 }
    );
}
