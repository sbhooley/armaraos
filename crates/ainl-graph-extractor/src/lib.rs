//! Graph extractor task: bump semantic `recurrence_count` from retrieval deltas, then run persona evolution.
//!
//! **Alpha:** API may change before 1.0.
//!
//! Persona evolution rows use [`ainl_persona::EVOLUTION_TRAIT_NAME`] — import that constant from `ainl-persona`
//! when matching evolution bundles; do not duplicate the string.
//!
//! **Note:** [`GraphExtractorTask::evolution_engine`] is the canonical in-process handle to
//! [`ainl_persona::EvolutionEngine`]. **ainl-runtime** exposes the same engine for direct
//! `ingest_signals` / `correction_tick` / `evolve` calls; this crate’s `run_pass` is one signal path, not the only one.
//!
//! ## `run_pass` and [`ExtractionReport`]
//!
//! [`GraphExtractorTask::run_pass`] returns an [`ExtractionReport`] (not `Result<_, _>`): semantic /
//! pattern / persona phases record errors in **`extract_error`**, **`pattern_error`**, and
//! **`persona_error`** respectively. Use [`ExtractionReport::has_errors`] before treating a pass as
//! clean. ArmaraOS **`GraphMemoryWriter::run_persona_evolution_pass`** surfaces the same struct
//! and logs warnings for populated error slots.

mod extractor;
mod persona_signals;
mod recurrence;
mod runner;
mod turn_extract;

pub use ainl_persona::{AXIS_EVOLUTION_SNAPSHOT, EVOLUTION_TRAIT_NAME};
pub use extractor::{ExtractionReport, GraphExtractorTask};
pub use persona_signals::{
    extract_pass, extract_pass_collect, flush_episode_pattern_tags, ExtractPassCollected,
    PersonaSignalExtractorState,
};
pub use recurrence::update_semantic_recurrence;
pub use runner::run_extraction_pass;
pub use ainl_semantic_tagger::{tag_tool_names, SemanticTag, TagNamespace};
pub use turn_extract::extract_turn_semantic_tags_for_memory;
