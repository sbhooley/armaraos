//! Graph extractor task: bump semantic `recurrence_count` from retrieval deltas, then run persona evolution.
//!
//! **Alpha:** API may change before 1.0.
//!
//! Persona evolution rows use [`ainl_persona::EVOLUTION_TRAIT_NAME`] — import that constant from `ainl-persona`
//! when matching evolution bundles; do not duplicate the string.

mod extractor;
mod persona_signals;
mod recurrence;
mod runner;

pub use ainl_persona::{AXIS_EVOLUTION_SNAPSHOT, EVOLUTION_TRAIT_NAME};
pub use extractor::{ExtractionReport, GraphExtractorTask};
pub use persona_signals::{extract_pass, PersonaSignalExtractorState};
pub use recurrence::update_semantic_recurrence;
pub use runner::run_extraction_pass;
