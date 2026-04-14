//! Reference [`GraphPatchAdapter`] — structured GraphPatch dispatch without executing AINL IR in Rust.
//!
//! ## What this is (and is not)
//!
//! - **Is:** normalizes active procedural patch rows (same shape as Python `memory.patch` / graph
//!   store patch records) into a JSON **dispatch envelope** for a host, or returns that envelope
//!   directly when no host hook is installed.
//! - **Is not:** an AINL compiler, an IR interpreter, or parity with Python `RuntimeEngine` GraphPatch.
//!
//! ## Payload shape
//!
//! The envelope includes `patch_label` (procedural `label` or `pattern_name`), `patch_node_id`,
//! `patch_version`, `declared_reads`, `compiled_graph_byte_len`, optional UTF-8 preview of
//! `compiled_graph` when valid UTF-8, `tool_sequence`, `trace_id`, and `frame_keys`. Hosts should
//! treat unknown fields as forward-compatible.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use super::PatchAdapter;
use crate::engine::PatchDispatchContext;
use ainl_memory::AinlNodeType;

/// Optional host hook: receives the normalized GraphPatch envelope (see module docs).
pub trait GraphPatchHostDispatch: Send + Sync {
    fn on_patch_dispatch(&self, envelope: Value) -> Result<Value, String>;
}

/// Reference adapter registered as [`Self::NAME`]. Used as a **fallback** when no label-specific
/// [`PatchAdapter`] is registered for the procedural patch label.
pub struct GraphPatchAdapter {
    host: Option<Arc<dyn GraphPatchHostDispatch>>,
}

impl GraphPatchAdapter {
    pub const NAME: &'static str = "graph_patch";

    pub fn new() -> Self {
        Self { host: None }
    }

    pub fn with_host(host: Arc<dyn GraphPatchHostDispatch>) -> Self {
        Self { host: Some(host) }
    }
}

impl Default for GraphPatchAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn build_envelope(ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
    let proc = match &ctx.node.node_type {
        AinlNodeType::Procedural { procedural } => procedural,
        _ => {
            return Err("graph_patch: PatchDispatchContext.node is not procedural".to_string());
        }
    };
    let utf8_preview = std::str::from_utf8(&proc.compiled_graph)
        .ok()
        .map(|s| s.chars().take(256).collect::<String>());
    Ok(json!({
        "kind": "graph_patch_dispatch",
        "patch_label": ctx.patch_label,
        "patch_node_id": ctx.node.id.to_string(),
        "pattern_name": proc.pattern_name,
        "patch_version": proc.patch_version,
        "procedure_type": format!("{:?}", proc.procedure_type),
        "declared_reads": proc.declared_reads,
        "compiled_graph_byte_len": proc.compiled_graph.len(),
        "compiled_graph_utf8_preview": utf8_preview,
        "tool_sequence": proc.tool_sequence,
        "trace_id": proc.trace_id,
        "frame_keys": ctx.frame.keys().cloned().collect::<Vec<String>>(),
    }))
}

impl PatchAdapter for GraphPatchAdapter {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn execute(
        &self,
        _label: &str,
        _frame: &HashMap<String, serde_json::Value>,
    ) -> Result<Value, String> {
        Err(
            "graph_patch: dispatch uses execute_patch with PatchDispatchContext; register via \
             AinlRuntime::register_adapter(GraphPatchAdapter::new()) or register_default_patch_adapters"
                .to_string(),
        )
    }

    fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
        let envelope = build_envelope(ctx)?;
        if let Some(h) = &self.host {
            h.on_patch_dispatch(envelope)
        } else {
            Ok(envelope)
        }
    }
}
