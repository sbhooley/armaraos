//! `EvolutionEngine` — persona axes with explicit extract / ingest / snapshot / write phases.
//!
//! Callers can compose these steps for dry runs (skip [`Self::write_persona_node`]),
//! correction ticks (`correction_tick` → [`Self::snapshot`] → [`Self::write_persona_node`]), or
//! a full pass via [`Self::evolve`].

use crate::axes::{default_axis_map, AxisState, PersonaAxis};
use crate::extractor::GraphExtractor;
use crate::fitness::PersonaSnapshot;
use crate::persona_node;
use crate::signals::RawSignal;
use ainl_memory::SqliteGraphStore;
use chrono::Utc;
use std::collections::HashMap;

/// Minimum absolute score delta on an axis for a signal to count as "applied" in [`EvolutionEngine::ingest_signals`].
pub const INGEST_SCORE_EPSILON: f32 = 0.001;

pub struct EvolutionEngine {
    pub agent_id: String,
    pub axes: HashMap<PersonaAxis, AxisState>,
}

impl EvolutionEngine {
    pub fn new(agent_id: impl Into<String>) -> Self {
        let agent_id = agent_id.into();
        Self {
            axes: default_axis_map(0.5),
            agent_id,
        }
    }

    /// Read signals from the graph store for this agent (no writes).
    pub fn extract_signals(&self, store: &SqliteGraphStore) -> Result<Vec<RawSignal>, String> {
        GraphExtractor::extract(store, &self.agent_id)
    }

    /// Applies each signal to its axis EMA. Returns how many signals produced a score change
    /// larger than [`INGEST_SCORE_EPSILON`] on an existing axis (skipped axes do not count).
    pub fn ingest_signals(&mut self, signals: Vec<RawSignal>) -> usize {
        let mut applied = 0usize;
        for sig in signals {
            if let Some(state) = self.axes.get_mut(&sig.axis) {
                let prior = state.score;
                state.update_weighted(sig.reward, sig.weight);
                if (state.score - prior).abs() > INGEST_SCORE_EPSILON {
                    applied += 1;
                }
            }
        }
        applied
    }

    /// Current axes as a snapshot (no writes).
    pub fn snapshot(&self) -> PersonaSnapshot {
        PersonaSnapshot {
            agent_id: self.agent_id.clone(),
            axes: self.axes.clone(),
            captured_at: Utc::now(),
        }
    }

    /// Persist `snapshot` to the evolution [`PersonaNode`](ainl_memory::PersonaNode) row.
    ///
    /// `snapshot.agent_id` must match this engine's agent id.
    pub fn write_persona_node(
        &self,
        store: &SqliteGraphStore,
        snapshot: &PersonaSnapshot,
    ) -> Result<(), String> {
        if snapshot.agent_id != self.agent_id {
            return Err(format!(
                "PersonaSnapshot agent_id {:?} does not match engine agent_id {:?}",
                snapshot.agent_id, self.agent_id
            ));
        }
        persona_node::write_evolved_persona_snapshot(store, &self.agent_id, &snapshot.axes)
    }

    /// Full evolution pass: extract → ingest → snapshot → write.
    pub fn evolve(&mut self, store: &SqliteGraphStore) -> Result<PersonaSnapshot, String> {
        let signals = self.extract_signals(store)?;
        self.ingest_signals(signals);
        let snapshot = self.snapshot();
        self.write_persona_node(store, &snapshot)?;
        Ok(snapshot)
    }

    pub fn correction_tick(&mut self, axis: PersonaAxis, correction: f32) {
        if let Some(state) = self.axes.get_mut(&axis) {
            state.update_weighted(correction.clamp(0.0, 1.0), 1.0);
        }
    }
}
