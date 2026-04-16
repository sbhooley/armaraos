//! Runtime telemetry for eco-mode prompt compression effectiveness.
//!
//! Keeps lightweight in-memory aggregates so hosts can inspect real-world
//! savings (p50/p95 and token reduction) by mode and by agent.

use dashmap::DashMap;
use serde_json::json;
use std::collections::{BTreeMap, VecDeque};
use std::sync::OnceLock;

const SAMPLE_CAP: usize = 2048;

#[derive(Clone, Default)]
struct CompressionStats {
    turns: u64,
    compressed_turns: u64,
    original_tokens: u64,
    compressed_tokens: u64,
    savings_pct_samples: VecDeque<u8>,
}

impl CompressionStats {
    fn record(&mut self, original_tokens: u64, compressed_tokens: u64, savings_pct: u8) {
        self.turns = self.turns.saturating_add(1);
        if savings_pct > 0 {
            self.compressed_turns = self.compressed_turns.saturating_add(1);
        }
        self.original_tokens = self.original_tokens.saturating_add(original_tokens);
        self.compressed_tokens = self.compressed_tokens.saturating_add(compressed_tokens);
        if self.savings_pct_samples.len() >= SAMPLE_CAP {
            let _ = self.savings_pct_samples.pop_front();
        }
        self.savings_pct_samples.push_back(savings_pct);
    }
}

fn mode_totals() -> &'static DashMap<String, CompressionStats> {
    static MAP: OnceLock<DashMap<String, CompressionStats>> = OnceLock::new();
    MAP.get_or_init(DashMap::new)
}

fn agent_mode_totals() -> &'static DashMap<String, CompressionStats> {
    static MAP: OnceLock<DashMap<String, CompressionStats>> = OnceLock::new();
    MAP.get_or_init(DashMap::new)
}

fn approx_tok(s: &str) -> u64 {
    (s.len() / 4 + 1) as u64
}

fn pct(v: u64, total: u64) -> u64 {
    if total == 0 {
        0
    } else {
        100u64.saturating_sub((v.saturating_mul(100)) / total.max(1))
    }
}

fn sample_percentile(mut vals: Vec<u8>, p: f64) -> u8 {
    if vals.is_empty() {
        return 0;
    }
    vals.sort_unstable();
    let idx = ((vals.len().saturating_sub(1) as f64) * p).round() as usize;
    vals[idx.min(vals.len().saturating_sub(1))]
}

fn stats_to_json(s: &CompressionStats) -> serde_json::Value {
    let sample_vec: Vec<u8> = s.savings_pct_samples.iter().copied().collect();
    let mean = if sample_vec.is_empty() {
        0.0
    } else {
        sample_vec.iter().map(|v| *v as u64).sum::<u64>() as f64 / sample_vec.len() as f64
    };
    json!({
        "turns": s.turns,
        "compressed_turns": s.compressed_turns,
        "compression_hit_rate_pct": if s.turns == 0 { 0.0 } else { (s.compressed_turns as f64 * 100.0) / s.turns as f64 },
        "sample_count": sample_vec.len(),
        "savings_pct_p50": sample_percentile(sample_vec.clone(), 0.50),
        "savings_pct_p95": sample_percentile(sample_vec.clone(), 0.95),
        "savings_pct_mean": mean,
        "estimated_original_tokens": s.original_tokens,
        "estimated_compressed_tokens": s.compressed_tokens,
        "estimated_token_reduction_pct": pct(s.compressed_tokens, s.original_tokens),
    })
}

/// Record one completed turn's compression outcome.
pub fn record_turn(agent_id: &str, mode: &str, original_text: &str, compressed_text: &str, savings_pct: u8) {
    let mode = mode.trim().to_ascii_lowercase();
    let mode = if mode.is_empty() {
        "off".to_string()
    } else {
        mode
    };
    let orig_tok = approx_tok(original_text);
    let comp_tok = approx_tok(compressed_text);

    {
        let mut entry = mode_totals()
            .entry(mode.clone())
            .or_default();
        entry.record(orig_tok, comp_tok, savings_pct);
    }

    let agent_key = format!("{agent_id}::{mode}");
    let mut entry = agent_mode_totals()
        .entry(agent_key)
        .or_default();
    entry.record(orig_tok, comp_tok, savings_pct);
}

/// Snapshot aggregate eco-mode compression metrics.
pub fn snapshot_json() -> serde_json::Value {
    let mut modes: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for kv in mode_totals().iter() {
        modes.insert(kv.key().clone(), stats_to_json(kv.value()));
    }

    let mut by_agent: BTreeMap<String, BTreeMap<String, serde_json::Value>> = BTreeMap::new();
    for kv in agent_mode_totals().iter() {
        if let Some((agent_id, mode)) = kv.key().split_once("::") {
            by_agent
                .entry(agent_id.to_string())
                .or_default()
                .insert(mode.to_string(), stats_to_json(kv.value()));
        }
    }

    let agents: Vec<serde_json::Value> = by_agent
        .into_iter()
        .map(|(agent_id, mode_map)| {
            json!({
                "agent_id": agent_id,
                "modes": mode_map,
            })
        })
        .collect();

    json!({
        "modes": modes,
        "agents": agents,
    })
}

