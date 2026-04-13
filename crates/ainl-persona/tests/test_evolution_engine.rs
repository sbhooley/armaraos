//! Integration tests for persona evolution over graph memory.

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_persona::{persona_node, EvolutionEngine, GraphExtractor, MemoryNodeType, PersonaAxis};
use serde_json::json;
use std::collections::HashMap;
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ainl_persona_test.db");
    let store = SqliteGraphStore::open(&path).expect("open store");
    (dir, store)
}

fn approx_eq(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.025
}

fn ema_step(score: f32, reward: f32, weight: f32) -> f32 {
    const ALPHA: f32 = 0.2;
    let target = (reward * weight).clamp(0.0, 1.0);
    (ALPHA * target + (1.0 - ALPHA) * score).clamp(0.0, 1.0)
}

#[test]
fn test_episodic_tool_signal() {
    let (_d, store) = open_store();
    let turn_id = Uuid::new_v4();
    let ts = chrono::Utc::now().timestamp();
    let mut ep = AinlMemoryNode::new_episode(
        turn_id,
        ts,
        vec!["shell_exec".to_string()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = "agent-tool".into();
    store.write_node(&ep).expect("write");

    let mut engine = EvolutionEngine::new("agent-tool");
    let snap = engine.evolve(&store).expect("evolve");
    let mut inst = 0.5_f32;
    inst = ema_step(inst, 0.8, 0.6);
    assert!(
        approx_eq(snap.score(PersonaAxis::Instrumentality), inst),
        "instrumentality={}",
        snap.score(PersonaAxis::Instrumentality)
    );
}

#[test]
fn test_episodic_persona_signals_emitted() {
    let (_d, store) = open_store();
    let turn_id = Uuid::new_v4();
    let ts = chrono::Utc::now().timestamp();
    let mut ep = AinlMemoryNode::new_episode(turn_id, ts, vec![], None, None);
    ep.agent_id = "agent-hint".into();
    if let AinlNodeType::Episode { episodic } = &mut ep.node_type {
        episodic.persona_signals_emitted = vec!["Instrumentality:0.9".to_string()];
    }
    store.write_node(&ep).expect("write");

    let mut engine = EvolutionEngine::new("agent-hint");
    let snap = engine.evolve(&store).expect("evolve");
    let expected = ema_step(0.5, 0.9, 0.8);
    assert!(
        approx_eq(snap.score(PersonaAxis::Instrumentality), expected),
        "instrumentality={}",
        snap.score(PersonaAxis::Instrumentality)
    );
}

#[test]
fn test_semantic_recurrence_signal() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("hold".into(), 0.8, tid);
    sem.agent_id = "agent-sem".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.recurrence_count = 3;
    }
    store.write_node(&sem).expect("write");

    let mut engine = EvolutionEngine::new("agent-sem");
    let snap = engine.evolve(&store).expect("evolve");
    let expected = ema_step(0.5, 0.7, 0.6);
    assert!(
        approx_eq(snap.score(PersonaAxis::Persistence), expected),
        "persistence={}",
        snap.score(PersonaAxis::Persistence)
    );
}

#[test]
fn test_procedural_fitness_signal() {
    let (_d, store) = open_store();
    let mut proc = AinlMemoryNode::new_procedural_tools("p1".into(), vec![], 0.9);
    proc.agent_id = "agent-proc".into();
    if let AinlNodeType::Procedural { procedural } = &mut proc.node_type {
        procedural.patch_version = 2;
        procedural.fitness = Some(0.8);
        procedural.declared_reads = vec!["ctx://session".into()];
    }
    store.write_node(&proc).expect("write");

    let mut engine = EvolutionEngine::new("agent-proc");
    let snap = engine.evolve(&store).expect("evolve");
    let mut pers = 0.5_f32;
    pers = ema_step(pers, 0.7, 0.55);
    let mut sys = 0.5_f32;
    sys = ema_step(sys, 0.8, 0.55);
    let mut inst = 0.5_f32;
    inst = ema_step(inst, 0.6, 0.5);
    assert!(approx_eq(snap.score(PersonaAxis::Persistence), pers));
    assert!(
        approx_eq(snap.score(PersonaAxis::Systematicity), sys),
        "systematicity={}",
        snap.score(PersonaAxis::Systematicity)
    );
    assert!(
        approx_eq(snap.score(PersonaAxis::Instrumentality), inst),
        "instrumentality={}",
        snap.score(PersonaAxis::Instrumentality)
    );
}

