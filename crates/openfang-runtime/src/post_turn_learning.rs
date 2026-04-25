//! Outcome-aware post-turn learning helpers.
//!
//! Most graph writes are still performed inline by `agent_loop` because they need many local
//! turn variables. This module owns the background part so spawned work has uniform telemetry
//! instead of disappearing as fire-and-forget tasks.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::graph_memory_writer::GraphMemoryWriter;

static PERSONA_BG_STARTED: AtomicU64 = AtomicU64::new(0);
static PERSONA_BG_SUCCEEDED: AtomicU64 = AtomicU64::new(0);
static PERSONA_BG_FAILED: AtomicU64 = AtomicU64::new(0);

#[must_use]
pub fn metrics_snapshot() -> serde_json::Value {
    serde_json::json!({
        "persona_background_started": PERSONA_BG_STARTED.load(Ordering::Relaxed),
        "persona_background_succeeded": PERSONA_BG_SUCCEEDED.load(Ordering::Relaxed),
        "persona_background_failed": PERSONA_BG_FAILED.load(Ordering::Relaxed),
    })
}

pub fn spawn_persona_background(
    gm: GraphMemoryWriter,
    turn_outcome: crate::persona_evolution::TurnOutcome,
    streaming: bool,
) {
    PERSONA_BG_STARTED.fetch_add(1, Ordering::Relaxed);
    tokio::spawn(async move {
        let agent_id = gm.agent_id().to_string();
        let mut failed = false;

        let report = gm.run_persona_evolution_pass().await;
        if report.has_errors() {
            failed = true;
            tracing::warn!(
                agent_id = %agent_id,
                streaming,
                report = ?report,
                "post-turn persona evolution pass completed with errors"
            );
        }

        let deleted_rows = gm.run_background_memory_consolidation().await;
        tracing::debug!(
            agent_id = %agent_id,
            streaming,
            deleted_rows,
            "post-turn background memory consolidation completed"
        );

        if let Err(e) = crate::persona_evolution::PersonaEvolutionHook::evolve_from_turn(
            &gm,
            &agent_id,
            &turn_outcome,
        )
        .await
        {
            failed = true;
            tracing::warn!(
                agent_id = %agent_id,
                streaming,
                error = %e,
                "AINL persona turn evolution (AINL_PERSONA_EVOLUTION) failed; continuing"
            );
        }

        crate::agent_loop::graph_memory_refresh_armaraos_export_json(&agent_id).await;

        if failed {
            PERSONA_BG_FAILED.fetch_add(1, Ordering::Relaxed);
        } else {
            PERSONA_BG_SUCCEEDED.fetch_add(1, Ordering::Relaxed);
        }
    });
}
