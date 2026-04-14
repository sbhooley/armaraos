//! Integration tests for graph extraction + persona evolution.

use ainl_graph_extractor::{
    extract_pass, run_extraction_pass, update_semantic_recurrence, GraphExtractorTask,
    PersonaSignalExtractorState, EVOLUTION_TRAIT_NAME,
};
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_persona::{
    EvolutionEngine, GraphExtractor, MemoryNodeType, PersonaAxis, RawSignal, INGEST_SCORE_EPSILON,
};

fn ema_weighted_step(score: f32, reward: f32, weight: f32) -> f32 {
    const ALPHA: f32 = 0.2;
    let target = (reward * weight).clamp(0.0, 1.0);
    (ALPHA * target + (1.0 - ALPHA) * score).clamp(0.0, 1.0)
}
use serde_json::json;
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("ainl_graph_extractor_test.db");
    let store = SqliteGraphStore::open(&path).expect("open store");
    (dir, store)
}

fn load_semantic(store: &SqliteGraphStore, id: Uuid) -> ainl_memory::SemanticNode {
    let n = store.read_node(id).expect("read").expect("exists");
    match n.node_type {
        AinlNodeType::Semantic { semantic } => semantic,
        _ => panic!("not semantic"),
    }
}

#[test]
fn test_recurrence_increments_on_delta() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = "a1".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 3;
        semantic.last_ref_snapshot = 0;
        semantic.recurrence_count = 0;
    }
    store.write_node(&sem).expect("write");
    let n = update_semantic_recurrence(&store, "a1").expect("upd");
    assert_eq!(n, 1);
    let s = load_semantic(&store, sem.id);
    assert_eq!(s.recurrence_count, 1);
    assert_eq!(s.last_ref_snapshot, 3);
}

#[test]
fn test_recurrence_no_change_when_no_delta() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = "a1".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 3;
        semantic.last_ref_snapshot = 3;
        semantic.recurrence_count = 5;
    }
    store.write_node(&sem).expect("write");
    let n = update_semantic_recurrence(&store, "a1").expect("upd");
    assert_eq!(n, 0);
    let s = load_semantic(&store, sem.id);
    assert_eq!(s.recurrence_count, 5);
    assert_eq!(s.last_ref_snapshot, 3);
}

#[test]
fn test_recurrence_max_one_per_pass() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = "a1".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 100;
        semantic.last_ref_snapshot = 0;
        semantic.recurrence_count = 0;
    }
    store.write_node(&sem).expect("write");
    update_semantic_recurrence(&store, "a1").expect("upd");
    let s = load_semantic(&store, sem.id);
    assert_eq!(s.recurrence_count, 1);
    assert_eq!(s.last_ref_snapshot, 100);
}

#[test]
fn test_high_reference_count_not_gating() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("solo fact".into(), 0.9, tid);
    sem.agent_id = "agent-ref".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 99;
        semantic.last_ref_snapshot = 0;
        semantic.recurrence_count = 0;
    }
    store.write_node(&sem).expect("write");

    let mut task = GraphExtractorTask::new("agent-ref");
    let r = task.run_pass(&store);
    assert!(!r.has_errors(), "{r:?}");

    let s = load_semantic(&store, sem.id);
    assert_eq!(s.recurrence_count, 1);

    let sigs = GraphExtractor::extract(&store, "agent-ref").expect("extract");
    let persistence_from_sem: Vec<&RawSignal> = sigs
        .iter()
        .filter(|s| {
            s.axis == PersonaAxis::Persistence
                && s.source_node_type == MemoryNodeType::Semantic
                && s.source_node_id == sem.id
        })
        .collect();
    assert!(
        persistence_from_sem.is_empty(),
        "persistence must not fire for recurrence_count < 3"
    );
}

#[test]
fn test_full_pass_writes_persona_node() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();

    let mut sem = AinlMemoryNode::new_fact("tagged".into(), 0.9, tid);
    sem.agent_id = "agent-p".into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.recurrence_count = 2;
        semantic.tags = vec!["research".into()];
    }
    store.write_node(&sem).expect("write sem");

    let mut proc = AinlMemoryNode::new_procedural_tools("p".into(), vec![], 0.9);
    proc.agent_id = "agent-p".into();
    if let AinlNodeType::Procedural { procedural } = &mut proc.node_type {
        procedural.patch_version = 1;
    }
    store.write_node(&proc).expect("write proc");

    let r = run_extraction_pass(&store, "agent-p");
    assert!(!r.has_errors(), "{r:?}");

    let personas: Vec<_> = store
        .find_by_type("persona")
        .expect("find")
        .into_iter()
        .filter(|n| n.agent_id == "agent-p")
        .collect();
    let evo = personas
        .iter()
        .find(|n| match &n.node_type {
            AinlNodeType::Persona { persona } => persona.trait_name == EVOLUTION_TRAIT_NAME,
            _ => false,
        })
        .expect("evolution persona");
    let AinlNodeType::Persona { persona } = &evo.node_type else {
        panic!();
    };
    assert!(
        !persona.axis_scores.is_empty(),
        "axis_scores should be populated"
    );
}

