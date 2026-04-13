//! Integration tests for `AinlRuntime` turn orchestration (patch dispatch, semantic relevance, persona cache, emit).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AinlRuntime, PatchSkipReason, RuntimeConfig, TurnHooks, TurnInput, TurnOutcome,
};
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("engine.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn default_rt_cfg(agent_id: &str) -> RuntimeConfig {
    RuntimeConfig {
        agent_id: agent_id.to_string(),
        extraction_interval: 0,
        max_steps: 100,
        ..Default::default()
    }
}

#[derive(Clone)]
struct EmitRecorder {
    emits: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
}

impl Default for EmitRecorder {
    fn default() -> Self {
        Self {
            emits: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl TurnHooks for EmitRecorder {
    fn on_emit(&self, target: &str, payload: &serde_json::Value) {
        self.emits
            .lock()
            .unwrap()
            .push((target.to_string(), payload.clone()));
    }
}

#[test]
fn test_patch_dispatch_satisfied_reads() {
    let (_d, store) = open_store();
    let ag = "patch-agent";
    let mut proc = AinlMemoryNode::new_pattern("p1".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L_patch".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["topic".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let mut frame = HashMap::new();
    frame.insert("topic".into(), serde_json::json!("rust"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(out.outcome, TurnOutcome::Success));
    assert_eq!(out.patch_dispatch_results.len(), 1);
    let r = &out.patch_dispatch_results[0];
    assert!(r.dispatched);
    assert!(r.fitness_after > r.fitness_before);
    assert_eq!(r.label, "L_patch");
}

#[test]
fn test_patch_dispatch_missing_read() {
    let (_d, store) = open_store();
    let ag = "patch-agent-2";
    let mut proc = AinlMemoryNode::new_pattern("p2".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L2".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["topic".into()];
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    let r = &out.patch_dispatch_results[0];
    assert!(!r.dispatched);
    assert_eq!(
        r.skip_reason,
        Some(PatchSkipReason::MissingDeclaredRead("topic".into()))
    );
}

#[test]
fn test_patch_dispatch_updates_fitness_in_store() {
    let (_d, store) = open_store();
    let ag = "patch-agent-3";
    let mut proc = AinlMemoryNode::new_pattern("p3".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "L3".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["x".into()];
        procedural.fitness = Some(0.5);
    }
    let pid = proc.id;
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let mut frame = HashMap::new();
    frame.insert("x".into(), serde_json::json!(1));
    rt.run_turn(TurnInput {
        user_message: "m".into(),
        tools_invoked: vec![],
        frame,
        ..Default::default()
    })
    .unwrap();

    let store = rt.sqlite_store();
    let n = store.read_node(pid).unwrap().expect("row");
    let fit = n.procedural().unwrap().fitness.expect("fitness");
    assert!((fit - 0.6).abs() < 0.001, "expected EMA ~0.6, got {fit}");
}

#[test]
fn test_patch_dispatch_skips_zero_version() {
    let (_d, store) = open_store();
    let ag = "patch-agent-4";
    let mut proc = AinlMemoryNode::new_pattern("p0".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.patch_version = 0;
        procedural.label = "zero".into();
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let out = rt
        .run_turn(TurnInput {
            user_message: "m".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    let r = &out.patch_dispatch_results[0];
    assert!(!r.dispatched);
    assert_eq!(r.skip_reason, Some(PatchSkipReason::ZeroVersion));
}

#[test]
fn test_semantic_relevance_filters_by_topic() {
    let (_d, store) = open_store();
    let ag = "sem-agent";
    let tid = Uuid::new_v4();

    let mut rust_n = AinlMemoryNode::new_fact("rust-fact".into(), 0.8, tid);
    rust_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = rust_n.node_type {
        semantic.topic_cluster = Some("rust,cargo".into());
        semantic.recurrence_count = 1;
    }
    store.write_node(&rust_n).unwrap();

    let mut trade_n = AinlMemoryNode::new_fact("trade-fact".into(), 0.8, tid);
    trade_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = trade_n.node_type {
        semantic.topic_cluster = Some("trading".into());
        semantic.recurrence_count = 5;
    }
    store.write_node(&trade_n).unwrap();

    let mut game_n = AinlMemoryNode::new_fact("game-fact".into(), 0.8, tid);
    game_n.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = game_n.node_type {
        semantic.topic_cluster = Some("gaming".into());
        semantic.recurrence_count = 5;
    }
    store.write_node(&game_n).unwrap();

    let rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let ctx = rt
        .compile_memory_context_for(Some("help me with my rust crate"))
        .unwrap();
    assert!(!ctx.relevant_semantic.is_empty());
    let first = ctx.relevant_semantic[0].semantic().unwrap();
    assert_eq!(first.fact, "rust-fact");
}

#[test]
fn test_semantic_relevance_fallback_no_tags() {
    let (_d, store) = open_store();
    let ag = "sem-fallback";
    let tid = Uuid::new_v4();

    let mut low = AinlMemoryNode::new_fact("low".into(), 0.8, tid);
    low.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = low.node_type {
        semantic.recurrence_count = 1;
    }
    store.write_node(&low).unwrap();

    let mut mid = AinlMemoryNode::new_fact("mid".into(), 0.8, tid);
    mid.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = mid.node_type {
        semantic.recurrence_count = 2;
    }
    store.write_node(&mid).unwrap();

    let mut high = AinlMemoryNode::new_fact("high".into(), 0.8, tid);
    high.agent_id = ag.into();
    if let AinlNodeType::Semantic { ref mut semantic } = high.node_type {
        semantic.recurrence_count = 4;
    }
    store.write_node(&high).unwrap();

    let rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let ctx = rt.compile_memory_context_for(Some("xyzzy")).unwrap();
    let facts: Vec<_> = ctx
        .relevant_semantic
        .iter()
        .filter_map(|n| n.semantic().map(|s| s.fact.as_str()))
        .collect();
    assert!(facts.contains(&"high"));
    assert!(facts.contains(&"mid"));
    assert!(!facts.contains(&"low"));
}

#[test]
fn test_persona_cache_invalidated_after_extraction() {
    let (_d, store) = open_store();
    let ag = "persona-cache-agent";
    let mut persona = AinlMemoryNode::new_persona("alpha".into(), 0.5, vec![]);
    persona.agent_id = ag.into();
    let persona_id = persona.id;
    store.write_node(&persona).unwrap();

    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 1,
        max_steps: 100,
        ..Default::default()
    };
    let mut rt = AinlRuntime::new(cfg, store);
    let out1 = rt
        .run_turn(TurnInput {
            user_message: "t1".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    assert!(out1.extraction_report.is_some());
    let c1 = out1.persona_prompt_contribution.clone().expect("p1");

    let store = rt.sqlite_store();
    let mut pn = store.read_node(persona_id).unwrap().expect("persona row");
    if let AinlNodeType::Persona { ref mut persona } = pn.node_type {
        persona.strength = 0.99;
    }
    store.write_node(&pn).unwrap();

    let out2 = rt
        .run_turn(TurnInput {
            user_message: "t2".into(),
            tools_invoked: vec![],
            ..Default::default()
        })
        .unwrap();
    let c2 = out2.persona_prompt_contribution.clone().expect("p2");
    assert_ne!(c1, c2);
    assert!(c2.contains("0.99"));
}

#[test]
fn test_emit_hook_fires_for_emit_to_edges() {
    let (_d, store) = open_store();
    let ag = "emit-agent";
    let bridge = AinlMemoryNode::new_persona("bridge_a".into(), 0.5, vec![]);
    let bridge_id = bridge.id;
    let mut bridge = bridge;
    bridge.agent_id = ag.into();
    store.write_node(&bridge).unwrap();

    let rec = EmitRecorder::default();
    let hooks = rec.clone();
    let cfg = default_rt_cfg(ag);
    let mut rt = AinlRuntime::new(cfg, store).with_hooks(hooks);

    rt.run_turn(TurnInput {
        user_message: "u".into(),
        tools_invoked: vec!["noop".into()],
        emit_targets: vec![bridge_id],
        ..Default::default()
    })
    .unwrap();

    let emits = rec.emits.lock().unwrap();
    assert_eq!(emits.len(), 1);
    assert_eq!(emits[0].0, "bridge_a");
    assert!(emits[0].1.get("user_message").is_some());
}

#[test]
fn test_emit_noop_when_no_edges() {
    let (_d, store) = open_store();
    let ag = "emit-noop";
    let rec = EmitRecorder::default();
    let hooks = rec.clone();
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store).with_hooks(hooks);
    rt.run_turn(TurnInput {
        user_message: "u".into(),
        tools_invoked: vec![],
        ..Default::default()
    })
    .unwrap();
    assert!(rec.emits.lock().unwrap().is_empty());
}
