//! Dashboard contract tests for `GET /api/usage/summary`.
//!
//! Verifies the endpoint returns the merged contract consumed by Get started /
//! Fleet UI: usage totals + quota enforcement + compression savings.

use openfang_api::routes::AppState;
use openfang_kernel::OpenFangKernel;
use openfang_memory::usage::{CompressionUsageRecord, QuotaBlockRecord, UsageRecord};
use openfang_types::agent::AgentId;
use openfang_types::config::{DefaultModelConfig, KernelConfig};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;

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

async fn spawn_test_server() -> TestServer {
    let tmp = tempfile::tempdir().expect("temp dir");
    let mut config = KernelConfig {
        home_dir: tmp.path().to_path_buf(),
        data_dir: tmp.path().join("data"),
        default_model: DefaultModelConfig {
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            api_key_env: "OLLAMA_API_KEY".to_string(),
            base_url: None,
        },
        ..KernelConfig::default()
    };
    // Keep tests deterministic and isolated.
    config.network_enabled = false;

    let kernel = OpenFangKernel::boot_with_config(config).expect("kernel boot");
    let kernel = Arc::new(kernel);
    kernel.set_self_handle();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    let (app, state) = openfang_api::server::build_router(kernel, addr).await;
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .expect("serve");
    });

    TestServer {
        base_url: format!("http://{}", addr),
        state,
        _tmp: tmp,
    }
}

async fn usage_summary_json(server: &TestServer) -> Value {
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("{}/api/usage/summary", server.base_url))
        .send()
        .await
        .expect("request")
        .error_for_status()
        .expect("200");
    resp.json::<Value>().await.expect("valid json")
}

fn f64_field(v: &Value, key: &str) -> f64 {
    v.get(key).and_then(Value::as_f64).unwrap_or_default()
}

fn u64_field(v: &Value, key: &str) -> u64 {
    v.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn pick_agent_id(server: &TestServer) -> AgentId {
    server
        .state
        .kernel
        .registry
        .list()
        .first()
        .map(|e| e.id)
        .expect("default assistant exists")
}

#[tokio::test]
async fn usage_summary_includes_dashboard_contract_fields_and_updates() {
    let server = spawn_test_server().await;
    let agent_id = pick_agent_id(&server);

    let before = usage_summary_json(&server).await;
    let before_calls = u64_field(&before, "call_count");
    let before_cost = f64_field(&before, "total_cost_usd");
    let before_tools = u64_field(&before, "total_tool_calls");

    let usage = server.state.kernel.memory.usage();
    usage
        .record(&UsageRecord {
            agent_id,
            model: "dashboard-fleet-model".to_string(),
            input_tokens: 300,
            output_tokens: 120,
            cost_usd: 0.1234,
            tool_calls: 2,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 40,
        })
        .expect("record usage");
    usage
        .record_quota_block(&QuotaBlockRecord {
            agent_id,
            reason: "dashboard_fleet_test_reason".to_string(),
            est_input_tokens: 222,
            est_output_tokens: 111,
            est_cost_usd: 0.02,
        })
        .expect("record quota block");
    usage
        .record_compression(&CompressionUsageRecord {
            agent_id,
            mode: "balanced".to_string(),
            model: "dashboard-fleet-model".to_string(),
            provider: "dashboard-provider".to_string(),
            original_tokens_est: 200,
            compressed_tokens_est: 140,
            input_tokens_saved: 60,
            input_price_per_million_usd: 2.0,
            est_input_cost_saved_usd: 60.0 * 2.0 / 1_000_000.0,
            billed_input_tokens: 140,
            billed_input_cost_usd: 140.0 * 2.0 / 1_000_000.0,
            savings_pct: 30,
            semantic_preservation_score: Some(0.91),
        })
        .expect("record compression");

    let after = usage_summary_json(&server).await;

    // Base usage totals: endpoint must expose `query_summary(None)` data.
    assert!(
        u64_field(&after, "call_count") > before_calls,
        "call_count should increase after recording usage event"
    );
    assert!(
        f64_field(&after, "total_cost_usd") >= before_cost + 0.1233,
        "total_cost_usd should include recorded usage cost"
    );
    assert!(
        u64_field(&after, "total_tool_calls") >= before_tools + 2,
        "total_tool_calls should include recorded tool calls"
    );

    // Quota enforcement block must be merged into usage summary payload.
    let quota = after
        .get("quota_enforcement")
        .and_then(Value::as_object)
        .expect("quota_enforcement object");
    assert!(
        quota
            .get("block_count")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            >= 1
    );
    assert!(
        quota
            .get("total_est_cost_avoided_usd")
            .and_then(Value::as_f64)
            .unwrap_or_default()
            >= 0.02
    );
    let by_reason = quota
        .get("by_reason")
        .and_then(Value::as_object)
        .expect("quota by_reason");
    let reason_entry = by_reason
        .get("dashboard_fleet_test_reason")
        .and_then(Value::as_object)
        .expect("reason row present");
    assert_eq!(
        reason_entry
            .get("count")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
        1
    );

    // Compression savings must also be merged and include by-provider/model rows.
    let cs = after
        .get("compression_savings")
        .and_then(Value::as_object)
        .expect("compression_savings object");
    assert!(
        cs.get("estimated_total_input_tokens_saved")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            >= 60
    );
    assert!(
        cs.get("estimated_total_cost_saved_usd")
            .and_then(Value::as_f64)
            .unwrap_or_default()
            > 0.0
    );
    let rows = cs
        .get("by_provider_model")
        .and_then(Value::as_array)
        .expect("by_provider_model array");
    let row = rows
        .iter()
        .find(|r| {
            r.get("provider").and_then(Value::as_str) == Some("dashboard-provider")
                && r.get("model").and_then(Value::as_str) == Some("dashboard-fleet-model")
        })
        .expect("dashboard provider/model row");
    assert!(row.get("turns").and_then(Value::as_u64).unwrap_or_default() >= 1);
    assert!(
        row.get("input_tokens_saved")
            .and_then(Value::as_u64)
            .unwrap_or_default()
            >= 60
    );
}