#[test]
fn test_prior_dampening() {
    let (_d, store) = open_store();
    let mut prior = AinlMemoryNode::new_persona("warm_prior".into(), 0.5, vec![]);
    prior.agent_id = "agent-prior".into();
    if let AinlNodeType::Persona { persona } = &mut prior.node_type {
        let mut m = HashMap::new();
        m.insert("Curiosity".to_string(), 0.95);
        persona.axis_scores = m;
    }
    store.write_node(&prior).expect("write prior");

    let mut engine = EvolutionEngine::new("agent-prior");
    let snap = engine.evolve(&store).expect("evolve");
    let c = snap.score(PersonaAxis::Curiosity);
    let expected = ema_step(0.5, 0.95, 0.3);
    assert!(
        approx_eq(c, expected),
        "curiosity should reflect dampened prior, got {c}"
    );
    assert!(c < 0.95, "prior must not copy raw 0.95 into axes wholesale");
}

#[test]
fn test_trigger_gating() {
    let (_d, store) = open_store();
    let turn_id = Uuid::new_v4();
    let ts = chrono::Utc::now().timestamp();
    let mut ep = AinlMemoryNode::new_episode(
        turn_id,
        ts,
        vec!["shell_exec".to_string()],
        None,
        Some(json!({ "outcome": "error" })),
    );
    ep.agent_id = "agent-gate".into();
    store.write_node(&ep).expect("write");

    let raw = GraphExtractor::extract(&store, "agent-gate").expect("extract");
    assert!(
        raw.is_empty(),
        "failed episode without persona hints should produce no raw signals"
    );

    let mut sem = AinlMemoryNode::new_fact("x".into(), 0.5, turn_id);
    sem.agent_id = "agent-gate".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 1;
        semantic.recurrence_count = 0;
    }
    store.write_node(&sem).expect("write sem");
    let raw2 = GraphExtractor::extract(&store, "agent-gate").expect("extract2");
    assert!(!raw2
        .iter()
        .any(|s| s.source_node_type == MemoryNodeType::Semantic));
}

#[test]
fn test_evolve_writes_persona_node() {
    let (_d, store) = open_store();
    let turn_id = Uuid::new_v4();
    let ts = chrono::Utc::now().timestamp();
    let mut ep = AinlMemoryNode::new_episode(
        turn_id,
        ts,
        vec!["shell_exec".to_string()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = "agent-rt".into();
    store.write_node(&ep).expect("write");

    let mut engine = EvolutionEngine::new("agent-rt");
    engine.evolve(&store).expect("evolve");

    let personas = store.find_by_type("persona").expect("personas");
    let evo = personas
        .iter()
        .find(|n| {
            n.agent_id == "agent-rt"
                && matches!(
                    &n.node_type,
                    AinlNodeType::Persona { persona }
                        if persona.trait_name == persona_node::EVOLUTION_TRAIT_NAME
                )
        })
        .expect("evolution persona row");
    if let AinlNodeType::Persona { persona } = &evo.node_type {
        assert!(!persona.axis_scores.is_empty());
        assert!(persona.evolution_cycle >= 1);
        assert!(!persona.last_evolved.is_empty());
        assert_eq!(persona.agent_id, "agent-rt");
    } else {
        panic!("expected persona");
    }
}

#[test]
fn test_correction_tick() {
    let mut engine = EvolutionEngine::new("noop");
    let before = engine.axes.get(&PersonaAxis::Curiosity).unwrap().score;
    engine.correction_tick(PersonaAxis::Curiosity, 0.99);
    let after = engine.axes.get(&PersonaAxis::Curiosity).unwrap().score;
    assert!(after > before);
    assert!((after - 0.598).abs() < 0.05, "after={after}");
}
