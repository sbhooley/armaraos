//! High-signal API boundary tests: MCP HTTP JSON-RPC, webhook routes, dashboard cookie auth.
//!
//! Uses the production router from [`openfang_api::server::build_router`] and `reqwest`
//! against a local listener (same pattern as `api_integration_test`).

use openfang_api::routes::AppState;
use openfang_kernel::OpenFangKernel;
use openfang_types::config::{DefaultModelConfig, KernelConfig, WebhookTriggerConfig};
use std::net::SocketAddr;
use std::sync::Arc;

struct TestServer {
    base_url: String,
    state: Arc<AppState>,
    _tmp: tempfile::TempDir,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        // Never block the Tokio test worker: `join()` triggers the same panic as synchronous
        // `shutdown()` (blocking pool teardown while inside an async task). Detached thread only.
        let k = self.state.kernel.clone();
        std::thread::spawn(move || k.shutdown());
    }
}

async fn spawn_test_server_with_kernel(
    kernel: Arc<OpenFangKernel>,
    tmp: tempfile::TempDir,
) -> TestServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local_addr");

    let (app, state) = openfang_api::server::build_router(kernel, addr).await;

    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("server");
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

async fn start_server_with_config(mut config: KernelConfig) -> TestServer {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Always isolate to the per-test tempdir. KernelConfig::default() pre-fills
    // home_dir/data_dir with the user's real ~/.armaraos paths, so without
    // unconditional override every parallel test collides on the same SQLite
    // file ("database is locked" during boot).
    config.home_dir = tmp.path().to_path_buf();
    config.data_dir = tmp.path().join("data");
    let kernel = OpenFangKernel::boot_with_config(config).expect("boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    spawn_test_server_with_kernel(kernel, tmp).await
}

fn default_test_config() -> KernelConfig {
    KernelConfig {
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    }
}

#[tokio::test]
async fn mcp_http_returns_jsonrpc_error_for_unknown_tool() {
    let mut cfg = default_test_config();
    cfg.api_key = "mcp-boundary-test-key-32chars-minimum__".to_string();
    let server = start_server_with_config(cfg).await;
    let client = reqwest::Client::new();
    let url = format!("{}/mcp", server.base_url);
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "name": "definitely_not_a_real_tool_openfang_boundary_test",
            "arguments": {}
        }
    });
    let res = client
        .post(&url)
        .header(
            "Authorization",
            "Bearer mcp-boundary-test-key-32chars-minimum__",
        )
        .json(&body)
        .send()
        .await
        .expect("post /mcp");
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = res.json().await.expect("json");
    assert_eq!(v["error"]["code"], -32602);
    assert!(
        v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unknown tool"),
        "{v}"
    );
}

#[tokio::test]
async fn webhook_wake_returns_404_when_webhooks_disabled() {
    let mut cfg = default_test_config();
    cfg.api_key = "wake-404-test-key-32chars-minimum___".to_string();
    cfg.webhook_triggers = None;
    let server = start_server_with_config(cfg).await;
    let client = reqwest::Client::new();
    let url = format!("{}/hooks/wake", server.base_url);
    let body = serde_json::json!({"text": "hello", "mode": "now"});
    let res = client
        .post(&url)
        .header(
            "Authorization",
            "Bearer wake-404-test-key-32chars-minimum___",
        )
        .json(&body)
        .send()
        .await
        .expect("post");
    assert_eq!(res.status(), reqwest::StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn webhook_wake_returns_401_when_token_mismatches_env() {
    let mut cfg = default_test_config();
    let api_key = "wake-api-key-32chars-minimum__________";
    cfg.api_key = api_key.to_string();
    cfg.webhook_triggers = Some(WebhookTriggerConfig {
        enabled: true,
        token_env: "OPENFANG_BOUNDARY_WEBHOOK_TOKEN".to_string(),
        ..WebhookTriggerConfig::default()
    });
    std::env::set_var(
        "OPENFANG_BOUNDARY_WEBHOOK_TOKEN",
        "different-32-char-token-for-webhook___",
    );
    let server = start_server_with_config(cfg).await;
    let client = reqwest::Client::new();
    let url = format!("{}/hooks/wake", server.base_url);
    let body = serde_json::json!({"text": "hello", "mode": "now"});
    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .json(&body)
        .send()
        .await
        .expect("post");
    assert_eq!(res.status(), reqwest::StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn webhook_wake_accepts_matching_bearer_token() {
    let token = "shared-32-char-token-for-hook-and-api__";
    let mut cfg = default_test_config();
    cfg.api_key = token.to_string();
    cfg.webhook_triggers = Some(WebhookTriggerConfig {
        enabled: true,
        token_env: "OPENFANG_BOUNDARY_WEBHOOK_TOKEN2".to_string(),
        ..WebhookTriggerConfig::default()
    });
    std::env::set_var("OPENFANG_BOUNDARY_WEBHOOK_TOKEN2", token);
    let server = start_server_with_config(cfg).await;
    let client = reqwest::Client::new();
    let url = format!("{}/hooks/wake", server.base_url);
    let body = serde_json::json!({"text": "hello", "mode": "now"});
    let res = client
        .post(&url)
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("post");
    assert_eq!(res.status(), reqwest::StatusCode::OK);
    let v: serde_json::Value = res.json().await.expect("json");
    assert_eq!(v["status"], "accepted");
}

#[tokio::test]
async fn dashboard_auth_login_sets_cookie_and_allows_protected_route() {
    let mut cfg = default_test_config();
    cfg.api_key = String::new();
    cfg.auth.enabled = true;
    cfg.auth.username = "dash_boundary_user".to_string();
    cfg.auth.password_hash = openfang_api::session_auth::hash_password("correct-horse-battery");
    let server = start_server_with_config(cfg).await;
    let agents_url = format!("{}/api/agents", server.base_url);

    let client_plain = reqwest::Client::new();
    let denied = client_plain
        .post(&agents_url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("spawn no cookie");
    assert_eq!(denied.status(), reqwest::StatusCode::UNAUTHORIZED);

    let client = reqwest::Client::new();

    let login_url = format!("{}/api/auth/login", server.base_url);
    let login_res = client
        .post(&login_url)
        .json(&serde_json::json!({
            "username": "dash_boundary_user",
            "password": "correct-horse-battery"
        }))
        .send()
        .await
        .expect("login");
    assert_eq!(login_res.status(), reqwest::StatusCode::OK);

    let set_cookie = login_res
        .headers()
        .get(reqwest::header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .expect("set-cookie header");
    let cookie_pair = set_cookie.split(';').next().expect("cookie pair").trim();

    let ok = client
        .post(&agents_url)
        .header(reqwest::header::COOKIE, cookie_pair)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("spawn with cookie");
    assert_eq!(
        ok.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "authenticated request should reach handler (missing manifest)"
    );
}
