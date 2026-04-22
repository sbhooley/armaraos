//! HTTP/WebSocket API server for the OpenFang Agent OS daemon.
//!
//! Exposes agent management, status, and chat via JSON REST endpoints.
//! The kernel runs in-process; the CLI connects over HTTP.

pub mod channel_bridge;
pub mod daemon_resources;
pub mod graph_memory;
pub mod middleware;
pub mod network_hints;
pub mod openai_compat;
pub mod rate_limiter;
pub mod routes;
pub mod server;
pub mod session_auth;
pub mod stream_chunker;
pub mod stream_dedup;
pub mod types;
pub mod webchat;
pub mod ws;

/// Phase 8 (SELF_LEARNING_INTEGRATION_MAP §8 / §16): keep operator JS aligned with
/// `openfang_types::event::SystemEvent` names surfaced over `armaraos-kernel-event`.
#[cfg(test)]
mod dashboard_learning_panels_js_guard {
    fn assert_contains(haystack: &str, needle: &str, ctx: &str) {
        assert!(
            haystack.contains(needle),
            "{ctx}: expected `{needle}` in bundled operator script"
        );
    }

    #[test]
    fn trajectories_page_js_covers_trajectory_and_failure_events() {
        let s = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/pages/trajectories.js"
        ));
        for needle in [
            "TrajectoryRecorded",
            "GraphMemoryWrite",
            "FailureLearned",
            "/api/graph-memory/failures/recent",
            "ImprovementProposalAdopted",
        ] {
            assert_contains(&s, needle, "trajectories.js");
        }
    }

    #[test]
    fn graph_failures_page_js_subscribes_to_failure_surface() {
        let s = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/pages/graph-failures.js"
        ));
        for needle in ["FailureLearned", "GraphMemoryWrite", "/api/graph-memory/failures/recent"]
        {
            assert_contains(&s, needle, "graph-failures.js");
        }
    }

    #[test]
    fn graph_proposals_page_js_subscribes_to_proposal_surface() {
        let s = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/static/js/pages/graph-proposals.js"
        ));
        for needle in ["ImprovementProposalAdopted", "improvement_proposal", "/improvement-proposals"]
        {
            assert_contains(&s, needle, "graph-proposals.js");
        }
    }
}