#[test]
fn test_extraction_report_counts() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let agent = "agent-count";

    for (i, (refc, snap)) in [(5u32, 0u32), (2u32, 0u32), (10u32, 10u32)]
        .into_iter()
        .enumerate()
    {
        let mut sem = AinlMemoryNode::new_fact(format!("f{i}"), 0.8, tid);
        sem.agent_id = agent.into();
        if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
            semantic.reference_count = refc;
            semantic.last_ref_snapshot = snap;
        }
        store.write_node(&sem).expect("write");
    }

    let report = run_extraction_pass(&store, agent);
    assert!(!report.has_errors(), "{report:?}");
    assert_eq!(report.semantic_nodes_updated, 2);
}

#[test]
fn test_idempotent_pass() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let agent = "agent-idem";
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = agent.into();
    if let AinlNodeType::Semantic { semantic } = &mut sem.node_type {
        semantic.reference_count = 2;
        semantic.last_ref_snapshot = 0;
    }
    store.write_node(&sem).expect("write");

    let r1 = run_extraction_pass(&store, agent);
    assert!(!r1.has_errors(), "{r1:?}");
    let r2 = run_extraction_pass(&store, agent);
    assert!(!r2.has_errors(), "{r2:?}");
    assert_eq!(r2.semantic_nodes_updated, 0);

    let evo = store
        .find_by_type("persona")
        .expect("find")
        .into_iter()
        .find(|n| {
            n.agent_id == agent
                && matches!(&n.node_type, AinlNodeType::Persona { persona } if persona.trait_name == EVOLUTION_TRAIT_NAME)
        })
        .expect("evo node");
    let AinlNodeType::Persona { persona } = &evo.node_type else {
        panic!();
    };
    assert_eq!(persona.evolution_cycle, 2);
}

#[test]
fn test_no_duplicate_instrumentality_from_tools() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut ep =
        AinlMemoryNode::new_episode(tid, 1, vec![], None, Some(json!({ "outcome": "success" })));
    ep.agent_id = "agent-dup-tools".into();
    if let AinlNodeType::Episode { ref mut episodic } = ep.node_type {
        episodic.tools_invoked = vec!["shell".into(), "mcp".into()];
    }
    store.write_node(&ep).expect("write");

    let from_graph = GraphExtractor::extract(&store, "agent-dup-tools").expect("extract");
    let inst_graph = from_graph
        .iter()
        .filter(|s| s.axis == PersonaAxis::Instrumentality)
        .count();
    assert_eq!(
        inst_graph, 2,
        "metadata extractor should emit one Instrumentality signal per tool"
    );

    let mut st = PersonaSignalExtractorState::default();
    let from_pass = extract_pass(&store, "agent-dup-tools", &mut st).expect("pass signals");
    let inst_pass = from_pass
        .iter()
        .filter(|s| s.axis == PersonaAxis::Instrumentality)
        .count();
    assert_eq!(
        inst_pass, 0,
        "pattern pass must not duplicate graph-backed tool Instrumentality"
    );

    let merged_inst = from_graph
        .iter()
        .chain(from_pass.iter())
        .filter(|s| s.axis == PersonaAxis::Instrumentality)
        .count();
    assert_eq!(merged_inst, 2);

    let report = run_extraction_pass(&store, "agent-dup-tools");
    assert!(!report.has_errors(), "{report:?}");
    let mut expected_inst = 0.5_f32;
    expected_inst = ema_weighted_step(expected_inst, 0.8, 0.6);
    expected_inst = ema_weighted_step(expected_inst, 0.8, 0.6);
    let got = report.persona_snapshot.score(PersonaAxis::Instrumentality);
    assert!(
        (got - expected_inst).abs() < 0.002,
        "instrumentality EMA should match two graph tool signals (~metadata weight 0.6), got {got} expected {expected_inst}"
    );
}

