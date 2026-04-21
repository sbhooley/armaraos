//! Core kernel for the OpenFang Agent Operating System.
//!
//! The kernel manages agent lifecycles, memory, permissions, scheduling,
//! and inter-agent communication.

pub mod ainl_intelligence_overlays;
pub mod ainl_library;
pub mod approval;
pub mod auth;
pub mod auto_reply;
pub mod background;
pub mod capabilities;
pub mod config;
mod config_toml_repair;
pub mod config_reload;
pub mod cron;
pub mod embedded_ainl_programs;
pub mod error;
pub mod event_bus;
pub mod heartbeat;
mod internal_automation_probe;
pub mod kernel;
pub mod metering;
pub mod openclaw_workspace;
pub mod orchestration_trace;
pub mod pairing;
pub mod registry;
pub mod scheduler;
pub mod skills_staging;
pub mod supervisor;
pub mod triggers;
pub mod whatsapp_gateway;
pub mod wizard;
pub mod workflow;

pub use kernel::DeliveryTracker;
pub use kernel::OpenFangKernel;
