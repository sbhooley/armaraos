//! Session persistence: `turn_count`, extraction cadence, and persona cache survive a simulated restart
//! (new [`AinlRuntime`] on the same SQLite file).

use ainl_memory::{AinlMemoryNode, GraphStore, SqliteGraphStore};
use ainl_runtime::{AinlRuntime, RuntimeConfig, TurnInput, TurnPhase};
use uuid::Uuid;

fn seed_agent_graph(store: &SqliteGraphStore, ag: &str) {
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();
    let mut p = AinlMemoryNode::new_persona("tone".into(), 0.7, vec![]);
    p.agent_id = ag.into();
    store.write_node(&p).unwrap();
}

#[test]
fn test_turn_count_restored_after_simulated_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session_turn_count.db");
    let _ = std::fs::remove_file(&db);
    let ag = "turn-restore";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        seed_agent_graph(&store, ag);
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 0,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        let mut rt = AinlRuntime::new(cfg.clone(), store);
        assert!(rt.load_artifact().unwrap().validation.is_valid);
        for i in 0..3 {
            rt.run_turn(TurnInput {
                user_message: format!("m{i}"),
                tools_invoked: vec![],
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(rt.test_turn_count(), 3);
    }
    let store_b = SqliteGraphStore::open(&db).unwrap();
    let rt_b = AinlRuntime::new(cfg, store_b);
    assert_eq!(rt_b.test_turn_count(), 3);
}

#[test]
fn test_extraction_cadence_restored_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session_extract_cadence.db");
    let _ = std::fs::remove_file(&db);
    let ag = "extract-restore";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        seed_agent_graph(&store, ag);
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 10,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        let mut rt = AinlRuntime::new(cfg.clone(), store);
        assert!(rt.load_artifact().unwrap().validation.is_valid);
        for i in 0..5 {
            rt.run_turn(TurnInput {
                user_message: format!("a{i}"),
                tools_invoked: vec![],
                ..Default::default()
            })
            .unwrap();
        }
        assert_eq!(rt.test_turn_count(), 5);
    }

    let store_b = SqliteGraphStore::open(&db).unwrap();
    let mut rt_b = AinlRuntime::new(cfg, store_b);
    assert_eq!(rt_b.test_turn_count(), 5);

    for i in 0..4 {
        let out = rt_b
            .run_turn(TurnInput {
                user_message: format!("b{i}"),
                tools_invoked: vec![],
                ..Default::default()
            })
            .unwrap();
        assert!(
            out.result().extraction_report.is_none(),
            "extraction should not run before combined turn 10"
        );
    }

    let out = rt_b
        .run_turn(TurnInput {
            user_message: "b4".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    assert!(
        out.result().extraction_report.is_some(),
        "extraction should run at combined turn 10"
    );
}

#[test]
fn test_persona_cache_warm_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session_persona_cache.db");
    let _ = std::fs::remove_file(&db);
    let ag = "persona-restart";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        seed_agent_graph(&store, ag);
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 0,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        let mut rt = AinlRuntime::new(cfg.clone(), store);
        assert!(rt.load_artifact().unwrap().validation.is_valid);
        rt.run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
        assert!(
            rt.test_persona_cache().is_some(),
            "first turn should compile persona"
        );
    }
    let store2 = SqliteGraphStore::open(&db).unwrap();
    let rt2 = AinlRuntime::new(cfg, store2);
    let cached = rt2.test_persona_cache();
    assert!(cached.is_some());
    assert!(cached.unwrap().contains("tone"));
}

#[test]
fn test_cold_start_with_no_persisted_state() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session_cold.db");
    let _ = std::fs::remove_file(&db);
    let ag = "cold-agent";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        seed_agent_graph(&store, ag);
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        ..RuntimeConfig::default()
    };
    let store = SqliteGraphStore::open(&db).unwrap();
    let rt = AinlRuntime::new(cfg, store);
    assert_eq!(rt.test_turn_count(), 0);
    assert!(rt.test_persona_cache().is_none());
}

#[test]
fn test_state_write_failure_is_non_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("session_persist_fail.db");
    let _ = std::fs::remove_file(&db);
    let ag = "persist-fail";
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        seed_agent_graph(&store, ag);
    }
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        ..RuntimeConfig::default()
    };
    let store = SqliteGraphStore::open(&db).unwrap();
    let mut rt = AinlRuntime::new(cfg, store);
    rt.test_set_force_runtime_state_write_failure(true);
    let out = rt
        .run_turn(TurnInput {
            user_message: "x".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    assert!(out.is_partial_success());
    let warns = out.warnings();
    assert!(warns
        .iter()
        .any(|w| w.phase == TurnPhase::RuntimeStatePersist));
}
