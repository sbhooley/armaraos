//! One-shot extraction pass wired to [`ainl_persona::EvolutionEngine`].
//!
//! Instrumentality from episode tools is emitted by [`ainl_persona::GraphExtractor`] when the
//! episode is processable; `extract_pass` skips its redundant `tool_affinity` leg in that case
//! (see [`crate::persona_signals::extract_pass`]).

use crate::persona_signals::{
    extract_pass_collect, flush_episode_pattern_tags, ExtractPassCollected,
    PersonaSignalExtractorState,
};
use crate::recurrence::update_semantic_recurrence;
use ainl_memory::{GraphStore, SqliteGraphStore};
use ainl_persona::{EvolutionEngine, PersonaSnapshot, RawSignal};
use chrono::{DateTime, Utc};

fn store_has_any_persona_for_agent(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<bool, String> {
    for n in store.find_by_type("persona")? {
        if n.agent_id == agent_id {
            return Ok(true);
        }
    }
    Ok(false)
}

fn merge_err_slot(slot: &mut Option<String>, e: String) {
    match slot {
        None => *slot = Some(e),
        Some(prev) => {
            prev.push_str("; ");
            prev.push_str(&e);
        }
    }
}

pub struct GraphExtractorTask {
    pub agent_id: String,
    pub evolution_engine: EvolutionEngine,
    pub signal_state: PersonaSignalExtractorState,
    /// Test-only: force [`ExtractionReport::extract_error`] without running the extract pipeline.
    #[doc(hidden)]
    test_inject_extract_error: Option<String>,
    /// Test-only: force [`ExtractionReport::pattern_error`] after a successful collect phase.
    #[doc(hidden)]
    test_inject_pattern_error: Option<String>,
    /// Test-only: force [`ExtractionReport::persona_error`] instead of a real persona write outcome.
    #[doc(hidden)]
    test_inject_persona_error: Option<String>,
}

/// Result of [`GraphExtractorTask::run_pass`]. Errors are carried per phase; the pass does not
/// return `Result` so callers can record partial progress and continue.
#[derive(Debug, Clone)]
pub struct ExtractionReport {
    pub agent_id: String,
    /// Signals merged from graph extractors + heuristic pass (before ingest).
    pub merged_signals: Vec<RawSignal>,
    /// Semantic recurrence rows updated this pass (`None` = recurrence phase did not complete).
    pub facts_written: Option<u32>,
    /// Error during recurrence update, graph signal read, or heuristic collect (before pattern flush).
    pub extract_error: Option<String>,
    /// Error flushing episode tag pattern writes.
    pub pattern_error: Option<String>,
    /// Error persisting the evolution persona row.
    pub persona_error: Option<String>,
    pub semantic_nodes_updated: usize,
    pub signals_extracted: usize,
    pub signals_applied: usize,
    pub persona_snapshot: PersonaSnapshot,
    pub timestamp: DateTime<Utc>,
}

impl ExtractionReport {
    pub fn has_errors(&self) -> bool {
        self.extract_error.is_some() || self.pattern_error.is_some() || self.persona_error.is_some()
    }
}

impl GraphExtractorTask {
    pub fn new(agent_id: &str) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            evolution_engine: EvolutionEngine::new(agent_id),
            signal_state: PersonaSignalExtractorState::new(),
            test_inject_extract_error: None,
            test_inject_pattern_error: None,
            test_inject_persona_error: None,
        }
    }

    /// Semantic recurrence updates, graph + heuristic signals, ingest, optional evolution write.
    ///
    /// Per-phase failures are recorded on [`ExtractionReport`]. Phases keep running so callers
    /// can observe independent `extract_error`, `pattern_error`, and `persona_error` slots.
    pub fn run_pass(&mut self, store: &SqliteGraphStore) -> ExtractionReport {
        let agent_id = self.agent_id.clone();
        let ts = Utc::now();
        let mut report = ExtractionReport {
            agent_id: agent_id.clone(),
            merged_signals: Vec::new(),
            facts_written: None,
            extract_error: None,
            pattern_error: None,
            persona_error: None,
            semantic_nodes_updated: 0,
            signals_extracted: 0,
            signals_applied: 0,
            persona_snapshot: self.evolution_engine.snapshot(),
            timestamp: ts,
        };

        let semantic_nodes_updated = match update_semantic_recurrence(store, &agent_id) {
            Ok(n) => n,
            Err(e) => {
                merge_err_slot(&mut report.pattern_error, e);
                0
            }
        };
        report.semantic_nodes_updated = semantic_nodes_updated;
        if report.pattern_error.is_none() {
            report.facts_written = Some(semantic_nodes_updated as u32);
        }

        let mut graph_signals = match self.evolution_engine.extract_signals(store) {
            Ok(s) => s,
            Err(e) => {
                merge_err_slot(&mut report.extract_error, e);
                Vec::new()
            }
        };

        let collected = match extract_pass_collect(store, &agent_id, &mut self.signal_state) {
            Ok(c) => c,
            Err(e) => {
                merge_err_slot(&mut report.extract_error, e);
                ExtractPassCollected::default()
            }
        };
        graph_signals.extend(collected.signals);

        if let Some(e) = self.test_inject_extract_error.take() {
            merge_err_slot(&mut report.extract_error, e);
        }

        if let Some(e) = self.test_inject_pattern_error.take() {
            merge_err_slot(&mut report.pattern_error, e);
        } else if let Err(e) = flush_episode_pattern_tags(store, &collected.pending_tags) {
            merge_err_slot(&mut report.pattern_error, e);
        }

        let signals_extracted = graph_signals.len();
        report.signals_extracted = signals_extracted;
        report.merged_signals = graph_signals.clone();

        let signals_applied = self.evolution_engine.ingest_signals(graph_signals);
        report.signals_applied = signals_applied;

        let persona_snapshot = self.evolution_engine.snapshot();
        report.persona_snapshot = persona_snapshot.clone();

        let had_prior_persona = match store_has_any_persona_for_agent(store, &agent_id) {
            Ok(b) => b,
            Err(e) => {
                merge_err_slot(&mut report.extract_error, format!("persona_row_probe: {e}"));
                false
            }
        };
        let should_persist_persona =
            signals_extracted > 0 || semantic_nodes_updated > 0 || had_prior_persona;

        if should_persist_persona {
            if let Some(e) = self.test_inject_persona_error.take() {
                merge_err_slot(&mut report.persona_error, e);
            } else if let Err(e) = self
                .evolution_engine
                .write_persona_node(store, &persona_snapshot)
            {
                merge_err_slot(&mut report.persona_error, e);
            }
        }

        report
    }

    /// Test hook: inject an extract-phase error on the next [`run_pass`](Self::run_pass).
    #[doc(hidden)]
    #[cfg(any(test, debug_assertions))]
    pub fn test_inject_extract_error_once(&mut self, message: impl Into<String>) {
        self.test_inject_extract_error = Some(message.into());
    }

    /// Test hook: inject a pattern-phase error on the next [`run_pass`](Self::run_pass).
    #[doc(hidden)]
    #[cfg(any(test, debug_assertions))]
    pub fn test_inject_pattern_error_once(&mut self, message: impl Into<String>) {
        self.test_inject_pattern_error = Some(message.into());
    }

    /// Test hook: inject a persona-phase error on the next [`run_pass`](Self::run_pass).
    #[doc(hidden)]
    #[cfg(any(test, debug_assertions))]
    pub fn test_inject_persona_error_once(&mut self, message: impl Into<String>) {
        self.test_inject_persona_error = Some(message.into());
    }
}
