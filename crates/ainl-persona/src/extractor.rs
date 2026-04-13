//! Pull raw persona signals from `SqliteGraphStore` for one agent.

use crate::signals::{extract_from_node, RawSignal};
use ainl_memory::{GraphStore, SqliteGraphStore};

const NODE_TYPES: [&str; 4] = ["episode", "semantic", "procedural", "persona"];

pub struct GraphExtractor;

impl GraphExtractor {
    pub fn extract(store: &SqliteGraphStore, agent_id: &str) -> Result<Vec<RawSignal>, String> {
        let mut raw = Vec::new();
        for nt in NODE_TYPES {
            let batch = store.find_by_type(nt)?;
            for node in batch {
                if node.agent_id != agent_id {
                    continue;
                }
                raw.extend(extract_from_node(&node));
            }
        }
        Ok(raw)
    }
}
