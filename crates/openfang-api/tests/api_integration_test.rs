//! Real HTTP integration tests for the OpenFang API.
//!
//! These tests boot a real kernel, serve the **production** app from
//! [`openfang_api::server::build_router`] on a random port, and hit endpoints
//! with `reqwest` (no hand-maintained route tables).
//!
//! Tests that require an LLM API call are gated behind GROQ_API_KEY.
//!
//! Run: cargo test -p openfang-api --test api_integration_test -- --nocapture

use openfang_api::routes::AppState;
use openfang_kernel::OpenFangKernel;
use openfang_runtime::kernel_handle::KernelHandle;
use openfang_types::config::{DefaultModelConfig, KernelConfig};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

struct TestServer {
    base_url: String,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.state.kernel.shutdown();
    }
}

/// Serve the **production** Axum app (`server::build_router`) on a random port.
async fn spawn_test_server_with_kernel(
    kernel: Arc<OpenFangKernel>,
    tmp: tempfile::TempDir,
) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind test server");
    let addr = listener.local_addr().expect("local_addr");

    let (app, state) = openfang_api::server::build_router(kernel, addr).await;

    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("test server exited with error");
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

/// Start a test server using ollama as default provider (no API key needed).
/// This lets the kernel boot without any real LLM credentials.
/// Tests that need actual LLM calls should use `start_test_server_with_llm()`.
async fn start_test_server() -> TestServer {
    start_test_server_with_provider("ollama", "test-model", "OLLAMA_API_KEY").await
}

/// Start a test server with Groq as the LLM provider (requires GROQ_API_KEY).
async fn start_test_server_with_llm() -> TestServer {
    start_test_server_with_provider("groq", "llama-3.3-70b-versatile", "GROQ_API_KEY").await
}

async fn start_test_server_with_provider(
    provider: &str,
    model: &str,
    api_key_env: &str,
) -> TestServer {
    start_test_server_with_provider_patch(provider, model, api_key_env, |_| {}).await
}

async fn start_test_server_with_provider_patch(
    provider: &str,
    model: &str,
    api_key_env: &str,
    patch: impl FnOnce(&mut KernelConfig),
) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: provider.to_string(),
            model: model.to_string(),
            api_key_env: api_key_env.to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    patch(&mut config);

    let kernel = OpenFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    spawn_test_server_with_kernel(kernel, tmp).await
}

/// Manifest that uses ollama (no API key required, won't make real LLM calls).
const TEST_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

/// Same as `TEST_MANIFEST` but with a distinct description for `PUT /update` tests.
const TEST_MANIFEST_PUT_UPDATE: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Updated via PUT /update integration test"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

fn graph_db_path_for(server: &TestServer, agent_id: &str) -> std::path::PathBuf {
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
    let db = graph_db_path_for(server, agent_id);
    if let Some(parent) = db.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let gm = ainl_memory::GraphMemory::new(&db).expect("graph memory open");
    let episode = gm
        .write_episode(vec!["file_read".to_string()], None, None)
        .expect("write episode");
    let _ = gm
        .write_fact("user prefers concise answers".to_string(), 0.92, episode)
        .expect("write fact");
}

/// Manifest that does not match the spawned agent name (must be rejected by PUT /update).
const WRONG_NAME_MANIFEST: &str = r#"
name = "wrong-agent-name"
version = "0.1.0"
description = "wrong"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "x"

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

/// Manifest that uses Groq for real LLM tests.
const LLM_MANIFEST: &str = r#"
name = "test-agent"
version = "0.1.0"
description = "Integration test agent"
author = "test"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = "You are a test agent. Reply concisely."

[capabilities]
tools = ["file_read"]
memory_read = ["*"]
memory_write = ["self.*"]
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_network_hints_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/system/network-hints", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("likely_vpn").is_some());
    assert!(body.get("confidence").is_some());
    assert!(body["tunnel_interface_names"].is_array());
    assert!(body["interface_names"].is_array());
    assert!(body["proxy_env"].is_object());
    assert!(body["notes"].is_array());
}

#[tokio::test]
async fn test_health_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Middleware injects x-request-id
    assert!(resp.headers().contains_key("x-request-id"));

    let body: serde_json::Value = resp.json().await.unwrap();
    // Public health endpoint returns minimal info (redacted for security)
    assert_eq!(body["status"], "ok");
    assert!(body["version"].is_string());
    // Detailed fields should NOT appear in public health endpoint
    assert!(body["database"].is_null());
    assert!(body["agent_count"].is_null());
}

/// Dashboard “daemon vs GitHub” uses this route (server-side GitHub fetch).
#[tokio::test]
async fn test_github_latest_release_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/version/github-latest", server.base_url))
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
        .expect("GET github-latest");
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.expect("json body");
    assert!(
        status.is_success(),
        "expected 2xx, got {status} body {body:?}"
    );
    let tag = body["tag_name"].as_str().unwrap_or("");
    assert!(!tag.is_empty(), "tag_name missing: {body:?}");
    let url = body["html_url"].as_str().unwrap_or("");
    assert!(
        url.contains("github.com"),
        "html_url should be a GitHub URL: {body:?}"
    );
}

/// GET /api/ainl/runtime-version — host toolchain + optional PyPI latest (dashboard bell).
#[tokio::test]
async fn test_get_ainl_runtime_version_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/ainl/runtime-version", server.base_url))
        .timeout(std::time::Duration::from_secs(45))
        .send()
        .await
        .expect("GET ainl runtime-version");
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.expect("json body");
    assert!(
        v.get("ainl_cli_line").is_some(),
        "expected ainl_cli_line: {v}"
    );
    assert!(v.get("pip_excerpt").is_some(), "expected pip_excerpt: {v}");
    assert!(
        v.get("pypi_latest_version").is_some() || v.get("pypi_error").is_some(),
        "expected pypi_latest_version or pypi_error: {v}"
    );
}

