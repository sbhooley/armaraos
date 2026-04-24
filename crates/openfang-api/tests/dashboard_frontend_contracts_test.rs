//! Frontend contracts for the Fleet / Get started dashboard screens.
//!
//! These assertions intentionally validate cross-file logic contracts that are
//! easy to regress during UI refactors (without requiring a browser harness).

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn read_rel(path: &str) -> String {
    std::fs::read_to_string(repo_root().join(path)).expect("read file")
}

#[test]
fn get_started_saved_metrics_contract_uses_usage_summary_plus_quota_and_compression() {
    let overview = read_rel("crates/openfang-api/static/js/pages/overview.js");

    // Get started must load usage summary as its primary saved/spent source.
    assert!(overview.contains("OpenFangAPI.get('/api/usage/summary')"));
    assert!(overview.contains("compression_savings: summary.compression_savings || null"));
    assert!(overview.contains("quota_enforcement: summary.quota_enforcement || {}"));

    // Cost/tokens saved should combine compression+cache savings with quota-avoided totals.
    assert!(overview.contains("get overviewTokensSaved()"));
    assert!(overview.contains("get overviewCostSavedUsd()"));
    assert!(
        overview.contains("return (Number(cs.estimated_total_input_tokens_saved) || 0) + this.overviewQuotaInputAvoided;")
    );
    assert!(
        overview.contains("return (Number(cs.estimated_total_cost_saved_usd) || 0) + this.overviewQuotaCostAvoidedUsd;")
    );

    // Contracted fallback: when `compression_savings` is not populated yet, fallback to status eco.
    assert!(overview.contains(
        "Falls back to `/api/status` 7d eco when summary has no `compression_savings` yet."
    ));
}

#[test]
fn fleet_activity_line_contract_has_phase_mapping_and_live_store_bridge() {
    // The phase-mapping helpers (`getAgentActivityEntry`, `agentCurrentPhaseClass`,
    // `agentPhaseGlyph`) were extracted from `pages/agents.js` into the shared
    // `fleet-vitals-mixin.js` so that any fleet view (Agents grid, Overview, Fleet
    // card) gets identical glyphs for the same phase. The contract still applies —
    // we just assert it on the new owner of the logic.
    let mixin = read_rel("crates/openfang-api/static/js/fleet-vitals-mixin.js");
    let app = read_rel("crates/openfang-api/static/js/app.js");
    let html = read_rel("crates/openfang-api/static/index_body.html");

    // Fleet vitals mixin consumes shared live activity entries from the app-level store.
    assert!(mixin.contains("Alpine.store('app').agentActivityLines"));
    assert!(mixin.contains("getAgentActivityEntry: function(agent)"));
    assert!(mixin.contains("agentCurrentPhaseClass: function(agent)"));
    assert!(mixin.contains("agentPhaseGlyph: function(agent)"));
    assert!(mixin.contains("if (ph === 'thinking') return '…';"));
    assert!(mixin.contains("if (ph === 'tool') return '⚙';"));
    assert!(mixin.contains("if (ph === 'streaming') return '▸';"));
    assert!(mixin.contains("if (ph === 'running') return '●';"));

    // App-level SSE/system payload pipeline must continue to feed activity lines.
    assert!(app.contains("setAgentActivityLine(agentId, text)"));
    assert!(app.contains("if (this.dashboardPage !== 'agents') return false;"));
    assert!(app.contains("if (p.type === 'System' && p.data && p.data.event === 'AgentActivity')"));

    // Fleet card template contract for live line + glyph.
    assert!(html.contains("agent-vitals-live-line"));
    assert!(html.contains("agent-vitals-live-glyph"));
    assert!(html.contains("agent-vitals-live-text"));
}

#[test]
fn fleet_header_hides_demo_preset_controls_and_url_hint() {
    let html = read_rel("crates/openfang-api/static/index_body.html");

    assert!(!html.contains("Add <code class=\"fleet-code\">?demo=1</code>"));
    assert!(!html.contains("Standard Demo"));
    assert!(!html.contains("Cinema Demo"));
    assert!(!html.contains("title=\"Toggle demo\""));
    assert!(!html.contains("title=\"Toggle standard/cinema profile\""));
}
