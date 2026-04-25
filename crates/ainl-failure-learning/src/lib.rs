//! Portable **failure learning** surface: search persisted [`ainl_memory::AinlMemoryNode`]
//! `Failure` rows and format short **prevention** blocks for the LLM.
//!
//! Ingestion stays in hosts (`openfang-runtime` / `ainl-runtime`) that already write
//! `Failure` nodes. This crate only *reads* via [`GraphMemory::search_failures_for_agent`].

#![forbid(unsafe_code)]

pub mod gate;
pub mod ingest;
pub mod procedure_patch;
pub mod search;
pub mod suggest;

pub use gate::should_emit_failure_suggestion;
pub use procedure_patch::{
    failure_patch_candidates, ProcedureFailureEvidence, ProcedurePatchPolicy,
};
pub use search::{search_failures_for_agent, FailureRecallHit};
pub use suggest::format_failure_prevention_block;
