//! End-to-end: kernel agent loop → `NativeInferDriver` (HTTP) → `PlanExecutor` → orchestration traces.
//!
//! Wiremock stands in for `ainl-inference-server` (no real LLM). Graph memory must open, so
//! `ARMARAOS_HOME` is aligned with the test kernel temp home before the turn runs.

use openfang_api::routes::AppState;
use openfang_kernel::OpenFangKernel;
use openfang_types::config::{DefaultModelConfig, KernelConfig};
use serde_json::json;
use serial_test::serial;
use std::net::SocketAddr;
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct TestServer {
    base_url: String,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let k = self.state.kernel.clone();
        std::thread::spawn(move || k.shutdown());
    }
}

async fn spawn_test_server_with_kernel(kernel: Arc<OpenFangKernel>, tmp: tempfile::TempDir) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local_addr");
    let (app, state) = openfang_api::server::build_router(kernel, addr).await;
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("test server exited");
    });
    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

/// `ollama_base_url`: when set, must be the OpenAI-compat base including `/v1` (e.g. `{wiremock}/v1`), as
/// chat completions resolve to `{base}/chat/completions`. Needed when the legacy loop runs after native
/// infer; otherwise `base_url: None` targets real Ollama (`localhost:11434/v1`) and CI machines without
/// `test-model` fail.
async fn start_test_server_for_planner_e2e(ollama_base_url: Option<String>) -> TestServer {
    let tmp = tempfile::tempdir().expect("tempdir");
    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".into(),
            model: "test-model".into(),
            api_key_env: "OLLAMA_API_KEY".into(),
            base_url: ollama_base_url,
        },
        ..KernelConfig::default()
    };
    config.home_dir = tmp.path().to_path_buf();
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot kernel");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    spawn_test_server_with_kernel(kernel, tmp).await
}

fn graph_db_path(server: &TestServer, agent_id: &str) -> std::path::PathBuf {
    server
        .state
        .kernel
        .config
        .home_dir
        .join("agents")
        .join(agent_id)
        .join("ainl_memory.db")
}

fn seed_graph_memory(server: &TestServer, agent_id: &str) {
    let db = graph_db_path(server, agent_id);
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let gm = ainl_memory::GraphMemory::new(&db).expect("graph memory open");
    let episode = gm
        .write_episode(vec!["e2e".into()], None, None)
        .expect("episode");
    let _ = gm
        .write_fact("planner e2e seed".into(), 0.9, episode)
        .expect("fact");
}

const PLANNER_MANIFEST: &str = r#"
name = "planner-e2e-agent"
version = "0.1.0"
description = "Planner native infer e2e"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]

[metadata]
planner_mode = "on"
"#;

/// Minimal `InferResponse`-shaped JSON (matches `armara-provider-api` / native infer client parse).
fn infer_response_empty_plan_json() -> serde_json::Value {
    json!({
        "request_id": "00000000-0000-0000-0000-000000000000",
        "provider_trace_id": uuid::Uuid::new_v4().to_string(),
        "backend": { "kind": "llama_cpp", "model": "mock", "latency_ms": 0 },
        "output": {
            "text": "{}",
            "tool_calls": [],
            "structured": {
                "kind": "deterministic_plan",
                "version": 1,
                "plan": {
                    "steps": [],
                    "graph_writes": [],
                    "confidence": 0.99,
                    "reasoning_required_at": []
                }
            }
        },
        "validation": {
            "schema_ok": true,
            "tool_calls_ok": true,
            "repair_attempts": 0,
            "violations": [],
            "violation_details": []
        },
        "usage": { "input_tokens": 2, "output_tokens": 3 },
        "decision_log": []
    })
}

