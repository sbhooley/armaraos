//! GraphPatch reference adapter + patch adapter registry integration tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphStore, SqliteGraphStore};
use ainl_runtime::{
    AdapterRegistry, AinlRuntime, GraphPatchAdapter, GraphPatchHostDispatch, PatchAdapter,
    PatchDispatchContext, RuntimeConfig, TurnInput, TurnStatus,
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
fn test_registered_adapter_is_called_for_matching_label() {
    let (_d, store) = open_store();
    let ag = "t-reg-adapter";
    write_active_patch(&store, ag, "my_custom", &["ctx"]);

    struct Custom;
    impl PatchAdapter for Custom {
        fn name(&self) -> &str {
            "my_custom"
        }

        fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
            Ok(serde_json::json!({
                "ok": true,
                "label": ctx.patch_label,
            }))
        }
    }

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_adapter(Custom);

    let mut frame = HashMap::new();
    frame.insert("ctx".into(), serde_json::json!({}));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "my_custom")
        .expect("patch");
    assert!(r.dispatched);
    assert_eq!(r.adapter_name.as_deref(), Some("my_custom"));
    assert_eq!(
        r.adapter_output.as_ref().unwrap().get("ok"),
        Some(&serde_json::json!(true))
    );
}

#[test]
fn test_fallback_to_graph_patch_adapter_when_no_label_match() {
    let (_d, store) = open_store();
    let ag = "t-fallback";
    write_active_patch(&store, ag, "L_only_graph_patch", &["k"]);

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_default_patch_adapters();

    let mut frame = HashMap::new();
    frame.insert("k".into(), serde_json::json!(1));

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
        .find(|x| x.label == "L_only_graph_patch")
        .unwrap();
    assert!(r.dispatched);
    assert_eq!(r.adapter_name.as_deref(), Some(GraphPatchAdapter::NAME));
    let v = r.adapter_output.as_ref().unwrap();
    assert_eq!(
        v.get("label").and_then(|x| x.as_str()),
        Some("L_only_graph_patch")
    );
    assert_eq!(v.get("patch_version"), Some(&serde_json::json!(1)));
    assert!(v.get("frame_keys").unwrap().is_array());
}

#[test]
fn test_adapter_execution_failure_produces_metadata_dispatch() {
    let (_d, store) = open_store();
    let ag = "t-adapter-fail";
    write_active_patch(&store, ag, "boom", &["z"]);

    struct Boom;
    impl PatchAdapter for Boom {
        fn name(&self) -> &str {
            "boom"
        }

        fn execute_patch(&self, _: &PatchDispatchContext<'_>) -> Result<Value, String> {
            Err("boom failed".into())
        }
    }

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    rt.register_adapter(Boom);

    let mut frame = HashMap::new();
    frame.insert("z".into(), serde_json::json!(0));

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
    assert_eq!(r.adapter_name.as_deref(), Some("boom"));
    assert!(r.fitness_after > r.fitness_before);
}

#[test]
fn test_registered_names_returns_all_adapters() {
    struct A;
    impl PatchAdapter for A {
        fn name(&self) -> &str {
            "alpha"
        }
        fn execute_patch(&self, _: &PatchDispatchContext<'_>) -> Result<Value, String> {
            Ok(serde_json::json!({}))
        }
    }
    struct B;
    impl PatchAdapter for B {
        fn name(&self) -> &str {
            "bravo"
        }
        fn execute_patch(&self, _: &PatchDispatchContext<'_>) -> Result<Value, String> {
            Ok(serde_json::json!({}))
        }
    }

    let mut reg = AdapterRegistry::new();
    reg.register(A);
    reg.register(B);
    reg.register(GraphPatchAdapter::new());

    let names = reg.registered_names();
    assert_eq!(names.len(), 3);
    assert_eq!(names, vec!["alpha", "bravo", GraphPatchAdapter::NAME]);
}

#[test]
fn test_no_adapter_registered_dispatches_without_output() {
    let (_d, store) = open_store();
    let ag = "t-no-adapter";
    write_active_patch(&store, ag, "orphan_label", &["m"]);

    let mut rt = AinlRuntime::new(rt_cfg(ag), store);
    let mut frame = HashMap::new();
    frame.insert("m".into(), serde_json::json!("hello"));

    let out = rt
        .run_turn(TurnInput {
            user_message: "hi".into(),
            tools_invoked: vec![],
            frame,
            ..Default::default()
        })
        .unwrap();
    let r = out
        .result()
        .patch_dispatch_results
        .iter()
        .find(|x| x.label == "orphan_label")
        .unwrap();
    assert!(r.dispatched);
    assert!(r.adapter_output.is_none());
    assert!(r.adapter_name.is_none());
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
    let v = r.adapter_output.as_ref().expect("summary");
    assert_eq!(
        v.get("label").and_then(|k| k.as_str()),
        Some("L_custom_patch")
    );
    assert_eq!(v.get("patch_version"), Some(&serde_json::json!(1)));
    let keys = v
        .get("frame_keys")
        .and_then(|k| k.as_array())
        .expect("frame_keys");
    assert!(keys.iter().any(|k| k.as_str() == Some("ctx")));
}

#[test]
fn graph_patch_adapter_returns_structured_result() {
    let (_d, store) = open_store();
    let ag = "gp-struct";
    write_active_patch(&store, ag, "L_metrics", &["a"]);

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
    assert_eq!(v.get("label").and_then(|x| x.as_str()), Some("L_metrics"));
    assert_eq!(v.get("patch_version"), Some(&serde_json::json!(1)));
    let keys = v.get("frame_keys").unwrap().as_array().unwrap();
    assert!(keys.iter().any(|k| k.as_str() == Some("a")));
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
    let got = captured.lock().unwrap().take().expect("host saw summary");
    assert_eq!(got.get("label").and_then(|x| x.as_str()), Some("L_host"));
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

        fn execute_patch(&self, ctx: &PatchDispatchContext<'_>) -> Result<Value, String> {
            Ok(serde_json::json!({
                "from": "echo",
                "label": ctx.patch_label,
                "keys": ctx.frame.keys().cloned().collect::<Vec<_>>(),
            }))
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