fn mk_implicit_brevity_episode(agent: &str, timestamp: i64, turn_id: Uuid) -> AinlMemoryNode {
    let mut ep = AinlMemoryNode::new_episode(
        turn_id,
        timestamp,
        vec![],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = agent.into();
    if let AinlNodeType::Episode { ref mut episodic } = ep.node_type {
        episodic.user_message_tokens = 8;
        episodic.assistant_response_tokens = 350;
    }
    ep
}

#[test]
fn test_brevity_streak_survives_pass_boundary() {
    let (_d, store) = open_store();
    let agent = "agent-brev-pass-boundary";
    let mut task = GraphExtractorTask::new(agent);

    let tid1 = Uuid::new_v4();
    let ep1 = mk_implicit_brevity_episode(agent, 1, tid1);
    store.write_node(&ep1).expect("write ep1");

    let r1 = task.run_pass(&store);
    assert!(!r1.has_errors(), "{r1:?}");
    let verbosity_pass1: Vec<_> = r1
        .merged_signals
        .iter()
        .filter(|s| s.axis == PersonaAxis::Verbosity)
        .collect();
    assert!(
        verbosity_pass1.is_empty(),
        "implicit brevity needs streak=2; one episode in pass 1 should not emit Verbosity"
    );

    let tid2 = Uuid::new_v4();
    let ep2 = mk_implicit_brevity_episode(agent, 2, tid2);
    store.write_node(&ep2).expect("write ep2");

    let r2 = task.run_pass(&store);
    assert!(!r2.has_errors(), "{r2:?}");
    let implicit_verbosity: Vec<_> = r2
        .merged_signals
        .iter()
        .filter(|s| s.axis == PersonaAxis::Verbosity && s.reward < 0.3)
        .collect();
    assert!(
        !implicit_verbosity.is_empty(),
        "same GraphExtractorTask: streak should survive run_pass boundary and emit implicit brevity"
    );
}

#[test]
fn test_run_pass_signals_extracted_ge_signals_applied() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(
        tid,
        1,
        vec!["file_read".into()],
        None,
        Some(json!({ "outcome": "success" })),
    );
    ep.agent_id = "agent-sigcount".into();
    store.write_node(&ep).expect("write");

    let report = run_extraction_pass(&store, "agent-sigcount");
    assert!(!report.has_errors(), "{report:?}");
    assert!(
        report.signals_extracted >= report.signals_applied,
        "extracted={} applied={}",
        report.signals_extracted,
        report.signals_applied
    );
}

#[test]
fn test_invalid_axis_hint_does_not_apply() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut ep = AinlMemoryNode::new_episode(tid, 1, vec![], None, None);
    ep.agent_id = "agent-badaxis".into();
    if let AinlNodeType::Episode { episodic } = &mut ep.node_type {
        episodic.persona_signals_emitted = vec!["NotAnAxis:0.99".into()];
    }
    store.write_node(&ep).expect("write");

    let mut engine = EvolutionEngine::new("agent-badaxis");
    let before = engine.snapshot().score(PersonaAxis::Instrumentality);
    let sigs = engine.extract_signals(&store).expect("extract");
    assert!(
        !sigs.iter().any(|s| (s.reward - 0.99).abs() < 0.01),
        "invalid axis hint must not become a RawSignal"
    );
    let applied = engine.ingest_signals(sigs);
    assert_eq!(applied, 0);
    let after = engine.snapshot().score(PersonaAxis::Instrumentality);
    assert!(
        (after - before).abs() < INGEST_SCORE_EPSILON,
        "axes should be unchanged when nothing parsed"
    );
}

#[test]
fn test_correction_tick_via_engine() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = "agent-corr".into();
    store.write_node(&sem).expect("write");

    let mut task = GraphExtractorTask::new("agent-corr");
    let r = task.run_pass(&store);
    assert!(!r.has_errors(), "{r:?}");

    let before = task
        .evolution_engine
        .axes
        .get(&PersonaAxis::Curiosity)
        .map(|s| s.score)
        .unwrap_or(0.5);
    task.evolution_engine
        .correction_tick(PersonaAxis::Curiosity, 1.0);
    let after = task
        .evolution_engine
        .axes
        .get(&PersonaAxis::Curiosity)
        .map(|s| s.score)
        .unwrap_or(0.5);
    assert!(after > before, "curiosity {after} should exceed {before}");
}