/// Response where `tool_calls_ok=false` — should cause agent_loop to fall back to legacy loop
/// and emit a `plan_fallback` orchestration trace rather than running `PlanExecutor`.
fn infer_response_validation_failed_json() -> serde_json::Value {
    json!({
        "request_id": "00000000-0000-0000-0000-000000000001",
        "provider_trace_id": uuid::Uuid::new_v4().to_string(),
        "backend": { "kind": "llama_cpp", "model": "mock", "latency_ms": 0 },
        "output": {
            "text": "{}",
            "tool_calls": [],
            "structured": {
                "kind": "deterministic_plan",
                "version": 1,
                "plan": {
                    "steps": [],
                    "graph_writes": [],
                    "confidence": 0.5,
                    "reasoning_required_at": []
                }
            }
        },
        "validation": {
            "schema_ok": true,
            "tool_calls_ok": false,
            "repair_attempts": 1,
            "violations": ["PLAN_UNDECLARED_TOOL"],
            "violation_details": [
                {
                    "code": "PLAN_UNDECLARED_TOOL",
                    "message": "tool 'dangerous_rm' not in agent tool_allowlist",
                    "step_id": "s1"
                }
            ]
        },
        "usage": { "input_tokens": 5, "output_tokens": 3 },
        "decision_log": []
    })
}

/// One-step `file_read` plan (workspace-relative path).
fn infer_response_file_read_plan_json() -> serde_json::Value {
    json!({
        "request_id": "00000000-0000-0000-0000-000000000000",
        "provider_trace_id": uuid::Uuid::new_v4().to_string(),
        "backend": { "kind": "llama_cpp", "model": "mock", "latency_ms": 0 },
        "output": {
            "text": "{}",
            "tool_calls": [],
            "structured": {
                "kind": "deterministic_plan",
                "version": 1,
                "plan": {
                    "steps": [{
                        "id": "read1",
                        "tool": "file_read",
                        "args": { "path": "hello.txt" },
                        "on_error": "abort"
                    }],
                    "graph_writes": [],
                    "confidence": 0.95,
                    "reasoning_required_at": []
                }
            }
        },
        "validation": {
            "schema_ok": true,
            "tool_calls_ok": true,
            "repair_attempts": 0,
            "violations": [],
            "violation_details": []
        },
        "usage": { "input_tokens": 2, "output_tokens": 3 },
        "decision_log": []
    })
}

