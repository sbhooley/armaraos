//! Tests for [`crate::ainl_graph_extractor_bridge`] + graph memory writes.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn extract_facts_writes_semantic_row_via_graph_memory_hook() {
    use ainl_memory::GraphMemory;
    use serde_json::json;

    use crate::ainl_graph_extractor_bridge::{format_turn_payload, AinlExtractorBridge};
    use crate::graph_memory_writer::GraphMemoryWriter;

    let fact_writes = Arc::new(AtomicUsize::new(0));
    let fw = fact_writes.clone();
    let tmp = tempfile::tempdir().unwrap();
    let db_path = tmp.path().join("bridge_semantic.db");
    let memory = GraphMemory::new(&db_path).expect("open graph memory");
    let writer = GraphMemoryWriter::from_memory_for_tests(
        memory,
        "bridge-sem-agent",
        Some(Arc::new(move |_agent, kind, _prov| {
            if kind == "fact" {
                fw.fetch_add(1, Ordering::SeqCst);
            }
        })),
    );

    let episode_id = writer
        .record_turn(
            vec!["shell_exec".into()],
            None,
            Some(json!({ "outcome": "success" })),
            &[],
            None,
            None,
            None,
            None,
        )
        .await
        .expect("episode id");

    let turn = format_turn_payload(
        "Help me fix my rust project: cargo errors and serde derives",
        "We can run clippy and check the borrow checker.",
        &["shell_exec".to_string()],
    );
    let facts = AinlExtractorBridge::extract_facts(&turn, "bridge-sem-agent");
    assert!(
        !facts.is_empty(),
        "expected ainl-graph-extractor tags for this turn, got {facts:?}"
    );

    for (text, confidence) in facts {
        writer
            .record_fact_with_tags(text, confidence, episode_id, &[], None)
            .await;
    }

    assert!(
        fact_writes.load(Ordering::SeqCst) >= 1,
        "expected at least one semantic (fact) write hook"
    );
}
