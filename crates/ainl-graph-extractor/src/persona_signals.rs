//! Heuristic persona-relevant signals for [`ainl_persona::EvolutionEngine`].
//!
//! Runs inside `GraphExtractorTask::run_pass` after graph-backed `EvolutionEngine::extract_signals`,
//! using episode text/tokens, tool lists, and semantic `topic_cluster` + `recurrence_count` only.

use ainl_memory::{
    AinlMemoryNode, AinlNodeType, EpisodicNode, GraphStore, SemanticNode, SqliteGraphStore,
};
use ainl_persona::{signals::episodic_should_process, MemoryNodeType, PersonaAxis, RawSignal};
use ainl_semantic_tagger::{
    extract_correction_behavior, infer_brevity_preference, infer_formality, tag_tool_names,
    SemanticTag, TagNamespace,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Rolling state for debounce / streak detectors (in-memory, per [`GraphExtractorTask`](crate::GraphExtractorTask)).
#[derive(Debug, Default, Clone)]
pub struct PersonaSignalExtractorState {
    /// Monotonic counter incremented each `extract_pass` invocation.
    pub pass_seq: u64,
    /// Chronological turn index (within agent episode stream) advanced each processed episode.
    pub global_turn_index: u32,
    implicit_brevity_streak: u8,
    /// Last `global_turn_index` at which a brevity-family signal was emitted (explicit or implicit).
    last_brevity_emit_turn: Option<u32>,
    /// Current run of same formality direction (`Informal` / `Formal`) on user text.
    formality_run: Option<(FormalityDir, u8)>,
    /// `topic_cluster` key → `pass_seq` when domain emergence last fired.
    domain_cluster_last_emit_pass: HashMap<String, u64>,
}

impl PersonaSignalExtractorState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormalityDir {
    Informal,
    Formal,
}

const BREVITY_DEBOUNCE_TURNS: u32 = 3;
const DOMAIN_COOLDOWN_PASSES: u64 = 2;
const DOMAIN_MIN_RECURRENCE_NODE: u32 = 3;
const DOMAIN_EMIT_AT_LEAST_NODES: usize = 2;
const DOMAIN_SINGLE_NODE_RECURRENCE: u32 = 6;

fn trace_obj(ep: &EpisodicNode) -> Option<&serde_json::Map<String, Value>> {
    ep.trace_event.as_ref()?.as_object()
}

fn user_text(ep: &EpisodicNode) -> String {
    if let Some(s) = &ep.user_message {
        return s.clone();
    }
    trace_obj(ep)
        .and_then(|m| m.get("user_message"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn assistant_tokens(ep: &EpisodicNode) -> u32 {
    if ep.assistant_response_tokens > 0 {
        return ep.assistant_response_tokens;
    }
    trace_obj(ep)
        .and_then(|m| m.get("assistant_response_tokens"))
        .and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f as u64)))
        .map(|u| u as u32)
        .unwrap_or(0)
}

fn user_tokens(ep: &EpisodicNode) -> u32 {
    if ep.user_message_tokens > 0 {
        return ep.user_message_tokens;
    }
    let t = user_text(ep);
    if t.is_empty() {
        0
    } else {
        t.split_whitespace().count() as u32
    }
}

fn implicit_brevity_shape(ep: &EpisodicNode) -> bool {
    let ut = user_tokens(ep);
    let atok = assistant_tokens(ep);
    ut < 12 && atok > 300
}

fn formality_direction_from_tag(user: &str) -> Option<FormalityDir> {
    infer_formality(user).and_then(|tag| match tag.value.as_str() {
        "informal" => Some(FormalityDir::Informal),
        "formal" => Some(FormalityDir::Formal),
        _ => None,
    })
}

fn brevity_debounce_allows(state: &PersonaSignalExtractorState, turn: u32) -> bool {
    match state.last_brevity_emit_turn {
        None => true,
        Some(prev) if turn.saturating_sub(prev) >= BREVITY_DEBOUNCE_TURNS => true,
        _ => false,
    }
}

fn append_episode_tags(
    store: &SqliteGraphStore,
    node_id: Uuid,
    tags: &[String],
) -> Result<(), String> {
    if tags.is_empty() {
        return Ok(());
    }
    let Some(mut node) = store.read_node(node_id)? else {
        return Ok(());
    };
    let AinlNodeType::Episode { ref mut episodic } = node.node_type else {
        return Ok(());
    };
    let existing: HashSet<&str> = episodic
        .persona_signals_emitted
        .iter()
        .map(|s| s.as_str())
        .collect();
    let mut seen_new: HashSet<String> = HashSet::new();
    let mut new_tags: Vec<String> = Vec::new();
    for t in tags.iter().filter(|t| !existing.contains(t.as_str())) {
        if seen_new.insert(t.clone()) {
            new_tags.push(t.clone());
        }
    }
    if new_tags.is_empty() {
        return Ok(());
    }
    episodic.persona_signals_emitted.extend(new_tags);
    store.write_node(&node)
}

fn tool_affinity_signals(episode_id: Uuid, ep: &EpisodicNode) -> Vec<RawSignal> {
    let tools: Vec<String> = ep.effective_tools().to_vec();
    let tagged = tag_tool_names(&tools);
    let mut out = Vec::new();
    for _ in tagged {
        out.push(RawSignal {
            axis: PersonaAxis::Instrumentality,
            reward: 0.68,
            weight: 0.5,
            source_node_id: episode_id,
            source_node_type: MemoryNodeType::Episodic,
        });
    }
    out
}

fn cluster_key(topic: Option<&String>) -> Option<String> {
    let t = topic?.trim();
    if t.is_empty() {
        return None;
    }
    Some(t.to_ascii_lowercase())
}

fn domain_emergence_signals(
    store: &SqliteGraphStore,
    agent_id: &str,
    state: &mut PersonaSignalExtractorState,
) -> Result<Vec<RawSignal>, String> {
    let mut by_cluster: HashMap<String, Vec<SemanticNode>> = HashMap::new();
    for node in store.find_by_type("semantic")? {
        if node.agent_id != agent_id {
            continue;
        }
        let AinlNodeType::Semantic { semantic } = node.node_type else {
            continue;
        };
        let Some(key) = cluster_key(semantic.topic_cluster.as_ref()) else {
            continue;
        };
        by_cluster.entry(key).or_default().push(semantic);
    }

    let mut out = Vec::new();
    for (cluster, nodes) in by_cluster {
        let strong_nodes = nodes
            .iter()
            .filter(|n| n.recurrence_count >= DOMAIN_MIN_RECURRENCE_NODE)
            .count();
        let max_rec = nodes.iter().map(|n| n.recurrence_count).max().unwrap_or(0);
        let crosses =
            strong_nodes >= DOMAIN_EMIT_AT_LEAST_NODES || max_rec >= DOMAIN_SINGLE_NODE_RECURRENCE;
        if !crosses {
            continue;
        }
        if let Some(last_pass) = state.domain_cluster_last_emit_pass.get(&cluster).copied() {
            if state.pass_seq.saturating_sub(last_pass) < DOMAIN_COOLDOWN_PASSES {
                continue;
            }
        }
        let Some(anchor) = nodes.first() else {
            continue;
        };
        state
            .domain_cluster_last_emit_pass
            .insert(cluster.clone(), state.pass_seq);
        out.push(RawSignal {
            axis: PersonaAxis::Persistence,
            reward: 0.72,
            weight: 0.6,
            source_node_id: anchor.source_turn_id,
            source_node_type: MemoryNodeType::Semantic,
        });
    }
    Ok(out)
}

fn correction_emit_tag(tag: &SemanticTag) -> String {
    match tag.namespace {
        TagNamespace::Behavior => format!("det:behavior:{}", tag.value),
        TagNamespace::Correction => format!("det:correction:{}", tag.value),
        _ => format!("det:{}", tag.to_canonical_string().replace(':', "_")),
    }
}

/// Collected signals plus episode tag writes deferred to [`flush_episode_pattern_tags`].
#[derive(Debug, Default)]
pub struct ExtractPassCollected {
    pub signals: Vec<RawSignal>,
    pub pending_tags: Vec<(Uuid, Vec<String>)>,
}

/// Episode-ordered heuristics plus semantic domain pass; updates `state` and may patch episode rows.
pub fn extract_pass(
    store: &SqliteGraphStore,
    agent_id: &str,
    state: &mut PersonaSignalExtractorState,
) -> Result<Vec<RawSignal>, String> {
    let collected = extract_pass_collect(store, agent_id, state)?;
    flush_episode_pattern_tags(store, &collected.pending_tags)?;
    Ok(collected.signals)
}

/// Build signals and pending episode tag patches without writing episodes yet.
pub fn extract_pass_collect(
    store: &SqliteGraphStore,
    agent_id: &str,
    state: &mut PersonaSignalExtractorState,
) -> Result<ExtractPassCollected, String> {
    state.pass_seq = state.pass_seq.saturating_add(1);

    let mut episodes: Vec<AinlMemoryNode> = store
        .find_by_type("episode")?
        .into_iter()
        .filter(|n| n.agent_id == agent_id)
        .collect();
    episodes.sort_by_key(|n| match &n.node_type {
        AinlNodeType::Episode { episodic } => episodic.timestamp,
        _ => 0,
    });

    let mut out = Vec::new();
    let mut pending_tags: Vec<(Uuid, Vec<String>)> = Vec::new();

    for ep_node in &episodes {
        let episode_id = ep_node.id;
        let AinlNodeType::Episode { episodic } = &ep_node.node_type else {
            continue;
        };
        let turn = state.global_turn_index;
        state.global_turn_index = state.global_turn_index.saturating_add(1);

        let mut tags: Vec<String> = Vec::new();

        // `GraphExtractor` / `extract_episodic_signals` already emits Instrumentality from
        // `effective_tools()` when `episodic_should_process` — skip redundant tool affinity here.
        if !episodic_should_process(episodic) {
            out.extend(tool_affinity_signals(episode_id, episodic));
        }

        let user = user_text(episodic);

        if let Some(tag) = extract_correction_behavior(&user) {
            out.push(RawSignal {
                axis: PersonaAxis::Systematicity,
                reward: 0.84,
                weight: 0.85,
                source_node_id: episode_id,
                source_node_type: MemoryNodeType::Episodic,
            });
            tags.push(correction_emit_tag(&tag));
        }

        if !user.is_empty()
            && infer_brevity_preference(&user).is_some()
            && brevity_debounce_allows(state, turn)
        {
            out.push(RawSignal {
                axis: PersonaAxis::Verbosity,
                reward: 0.22,
                weight: 0.75,
                source_node_id: episode_id,
                source_node_type: MemoryNodeType::Episodic,
            });
            tags.push("det:brevity:explicit".into());
            state.last_brevity_emit_turn = Some(turn);
            state.implicit_brevity_streak = 0;
        } else if implicit_brevity_shape(episodic) {
            state.implicit_brevity_streak = state.implicit_brevity_streak.saturating_add(1);
            if state.implicit_brevity_streak >= 2 && brevity_debounce_allows(state, turn) {
                out.push(RawSignal {
                    axis: PersonaAxis::Verbosity,
                    reward: 0.24,
                    weight: 0.7,
                    source_node_id: episode_id,
                    source_node_type: MemoryNodeType::Episodic,
                });
                tags.push("det:brevity:implicit_shape".into());
                state.last_brevity_emit_turn = Some(turn);
                state.implicit_brevity_streak = 0;
            }
        } else {
            state.implicit_brevity_streak = 0;
        }

        if !user.is_empty() {
            match formality_direction_from_tag(&user) {
                Some(dir) => {
                    let bump = match &mut state.formality_run {
                        Some((cur, n)) if *cur == dir => {
                            *n = n.saturating_add(1);
                            *n
                        }
                        _ => {
                            state.formality_run = Some((dir, 1));
                            1
                        }
                    };
                    if bump >= 3 {
                        let (reward, tag) = match dir {
                            FormalityDir::Formal => (0.78_f32, "det:formality:formal_run"),
                            FormalityDir::Informal => (0.28_f32, "det:formality:informal_run"),
                        };
                        out.push(RawSignal {
                            axis: PersonaAxis::Systematicity,
                            reward,
                            weight: 0.65,
                            source_node_id: episode_id,
                            source_node_type: MemoryNodeType::Episodic,
                        });
                        tags.push(tag.into());
                        state.formality_run = None;
                    }
                }
                None => {
                    state.formality_run = None;
                }
            }
        }

        if !tags.is_empty() {
            pending_tags.push((episode_id, tags));
        }
    }

    out.extend(domain_emergence_signals(store, agent_id, state)?);
    Ok(ExtractPassCollected {
        signals: out,
        pending_tags,
    })
}

/// Apply episode `persona_signals_emitted` tag patches from [`extract_pass_collect`].
pub fn flush_episode_pattern_tags(
    store: &SqliteGraphStore,
    pending: &[(Uuid, Vec<String>)],
) -> Result<(), String> {
    for (episode_id, tags) in pending {
        append_episode_tags(store, *episode_id, tags)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_memory::{AinlMemoryNode, AinlNodeType, SqliteGraphStore};
    use ainl_semantic_tagger::{
        extract_correction_behavior, infer_brevity_preference, infer_formality, TagNamespace,
    };
    use uuid::Uuid;

    fn ep_with_tokens(user_t: u32, asst_t: u32) -> EpisodicNode {
        let tid = Uuid::new_v4();
        EpisodicNode {
            turn_id: tid,
            timestamp: 0,
            tool_calls: vec![],
            delegation_to: None,
            trace_event: None,
            turn_index: 0,
            user_message_tokens: user_t,
            assistant_response_tokens: asst_t,
            tools_invoked: vec![],
            persona_signals_emitted: vec![],
            sentiment: None,
            flagged: false,
            conversation_id: String::new(),
            follows_episode_id: None,
            user_message: None,
            assistant_response: None,
            tags: vec![],
            vitals_gate: None,
            vitals_phase: None,
            vitals_trust: None,
        }
    }

    #[test]
    fn brevity_explicit_keyword_emits() {
        let mut st = PersonaSignalExtractorState::default();
        let tid = Uuid::new_v4();
        let mut ep = ep_with_tokens(0, 0);
        ep.user_message = Some("Please be more concise here.".into());
        let mut out: Vec<RawSignal> = Vec::new();
        let mut tags: Vec<String> = Vec::new();
        let turn = 0;
        let user = user_text(&ep);
        if !user.is_empty()
            && infer_brevity_preference(&user).is_some()
            && brevity_debounce_allows(&st, turn)
        {
            out.push(RawSignal {
                axis: PersonaAxis::Verbosity,
                reward: 0.22,
                weight: 0.75,
                source_node_id: tid,
                source_node_type: MemoryNodeType::Episodic,
            });
            tags.push("det:brevity:explicit".into());
            st.last_brevity_emit_turn = Some(turn);
        }
        assert_eq!(out.len(), 1);
        assert_eq!(tags.len(), 1);
    }

    #[test]
    fn brevity_implicit_single_no_emit_double_emits() {
        let mut st = PersonaSignalExtractorState::default();
        let ep = ep_with_tokens(5, 400);
        assert!(implicit_brevity_shape(&ep));
        st.implicit_brevity_streak = st.implicit_brevity_streak.saturating_add(1);
        assert_eq!(st.implicit_brevity_streak, 1);
        assert!(st.implicit_brevity_streak < 2);
    }

    #[test]
    fn brevity_implicit_two_consecutive_emits_via_pass() {
        let dir = tempfile::tempdir().expect("d");
        let store = SqliteGraphStore::open(&dir.path().join("br.db")).expect("open");
        let agent = "agent-br";
        let mut st = PersonaSignalExtractorState::default();
        for (ts, ut, at) in [(1_i64, 5_u32, 400_u32), (2_i64, 4_u32, 350_u32)] {
            let tid = Uuid::new_v4();
            let mut n = AinlMemoryNode::new_episode(tid, ts, vec![], None, None);
            n.agent_id = agent.into();
            if let AinlNodeType::Episode { episodic } = &mut n.node_type {
                episodic.user_message_tokens = ut;
                episodic.assistant_response_tokens = at;
            }
            store.write_node(&n).expect("w");
        }
        let sigs = extract_pass(&store, agent, &mut st).expect("extract");
        let brevity = sigs
            .iter()
            .filter(|s| s.axis == PersonaAxis::Verbosity)
            .count();
        assert!(
            brevity >= 1,
            "expected implicit brevity after two qualifying turns"
        );
    }

    #[test]
    fn brevity_debounce_blocks() {
        let st = PersonaSignalExtractorState {
            last_brevity_emit_turn: Some(0),
            ..Default::default()
        };
        assert!(!brevity_debounce_allows(&st, 1));
        assert!(!brevity_debounce_allows(&st, 2));
        assert!(brevity_debounce_allows(&st, 3));
    }

    #[test]
    fn tool_invocations_emit_one_each() {
        let tid = Uuid::new_v4();
        let mut ep = ep_with_tokens(0, 0);
        ep.tools_invoked = vec!["file_read".into(), "shell_exec".into()];
        let sigs = tool_affinity_signals(tid, &ep);
        assert_eq!(sigs.len(), 2);
        assert!(sigs.iter().all(|s| s.axis == PersonaAxis::Instrumentality));
    }

    #[test]
    fn append_episode_tags_dedupes_existing_and_within_batch() {
        let dir = tempfile::tempdir().expect("d");
        let store = SqliteGraphStore::open(&dir.path().join("ep_tags.db")).expect("open");
        let tid = Uuid::new_v4();
        let mut n = AinlMemoryNode::new_episode(tid, 1, vec![], None, None);
        n.agent_id = "a".into();
        store.write_node(&n).expect("w");
        append_episode_tags(
            &store,
            n.id,
            &["det:brevity:explicit".into(), "det:brevity:explicit".into()],
        )
        .expect("append");
        let r = store.read_node(n.id).expect("r").expect("node");
        let AinlNodeType::Episode { episodic } = r.node_type else {
            panic!();
        };
        assert_eq!(
            episodic.persona_signals_emitted,
            vec!["det:brevity:explicit".to_string()]
        );
        append_episode_tags(&store, n.id, &["det:brevity:explicit".into()]).expect("append2");
        let r2 = store.read_node(n.id).expect("r2").expect("node");
        let AinlNodeType::Episode { episodic: e2 } = r2.node_type else {
            panic!();
        };
        assert_eq!(e2.persona_signals_emitted.len(), 1);
    }

    #[test]
    fn formality_single_informal_no_emit_until_three() {
        let t = infer_formality("yo gonna grab some food lol yeah").expect("tag");
        assert_eq!(t.value, "informal");
    }

    #[test]
    fn formality_three_informal_emits_logic() {
        let mut run: Option<(FormalityDir, u8)> = None;
        let informal_line = "yeah gonna wanna grab some cool stuff lol";
        let mut emitted = false;
        for _ in 0..3 {
            let dir = formality_direction_from_tag(informal_line).expect("dir");
            assert_eq!(dir, FormalityDir::Informal);
            let bump = match &mut run {
                Some((FormalityDir::Informal, n)) => {
                    *n += 1;
                    *n
                }
                _ => {
                    run = Some((FormalityDir::Informal, 1));
                    1
                }
            };
            if bump >= 3 {
                emitted = true;
            }
        }
        assert!(emitted);
    }

    #[test]
    fn formality_mixed_resets() {
        let mut run: Option<(FormalityDir, u8)> = None;
        let msgs = [
            "gonna grab food",
            "Therefore, the coefficient matrix exhibits stability.",
            "ok lol",
        ];
        let mut max_run = 0u8;
        for m in msgs {
            match formality_direction_from_tag(m) {
                Some(dir) => {
                    let bump = match &mut run {
                        Some((cur, n)) if *cur == dir => {
                            *n += 1;
                            *n
                        }
                        _ => {
                            run = Some((dir, 1));
                            1
                        }
                    };
                    max_run = max_run.max(bump);
                }
                None => run = None,
            }
        }
        assert!(max_run < 3);
    }

    #[test]
    fn domain_recurrence_not_reference() {
        let (_d, store) = {
            let dir = tempfile::tempdir().expect("d");
            let p = dir.path().join("t.db");
            let s = SqliteGraphStore::open(&p).expect("open");
            (dir, s)
        };
        let tid = Uuid::new_v4();
        let mut s1 = AinlMemoryNode::new_fact("a".into(), 0.8, tid);
        s1.agent_id = "ag".into();
        if let AinlNodeType::Semantic { semantic } = &mut s1.node_type {
            semantic.topic_cluster = Some("rust".into());
            semantic.recurrence_count = 1;
            semantic.reference_count = 99;
        }
        store.write_node(&s1).expect("w");
        let mut s2 = AinlMemoryNode::new_fact("b".into(), 0.8, tid);
        s2.agent_id = "ag".into();
        if let AinlNodeType::Semantic { semantic } = &mut s2.node_type {
            semantic.topic_cluster = Some("rust".into());
            semantic.recurrence_count = 1;
            semantic.reference_count = 99;
        }
        store.write_node(&s2).expect("w");
        let mut st = PersonaSignalExtractorState {
            pass_seq: 1,
            ..Default::default()
        };
        let sigs = domain_emergence_signals(&store, "ag", &mut st).expect("d");
        assert!(sigs.is_empty(), "high reference_count must not gate domain");
    }

    #[test]
    fn domain_threshold_crosses() {
        let dir = tempfile::tempdir().expect("d");
        let store = SqliteGraphStore::open(&dir.path().join("d.db")).expect("open");
        let tid = Uuid::new_v4();
        for fact in ["a", "b"] {
            let mut s = AinlMemoryNode::new_fact(fact.into(), 0.8, tid);
            s.agent_id = "ag".into();
            if let AinlNodeType::Semantic { semantic } = &mut s.node_type {
                semantic.topic_cluster = Some("rust".into());
                semantic.recurrence_count = 3;
            }
            store.write_node(&s).expect("w");
        }
        let mut st = PersonaSignalExtractorState {
            pass_seq: 1,
            ..Default::default()
        };
        let sigs = domain_emergence_signals(&store, "ag", &mut st).expect("d");
        assert_eq!(sigs.len(), 1);
    }

    #[test]
    fn domain_cooldown_second_pass_suppressed() {
        let dir = tempfile::tempdir().expect("d");
        let store = SqliteGraphStore::open(&dir.path().join("d2.db")).expect("open");
        let tid = Uuid::new_v4();
        for fact in ["a", "b"] {
            let mut s = AinlMemoryNode::new_fact(fact.into(), 0.8, tid);
            s.agent_id = "ag".into();
            if let AinlNodeType::Semantic { semantic } = &mut s.node_type {
                semantic.topic_cluster = Some("go".into());
                semantic.recurrence_count = 3;
            }
            store.write_node(&s).expect("w");
        }
        let mut st = PersonaSignalExtractorState {
            pass_seq: 1,
            ..Default::default()
        };
        let n1 = domain_emergence_signals(&store, "ag", &mut st)
            .expect("d")
            .len();
        st.pass_seq = 2;
        let n2 = domain_emergence_signals(&store, "ag", &mut st)
            .expect("d")
            .len();
        assert_eq!(n1, 1);
        assert_eq!(n2, 0);
    }

    #[test]
    fn correction_dont_use_bullets() {
        let t = extract_correction_behavior("don't use bullet points").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Correction);
        assert_eq!(t.value, "avoid_bullets");
    }

    #[test]
    fn correction_you_keep_caveats() {
        let t = extract_correction_behavior("you keep adding caveats").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Behavior);
        assert_eq!(t.value, "adding_caveats");
    }

    #[test]
    fn correction_told_emojis() {
        let t = extract_correction_behavior("I told you not to use emojis").expect("tag");
        assert_eq!(t.namespace, TagNamespace::Correction);
        assert_eq!(t.value, "avoid_emojis");
    }

    #[test]
    fn correction_stop_alone() {
        assert!(extract_correction_behavior("stop").is_none());
    }

    #[test]
    fn correction_i_said_so() {
        assert!(extract_correction_behavior("I said so").is_none());
    }

    #[test]
    fn correction_dont_do_that_no_behavior() {
        assert!(extract_correction_behavior("don't do that").is_none());
    }
}
