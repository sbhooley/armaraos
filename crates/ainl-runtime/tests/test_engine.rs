//! Integration tests for `AinlRuntime` turn orchestration (patch dispatch, semantic relevance, persona cache, emit).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AinlRuntime, MemoryNodeType, PatchAdapter, PatchSkipReason, PersonaAxis, RawSignal, RuntimeConfig,
    TurnHooks, TurnInput, TurnOutcome, EVOLUTION_TRAIT_NAME,
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

fn evolution_axis_score(
    store: &SqliteGraphStore,
    agent_id: &str,
    axis: PersonaAxis,
) -> Option<f32> {
    let key = axis.name();
    let mut best: Option<(u32, f32)> = None;
    let nodes = store.find_by_type("persona").ok()?;
    for n in nodes {
        if n.agent_id != agent_id {
            continue;
        }
        if let AinlNodeType::Persona { persona } = &n.node_type {
            if persona.trait_name != EVOLUTION_TRAIT_NAME {
                continue;
            }
            let sc = *persona.axis_scores.get(key)?;
            let cyc = persona.evolution_cycle;
            best = match best {
                None => Some((cyc, sc)),
                Some((c0, _)) if cyc >= c0 => Some((cyc, sc)),
                Some(b) => Some(b),
            };
        }
    }
    best.map(|(_, s)| s)
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

struct EchoAdapter;

impl PatchAdapter for EchoAdapter {
    fn name(&self) -> &str {
        "echo"
    }

    fn execute(
        &self,
        label: &str,
        frame: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        Ok(serde_json::json!({
            "echoed_label": label,
            "frame_keys": frame.keys().cloned().collect::<Vec<String>>(),
        }))
    }
}

struct FailAdapter;

impl PatchAdapter for FailAdapter {
    fn name(&self) -> &str {
        "fail"
    }

    fn execute(
        &self,
        _: &str,
        _: &HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        Err("intentional failure".to_string())
    }
}

#[test]
fn test_adapter_registry_registers_and_executes() {
    let (_d, store) = open_store();
    let ag = "adapter-echo-agent";
    let mut proc = AinlMemoryNode::new_pattern("echo-pat".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "echo".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["msg".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.register_adapter(EchoAdapter);
    let names = rt.registered_adapters();
    assert_eq!(names.len(), 1);
    assert!(names.contains(&"echo"));

    let mut frame = HashMap::new();
    frame.insert("msg".into(), serde_json::json!("hello"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(out.outcome, TurnOutcome::Success));
    let r = out
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "echo")
        .expect("echo patch");
    assert!(r.dispatched);
    assert_eq!(r.adapter_name.as_deref(), Some("echo"));
    let outv = r.adapter_output.as_ref().expect("adapter output");
    assert_eq!(outv.get("echoed_label").and_then(|v| v.as_str()), Some("echo"));
    let keys = outv
        .get("frame_keys")
        .and_then(|v| v.as_array())
        .expect("frame_keys");
    assert!(keys.iter().any(|k| k.as_str() == Some("msg")));
}

#[test]
fn test_no_adapter_metadata_only() {
    let (_d, store) = open_store();
    let ag = "adapter-none-agent";
    let mut proc = AinlMemoryNode::new_pattern("echo-pat2".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "echo".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["msg".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let mut frame = HashMap::new();
    frame.insert("msg".into(), serde_json::json!("hello"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = out
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "echo")
        .expect("echo patch");
    assert!(r.dispatched);
    assert!(r.adapter_output.is_none());
    assert!(r.adapter_name.is_none());
}

#[test]
fn test_adapter_failure_graceful() {
    let (_d, store) = open_store();
    let ag = "adapter-fail-agent";
    let mut proc = AinlMemoryNode::new_pattern("fail-pat".into(), vec![]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = "fail".into();
        procedural.patch_version = 1;
        procedural.declared_reads = vec!["msg".into()];
        procedural.fitness = Some(0.5);
    }
    store.write_node(&proc).unwrap();

    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.register_adapter(FailAdapter);

    let mut frame = HashMap::new();
    frame.insert("msg".into(), serde_json::json!("hello"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    assert!(
        matches!(out.outcome, TurnOutcome::Success)
            || matches!(out.outcome, TurnOutcome::PartialSuccess { .. })
    );
    let r = out
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "fail")
        .expect("fail patch");
    assert!(r.dispatched);
    assert!(r.adapter_output.is_none());
    assert_eq!(r.adapter_name.as_deref(), Some("fail"));
    assert!(r.fitness_after > r.fitness_before);
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

#[test]
fn test_episode_tools_synonyms_normalize_to_single_bash() {
    let (_d, store) = open_store();
    let ag = "tool-norm-1";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.run_turn(TurnInput {
        user_message: "a".into(),
        tools_invoked: vec!["bash".into(), "shell".into()],
        ..Default::default()
    })
    .unwrap();
    rt.run_turn(TurnInput {
        user_message: "b".into(),
        tools_invoked: vec!["Bash".into(), "sh".into()],
        ..Default::default()
    })
    .unwrap();

    let eps = rt
        .sqlite_store()
        .query(ag)
        .episodes_with_tool("bash", 10)
        .unwrap();
    assert_eq!(eps.len(), 2);
    for n in &eps {
        let ep = match &n.node_type {
            AinlNodeType::Episode { episodic } => episodic,
            _ => panic!("expected episode"),
        };
        assert_eq!(ep.tools_invoked, vec!["bash".to_string()]);
    }
    assert!(rt
        .sqlite_store()
        .query(ag)
        .episodes_with_tool("shell", 10)
        .unwrap()
        .is_empty());
}

#[test]
fn test_episode_tools_dedup_many_variants_one_bash() {
    let (_d, store) = open_store();
    let ag = "tool-norm-2";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.run_turn(TurnInput {
        user_message: "c".into(),
        tools_invoked: vec![
            "Bash".into(),
            "bash".into(),
            "sh".into(),
            "shell".into(),
        ],
        ..Default::default()
    })
    .unwrap();

    let ep = rt
        .sqlite_store()
        .query(ag)
        .episodes_with_tool("bash", 5)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let tools = match &ep.node_type {
        AinlNodeType::Episode { episodic } => &episodic.tools_invoked,
        _ => panic!("expected episode"),
    };
    assert_eq!(tools, &vec!["bash".to_string()]);
}

#[test]
fn test_episode_tools_unrelated_tools_distinct() {
    let (_d, store) = open_store();
    let ag = "tool-norm-3";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.run_turn(TurnInput {
        user_message: "d".into(),
        tools_invoked: vec!["bash".into(), "search_web".into()],
        ..Default::default()
    })
    .unwrap();

    let ep = rt
        .sqlite_store()
        .query(ag)
        .episodes_with_tool("bash", 5)
        .unwrap()
        .pop()
        .unwrap();
    let tools = match &ep.node_type {
        AinlNodeType::Episode { episodic } => episodic.tools_invoked.clone(),
        _ => panic!("expected episode"),
    };
    assert_eq!(tools, vec!["bash".to_string(), "search_web".to_string()]);

    assert_eq!(
        rt.sqlite_store()
            .query(ag)
            .episodes_with_tool("search_web", 5)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn test_episode_tools_empty_uses_turn_sentinel() {
    let (_d, store) = open_store();
    let ag = "tool-norm-4";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    rt.run_turn(TurnInput {
        user_message: "e".into(),
        tools_invoked: vec![],
        ..Default::default()
    })
    .unwrap();

    let ep = rt
        .sqlite_store()
        .query(ag)
        .episodes_with_tool("turn", 5)
        .unwrap()
        .pop()
        .unwrap();
    let tools = match &ep.node_type {
        AinlNodeType::Episode { episodic } => &episodic.tools_invoked,
        _ => panic!("expected episode"),
    };
    assert_eq!(tools, &vec!["turn".to_string()]);
}

#[test]
fn test_manual_evolution_signals_without_extractor_pass() {
    let (_d, store) = open_store();
    let ag = "evo-manual";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let nid = Uuid::new_v4();
    let applied = rt.apply_evolution_signals(vec![RawSignal {
        axis: PersonaAxis::Verbosity,
        reward: 0.95,
        weight: 1.0,
        source_node_id: nid,
        source_node_type: MemoryNodeType::Episodic,
    }]);
    assert!(applied >= 1);
    let snap = rt.persist_evolution_snapshot().unwrap();
    assert!(snap.score(PersonaAxis::Verbosity) > 0.5);
    let stored = evolution_axis_score(rt.sqlite_store(), ag, PersonaAxis::Verbosity).unwrap();
    assert!((stored - snap.score(PersonaAxis::Verbosity)).abs() < 0.001);
}

#[test]
fn test_evolution_correction_tick_persisted() {
    let (_d, store) = open_store();
    let ag = "evo-corr";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let before = rt.evolution_engine().snapshot().score(PersonaAxis::Curiosity);
    rt.evolution_correction_tick(PersonaAxis::Curiosity, 0.15);
    let mid = rt.evolution_engine().snapshot().score(PersonaAxis::Curiosity);
    assert!((mid - before).abs() > 0.0001);
    let snap = rt.persist_evolution_snapshot().unwrap();
    let stored = evolution_axis_score(rt.sqlite_store(), ag, PersonaAxis::Curiosity).unwrap();
    assert!((stored - snap.score(PersonaAxis::Curiosity)).abs() < 0.001);
}

#[test]
fn test_evolve_persona_from_graph_signals_without_scheduled_extractor() {
    let (_d, store) = open_store();
    let ag = "evo-graph";
    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(
        tid,
        chrono::Utc::now().timestamp(),
        vec![],
        None,
        None,
    );
    ep.agent_id = ag.into();
    if let AinlNodeType::Episode { ref mut episodic } = ep.node_type {
        episodic.persona_signals_emitted = vec!["Instrumentality:0.9".to_string()];
    }
    store.write_node(&ep).unwrap();
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store);
    let snap = rt.evolve_persona_from_graph_signals().unwrap();
    assert!(snap.score(PersonaAxis::Instrumentality) > 0.5);
    let stored = evolution_axis_score(rt.sqlite_store(), ag, PersonaAxis::Instrumentality).unwrap();
    assert!((stored - snap.score(PersonaAxis::Instrumentality)).abs() < 0.001);
}

#[test]
fn test_evolution_writes_disabled_errors_on_persist_and_evolve() {
    let (_d, store) = open_store();
    let ag = "evo-guard";
    let mut rt = AinlRuntime::new(default_rt_cfg(ag), store).with_evolution_writes_enabled(false);
    let msg_persist = rt.persist_evolution_snapshot().unwrap_err();
    assert!(
        msg_persist.contains("evolution_writes_enabled is false"),
        "unexpected persist err: {msg_persist}"
    );
    let msg_evolve = rt.evolve_persona_from_graph_signals().unwrap_err();
    assert!(
        msg_evolve.contains("evolution_writes_enabled is false"),
        "unexpected evolve err: {msg_evolve}"
    );
}

#[test]
fn test_internal_depth_enforced() {
    let (_d, store) = open_store();
    let ag = "depth-agent";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        max_delegation_depth: 1,
        extraction_interval: 0,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    let mut rt = AinlRuntime::new(cfg, store);
    assert!(rt.load_artifact().unwrap().validation.is_valid);

    let out1 = rt
        .run_turn(TurnInput {
            user_message: "first".into(),
            tools_invoked: vec![],
            depth: 99,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(out1.outcome, TurnOutcome::Success));
    assert_eq!(rt.test_delegation_depth(), 0);

    // Prime internal nesting to the configured ceiling before re-entry: next `run_turn` increments
    // to 2, which exceeds `max_delegation_depth` of 1.
    rt.test_set_delegation_depth(1);
    let out2 = rt
        .run_turn(TurnInput {
            user_message: "second".into(),
            tools_invoked: vec![],
            depth: 0,
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(out2.outcome, TurnOutcome::DepthLimitExceeded));
    assert_eq!(rt.test_delegation_depth(), 1);
}

#[test]
fn test_partial_success_extraction_failure() {
    let (_d, store) = open_store();
    let ag = "partial-extract-agent";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();

    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 1,
        max_steps: 50,
        ..RuntimeConfig::default()
    };
    let mut rt = AinlRuntime::new(cfg, store);
    assert!(rt.load_artifact().unwrap().validation.is_valid);
    rt.test_set_force_extraction_failure(true);

    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec!["noop".into()],
            ..Default::default()
        })
        .unwrap();

    assert!(matches!(
        out.outcome,
        TurnOutcome::PartialSuccess {
            extraction_failed: true,
            ..
        }
    ));
    assert_ne!(out.episode_id, Uuid::nil());
    assert!(out.extraction_report.is_none());
}

#[test]
fn test_runtime_state_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("runtime_restart.db");
    let _ = std::fs::remove_file(&db);
    let ag = "restart-agent";
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 3_000_000_000, vec![], None, None);
    ep.agent_id = ag.into();
    {
        let store = SqliteGraphStore::open(&db).unwrap();
        store.write_node(&ep).unwrap();
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
            out.extraction_report.is_none(),
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
        out.extraction_report.is_some(),
        "extraction should run at combined turn 10"
    );
}

#[test]
fn test_scheduled_extractor_pass_still_runs_after_turn() {
    let (_d, store) = open_store();
    let ag = "evo-sched";
    let mut ep = AinlMemoryNode::new_episode(
        Uuid::new_v4(),
        3_000_000_000,
        vec![],
        None,
        None,
    );
    ep.agent_id = ag.into();
    store.write_node(&ep).unwrap();
    let cfg = RuntimeConfig {
        agent_id: ag.into(),
        extraction_interval: 1,
        max_steps: 50,
        ..Default::default()
    };
    let mut rt = AinlRuntime::new(cfg, store);
    assert!(rt.load_artifact().unwrap().validation.is_valid);
    let out = rt
        .run_turn(TurnInput {
            user_message: "hello".into(),
            tools_invoked: vec!["noop".into()],
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(out.outcome, TurnOutcome::Success));
    assert!(out.extraction_report.is_some());
}
