//! Replay-style evaluation harness for adaptive eco (shadow resolver + circuit breaker).
//!
//! Run:
//! ```text
//! cargo test -p openfang-runtime adaptive_eco_eval -- --nocapture
//! ```
//!
//! This is the **regression harness** comparing representative traces under fixed config: it does not
//! hit the network or full kernel — use it to validate policy changes before enabling **`enforce`**.

use openfang_runtime::eco_mode_resolver::{circuit_breaker_adjust_base, resolve_adaptive_eco_turn};
use openfang_runtime::model_catalog::ModelCatalog;
use openfang_types::adaptive_eco::AdaptiveEcoConfig;
use openfang_types::agent::AgentManifest;

fn manifest(mode: &str, provider: &str) -> AgentManifest {
    let mut m = AgentManifest::default();
    m.model.provider = provider.to_string();
    m.model.model = "claude-sonnet-4-20250514".to_string();
    m.metadata.insert(
        "efficient_mode".to_string(),
        serde_json::Value::String(mode.to_string()),
    );
    m
}

#[test]
fn adaptive_eco_eval_trace_matrix_smoke() {
    let cat = ModelCatalog::new();
    let structured_msg = format!(
        "{}\n```json\n{{\"x\":1}}\n```\n```sql\nSELECT 1;\n```\n",
        "{\"a\":1,\"b\":2,\"c\":3,\"d\":4,\"e\":5,\"f\":6,\"g\":7,\"h\":8}".repeat(8)
    );
    let traces: &[(&str, &str, &str)] = &[
        (
            "conversational",
            "balanced",
            "Please summarize yesterday's standup in three bullets.",
        ),
        ("structured_json", "aggressive", structured_msg.as_str()),
        (
            "technical",
            "balanced",
            "R http.GET https://example.com/api?v=1 ->res",
        ),
    ];

    let cfg = AdaptiveEcoConfig {
        enabled: true,
        allow_aggressive_on_structured: false,
        ..Default::default()
    };

    let mut structured_recommendation_ok = false;
    for (name, base, msg) in traces {
        let man = manifest(base, "anthropic");
        let snap = resolve_adaptive_eco_turn(&cfg, &man, msg, &cat);
        assert!(
            snap.reason_codes
                .iter()
                .any(|s| s.contains("adaptive_eco:v1")),
            "{name}: expected resolver version tag"
        );
        if *name == "structured_json" {
            assert_eq!(
                snap.recommended_mode, "balanced",
                "structured trace should cap aggressive recommendation when allow_aggressive_on_structured is false"
            );
            structured_recommendation_ok = true;
        }
    }
    assert!(structured_recommendation_ok);
}

#[test]
fn adaptive_eco_eval_circuit_breaker_semantic_regression() {
    let cfg = AdaptiveEcoConfig {
        circuit_breaker_enabled: true,
        semantic_floor: 0.85,
        circuit_breaker_window: 6,
        circuit_breaker_min_below_floor: 2,
        ..Default::default()
    };
    let scores_bad = vec![0.70_f32, 0.71_f32, 0.72_f32];
    let (m, trip) = circuit_breaker_adjust_base("aggressive", &cfg, &scores_bad);
    assert!(
        trip || m != "aggressive",
        "bad semantics should step down or trip from aggressive"
    );
    let scores_ok = vec![0.92_f32, 0.93_f32, 0.94_f32];
    let (m2, trip2) = circuit_breaker_adjust_base("aggressive", &cfg, &scores_ok);
    assert!(!trip2);
    assert_eq!(m2, "aggressive");
}
