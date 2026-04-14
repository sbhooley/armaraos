//! Tokio `run_turn_async` integration tests (requires `--features async` on this crate).
//! Graph memory uses `Arc<std::sync::Mutex<_>>` so these tests can construct the runtime on the
//! multi-threaded Tokio pool without async-mutex `blocking_lock` issues; see crate `README.md`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AinlRuntime, AinlRuntimeError, PatchDispatchContext, RuntimeConfig, TurnHooksAsync, TurnInput,
    TurnOutcome, TurnPhase, TurnStatus,
};
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("async_rt.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn rt_cfg(agent_id: &str) -> RuntimeConfig {
    RuntimeConfig {
        agent_id: agent_id.to_string(),
        extraction_interval: 0,
        max_steps: 50,
        ..Default::default()
    }
}

#[tokio::test]
async fn test_run_turn_async_basic_completes() {
    let (_d, store) = open_store();
    let ag = "async-basic";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    let out = rt
        .run_turn_async(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec!["noop".into()],
            ..Default::default()
        })
        .await
        .expect("turn");
    assert!(out.is_complete());
    assert_eq!(out.turn_status(), TurnStatus::Ok);
    assert_ne!(out.result().episode_id, Uuid::nil());
}

#[tokio::test]
async fn test_run_turn_async_partial_success_on_write_failure() {
    let (_d, store) = open_store();
    let ag = "async-fit-fail";
    let mut proc = AinlMemoryNode::new_pattern("p1".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L_async_fail".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["k".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(
        RuntimeConfig {
            agent_id: ag.into(),
            extraction_interval: 0,
            max_steps: 100,
            ..Default::default()
        },
        store,
    );
    rt.test_set_force_fitness_write_failure(true);
    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));

    let out = rt
        .run_turn_async(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .await
        .expect("turn");

    let TurnOutcome::PartialSuccess { warnings, .. } = &out else {
        panic!("expected PartialSuccess, got {out:?}");
    };
    assert!(
        warnings
            .iter()
            .any(|w| w.phase == TurnPhase::FitnessWriteBack),
        "{warnings:?}"
    );
}

struct OrderHooks {
    log: Arc<Mutex<Vec<String>>>,
}

#[async_trait::async_trait]
impl TurnHooksAsync for OrderHooks {
    async fn on_turn_start(&self, _input: &TurnInput) {
        self.log.lock().unwrap().push("start".into());
    }

    async fn on_patch_dispatched(
        &self,
        _ctx: &PatchDispatchContext<'_>,
    ) -> Result<serde_json::Value, String> {
        self.log.lock().unwrap().push("patch".into());
        Ok(serde_json::Value::Null)
    }

    async fn on_turn_complete(&self, _outcome: &TurnOutcome) {
        self.log.lock().unwrap().push("done".into());
    }
}

#[tokio::test]
async fn test_async_hooks_called_in_order() {
    let (_d, store) = open_store();
    let ag = "async-hooks";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut proc = AinlMemoryNode::new_pattern("ph".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L_hook".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["k".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let log = Arc::new(Mutex::new(Vec::new()));
    let hooks = Arc::new(OrderHooks {
        log: Arc::clone(&log),
    });

    let mut rt = AinlRuntime::new(rt_cfg(ag), store).with_hooks_async(hooks);
    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));
    rt.run_turn_async(TurnInput {
        user_message: "x".into(),
        tools_invoked: vec![],
        frame,
        ..Default::default()
    })
    .await
    .unwrap();

    let v = log.lock().unwrap().clone();
    assert_eq!(v, vec!["start", "patch", "done"]);
}

#[tokio::test]
async fn test_delegation_depth_enforced_in_async_path() {
    let (_d, store) = open_store();
    let ag = "async-depth";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let mut rt = AinlRuntime::new(
        RuntimeConfig {
            agent_id: ag.into(),
            max_delegation_depth: 3,
            extraction_interval: 0,
            max_steps: 50,
            ..Default::default()
        },
        store,
    );
    rt.test_set_delegation_depth(3);
    let err = rt
        .run_turn_async(TurnInput::default())
        .await
        .expect_err("depth");
    assert_eq!(
        err,
        AinlRuntimeError::DelegationDepthExceeded { depth: 3, max: 3 }
    );
}

#[tokio::test]
async fn test_session_state_restored_on_async() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("async_session.db");
    let _ = std::fs::remove_file(&db);
    let ag = "async-session";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
        ep.agent_id = ag.into();
        store.write_node(&ep).unwrap();
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 0,
        max_steps: 50,
        ..Default::default()
    };
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        let mut rt = AinlRuntime::new(cfg.clone(), store);
        for i in 0..3 {
            rt.run_turn_async(TurnInput {
                user_message: format!("m{i}"),
                tools_invoked: vec![],
                ..Default::default()
            })
            .await
            .unwrap();
        }
        assert_eq!(rt.test_turn_count(), 3);
    }
    let store_b = SqliteGraphStore::open(&db).unwrap();
    let rt_b = AinlRuntime::new(cfg, store_b);
    assert_eq!(rt_b.test_turn_count(), 3);
}
