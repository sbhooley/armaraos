//! Portable **failure learning** surface: search persisted [`ainl_memory::AinlMemoryNode`]
//! `Failure` rows and format short **prevention** blocks for the LLM.
//!
//! Ingestion stays in hosts (`openfang-runtime` / `ainl-runtime`) that already write
//! `Failure` nodes. This crate only *reads* via [`GraphMemory::search_failures_for_agent`].

#![forbid(unsafe_code)]

pub mod ingest;
pub mod search;
pub mod suggest;

pub use search::{search_failures_for_agent, FailureRecallHit};
pub use suggest::format_failure_prevention_block;