#[tokio::test]
async fn armaraos_home_browser_lists_and_reads_under_home() {
    let server = start_test_server().await;
    let home = server.state.kernel.config.home_dir.clone();
    std::fs::write(home.join("z_browser_test.txt"), b"hello-armaraos-home").unwrap();
    std::fs::create_dir_all(home.join("z_browser_sub")).unwrap();

    let client = reqwest::Client::new();

    let list_root = client
        .get(format!("{}/api/armaraos-home/list", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(list_root.status(), 200);
    let j: serde_json::Value = list_root.json().await.unwrap();
    let names: Vec<&str> = j["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["name"].as_str())
        .collect();
    assert!(names.contains(&"z_browser_test.txt"));
    assert!(names.contains(&"z_browser_sub"));

    let read = client
        .get(format!(
            "{}/api/armaraos-home/read?path=z_browser_test.txt",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let body: serde_json::Value = read.json().await.unwrap();
    assert_eq!(body["encoding"], "utf8");
    assert_eq!(body["content"], "hello-armaraos-home");

    let trav = client
        .get(format!(
            "{}/api/armaraos-home/list?path=../../../etc",
            server.base_url,
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(trav.status(), 400);
}

#[tokio::test]
async fn armaraos_home_write_allowlist_and_blocks() {
    let server =
        start_test_server_with_provider_patch("ollama", "test-model", "OLLAMA_API_KEY", |c| {
            c.dashboard.home_editable_globs = vec!["z_notes/**".to_string()];
        })
        .await;
    let home = server.state.kernel.config.home_dir.clone();
    std::fs::create_dir_all(home.join("z_notes")).unwrap();
    std::fs::write(home.join("z_notes").join("editable.txt"), b"orig").unwrap();
    std::fs::write(home.join(".env"), b"SECRET=1").unwrap();

    let client = reqwest::Client::new();

    let write_ok = client
        .post(format!("{}/api/armaraos-home/write", server.base_url))
        .json(&serde_json::json!({
            "path": "z_notes/editable.txt",
            "content": "nope",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        write_ok.status(),
        200,
        "write under z_notes/** should succeed"
    );

    let read = client
        .get(format!(
            "{}/api/armaraos-home/read?path=z_notes/editable.txt",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    let body: serde_json::Value = read.json().await.unwrap();
    assert_eq!(body["content"], "nope");

    let dotenv = client
        .post(format!("{}/api/armaraos-home/write", server.base_url))
        .json(&serde_json::json!({
            "path": ".env",
            "content": "x",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dotenv.status(), 403);

    let server_star =
        start_test_server_with_provider_patch("ollama", "test-model", "OLLAMA_API_KEY", |c| {
            c.dashboard.home_editable_globs = vec!["**".to_string()];
        })
        .await;
    let home2 = server_star.state.kernel.config.home_dir.clone();
    std::fs::write(home2.join(".env"), b"y").unwrap();
    let client2 = reqwest::Client::new();
    let block_env = client2
        .post(format!("{}/api/armaraos-home/write", server_star.base_url))
        .json(&serde_json::json!({
            "path": ".env",
            "content": "z",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(block_env.status(), 403);
}

#[tokio::test]
async fn armaraos_home_write_disabled_without_globs() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let r = client
        .post(format!("{}/api/armaraos-home/write", server.base_url))
        .json(&serde_json::json!({
            "path": "any.txt",
            "content": "x",
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn test_status_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
    assert_eq!(body["agent_count"], 1); // default assistant auto-spawned
    assert!(body["uptime_seconds"].is_number());
    assert_eq!(body["default_provider"], "ollama");
    assert_eq!(body["agents"].as_array().unwrap().len(), 1);

    let ainl = body
        .get("openfang_runtime_ainl")
        .expect("status should include openfang_runtime_ainl");
    assert_eq!(
        ainl.get("ainl_runtime_engine").and_then(|v| v.as_bool()),
        Some(true),
        "release builds must compile openfang-runtime with default feature ainl-runtime-engine"
    );
    assert_eq!(
        ainl.get("ainl_runtime_engine_env_disabled")
            .and_then(|v| v.as_bool()),
        Some(false),
        "tests do not set ARMARAOS_DISABLE_AINL_RUNTIME_ENGINE"
    );
    assert_eq!(
        ainl.get("ainl_runtime_engine_forced_by_env")
            .and_then(|v| v.as_bool()),
        Some(false),
        "tests do not set AINL_RUNTIME_ENGINE=1"
    );
}

/// `GET /api/mcp/servers` — generic `readiness` map + backward-compatible `calendar_readiness`.
#[tokio::test]
async fn test_mcp_servers_readiness_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/mcp/servers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("configured").is_some());
    assert!(body.get("connected").is_some());

    let readiness = body.get("readiness").expect("readiness object");
    assert_eq!(
        readiness.get("version").and_then(|v| v.as_u64()),
        Some(u64::from(
            openfang_runtime::mcp_readiness::READINESS_SCHEMA_VERSION
        ))
    );
    let checks = readiness
        .get("checks")
        .and_then(|c| c.as_object())
        .expect("readiness.checks");
    assert!(checks.contains_key("calendar"));
    let cal = checks.get("calendar").expect("calendar check");
    assert!(cal.get("ready").is_some());

    let legacy = body
        .get("calendar_readiness")
        .expect("calendar_readiness alias");
    assert_eq!(legacy.get("ready"), cal.get("ready"));
}

/// Guided MCP preset catalog for the dashboard installer.
#[tokio::test]
async fn test_integrations_mcp_presets_endpoint() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/integrations/mcp-presets", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let presets = body["presets"].as_array().expect("presets array");
    assert!(
        presets.iter().any(|p| p["integration_id"] == "postgresql"),
        "postgres preset should map to postgresql integration id"
    );
    assert!(presets.iter().any(|p| p["integration_id"] == "filesystem"));
}

/// `POST /api/integrations/validate` — structured field errors for filesystem preset.
#[tokio::test]
async fn test_integrations_validate_filesystem_requires_allowed_paths() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/integrations/validate", server.base_url))
        .json(&serde_json::json!({
            "id": "filesystem",
            "env": {},
            "config": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false);
    let fe = body["field_errors"]
        .as_object()
        .expect("field_errors object");
    assert!(fe.contains_key("allowed_paths"), "{fe:?}");
}

/// `POST /api/integrations/custom/validate` — rejects bundled ids.
#[tokio::test]
async fn test_integrations_custom_validate_rejects_bundled_id() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/api/integrations/custom/validate",
            server.base_url
        ))
        .json(&serde_json::json!({
            "id": "github",
            "name": "X",
            "transport": { "type": "stdio", "command": "npx" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], false);
    let fe = body["field_errors"].as_object().expect("field_errors");
    assert!(fe.contains_key("id"), "{fe:?}");
}

/// `POST /api/integrations/custom/validate` — happy path (stdio, no env).
#[tokio::test]
async fn test_integrations_custom_validate_ok() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!(
            "{}/api/integrations/custom/validate",
            server.base_url
        ))
        .json(&serde_json::json!({
            "id": "mytools",
            "name": "My Tools",
            "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@foo/bar"] },
            "env": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ok"], true);
    assert_eq!(body["id"], "mytools");
}

/// Custom MCP add persists template + install, then `GET /api/integrations/available` lists it.
#[tokio::test]
async fn test_integrations_custom_add_stdio_persists() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/integrations/custom/add", server.base_url))
        .json(&serde_json::json!({
            "id": "zcusttest01",
            "name": "Z Cust Test",
            "transport": { "type": "stdio", "command": "npx", "args": ["-y", "noop"] },
            "env": []
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["mode"], "custom");
    assert_eq!(body["id"], "zcusttest01");

    let avail = client
        .get(format!("{}/api/integrations/available", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(avail.status(), 200);
    let aj: serde_json::Value = avail.json().await.unwrap();
    let list = aj["integrations"].as_array().expect("integrations");
    assert!(
        list.iter().any(|t| t["id"] == "zcusttest01"),
        "custom template should appear in available list"
    );
}

/// Configured install rejects unknown env/config keys (typo guard).
#[tokio::test]
async fn test_integrations_add_configured_rejects_unknown_field() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/integrations/add", server.base_url))
        .json(&serde_json::json!({
            "id": "github",
            "env": { "NOT_A_REAL_KEY": "x" },
            "config": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

/// Legacy `{ "id" }` install remains available (no `env` / `config` keys).
#[tokio::test]
async fn test_integrations_add_legacy_mode_unchanged() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/api/integrations/add", server.base_url))
        .json(&serde_json::json!({ "id": "github" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["mode"], "legacy");
}

#[tokio::test]
async fn test_metrics_endpoint_includes_ainl_bridge_cache_counters() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/metrics", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body = resp.text().await.unwrap();

    assert!(
        body.contains("openfang_ainl_runtime_bridge_cache_hits_total"),
        "missing bridge cache hits metric: {body}"
    );
    assert!(
        body.contains("openfang_ainl_runtime_bridge_cache_misses_total"),
        "missing bridge cache misses metric: {body}"
    );
    assert!(
        body.contains("openfang_ainl_runtime_bridge_construct_failures_total"),
        "missing bridge construct failures metric: {body}"
    );
    assert!(
        body.contains("openfang_ainl_runtime_bridge_run_failures_total"),
        "missing bridge run failures metric: {body}"
    );
}

#[tokio::test]
async fn test_agents_runtime_effective_state_fields_present() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(!agents.is_empty(), "default assistant should exist");
    let first = &agents[0];

    let manifest_flag = first["ainl_runtime_engine"]
        .as_bool()
        .expect("manifest ainl_runtime_engine should be bool");
    let effective = first["ainl_runtime_engine_effective"]
        .as_bool()
        .expect("ainl_runtime_engine_effective should be bool");
    let forced = first["ainl_runtime_engine_forced_by_env"]
        .as_bool()
        .expect("ainl_runtime_engine_forced_by_env should be bool");
    let disabled = first["ainl_runtime_engine_env_disabled"]
        .as_bool()
        .expect("ainl_runtime_engine_env_disabled should be bool");
    let compiled = first["ainl_runtime_engine_compiled"]
        .as_bool()
        .expect("ainl_runtime_engine_compiled should be bool");

    assert!(!forced, "tests should not force AINL runtime via env");
    assert!(!disabled, "tests should not disable AINL runtime via env");
    assert!(
        compiled,
        "openfang-runtime default features should compile runtime"
    );
    assert_eq!(
        effective,
        compiled && !disabled && (manifest_flag || forced)
    );
}

#[tokio::test]
async fn test_send_message_includes_ainl_runtime_telemetry_field() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let agents_resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(agents_resp.status(), 200);
    let agents: Vec<serde_json::Value> = agents_resp.json().await.unwrap();
    let agent_id = agents[0]["id"]
        .as_str()
        .expect("default assistant id")
        .to_string();

    let resp = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message":"ping"}))
        .send()
        .await
        .unwrap();
    let status = resp.status();
    let body_text = resp.text().await.unwrap();
    if status != 200 {
        // Default integration server uses ollama + `test-model`; local installs may not have it.
        if status == 500
            && (body_text.contains("not_found_error")
                || body_text.contains("model") && body_text.contains("not found"))
        {
            eprintln!(
                "skipping test_send_message_includes_ainl_runtime_telemetry_field: model unavailable ({body_text})"
            );
            return;
        }
        panic!("POST /message: {status}: {body_text}");
    }
    let body: serde_json::Value = serde_json::from_str(&body_text).unwrap();

    assert!(
        body.get("ainl_runtime_telemetry").is_some(),
        "message response must include ainl_runtime_telemetry key"
    );
}

#[tokio::test]
async fn test_send_message_stream_emits_ainl_runtime_telemetry_event() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let agents_resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(agents_resp.status(), 200);
    let agents: Vec<serde_json::Value> = agents_resp.json().await.unwrap();
    let agent_id = agents[0]["id"]
        .as_str()
        .expect("default assistant id")
        .to_string();

    let resp = client
        .post(format!(
            "{}/api/agents/{}/message/stream",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message":"ping"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "unexpected content-type: {ct}"
    );
    let body = resp.text().await.unwrap();
    assert!(
        body.contains("event: ainl_runtime_telemetry"),
        "expected ainl runtime telemetry event in stream body: {body}"
    );
}

#[tokio::test]
async fn test_ui_prefs_get_empty_when_missing() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/ui-prefs", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.is_object());
    assert!(body.as_object().unwrap().is_empty());
}

#[tokio::test]
async fn test_ui_prefs_put_get_roundtrip_agent_eco_modes() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let put_body = serde_json::json!({
        "pinned_agents": ["00000000-0000-0000-0000-000000000042"],
        "agent_eco_modes": {
            "00000000-0000-0000-0000-000000000042": "aggressive"
        },
        "overview_checklist_dismissed": true
    });

    let resp = client
        .put(format!("{}/api/ui-prefs", server.base_url))
        .json(&put_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client
        .get(format!("{}/api/ui-prefs", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["pinned_agents"], put_body["pinned_agents"]);
    assert_eq!(body["agent_eco_modes"], put_body["agent_eco_modes"]);
    assert_eq!(
        body["overview_checklist_dismissed"],
        put_body["overview_checklist_dismissed"]
    );

    // Full overwrite: omitting keys should drop them.
    let resp = client
        .put(format!("{}/api/ui-prefs", server.base_url))
        .json(&serde_json::json!({ "agent_eco_modes": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client
        .get(format!("{}/api/ui-prefs", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body, serde_json::json!({ "agent_eco_modes": {} }));
}

#[tokio::test]
async fn test_ui_prefs_put_rejects_non_object_body() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .put(format!("{}/api/ui-prefs", server.base_url))
        .json(&serde_json::json!([]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_ui_prefs_notify_chat_replies_roundtrip() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let put_body = serde_json::json!({
        "notify_chat_replies": "hidden",
        "pinned_agents": []
    });

    let resp = client
        .put(format!("{}/api/ui-prefs", server.base_url))
        .json(&put_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let resp = client
        .get(format!("{}/api/ui-prefs", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["notify_chat_replies"], "hidden");
}

#[tokio::test]
async fn test_spawn_list_kill_agent() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // --- Spawn ---
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["name"], "test-agent");
    let agent_id = body["agent_id"].as_str().unwrap().to_string();
    assert!(!agent_id.is_empty());

    // --- List (2 agents: default assistant + test-agent) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 2);
    let test_agent = agents.iter().find(|a| a["name"] == "test-agent").unwrap();
    assert_eq!(test_agent["id"], agent_id);
    assert_eq!(test_agent["model_provider"], "ollama");

    // --- Kill ---
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "killed");

    // --- List (only default assistant remains) ---
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "assistant");
}

#[tokio::test]
async fn test_put_agent_update_applies_manifest_and_writes_toml() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .put(format!(
            "{}/api/agents/{}/update",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST_PUT_UPDATE}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["name"], "test-agent");
    assert!(body["note"]
        .as_str()
        .map(|n| !n.is_empty())
        .unwrap_or(false));

    let audit = server.state.kernel.audit_log.recent(20);
    assert!(
        audit.iter().any(|e| {
            matches!(
                &e.action,
                &openfang_runtime::audit::AuditAction::AgentManifestUpdate
            ) && e.outcome == "ok"
                && e.detail.contains("PUT agent manifest update")
                && e.detail.contains("test-agent")
        }),
        "expected AgentManifestUpdate audit entry for successful PUT /update, got: {:?}",
        audit
            .iter()
            .map(|e| (&e.action, &e.detail, &e.outcome))
            .collect::<Vec<_>>()
    );

    let path = server.state.kernel.agent_toml_path("test-agent");
    assert!(
        path.exists(),
        "agent.toml should exist at {}",
        path.display()
    );
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(
        disk.contains("Updated via PUT /update integration test"),
        "on-disk manifest should include new description, got: {disk}"
    );
}

#[tokio::test]
async fn test_put_agent_update_manifest_name_mismatch_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .put(format!(
            "{}/api/agents/{}/update",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"manifest_toml": WRONG_NAME_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Manifest rejected");
}

#[tokio::test]
async fn test_get_agent_includes_manifest_toml() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .get(format!("{}/api/agents/{}", server.base_url, agent_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let detail: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(detail["name"], "test-agent");
    let mt = detail["manifest_toml"]
        .as_str()
        .expect("manifest_toml string");
    assert!(!mt.is_empty(), "manifest_toml must be non-empty");
    assert!(
        mt.contains("name") && mt.contains("test-agent"),
        "expected canonical TOML to include agent name, got: {mt}"
    );
}

#[tokio::test]
async fn test_get_agent_omit_manifest_toml() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .get(format!(
            "{}/api/agents/{}?omit=manifest_toml",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let detail: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(detail["name"], "test-agent");
    assert!(
        detail.get("manifest_toml").is_none(),
        "manifest_toml should be omitted when ?omit=manifest_toml"
    );
}

#[tokio::test]
async fn test_get_agent_omit_comma_separated_includes_manifest_toml_other_token() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .get(format!(
            "{}/api/agents/{}?omit=foo,manifest_toml",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let detail: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(detail["name"], "test-agent");
    assert!(
        detail.get("manifest_toml").is_none(),
        "comma-separated omit should still drop manifest_toml"
    );
}

#[tokio::test]
async fn test_post_schedules_returns_top_level_id_and_name() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/schedules", server.base_url))
        .json(&serde_json::json!({
            "name": "integration-test-cron",
            "cron": "0 9 * * *",
            "agent_id": "assistant",
            "message": "[integration test schedule]"
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        status,
        reqwest::StatusCode::CREATED,
        "unexpected body: {body}"
    );
    assert_eq!(body["source"], "kernel_cron");
    let id = body["id"]
        .as_str()
        .expect("POST /api/schedules should include top-level id for clients");
    uuid::Uuid::parse_str(id).expect("id should be a UUID");
    assert_eq!(
        body["result"]["job_id"].as_str(),
        Some(id),
        "result.job_id should match top-level id"
    );
    assert!(
        body["name"].as_str().unwrap().contains("integration"),
        "expected sanitized name to retain marker: {}",
        body["name"]
    );

    let del = client
        .delete(format!("{}/api/schedules/{}", server.base_url, id))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
}

#[tokio::test]
async fn test_agent_session_empty() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    // Session should be empty — no messages sent yet
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["message_count"], 0);
    assert_eq!(body["messages"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn test_send_message_with_llm() {
    if std::env::var("GROQ_API_KEY").is_err() {
        eprintln!("GROQ_API_KEY not set, skipping LLM integration test");
        return;
    }

    let server = start_test_server_with_llm().await;
    let client = reqwest::Client::new();

    // Spawn
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": LLM_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Send message through the real HTTP endpoint → kernel → Groq LLM
    let resp = client
        .post(format!(
            "{}/api/agents/{}/message",
            server.base_url, agent_id
        ))
        .json(&serde_json::json!({"message": "Say hello in exactly 3 words."}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let response_text = body["response"].as_str().unwrap();
    assert!(
        !response_text.is_empty(),
        "LLM response should not be empty"
    );
    assert!(body["input_tokens"].as_u64().unwrap() > 0);
    assert!(body["output_tokens"].as_u64().unwrap() > 0);

    // Session should now have messages
    let resp = client
        .get(format!(
            "{}/api/agents/{}/session",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    let session: serde_json::Value = resp.json().await.unwrap();
    assert!(session["message_count"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn test_workflow_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for workflow
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_name = body["name"].as_str().unwrap().to_string();

    // Create workflow
    let resp = client
        .post(format!("{}/api/workflows", server.base_url))
        .json(&serde_json::json!({
            "name": "test-workflow",
            "description": "Integration test workflow",
            "steps": [
                {
                    "name": "step1",
                    "agent_name": agent_name,
                    "prompt": "Echo: {{input}}",
                    "mode": "sequential",
                    "timeout_secs": 30
                }
            ]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let workflow_id = body["workflow_id"].as_str().unwrap().to_string();
    assert!(!workflow_id.is_empty());

    // List workflows
    let resp = client
        .get(format!("{}/api/workflows", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let workflows: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0]["name"], "test-workflow");
    assert_eq!(workflows[0]["steps"], 1);
}

#[tokio::test]
async fn test_trigger_crud() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn agent for trigger
    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    // Create trigger (Lifecycle pattern — simplest variant)
    let resp = client
        .post(format!("{}/api/triggers", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "pattern": "lifecycle",
            "prompt_template": "Handle: {{event}}",
            "max_fires": 5
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let trigger_id = body["trigger_id"].as_str().unwrap().to_string();
    assert_eq!(body["agent_id"], agent_id);

    // List triggers (unfiltered)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 1);
    assert_eq!(triggers[0]["agent_id"], agent_id);
    assert_eq!(triggers[0]["enabled"], true);
    assert_eq!(triggers[0]["max_fires"], 5);

    // List triggers (filtered by agent_id)
    let resp = client
        .get(format!(
            "{}/api/triggers?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 1);

    // Delete trigger
    let resp = client
        .delete(format!("{}/api/triggers/{}", server.base_url, trigger_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List triggers (should be empty)
    let resp = client
        .get(format!("{}/api/triggers", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let triggers: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(triggers.len(), 0);
}

#[tokio::test]
async fn test_invalid_agent_id_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Send message to invalid ID
    let resp = client
        .post(format!("{}/api/agents/not-a-uuid/message", server.base_url))
        .json(&serde_json::json!({"message": "hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));

    // Kill invalid ID
    let resp = client
        .delete(format!("{}/api/agents/not-a-uuid", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);

    // Session for invalid ID
    let resp = client
        .get(format!("{}/api/agents/not-a-uuid/session", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
}

#[tokio::test]
async fn test_kill_nonexistent_agent_returns_404() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let fake_id = uuid::Uuid::new_v4();
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, fake_id))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);
}

#[tokio::test]
async fn test_spawn_invalid_manifest_returns_400() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": "this is {{ not valid toml"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid manifest"));
}

#[tokio::test]
async fn test_request_id_header_is_uuid() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();

    let request_id = resp
        .headers()
        .get("x-request-id")
        .expect("x-request-id header should be present");
    let id_str = request_id.to_str().unwrap();
    assert!(
        uuid::Uuid::parse_str(id_str).is_ok(),
        "x-request-id should be a valid UUID, got: {}",
        id_str
    );
}

#[tokio::test]
async fn test_spawn_missing_manifest_returns_structured_error_and_request_id() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": ""}))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let request_id_hdr = resp
        .headers()
        .get("x-request-id")
        .expect("x-request-id should be set by request_logging middleware")
        .to_str()
        .unwrap()
        .to_string();
    assert!(uuid::Uuid::parse_str(&request_id_hdr).is_ok());

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"], "Missing manifest");
    assert!(!body["detail"].as_str().unwrap().is_empty());
    assert_eq!(body["path"], "/api/agents");
    assert_eq!(body["request_id"].as_str().unwrap(), request_id_hdr);
    assert!(!body
        .get("hint")
        .and_then(|h| h.as_str())
        .unwrap_or("")
        .is_empty());
}

#[tokio::test]
async fn test_multiple_agents_lifecycle() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Spawn 3 agents
    let mut ids = Vec::new();
    for i in 0..3 {
        let manifest = format!(
            r#"
name = "agent-{i}"
version = "0.1.0"
description = "Multi-agent test {i}"
author = "test"
module = "builtin:chat"

[model]
provider = "ollama"
model = "test-model"
system_prompt = "Agent {i}."

[capabilities]
memory_read = ["*"]
memory_write = ["self.*"]
"#
        );

        let resp = client
            .post(format!("{}/api/agents", server.base_url))
            .json(&serde_json::json!({"manifest_toml": manifest}))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        ids.push(body["agent_id"].as_str().unwrap().to_string());
    }

    // List should show 4 (3 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 4);

    // Status should agree
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    let status: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(status["agent_count"], 4);

    // Kill one
    let resp = client
        .delete(format!("{}/api/agents/{}", server.base_url, ids[1]))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    // List should show 3 (2 spawned + default assistant)
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 3);

    // Kill the rest
    for id in [&ids[0], &ids[2]] {
        client
            .delete(format!("{}/api/agents/{}", server.base_url, id))
            .send()
            .await
            .unwrap();
    }

    // List should have only default assistant
    let resp = client
        .get(format!("{}/api/agents", server.base_url))
        .send()
        .await
        .unwrap();
    let agents: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert_eq!(agents.len(), 1);
}

/// POST /api/ainl/library/register-curated returns `registered` and `embedded_programs_written`.
#[tokio::test]
async fn test_register_curated_response_includes_embedded_counts() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/ainl/library/register-curated", server.base_url);
    let resp = client.post(&url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body.get("registered").is_some(),
        "missing registered: {body}"
    );
    assert!(
        body.get("embedded_programs_written").is_some(),
        "missing embedded_programs_written: {body}"
    );
}

/// POST /api/ainl/library/register-curated allows 5 calls per IP per 60s; the 6th returns 429.
#[tokio::test]
async fn test_register_curated_rate_limit_6th_returns_429() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/ainl/library/register-curated", server.base_url);

    for i in 1..=5 {
        let resp = client.post(&url).send().await.unwrap();
        assert_ne!(
            resp.status(),
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "request {i} should not be rate-limited"
        );
    }

    let resp = client.post(&url).send().await.unwrap();
    assert_eq!(resp.status(), 429);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"].as_str(), Some("Rate limited"));
    let detail = body["detail"].as_str().expect("detail field");
    assert!(
        detail.contains("Too many register-curated"),
        "unexpected body: {body}"
    );
}

/// GET /api/events/stream returns `text/event-stream` and at least one `data:` SSE line (loopback).
#[tokio::test]
async fn test_kernel_events_stream_sse_smoke() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let url = format!("{}/api/events/stream", server.base_url);
    let mut resp = client.get(url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "expected text/event-stream, got {ct}"
    );
    let chunk = resp.chunk().await.unwrap().expect("first chunk");
    let s = String::from_utf8_lossy(&chunk);
    assert!(
        s.contains("data:") || s.contains("ping"),
        "expected SSE data or comment, got: {s:?}"
    );
}

/// `GraphMemoryWrite` appears on `GET /api/events/stream` when the kernel publishes it (graph-memory live timeline contract).
#[tokio::test]
async fn test_kernel_events_stream_includes_graph_memory_write() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap().to_string();

    let sse_url = format!("{}/api/events/stream", server.base_url);
    let mut sse_resp = client.get(sse_url).send().await.unwrap();
    assert_eq!(sse_resp.status(), 200);

    let kernel = server.state.kernel.clone();
    let aid = agent_id.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(120)).await;
        kernel
            .notify_graph_memory_write(&aid, "episode", None)
            .await
            .expect("notify_graph_memory_write should succeed for a spawned agent id");
    });

    let mut accumulated = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    loop {
        if accumulated.contains("GraphMemoryWrite") {
            break;
        }
        if tokio::time::Instant::now() > deadline {
            let tail: String = accumulated.chars().rev().take(900).collect::<String>();
            let tail: String = tail.chars().rev().collect();
            panic!("timed out waiting for GraphMemoryWrite on SSE; tail={tail}");
        }
        match tokio::time::timeout(Duration::from_millis(700), sse_resp.chunk()).await {
            Ok(Ok(Some(bytes))) => {
                accumulated.push_str(&String::from_utf8_lossy(&bytes));
            }
            Ok(Ok(None)) => {
                panic!("SSE closed before GraphMemoryWrite; got={accumulated}");
            }
            Ok(Err(e)) => panic!("SSE chunk error: {e}"),
            Err(_) => {}
        }
    }
}

/// GET /api/budget returns JSON used by the dashboard (notification center + Settings).
#[tokio::test]
async fn test_get_budget_status_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/budget", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(
        v.get("hourly_spend").is_some(),
        "expected hourly_spend: {v}"
    );
    assert!(
        v.get("alert_threshold").is_some(),
        "expected alert_threshold: {v}"
    );
    assert!(
        v.get("monthly_limit").is_some(),
        "expected monthly_limit: {v}"
    );
}

/// GET /api/system/daemon-resources returns JSON for the dashboard footprint strip.
#[tokio::test]
async fn test_get_daemon_resources_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/system/daemon-resources", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(v.get("cpu_percent").is_some(), "expected cpu_percent: {v}");
    assert!(
        v.get("memory_percent").is_some(),
        "expected memory_percent: {v}"
    );
    assert!(
        v.get("logical_cpus").is_some(),
        "expected logical_cpus: {v}"
    );
    assert!(v.get("supported").is_some(), "expected supported: {v}");
}

/// GET /api/approvals returns `{ approvals, total }` for the dashboard queue.
#[tokio::test]
async fn test_get_approvals_list_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/approvals", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(
        v.get("approvals").is_some(),
        "expected approvals array: {v}"
    );
    assert!(v.get("total").is_some(), "expected total: {v}");
}

/// GET /api/agents/{id}/session/digest — unread / sync counts for dashboard polling.
#[tokio::test]
async fn test_session_digest_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .get(format!(
            "{}/api/agents/{}/session/digest",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert!(v.get("session_id").is_some(), "expected session_id: {v}");
    assert!(v.get("agent_id").is_some(), "expected agent_id: {v}");
    assert!(
        v.get("message_count").is_some(),
        "expected message_count: {v}"
    );
    assert!(
        v.get("assistant_message_count").is_some(),
        "expected assistant_message_count: {v}"
    );
}

/// GET /api/usage/compression — SQLite-backed eco compression aggregates.
#[tokio::test]
async fn test_usage_compression_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/usage/compression?window=7d",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v.get("window").and_then(|x| x.as_str()), Some("7d"));
    assert!(v.get("modes").is_some(), "expected modes: {v}");
    assert!(v.get("agents").is_some(), "expected agents: {v}");
    // Degraded path should surface flags (only when compression rollup query fails).
    if v.get("compression_summary_error").and_then(|x| x.as_bool()) == Some(true) {
        assert!(
            v.get("adaptive_eco_filled_from_fallback").is_some(),
            "expected fallback flag when degraded: {v}"
        );
    }
}

/// GET /api/orchestration/traces — recent trace summaries (empty vec is valid).
#[tokio::test]
async fn test_orchestration_traces_list_json_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "{}/api/orchestration/traces?limit=10",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let summaries: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(summaries.len() <= 10);
}

/// GET /api/orchestration/traces/:id — planner `plan_*` events round-trip with stable `type` tags (dashboard + Gantt).
#[tokio::test]
async fn test_orchestration_trace_plan_events_json_roundtrip() {
    use chrono::Utc;
    use openfang_types::agent::AgentId;
    use openfang_types::orchestration_trace::{OrchestrationTraceEvent, TraceEventType};

    let server = start_test_server().await;
    let trace_id = "orch-it-plan-trace";
    let aid = AgentId(uuid::Uuid::new_v4());

    server.state.kernel.orchestration_traces.push(OrchestrationTraceEvent {
        trace_id: trace_id.to_string(),
        orchestrator_id: aid,
        agent_id: aid,
        parent_agent_id: None,
        event_type: TraceEventType::PlanStarted {
            step_count: 3,
            confidence: 0.91,
            reasoning_step_ids: vec!["r1".to_string()],
        },
        timestamp: Utc::now(),
        metadata: std::collections::HashMap::new(),
    });
    server.state.kernel.orchestration_traces.push(OrchestrationTraceEvent {
        trace_id: trace_id.to_string(),
        orchestrator_id: aid,
        agent_id: aid,
        parent_agent_id: None,
        event_type: TraceEventType::PlanFallback {
            reason: "planner_invalid_plan".to_string(),
        },
        timestamp: Utc::now(),
        metadata: std::collections::HashMap::new(),
    });

    let client = reqwest::Client::new();
    let url = format!(
        "{}/api/orchestration/traces/{}",
        server.base_url, trace_id
    );
    let resp = client.get(url).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    let events: Vec<serde_json::Value> = resp.json().await.unwrap();
    assert!(
        events.len() >= 2,
        "expected seeded plan events, got {}",
        events.len()
    );
    assert_eq!(
        events[0]["event_type"]["type"].as_str(),
        Some("plan_started")
    );
    assert_eq!(
        events[0]["event_type"]["step_count"].as_u64(),
        Some(3)
    );
    assert_eq!(
        events[1]["event_type"]["type"].as_str(),
        Some("plan_fallback")
    );
    assert_eq!(
        events[1]["event_type"]["reason"].as_str(),
        Some("planner_invalid_plan")
    );
}

/// POST /api/agents/{id}/btw — 409 when no agent loop is active (idle after spawn).
#[tokio::test]
async fn test_btw_returns_409_when_agent_idle() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/agents", server.base_url))
        .json(&serde_json::json!({"manifest_toml": TEST_MANIFEST}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: serde_json::Value = resp.json().await.unwrap();
    let agent_id = body["agent_id"].as_str().unwrap();

    let resp = client
        .post(format!("{}/api/agents/{}/btw", server.base_url, agent_id))
        .json(&serde_json::json!({"text": "injected context"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn test_graph_memory_snapshot_reset_and_rollback_flow() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let agent_id = "graph-memory-it-1";
    seed_graph_memory(&server, agent_id);

    let graph_before: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let before_nodes = graph_before["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(before_nodes > 0, "expected seeded graph nodes");

    let snap_resp: serde_json::Value = client
        .post(format!("{}/api/graph-memory/snapshot", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "label": "it_snapshot"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(snap_resp["ok"].as_bool(), Some(true), "{snap_resp}");
    let snapshot_id = snap_resp["snapshot_id"]
        .as_str()
        .expect("snapshot id")
        .to_string();

    let preview_graph: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory/snapshot-graph?agent_id={}&snapshot_id={}",
            server.base_url, agent_id, snapshot_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let preview_nodes = preview_graph["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        preview_nodes >= before_nodes,
        "expected preview snapshot nodes >= before nodes"
    );

    let reset_resp: serde_json::Value = client
        .post(format!("{}/api/graph-memory/reset", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "reason": "integration test",
            "create_snapshot": false
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(reset_resp["ok"].as_bool(), Some(true), "{reset_resp}");

    let graph_after_reset: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let reset_nodes = graph_after_reset["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert_eq!(reset_nodes, 0, "expected reset to clear nodes");

    let rollback_resp: serde_json::Value = client
        .post(format!("{}/api/graph-memory/rollback", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "snapshot_id": snapshot_id,
            "reason": "integration rollback"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(rollback_resp["ok"].as_bool(), Some(true), "{rollback_resp}");

    let graph_after_rollback: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let rollback_nodes = graph_after_rollback["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        rollback_nodes >= before_nodes,
        "expected rollback nodes >= initial nodes"
    );
}

#[tokio::test]
async fn test_graph_memory_delete_node_and_audit() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let agent_id = "graph-memory-it-2";
    seed_graph_memory(&server, agent_id);

    let graph_before: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let before_nodes = graph_before["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(before_nodes > 0, "expected nodes before deletion");
    let target_node_id = graph_before["nodes"][0]["id"]
        .as_str()
        .expect("node id")
        .to_string();

    let delete_resp: serde_json::Value = client
        .post(format!("{}/api/graph-memory/delete-node", server.base_url))
        .json(&serde_json::json!({
            "agent_id": agent_id,
            "node_id": target_node_id,
            "reason": "integration delete"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(delete_resp["ok"].as_bool(), Some(true), "{delete_resp}");

    let graph_after: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let after_nodes = graph_after["nodes"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);
    assert!(
        after_nodes < before_nodes,
        "expected fewer nodes after delete (before={before_nodes}, after={after_nodes})"
    );

    let audit: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory/audit?agent_id={}&limit=20",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let entries = audit["entries"].as_array().cloned().unwrap_or_default();
    let has_delete = entries
        .iter()
        .any(|e| e["action"].as_str() == Some("node_deleted"));
    assert!(has_delete, "expected node_deleted audit entry: {audit}");
}

#[tokio::test]
async fn test_graph_memory_explain_payload_shape() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let agent_id = "graph-memory-it-explain";
    seed_graph_memory(&server, agent_id);

    let graph: serde_json::Value = client
        .get(format!(
            "{}/api/graph-memory?agent_id={}",
            server.base_url, agent_id
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let nodes = graph["nodes"].as_array().expect("nodes array");
    assert!(!nodes.is_empty(), "expected seeded nodes");
    for n in nodes {
        let explain = &n["explain"];
        assert!(
            explain.is_object(),
            "each node must include explain object: {n}"
        );
        for key in [
            "what_happened",
            "why_happened",
            "evidence",
            "relations",
            "node_kind",
        ] {
            assert!(
                explain.get(key).is_some(),
                "explain must include {key}: {explain}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Auth integration tests
// ---------------------------------------------------------------------------

/// Start a test server with Bearer-token authentication enabled.
async fn start_test_server_with_auth(api_key: &str) -> TestServer {
    let tmp = tempfile::tempdir().expect("Failed to create temp dir");

    let config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        api_key: api_key.to_string(),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };

    let kernel = OpenFangKernel::boot_with_config(config).expect("Kernel should boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    spawn_test_server_with_kernel(kernel, tmp).await
}

#[tokio::test]
async fn test_auth_health_is_public() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // /api/health should be accessible without auth
    let resp = client
        .get(format!("{}/api/health", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_auth_rejects_no_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Protected endpoint without auth header → 401
    // Note: /api/status is public (dashboard needs it), so use a protected endpoint
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Missing"));
}

#[tokio::test]
async fn test_auth_rejects_wrong_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Wrong bearer token → 401
    // Note: /api/status is public (dashboard needs it), so use a protected endpoint
    let resp = client
        .get(format!("{}/api/commands", server.base_url))
        .header("authorization", "Bearer wrong-key")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"].as_str().unwrap().contains("Invalid"));
}

#[tokio::test]
async fn test_auth_accepts_correct_token() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    // Correct bearer token → 200
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .header("authorization", "Bearer secret-key-123")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "running");
}

#[tokio::test]
async fn test_auth_schedules_get_without_token_is_public_read() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{}/api/schedules", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["source"], "kernel_cron");
}

#[tokio::test]
async fn test_auth_schedules_post_delete_with_bearer() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();
    const TOKEN: &str = "secret-key-123";

    let list = client
        .get(format!("{}/api/schedules", server.base_url))
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), 200);
    let list_body: serde_json::Value = list.json().await.unwrap();
    assert_eq!(list_body["source"], "kernel_cron");

    let resp = client
        .post(format!("{}/api/schedules", server.base_url))
        .header("authorization", format!("Bearer {TOKEN}"))
        .json(&serde_json::json!({
            "name": "auth-server-schedule-test",
            "cron": "30 10 * * *",
            "agent_id": "assistant",
            "message": "[auth integration test]"
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        status,
        reqwest::StatusCode::CREATED,
        "unexpected body: {body}"
    );
    let id = body["id"].as_str().expect("top-level id");
    uuid::Uuid::parse_str(id).expect("id is uuid");

    let del = client
        .delete(format!("{}/api/schedules/{}", server.base_url, id))
        .header("authorization", format!("Bearer {TOKEN}"))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 200);
}

#[tokio::test]
async fn test_auth_schedules_post_delete_loopback_without_bearer() {
    let server = start_test_server_with_auth("secret-key-123").await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("{}/api/schedules", server.base_url))
        .json(&serde_json::json!({
            "name": "loopback-no-bearer-sched",
            "cron": "45 11 * * *",
            "agent_id": "assistant",
            "message": "[loopback auth test]"
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        status,
        reqwest::StatusCode::CREATED,
        "loopback should allow POST /api/schedules without Bearer: {body}"
    );
    let id = body["id"].as_str().expect("id");
    uuid::Uuid::parse_str(id).unwrap();

    let del = client
        .delete(format!("{}/api/schedules/{}", server.base_url, id))
        .send()
        .await
        .unwrap();
    assert_eq!(
        del.status(),
        200,
        "loopback should allow DELETE /api/schedules/:id without Bearer"
    );
}

#[tokio::test]
async fn test_auth_disabled_when_no_key() {
    // Empty API key = auth disabled
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    // Protected endpoint accessible without auth when no key is configured
    let resp = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
}

#[tokio::test]
async fn test_usage_adaptive_eco_and_replay_endpoints() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();

    let eco = client
        .get(format!("{}/api/usage/adaptive-eco", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(eco.status(), 200);
    let eco_body: serde_json::Value = eco.json().await.unwrap();
    assert!(
        eco_body.get("events").is_some(),
        "expected events summary: {eco_body}"
    );

    let replay = client
        .get(format!(
            "{}/api/usage/adaptive-eco/replay?window=7d",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    let rep: serde_json::Value = replay.json().await.unwrap();
    assert_eq!(rep["window"], "7d");
    assert!(rep.get("adaptive_eco_events").is_some());
    assert!(rep.get("shadow_mismatch_rate").is_some());
    assert!(rep.get("eco_compression_turns").is_some());
    assert!(
        rep.get("effective_mode_flip_rate").is_some(),
        "replay should include mode flip rate: {rep}"
    );
    assert!(
        rep.get("adaptive_confidence_bucket_low").is_some(),
        "replay should include confidence buckets: {rep}"
    );

    let comp = client
        .get(format!(
            "{}/api/usage/compression?window=7d",
            server.base_url
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(comp.status(), 200);
    let comp_body: serde_json::Value = comp.json().await.unwrap();
    assert!(
        comp_body.get("adaptive_eco").is_some(),
        "compression response should embed adaptive_eco bundle: {comp_body}"
    );
    let bundle = comp_body.get("adaptive_eco").unwrap();
    assert!(bundle
        .get("summary")
        .and_then(|s| s.get("events"))
        .is_some());
    assert!(bundle
        .get("replay")
        .and_then(|r| r.get("shadow_mismatch_rate"))
        .is_some());
    assert_ne!(
        comp_body
            .get("compression_summary_error")
            .and_then(|x| x.as_bool()),
        Some(true),
        "integration server should return full compression summary (not degraded): {comp_body}"
    );
}

#[tokio::test]
async fn test_status_adaptive_eco_reflects_boot_config() {
    let server_on =
        start_test_server_with_provider_patch("ollama", "test-model", "OLLAMA_API_KEY", |c| {
            c.adaptive_eco.enabled = true;
            c.adaptive_eco.enforce = false;
        })
        .await;
    let client = reqwest::Client::new();
    let st = client
        .get(format!("{}/api/status", server_on.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(st.status(), 200);
    let body: serde_json::Value = st.json().await.unwrap();
    assert_eq!(body["adaptive_eco"]["enabled"], true);

    let server_off =
        start_test_server_with_provider_patch("ollama", "test-model", "OLLAMA_API_KEY", |c| {
            c.adaptive_eco.enabled = false;
        })
        .await;
    let st2 = client
        .get(format!("{}/api/status", server_off.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(st2.status(), 200);
    let body2: serde_json::Value = st2.json().await.unwrap();
    assert_eq!(body2["adaptive_eco"]["enabled"], false);
}

#[tokio::test]
async fn test_config_set_persists_adaptive_eco_enabled_and_reload_updates_status() {
    let server =
        start_test_server_with_provider_patch("ollama", "test-model", "OLLAMA_API_KEY", |c| {
            c.adaptive_eco.enabled = true;
        })
        .await;
    let client = reqwest::Client::new();

    let st = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    let before: serde_json::Value = st.json().await.unwrap();
    assert_eq!(before["adaptive_eco"]["enabled"], true);

    let cfg_path = server.state.kernel.config.home_dir.join("config.toml");

    let resp = client
        .post(format!("{}/api/config/set", server.base_url))
        .json(&serde_json::json!({
            "path": "adaptive_eco.enabled",
            "value": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);

    let raw = std::fs::read_to_string(&cfg_path).expect("config.toml written");
    let table: toml::value::Table = toml::from_str(&raw).expect("parse config");
    let adaptive = table
        .get("adaptive_eco")
        .and_then(|v| v.as_table())
        .expect("[adaptive_eco] section");
    assert_eq!(
        adaptive.get("enabled").and_then(|v| v.as_bool()),
        Some(false)
    );

    let after = client
        .get(format!("{}/api/status", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(after.status(), 200);
    let body: serde_json::Value = after.json().await.unwrap();
    assert_eq!(
        body["adaptive_eco"]["enabled"], false,
        "status should reflect reloaded adaptive_eco.enabled"
    );
}

#[tokio::test]
async fn test_dashboard_html_includes_adaptive_eco_budget_section() {
    let server = start_test_server().await;
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/", server.base_url))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 200);
    let html = resp.text().await.unwrap();
    assert!(
        html.contains("Adaptive eco policy"),
        "budget page should include adaptive eco heading"
    );
    assert!(
        html.contains("adaptive_confidence_p50") && html.contains("effective_mode_flip_rate"),
        "budget adaptive strip should expose confidence + flip-rate bindings"
    );
}
