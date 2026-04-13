//! Convenience entry for the agent loop.

use crate::extractor::{ExtractionReport, GraphExtractorTask};
use ainl_memory::SqliteGraphStore;

pub fn run_extraction_pass(
    store: &SqliteGraphStore,
    agent_id: &str,
) -> Result<ExtractionReport, String> {
    GraphExtractorTask::new(agent_id).run_pass(store)
}
