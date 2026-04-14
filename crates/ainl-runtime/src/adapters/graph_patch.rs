//! Reference [`GraphPatchAdapter`] — built-in fallback for procedural patches (`"graph_patch"`).
//!
//! Returns a small JSON summary for hosts; optional [`GraphPatchHostDispatch`] forwards that value.

use std::sync::Arc;

use serde_json::{json, Value};

use super::PatchAdapter;
use crate::engine::PatchDispatchContext;
use ainl_memory::AinlNodeType;

/// Optional host hook: receives the same JSON summary as [`GraphPatchAdapter::execute_patch`].
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

fn build_summary(ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
    let proc = match &ctx.node.node_type {
        AinlNodeType::Procedural { procedural } => procedural,
        _ => {
            return Err("graph_patch: PatchDispatchContext.node is not procedural".to_string());
        }
    };
    for key in &proc.declared_reads {
        if !ctx.frame.contains_key(key) {
            return Err(format!(
                "graph_patch: declared read {key:?} missing from frame (adapter safety check)"
            ));
        }
    }
    let mut frame_keys: Vec<String> = ctx.frame.keys().cloned().collect();
    frame_keys.sort_unstable();
    Ok(json!({
        "label": ctx.patch_label,
        "patch_version": proc.patch_version,
        "frame_keys": frame_keys,
    }))
}

impl PatchAdapter for GraphPatchAdapter {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
        let summary = build_summary(ctx)?;
        if let Some(h) = &self.host {
            h.on_patch_dispatch(summary)
        } else {
            Ok(summary)
        }
    }
}