#[tokio::test]
#[serial(armara_planner_native_infer_e2e)]
async fn daemon_infer_plan_executor_emits_plan_started_trace() {
    let mock = MockServer::start().await;
    let infer_json = infer_response_empty_plan_json();
    Mock::given(method("POST"))
        .and(path("/armara/v1/infer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(infer_json))
        .mount(&mock)
        .await;

    let prev_home = std::env::var("ARMARAOS_HOME").ok();
    let prev_infer = std::env::var("ARMARA_NATIVE_INFER_URL").ok();
    let prev_pm = std::env::var("ARMARA_PLANNER_MODE").ok();

    let server = start_test_server_for_planner_e2e(None).await;
    let home = server.state.kernel.config.home_dir.clone();
    std::env::set_var("ARMARAOS_HOME", home.as_os_str());
    std::env::set_var("ARMARA_NATIVE_INFER_URL", mock.uri());
    std::env::set_var("ARMARA_PLANNER_MODE", "1");

    let client = reqwest::Client::new();
    let create = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": PLANNER_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 201, "{}", create.text().await.unwrap());
    let created: serde_json::Value = create.json().await.unwrap();
    let agent_id = created["agent_id"].as_str().expect("agent_id");

    seed_graph_memory(&server, agent_id);

    let msg = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message":"hello planner e2e"}))
        .send()
        .await
        .unwrap();
    let status = msg.status();
    let text = msg.text().await.unwrap();
    assert_eq!(status, 200, "POST /message: {text}");

    let summaries: Vec<serde_json::Value> = client
        .get(format!(
            "{}/api/orchestration/traces?limit=30",
            server.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut saw_plan_started = false;
    for row in &summaries {
        let tid = match row["trace_id"].as_str() {
            Some(t) => t,
            None => continue,
        };
        let ev: Vec<serde_json::Value> = client
            .get(format!(
                "{}/api/orchestration/traces/{}",
                server.base_url, tid
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if ev.iter().any(|e| {
            e.get("event_type")
                .and_then(|et| et.get("type"))
                .and_then(|t| t.as_str())
                == Some("plan_started")
        }) {
            saw_plan_started = true;
            break;
        }
    }
    assert!(
        saw_plan_started,
        "expected at least one orchestration trace with plan_started (summaries={summaries:?})"
    );

    match prev_home {
        Some(h) => std::env::set_var("ARMARAOS_HOME", h),
        None => std::env::remove_var("ARMARAOS_HOME"),
    }
    match prev_infer {
        Some(u) => std::env::set_var("ARMARA_NATIVE_INFER_URL", u),
        None => std::env::remove_var("ARMARA_NATIVE_INFER_URL"),
    }
    match prev_pm {
        Some(p) => std::env::set_var("ARMARA_PLANNER_MODE", p),
        None => std::env::remove_var("ARMARA_PLANNER_MODE"),
    }
}

#[tokio::test]
#[serial(armara_planner_native_infer_e2e)]
async fn daemon_infer_plan_executor_runs_file_read_step_and_emits_plan_step_events() {
    let mock = MockServer::start().await;
    let infer_json = infer_response_file_read_plan_json();
    Mock::given(method("POST"))
        .and(path("/armara/v1/infer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(infer_json))
        .mount(&mock)
        .await;

    let prev_home = std::env::var("ARMARAOS_HOME").ok();
    let prev_infer = std::env::var("ARMARA_NATIVE_INFER_URL").ok();
    let prev_pm = std::env::var("ARMARA_PLANNER_MODE").ok();

    let server = start_test_server_for_planner_e2e(None).await;
    let home = server.state.kernel.config.home_dir.clone();
    std::env::set_var("ARMARAOS_HOME", home.as_os_str());
    std::env::set_var("ARMARA_NATIVE_INFER_URL", mock.uri());
    std::env::set_var("ARMARA_PLANNER_MODE", "1");

    let client = reqwest::Client::new();
    let create = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": PLANNER_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 201, "{}", create.text().await.unwrap());
    let created: serde_json::Value = create.json().await.unwrap();
    let agent_id = created["agent_id"].as_str().expect("agent_id");

    seed_graph_memory(&server, agent_id);

    let agents: Vec<serde_json::Value> = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace = agents
        .iter()
        .find(|a| a["id"].as_str() == Some(agent_id))
        .and_then(|a| a["workspace"].as_str())
        .expect("agent workspace from GET /api/agents");
    let hello = std::path::Path::new(workspace).join("hello.txt");
    if let Some(parent) = hello.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&hello, "e2e hello").unwrap();

    let msg = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message":"read hello.txt via planner"}))
        .send()
        .await
        .unwrap();
    let status = msg.status();
    let text = msg.text().await.unwrap();
    assert_eq!(status, 200, "POST /message: {text}");

    let summaries: Vec<serde_json::Value> = client
        .get(format!(
            "{}/api/orchestration/traces?limit=30",
            server.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut saw_step_started = false;
    let mut saw_step_completed = false;
    for row in &summaries {
        let tid = match row["trace_id"].as_str() {
            Some(t) => t,
            None => continue,
        };
        let ev: Vec<serde_json::Value> = client
            .get(format!(
                "{}/api/orchestration/traces/{}",
                server.base_url, tid
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        for e in &ev {
            let t = e
                .get("event_type")
                .and_then(|et| et.get("type"))
                .and_then(|t| t.as_str());
            if t == Some("plan_step_started") {
                saw_step_started = true;
            }
            if t == Some("plan_step_completed") {
                saw_step_completed = true;
            }
        }
        if saw_step_started && saw_step_completed {
            break;
        }
    }
    assert!(
        saw_step_started && saw_step_completed,
        "expected plan_step_started and plan_step_completed (summaries={summaries:?})"
    );

    match prev_home {
        Some(h) => std::env::set_var("ARMARAOS_HOME", h),
        None => std::env::remove_var("ARMARAOS_HOME"),
    }
    match prev_infer {
        Some(u) => std::env::set_var("ARMARA_NATIVE_INFER_URL", u),
        None => std::env::remove_var("ARMARA_NATIVE_INFER_URL"),
    }
    match prev_pm {
        Some(p) => std::env::set_var("ARMARA_PLANNER_MODE", p),
        None => std::env::remove_var("ARMARA_PLANNER_MODE"),
    }
}

// ---

/// When the infer server returns `validation.tool_calls_ok=false`, the kernel must:
///  1. NOT execute `PlanExecutor` — the turn still returns 200 (legacy loop takes over).
///  2. Emit a `plan_fallback` orchestration trace event with a `server_validation_failed` reason.
///
/// Wiremock mounts two routes:
///  • `POST /armara/v1/infer`      → validation-failed response (planner path, fired first)
///  • `POST /v1/chat/completions`  → plain completion (legacy fallback)
#[tokio::test]
#[serial(armara_planner_native_infer_e2e)]
async fn daemon_validation_failed_falls_back_to_legacy_and_emits_plan_fallback_trace() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/armara/v1/infer"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(infer_response_validation_failed_json()),
        )
        .expect(1)
        .mount(&mock)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "chatcmpl-fallback",
            "object": "chat.completion",
            "created": 0,
            "model": "test-model",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Fallback legacy reply." },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        })))
        .mount(&mock)
        .await;

    let prev_home = std::env::var("ARMARAOS_HOME").ok();
    let prev_infer = std::env::var("ARMARA_NATIVE_INFER_URL").ok();
    let prev_pm = std::env::var("ARMARA_PLANNER_MODE").ok();

    // OpenAI-compatible chat URL is `{base_url}/chat/completions`; ollama defaults use
    // `.../v1` as base (see `OLLAMA_BASE_URL`). Wiremock mounts `POST /v1/chat/completions`.
    let origin = mock.uri().as_str().trim_end_matches('/').to_string();
    let ollama_openai_base = format!("{origin}/v1");
    let server = start_test_server_for_planner_e2e(Some(ollama_openai_base)).await;
    let home = server.state.kernel.config.home_dir.clone();
    std::env::set_var("ARMARAOS_HOME", home.as_os_str());
    std::env::set_var("ARMARA_NATIVE_INFER_URL", &origin);
    std::env::set_var("ARMARA_PLANNER_MODE", "1");

    let client = reqwest::Client::new();
    let create = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": PLANNER_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(create.status(), 201, "{}", create.text().await.unwrap());
    let created: serde_json::Value = create.json().await.unwrap();
    let agent_id = created["agent_id"].as_str().expect("agent_id");

    seed_graph_memory(&server, agent_id);

    let msg = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message": "test validation fallback"}))
        .send()
        .await
        .unwrap();
    let status = msg.status();
    let body = msg.text().await.unwrap();
    assert_eq!(status, 200, "POST /message: {body}");

    let summaries: Vec<serde_json::Value> = client
        .get(format!(
            "{}/api/orchestration/traces?limit=30",
            server.base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut saw_plan_fallback = false;
    'outer: for row in &summaries {
        let tid = match row["trace_id"].as_str() {
            Some(t) => t,
            None => continue,
        };
        let ev: Vec<serde_json::Value> = client
            .get(format!(
                "{}/api/orchestration/traces/{}",
                server.base_url, tid
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        for e in &ev {
            if e.get("event_type")
                .and_then(|et| et.get("type"))
                .and_then(|t| t.as_str())
                == Some("plan_fallback")
            {
                saw_plan_fallback = true;
                break 'outer;
            }
        }
    }
    assert!(
        saw_plan_fallback,
        "expected plan_fallback trace when tool_calls_ok=false (summaries={summaries:?})"
    );

    match prev_home {
        Some(h) => std::env::set_var("ARMARAOS_HOME", h),
        None => std::env::remove_var("ARMARAOS_HOME"),
    }
    match prev_infer {
        Some(u) => std::env::set_var("ARMARA_NATIVE_INFER_URL", u),
        None => std::env::remove_var("ARMARA_NATIVE_INFER_URL"),
    }
    match prev_pm {
        Some(p) => std::env::set_var("ARMARA_PLANNER_MODE", p),
        None => std::env::remove_var("ARMARA_PLANNER_MODE"),
    }
}
