//! Integration tests for graph extraction + persona evolution.

use ainl_graph_extractor::{
    run_extraction_pass, update_semantic_recurrence, GraphExtractorTask, EVOLUTION_TRAIT_NAME,
};
use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_persona::{GraphExtractor, MemoryNodeType, PersonaAxis, RawSignal};
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
    task.run_pass(&store).expect("pass");

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

    run_extraction_pass(&store, "agent-p").expect("pass");

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

    let report = run_extraction_pass(&store, agent).expect("pass");
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

    run_extraction_pass(&store, agent).expect("first");
    let r2 = run_extraction_pass(&store, agent).expect("second");
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
fn test_correction_tick_via_engine() {
    let (_d, store) = open_store();
    let tid = Uuid::new_v4();
    let mut sem = AinlMemoryNode::new_fact("f".into(), 0.9, tid);
    sem.agent_id = "agent-corr".into();
    store.write_node(&sem).expect("write");

    let mut task = GraphExtractorTask::new("agent-corr");
    task.run_pass(&store).expect("pass");

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
