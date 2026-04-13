//! Convenience entry for the agent loop.

use crate::extractor::{ExtractionReport, GraphExtractorTask};
use ainl_memory::SqliteGraphStore;

/// Convenience wrapper for one-off extraction. Creates a fresh
/// [`GraphExtractorTask`] with a new [`crate::PersonaSignalExtractorState`] on each
/// call, so streak-based detectors (brevity, formality) cannot fire across
/// invocations. For long-running agent loops, instantiate
/// [`GraphExtractorTask`] directly and call [`GraphExtractorTask::run_pass`]
/// to preserve streak state between passes.
pub fn run_extraction_pass(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<ExtractionReport, String> {
    GraphExtractorTask::new(agent_id).run_pass(store)
}
