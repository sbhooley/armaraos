//! Post-turn persona axis updates from live tool/delegation signals.
//!
//! Dashboard episodes often omit `trace_event.outcome: success`, so graph extraction skips
//! episodic tool hints. This module layers **explicit** turn signals on top of the latest
//! persisted [`ainl_persona::EVOLUTION_TRAIT_NAME`] snapshot after
//! [`crate::graph_memory_writer::GraphMemoryWriter::run_persona_evolution_pass`].
//!
//! **Activation:** the `ainl-persona-evolution` feature (on by default) is the primary control.
//! Evolution runs automatically for every turn when the feature is compiled in.
//! Set `AINL_PERSONA_EVOLUTION=0` (or `false`/`no`/`off`) to opt out at runtime without
//! recompiling. Any other value (or absence) keeps evolution enabled.

use crate::graph_memory_writer::GraphMemoryWriter;
use std::sync::Arc;

/// Canonical inputs for one completed agent turn (tool names + optional delegation target).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TurnOutcome {
    pub tool_calls: Vec<String>,
    pub delegation_to: Option<String>,
}

/// `true` when persona evolution should run for this process.
///
/// When the `ainl-persona-evolution` feature is compiled in, evolution is **enabled by default**.
/// Set `AINL_PERSONA_EVOLUTION=0` (or `false`/`no`/`off`) to opt out at runtime.
/// Any other value (or absence) leaves evolution enabled.
pub fn persona_turn_evolution_env_enabled() -> bool {
    !std::env::var("AINL_PERSONA_EVOLUTION")
        .map(|v| {
            let t = v.trim().to_ascii_lowercase();
            matches!(t.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(false)
}

/// Optional handle for hosts that keep a shared [`GraphMemoryWriter`].
pub struct PersonaEvolutionHook {
    pub writer: Arc<GraphMemoryWriter>,
}

impl PersonaEvolutionHook {
    pub fn new(writer: GraphMemoryWriter) -> Self {
        Self {
            writer: Arc::new(writer),
        }
    }

    pub async fn evolve_turn(&self, agent_id: &str, turn: &TurnOutcome) -> Result<(), String> {
        Self::evolve_from_turn(&self.writer, agent_id, turn).await
    }

    /// Incremental axis ingest for this turn (env-gated; no-op when the crate feature is off).
    pub async fn evolve_from_turn(
        writer: &GraphMemoryWriter,
        agent_id: &str,
        turn: &TurnOutcome,
    ) -> Result<(), String> {
        evolve_from_turn_impl(writer, agent_id, turn).await
    }
}

#[cfg(feature = "ainl-persona-evolution")]
async fn evolve_from_turn_impl(
    writer: &GraphMemoryWriter,
    agent_id: &str,
    turn: &TurnOutcome,
) -> Result<(), String> {
    use ainl_memory::PersonaNode;
    use ainl_persona::EvolutionEngine;

    if !persona_turn_evolution_env_enabled() {
        return Ok(());
    }
    if agent_id != writer.agent_id() {
        return Err(format!(
            "persona evolution agent_id mismatch: arg={agent_id} writer={}",
            writer.agent_id()
        ));
    }

    let lookback = crate::graph_memory_writer::PERSONA_PRIOR_LOOKBACK_SECS;
    let persona_rows: Vec<PersonaNode> = writer.recall_persona_for_agent(agent_id, lookback).await;

    tracing::debug!(
        agent_id = %agent_id,
        persona_rows = persona_rows.len(),
        tools = ?turn.tool_calls,
        delegation = ?turn.delegation_to,
        "AINL persona turn evolution: loaded persona rows + turn signals"
    );

    let synthetic = synthetic_turn_raw_signals(turn);
    let mut engine = EvolutionEngine::new(agent_id);
    let inner = writer.inner.lock().await;
    let store = inner.sqlite_store();
    seed_engine_axes_from_evolution_row(&mut engine, store)?;
    engine.ingest_signals(synthetic);
    let snap = engine.snapshot();
    engine.write_persona_node(store, &snap)?;
    let tools = turn
        .tool_calls
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let summary = if tools.is_empty() {
        "Persona axis update from turn signals".to_string()
    } else {
        format!("Axis ingest from tools: {tools}")
    };
    writer.emit_write_observed(
        "persona",
        Some(openfang_types::event::GraphMemoryWriteProvenance {
            node_ids: vec![],
            node_kind: Some("persona".to_string()),
            reason: Some("turn_axis_ingest".to_string()),
            summary: Some(summary),
            trace_id: None,
            tool_name: None,
        }),
    );
    Ok(())
}

#[cfg(feature = "ainl-persona-evolution")]
fn seed_engine_axes_from_evolution_row(
    engine: &mut ainl_persona::EvolutionEngine,
    store: &ainl_memory::SqliteGraphStore,
) -> Result<(), String> {
    use ainl_memory::{AinlNodeType, GraphStore};
    use ainl_persona::{PersonaAxis, EVOLUTION_TRAIT_NAME};

    for n in store.find_by_type("persona")? {
        if n.agent_id != engine.agent_id {
            continue;
        }
        if let AinlNodeType::Persona { persona } = &n.node_type {
            if persona.trait_name != EVOLUTION_TRAIT_NAME {
                continue;
            }
            for (name, sc) in &persona.axis_scores {
                if let Some(ax) = PersonaAxis::parse(name) {
                    if let Some(st) = engine.axes.get_mut(&ax) {
                        st.score = (*sc).clamp(0.0, 1.0);
                    }
                }
            }
            break;
        }
    }
    Ok(())
}

/// Mirrors episodic tool / outcome heuristics from `ainl-persona` so turns without
/// `trace_event.outcome` still move axes (not exported from the crate root).
#[cfg(feature = "ainl-persona-evolution")]
fn synthetic_turn_raw_signals(turn: &TurnOutcome) -> Vec<ainl_persona::RawSignal> {
    use ainl_persona::{MemoryNodeType, PersonaAxis, RawSignal};
    use uuid::Uuid;
    let synthetic_id = Uuid::nil();
    let mut out = Vec::new();
    for tool in &turn.tool_calls {
        let n = tool.to_ascii_lowercase();
        if n.contains("shell")
            || n.contains("cli")
            || n.contains("mcp")
            || n.contains("compile")
            || n.contains("compiler")
            || n == "ainl"
            || n.contains("ainl_")
        {
            out.push(RawSignal {
                axis: PersonaAxis::Instrumentality,
                reward: 0.8,
                weight: 0.6,
                source_node_id: synthetic_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
        if n.contains("web_search") || n.contains("web_fetch") || n.contains("web.fetch") {
            out.push(RawSignal {
                axis: PersonaAxis::Curiosity,
                reward: 0.7,
                weight: 0.5,
                source_node_id: synthetic_id,
                source_node_type: MemoryNodeType::Episodic,
            });
        }
    }
    if !turn.tool_calls.is_empty() {
        out.push(RawSignal {
            axis: PersonaAxis::Systematicity,
            reward: 0.6,
            weight: 0.5,
            source_node_id: synthetic_id,
            source_node_type: MemoryNodeType::Episodic,
        });
    }
    if turn.delegation_to.is_some() {
        out.push(RawSignal {
            axis: PersonaAxis::Curiosity,
            reward: 0.55,
            weight: 0.4,
            source_node_id: synthetic_id,
            source_node_type: MemoryNodeType::Episodic,
        });
    }
    out
}

#[cfg(not(feature = "ainl-persona-evolution"))]
async fn evolve_from_turn_impl(
    _writer: &GraphMemoryWriter,
    _agent_id: &str,
    _turn: &TurnOutcome,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(test, feature = "ainl-persona-evolution"))]
mod tests {
    use super::*;
    use ainl_memory::{AinlNodeType, GraphMemory, GraphStore};
    use ainl_persona::EVOLUTION_TRAIT_NAME;
    use serde_json::json;

    fn set_evolution_env() -> std::ffi::OsString {
        let key = "AINL_PERSONA_EVOLUTION";
        let prev = std::env::var_os(key).unwrap_or_default();
        std::env::set_var(key, "1");
        prev
    }

    fn restore_evolution_env(prev: std::ffi::OsString) {
        let key = "AINL_PERSONA_EVOLUTION";
        if prev.is_empty() {
            std::env::remove_var(key);
        } else {
            std::env::set_var(key, prev);
        }
    }

    async fn evolution_snapshot_cycle(writer: &GraphMemoryWriter) -> u32 {
        let inner = writer.inner.lock().await;
        let store = inner.sqlite_store();
        for n in store.find_by_type("persona").unwrap_or_else(|_| vec![]) {
            if let AinlNodeType::Persona { persona } = &n.node_type {
                if persona.trait_name == EVOLUTION_TRAIT_NAME {
                    return persona.evolution_cycle;
                }
            }
        }
        0
    }

    /// Two hook passes with the same tool each persist a new evolution snapshot (`evolution_cycle` rises).
    /// Per-axis scores are EMA-smoothed toward `reward * weight` and need not increase every step.
    #[tokio::test]
    async fn test_persona_strength_increases_after_repeated_tool() {
        let prev = set_evolution_env();
        let tmp = tempfile::tempdir().expect("tempdir");
        let db_path = tmp.path().join("persona_evo_turn.db");
        let memory = GraphMemory::new(&db_path).expect("open");
        let writer = GraphMemoryWriter::from_memory_for_tests(memory, "evo-turn-agent", None);

        {
            let inner = writer.inner.lock().await;
            inner
                .write_persona("seed_trait", 0.6, vec![])
                .expect("seed persona so cold passes can persist");
        }

        assert!(writer
            .record_turn(
                vec!["shell_exec".into()],
                None,
                Some(json!({ "outcome": "success" })),
                &[],
                None,
                None,
                None,
                None,
            )
            .await
            .is_some());

        let _ = writer.run_persona_evolution_pass().await;
        let cycle_after_pass = evolution_snapshot_cycle(&writer).await;
        assert!(
            cycle_after_pass >= 1,
            "expected evolution snapshot row after extractor pass"
        );
        let turn = TurnOutcome {
            tool_calls: vec!["shell_exec".into()],
            delegation_to: None,
        };
        PersonaEvolutionHook::evolve_from_turn(&writer, "evo-turn-agent", &turn)
            .await
            .expect("evolve 1");
        let cycle_after_first_hook = evolution_snapshot_cycle(&writer).await;

        // Second hook pass only: each successful write bumps `evolution_cycle` on the snapshot row.
        PersonaEvolutionHook::evolve_from_turn(&writer, "evo-turn-agent", &turn)
            .await
            .expect("evolve 2");
        let cycle_after_second_hook = evolution_snapshot_cycle(&writer).await;

        assert!(
            cycle_after_first_hook > cycle_after_pass,
            "first hook should persist a newer snapshot; {cycle_after_pass} -> {cycle_after_first_hook}"
        );
        assert!(
            cycle_after_second_hook > cycle_after_first_hook,
            "second hook with the same tool should persist again; {cycle_after_first_hook} -> {cycle_after_second_hook}"
        );
        restore_evolution_env(prev);
    }
}
