//! GraphPatch reference adapter + fallback dispatch integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AinlRuntime, GraphPatchAdapter, GraphPatchHostDispatch, PatchAdapter, PatchDispatchContext,
    RuntimeConfig, TurnInput, TurnStatus,
};
use serde_json::Value;
use uuid::Uuid;

fn open_store() -> (tempfile::TempDir, SqliteGraphStore) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("gp_engine.db");
    let _ = std::fs::remove_file(&db);
    let store = SqliteGraphStore::open(&db).unwrap();
    (dir, store)
}

fn rt_cfg(agent_id: &str) -> RuntimeConfig {
    RuntimeConfig {
        agent_id: agent_id.to_string(),
        extraction_interval: 0,
        max_steps: 100,
        ..Default::default()
    }
}

fn write_active_patch(store: &SqliteGraphStore, ag: &str, label: &str, reads: &[&str]) -> Uuid {
    let mut proc = AinlMemoryNode::new_pattern(format!("pat_{label}"), vec![1, 2, 3]);
    proc.agent_id = ag.into();
    if let AinlNodeType::Procedural { ref mut procedural } = proc.node_type {
        procedural.label = label.into();
        procedural.patch_version = 1;
        procedural.declared_reads = reads.iter().map(|s| (*s).to_string()).collect();
        procedural.fitness = Some(0.55);
    }
    let id = proc.id;
    store.write_node(&proc).unwrap();
    id
}

#[test]
fn graph_patch_adapter_matches_active_patch() {
    let (_d, store) = open_store();
    let ag = "gp-match";
    write_active_patch(&store, ag, "L_custom_patch", &["ctx"]);

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_default_patch_adapters();

    let mut frame = HashMap::new();
    frame.insert("ctx".into(), serde_json::json!({"x": 1}));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    assert!(out.is_complete());
    assert_eq!(out.turn_status(), TurnStatus::Ok);
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "L_custom_patch")
        .expect("patch result");
    assert!(r.dispatched);
    assert_eq!(r.adapter_name.as_deref(), Some(GraphPatchAdapter::NAME));
    let v = r.adapter_output.as_ref().expect("envelope");
    assert_eq!(
        v.get("kind").and_then(|k| k.as_str()),
        Some("graph_patch_dispatch")
    );
    assert_eq!(
        v.get("patch_label").and_then(|k| k.as_str()),
        Some("L_custom_patch")
    );
}

#[test]
fn graph_patch_adapter_returns_structured_result() {
    let (_d, store) = open_store();
    let ag = "gp-struct";
    let nid = write_active_patch(&store, ag, "L_metrics", &["a"]);

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_default_patch_adapters();

    let mut frame = HashMap::new();
    frame.insert("a".into(), serde_json::json!(true));

    let out = rt
        .run_turn(TurnInput {
            user_message: "m".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "L_metrics")
        .unwrap();
    assert!(r.dispatched);
    assert!(r.skip_reason.is_none());
    assert!(r.fitness_after > r.fitness_before);
    let v = r.adapter_output.as_ref().unwrap();
    let nid_s = nid.to_string();
    assert_eq!(
        v.get("patch_node_id").and_then(|x| x.as_str()),
        Some(nid_s.as_str())
    );
    assert_eq!(v.get("patch_version"), Some(&serde_json::json!(1)));
    assert!(v.get("declared_reads").unwrap().is_array());
}

#[test]
fn graph_patch_adapter_host_callback_receives_envelope() {
    let (_d, store) = open_store();
    let ag = "gp-host";
    write_active_patch(&store, ag, "L_host", &["z"]);

    let captured = Arc::new(Mutex::new(None::<Value>));
    let cap2 = Arc::clone(&captured);

    struct Hook(Arc<Mutex<Option<Value>>>);
    impl GraphPatchHostDispatch for Hook {
        fn on_patch_dispatch(&self, envelope: Value) -> Result<Value, String> {
            *self.0.lock().unwrap() = Some(envelope.clone());
            Ok(envelope)
        }
    }

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_adapter(GraphPatchAdapter::with_host(Arc::new(Hook(cap2))));

    let mut frame = HashMap::new();
    frame.insert("z".into(), serde_json::json!("ok"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "h".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    assert!(out.result().patch_dispatch_results[0].dispatched);
    let got = captured.lock().unwrap().take().expect("host saw envelope");
    assert_eq!(
        got.get("patch_label").and_then(|x| x.as_str()),
        Some("L_host")
    );
}

#[test]
fn graph_patch_adapter_host_error_nonfatal() {
    let (_d, store) = open_store();
    let ag = "gp-err";
    write_active_patch(&store, ag, "L_err", &["k"]);

    struct Reject;
    impl GraphPatchHostDispatch for Reject {
        fn on_patch_dispatch(&self, _: Value) -> Result<Value, String> {
            Err("intentional host reject".into())
        }
    }

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_adapter(GraphPatchAdapter::with_host(Arc::new(Reject)));

    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));

    let out = rt
        .run_turn(TurnInput {
            user_message: "e".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = &out.result().patch_dispatch_results[0];
    assert!(r.dispatched);
    assert!(r.adapter_output.is_none());
    assert_eq!(r.adapter_name.as_deref(), Some(GraphPatchAdapter::NAME));
    assert!(r.fitness_after > r.fitness_before);
}

#[test]
fn runtime_with_default_patch_adapters_registers_graph_patch() {
    let (_d, store) = open_store();
    let mut rt = AinlRuntime::new(rt_cfg("gp-reg"), store);
    assert!(rt.registered_adapters().is_empty());
    rt.register_default_patch_adapters();
    let names = rt.registered_adapters();
    assert_eq!(names.len(), 1);
    assert!(names.contains(&GraphPatchAdapter::NAME));
}

#[test]
fn graph_patch_legacy_execute_returns_err_documentation() {
    let a = GraphPatchAdapter::new();
    let r = a.execute("any", &HashMap::new());
    assert!(r.is_err());
}

#[test]
fn graph_patch_execute_patch_nonprocedural_errors_cleanly() {
    let mut ep = AinlMemoryNode::new_episode(Uuid::new_v4(), 0, vec![], None, None);
    ep.agent_id = "ag".into();
    let frame = HashMap::new();
    let ctx = PatchDispatchContext {
        patch_label: "L_bad_ctx",
        node: &ep,
        frame: &frame,
    };
    let a = GraphPatchAdapter::new();
    let r = PatchAdapter::execute_patch(&a, &ctx);
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("not procedural"));
}

#[test]
fn label_specific_adapter_beats_graph_patch_fallback() {
    let (_d, store) = open_store();
    let ag = "gp-priority";
    write_active_patch(&store, ag, "echo", &["m"]);

    struct Echo;
    impl PatchAdapter for Echo {
        fn name(&self) -> &str {
            "echo"
        }

        fn execute(
            &self,
            label: &str,
            frame: &HashMap<String, serde_json::Value>,
        ) -> Result<Value, String> {
            Ok(
                serde_json::json!({"from": "echo", "label": label, "keys": frame.keys().cloned().collect::<Vec<_>>()}),
            )
        }
    }

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_adapter(Echo);
    rt.register_default_patch_adapters();

    let mut frame = HashMap::new();
    frame.insert("m".into(), serde_json::json!(0));

    let out = rt
        .run_turn(TurnInput {
            user_message: "x".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "echo")
        .unwrap();
    assert_eq!(r.adapter_name.as_deref(), Some("echo"));
    assert_eq!(
        r.adapter_output
            .as_ref()
            .unwrap()
            .get("from")
            .and_then(|x| x.as_str()),
        Some("echo")
    );
}
