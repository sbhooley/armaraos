//! Optional **compressed** host hint for `mcp_ainl_ainl_run` adapter registration.
//!
//! **Primary source of truth** for adapter verbs in production is live
//! `mcp_ainl_ainl_capabilities` (and the graph-memory snapshot ArmaraOS persists). Use this
//! **static** string only as a **fallback** when no capabilities digest is available (e.g. cold
//! start before the first MCP call).
//!
//! Hosts may inject [`Segment::tool_definitions`] or [`Segment::memory_block`] from
//! [`mcp_ainl_run_adapters_cheatsheet_segment`] near other tool-definition bytes to save
//! full JSON-schema tokens on the LLM path.

use crate::segment::Segment;

/// Short, model-oriented summary of the `adapters` object for MCP `ainl_run` (ArmaraOS default).
pub const MCP_AINL_RUN_ADAPTERS_CHEATSHEET: &str = r#"mcp_ainl_ainl_run — adapters (host registration, not inline AINL dicts on R lines)
- Required when the graph uses these adapters: pass `adapters` in the tool JSON.
- enable: ["http","fs","cache","sqlite",...] — list every adapter the IR references.
- http: { "allow_hosts": ["example.com"], "timeout_s": 15, "payment_profile": "none"|"auto"|"x402"|"mpp", "max_payment_rounds": 2 }
- fs: { "root": "/abs/workspace", "allow_extensions": [".json",".csv"] }
- cache: { "path": "/abs/workspace/cache.json" }
- sqlite: { "db_path": "/abs/db.sqlite" }
- Do not put `{"k":v}` inline on `R` lines — build dicts in `frame` and reference by variable name.
- Compiler-only success is not runnable proof: align `adapters` with `required_adapters` / `runtime_readiness` from validate/compile; use `mcp_ainl_ainl_get_started` plus `mcp_resource_read` on `ainl://strict-authoring-cheatsheet` / `strict-valid-examples` / `adapter-contracts` when authoring unfamiliar graphs."#;

/// [`Segment::tool_definitions`] with [`MCP_AINL_RUN_ADAPTERS_CHEATSHEET`].
#[must_use]
pub fn mcp_ainl_run_adapters_cheatsheet_segment() -> Segment {
    Segment::tool_definitions(MCP_AINL_RUN_ADAPTERS_CHEATSHEET)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_is_tool_definitions() {
        let s = mcp_ainl_run_adapters_cheatsheet_segment();
        assert_eq!(s.kind, crate::SegmentKind::ToolDefinitions);
        assert!(s.content.contains("allow_hosts"));
        assert!(s.content.contains("strict-authoring-cheatsheet"));
    }
}
