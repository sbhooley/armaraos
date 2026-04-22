//! Usage tracking store — records LLM usage events for cost monitoring.

use crate::MemorySqlitePool;
use chrono::Utc;
use openfang_types::adaptive_eco::{
    AdaptiveEcoReplayReport, AdaptiveEcoUsageRecord, AdaptiveEcoUsageSummary,
};
use openfang_types::agent::AgentId;
use openfang_types::error::{OpenFangError, OpenFangResult};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A single usage event recording an LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    /// Which agent made the call.
    pub agent_id: AgentId,
    /// Model used.
    pub model: String,
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens consumed.
    pub output_tokens: u64,
    /// Estimated cost in USD.
    pub cost_usd: f64,
    /// Number of tool calls in this interaction.
    pub tool_calls: u32,
    /// Prompt-cache tokens written this turn (provider-specific).
    pub cache_creation_input_tokens: u64,
    /// Prompt-cache tokens read this turn (provider-specific).
    pub cache_read_input_tokens: u64,
}

/// Summary of usage over a period.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageSummary {
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Total estimated cost in USD.
    pub total_cost_usd: f64,
    /// Total number of calls.
    pub call_count: u64,
    /// Total tool calls.
    pub total_tool_calls: u64,
}

/// Usage grouped by model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelUsage {
    /// Model name.
    pub model: String,
    /// Total cost for this model.
    pub total_cost_usd: f64,
    /// Total input tokens.
    pub total_input_tokens: u64,
    /// Total output tokens.
    pub total_output_tokens: u64,
    /// Number of calls.
    pub call_count: u64,
}

/// OpenRouter (and similar) `…:free` routes: treat as **$0** marginal in DB repairs and
/// [UsageStore::backfill_compression_pricing] skips, so we never re-price from catalog.
fn is_marginal_free_model_id(model: &str) -> bool {
    model.to_ascii_lowercase().contains(":free")
}

/// Daily usage breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyBreakdown {
    /// Date string (YYYY-MM-DD).
    pub date: String,
    /// Total cost for this day.
    pub cost_usd: f64,
    /// Total tokens (input + output).
    pub tokens: u64,
    /// Number of API calls.
    pub calls: u64,
}

/// Durable prompt-compression telemetry event.
///
/// One row per agent turn that ran through the prompt compressor (including `off` for audits).
/// All token counts are pre-LLM heuristic estimates **except** [Self::billed_input_tokens] and
/// [Self::billed_input_cost_usd], which are populated from the provider's reported `usage` after
/// the LLM returns (so dashboards can show *true* input billed vs the pre-compression input).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionUsageRecord {
    pub agent_id: AgentId,
    pub mode: String,
    /// Model id used for this turn (must match [UsageRecord::model] / model catalog key).
    pub model: String,
    /// Provider id (e.g. `anthropic`, `openai`, `openrouter`) — empty if unknown.
    #[serde(default)]
    pub provider: String,
    pub original_tokens_est: u64,
    pub compressed_tokens_est: u64,
    /// `original_tokens_est` − `compressed_tokens_est` (input side); persisted for audits.
    pub input_tokens_saved: u64,
    /// Snapshot: catalog input $/1M in USD for `model` at insert time.
    pub input_price_per_million_usd: f64,
    /// (input_tokens_saved / 1e6) * input_price_per_million_usd — not billable, counterfactual savings.
    pub est_input_cost_saved_usd: f64,
    /// Provider-reported input tokens for this turn (post-compression, what the provider actually billed).
    /// `0` means "unknown" — older turns recorded before v15 will report 0 here.
    #[serde(default)]
    pub billed_input_tokens: u64,
    /// Catalog-priced cost of [Self::billed_input_tokens] in USD.
    #[serde(default)]
    pub billed_input_cost_usd: f64,
    pub savings_pct: u8,
    pub semantic_preservation_score: Option<f32>,
}

impl CompressionUsageRecord {
    fn input_tokens_saturated(&self) -> u64 {
        if self.input_tokens_saved > 0 {
            self.input_tokens_saved
        } else {
            self.original_tokens_est
                .saturating_sub(self.compressed_tokens_est)
        }
    }
}

/// Aggregated compression effectiveness for a mode bucket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionModeSummary {
    pub turns: u64,
    pub compressed_turns: u64,
    pub compression_hit_rate_pct: f64,
    pub sample_count: usize,
    pub savings_pct_p50: u8,
    pub savings_pct_p95: u8,
    pub savings_pct_mean: f64,
    pub semantic_score_mean: Option<f64>,
    pub semantic_score_p50: Option<f64>,
    pub semantic_score_p95: Option<f64>,
    pub semantic_score_samples: usize,
    pub estimated_original_tokens: u64,
    pub estimated_compressed_tokens: u64,
    pub estimated_token_reduction_pct: f64,
}

/// Compression summary by mode and by agent/mode.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionSummary {
    pub window: String,
    pub modes: BTreeMap<String, CompressionModeSummary>,
    pub agents: Vec<CompressionAgentSummary>,
    pub estimated_compression_tokens_saved: u64,
    pub cache_read_input_tokens: u64,
    pub estimated_total_input_tokens_saved: u64,
    pub estimated_cache_cost_saved_usd: f64,
    pub estimated_compression_cost_saved_usd: f64,
    pub estimated_total_cost_saved_usd: f64,
    /// Sum of pre-compression input tokens across recorded compression turns (audit baseline).
    #[serde(default)]
    pub original_input_tokens_total: u64,
    /// Sum of provider-reported (billed) input tokens across recorded compression turns.
    /// `0` if no rows include billed counts (pre-v15 data only).
    #[serde(default)]
    pub billed_input_tokens_total: u64,
    /// Sum of catalog-priced input cost actually paid for the rows above (post-compression).
    #[serde(default)]
    pub billed_input_cost_usd_total: f64,
    /// Per-provider/model rollup of original vs billed input tokens & cost.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_provider_model: Vec<CompressionProviderModelRollup>,
    /// Same window as compression rows: adaptive eco summary + full replay report (also on dedicated endpoints).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adaptive_eco: Option<CompressionAdaptiveEcoBundle>,
}

/// One row of the (provider, model) rollup attached to [CompressionSummary::by_provider_model].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionProviderModelRollup {
    pub provider: String,
    pub model: String,
    pub turns: u64,
    pub original_input_tokens: u64,
    pub compressed_input_tokens: u64,
    pub billed_input_tokens: u64,
    pub input_tokens_saved: u64,
    pub input_price_per_million_usd: f64,
    pub est_input_cost_saved_usd: f64,
    pub billed_input_cost_usd: f64,
}

/// Catalog-priced snapshot for a single model id, returned by the closure passed into
/// [`UsageStore::backfill_compression_pricing`]. Lets the memory crate stay decoupled from the
/// kernel's `ModelCatalog` while still using current catalog pricing/provider mapping to repair
/// pre-v15 rows.
#[derive(Debug, Clone)]
pub struct CompressionPricingSnapshot {
    /// Provider id (e.g. `anthropic`, `openrouter`) sourced from the catalog entry.
    pub provider: String,
    /// Catalog input $/1M (USD). `0.0` is treated as "unknown" and skips the price write.
    pub input_per_million_usd: f64,
}

/// Adaptive eco telemetry bundled for `GET /api/usage/compression` (same `window` as parent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionAdaptiveEcoBundle {
    pub summary: AdaptiveEcoUsageSummary,
    pub replay: AdaptiveEcoReplayReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionAgentSummary {
    pub agent_id: String,
    pub modes: BTreeMap<String, CompressionModeSummary>,
}

/// One persisted row when a turn is blocked by token or cost quotas / global budget (before LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaBlockRecord {
    pub agent_id: AgentId,
    pub reason: String,
    pub est_input_tokens: u64,
    pub est_output_tokens: u64,
    pub est_cost_usd: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotaBlockReasonRollup {
    pub count: u64,
    pub est_input_tokens: u64,
    pub est_output_tokens: u64,
    pub est_cost_usd: f64,
}

/// Aggregated quota-block telemetry for dashboards (`GET /api/usage/summary`, analytics).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QuotaBlockSummary {
    pub window: String,
    pub block_count: u64,
    pub total_est_input_tokens: u64,
    pub total_est_output_tokens: u64,
    pub total_est_cost_usd: f64,
    pub by_reason: BTreeMap<String, QuotaBlockReasonRollup>,
}

/// Usage store backed by SQLite.
#[derive(Clone)]
pub struct UsageStore {
    pool: MemorySqlitePool,
}

impl UsageStore {
    /// Create a new usage store wrapping the given pool.
    pub fn new(pool: MemorySqlitePool) -> Self {
        Self { pool }
    }

    /// Record a usage event.
    pub fn record(&self, record: &UsageRecord) -> OpenFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO usage_events (id, agent_id, timestamp, model, input_tokens, output_tokens, cost_usd, tool_calls)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.model,
                record.input_tokens as i64,
                record.output_tokens as i64,
                record.cost_usd,
                record.tool_calls as i64,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        conn.execute(
            "UPDATE usage_events
             SET cache_creation_input_tokens = ?1, cache_read_input_tokens = ?2
             WHERE id = ?3",
            rusqlite::params![
                record.cache_creation_input_tokens as i64,
                record.cache_read_input_tokens as i64,
                id,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Record a quota / budget block (turn not started — estimates only).
    pub fn record_quota_block(&self, record: &QuotaBlockRecord) -> OpenFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO quota_block_events (id, timestamp, agent_id, reason, est_input_tokens, est_output_tokens, est_cost_usd)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                id,
                now,
                record.agent_id.0.to_string(),
                record.reason,
                record.est_input_tokens as i64,
                record.est_output_tokens as i64,
                record.est_cost_usd,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Aggregate quota-block rows for dashboards.
    ///
    /// `window_days`: `Some(N)` => last N days; `None` => all time.
    pub fn query_quota_block_summary(
        &self,
        window_days: Option<u32>,
    ) -> OpenFangResult<QuotaBlockSummary> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let window = window_days
            .map(|d| format!("{d}d"))
            .unwrap_or_else(|| "all".to_string());
        let where_clause = match window_days {
            Some(d) => format!("WHERE timestamp >= datetime('now', '-{d} days')"),
            None => String::new(),
        };

        let totals: (u64, u64, u64, f64) = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*),
                            COALESCE(SUM(est_input_tokens), 0),
                            COALESCE(SUM(est_output_tokens), 0),
                            COALESCE(SUM(est_cost_usd), 0.0)
                     FROM quota_block_events {}",
                    where_clause
                ),
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                        row.get::<_, f64>(3)?,
                    ))
                },
            )
            .unwrap_or((0, 0, 0, 0.0));

        let mut stmt = conn
            .prepare(&format!(
                "SELECT reason,
                        COUNT(*),
                        COALESCE(SUM(est_input_tokens), 0),
                        COALESCE(SUM(est_output_tokens), 0),
                        COALESCE(SUM(est_cost_usd), 0.0)
                 FROM quota_block_events {}
                 GROUP BY reason
                 ORDER BY COUNT(*) DESC",
                where_clause
            ))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let mut by_reason = BTreeMap::new();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, f64>(4)?,
                ))
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        for r in rows {
            let (reason, count, tin, tout, cost) = r.map_err(|e| OpenFangError::Memory(e.to_string()))?;
            by_reason.insert(
                reason,
                QuotaBlockReasonRollup {
                    count,
                    est_input_tokens: tin,
                    est_output_tokens: tout,
                    est_cost_usd: cost,
                },
            );
        }

        Ok(QuotaBlockSummary {
            window,
            block_count: totals.0,
            total_est_input_tokens: totals.1,
            total_est_output_tokens: totals.2,
            total_est_cost_usd: totals.3,
            by_reason,
        })
    }

    /// Recent semantic preservation scores (newest first), up to `limit`, excluding NULLs.
    pub fn query_recent_semantic_scores(
        &self,
        agent_id: AgentId,
        limit: usize,
    ) -> OpenFangResult<Vec<f32>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let lim = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut stmt = conn
            .prepare(
                "SELECT semantic_preservation_score FROM eco_compression_events
                 WHERE agent_id = ?1 AND semantic_preservation_score IS NOT NULL
                 ORDER BY timestamp DESC
                 LIMIT ?2",
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![agent_id.0.to_string(), lim], |row| {
                row.get::<_, f64>(0)
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let mut out = Vec::new();
        for r in rows {
            let v = r.map_err(|e| OpenFangError::Memory(e.to_string()))?;
            out.push(v as f32);
        }
        Ok(out)
    }

    /// Persist one adaptive-eco turn (shadow or enforced).
    pub fn record_adaptive_eco(&self, record: &AdaptiveEcoUsageRecord) -> OpenFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let codes = serde_json::to_string(&record.reason_codes)
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let counterfactual_json = record
            .counterfactual
            .as_ref()
            .and_then(|c| serde_json::to_string(c).ok());
        conn.execute(
            "INSERT INTO adaptive_eco_events (
                id, agent_id, timestamp,
                effective_mode, recommended_mode, base_mode_before_circuit,
                circuit_breaker_tripped, hysteresis_blocked,
                shadow_only, enforce,
                provider, model, cache_capability, input_price_per_million,
                reason_codes_json, semantic_preservation_score, adaptive_confidence, counterfactual_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.effective_mode.to_ascii_lowercase(),
                record.recommended_mode.to_ascii_lowercase(),
                record.base_mode_before_circuit.as_deref(),
                if record.circuit_breaker_tripped { 1i64 } else { 0i64 },
                if record.hysteresis_blocked { 1i64 } else { 0i64 },
                if record.shadow_only { 1i64 } else { 0i64 },
                if record.enforce { 1i64 } else { 0i64 },
                record.provider,
                record.model,
                record.cache_capability,
                record.input_price_per_million,
                codes,
                record.semantic_preservation_score.map(|v| v as f64),
                record.adaptive_confidence.map(|v| v as f64),
                counterfactual_json,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }

    #[cfg(test)]
    /// Test helper: read persisted `counterfactual_json` for the latest adaptive eco row.
    pub fn test_last_adaptive_eco_counterfactual_json(
        &self,
        agent_id: AgentId,
    ) -> OpenFangResult<Option<String>> {
        use rusqlite::OptionalExtension;
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let v = conn
            .query_row(
                "SELECT counterfactual_json FROM adaptive_eco_events WHERE agent_id = ?1 ORDER BY timestamp DESC LIMIT 1",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(v.flatten())
    }

    /// Aggregate adaptive eco events for dashboards.
    pub fn query_adaptive_eco_summary(
        &self,
        window_days: Option<u32>,
    ) -> OpenFangResult<AdaptiveEcoUsageSummary> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let time_filter = window_days
            .map(|d| format!("timestamp > datetime('now', '-{} days')", d))
            .unwrap_or_else(|| "1=1".to_string());

        let events: u64 = conn
            .query_row(
                &format!("SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}"),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let mismatch: u64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND shadow_only = 1 AND recommended_mode != effective_mode"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let trips: u64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND circuit_breaker_tripped = 1"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let blocks: u64 = conn
            .query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND hysteresis_blocked = 1"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        Ok(AdaptiveEcoUsageSummary {
            window: window_days
                .map(|d| format!("{d}d"))
                .unwrap_or_else(|| "all".to_string()),
            events,
            shadow_mismatch_turns: mismatch,
            circuit_breaker_trips: trips,
            hysteresis_blocks: blocks,
        })
    }

    /// Aggregate adaptive eco + compression semantics for policy replay / audits (`GET .../replay`).
    pub fn query_adaptive_eco_replay_report(
        &self,
        window_days: Option<u32>,
    ) -> OpenFangResult<AdaptiveEcoReplayReport> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let time_filter = window_days
            .map(|d| format!("timestamp > datetime('now', '-{} days')", d))
            .unwrap_or_else(|| "1=1".to_string());

        let window_str = window_days
            .map(|d| format!("{d}d"))
            .unwrap_or_else(|| "all".to_string());

        let adaptive_eco_events: u64 =
            conn.query_row(
                &format!("SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}"),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let shadow_mismatch_turns: u64 =
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND shadow_only = 1 AND recommended_mode != effective_mode"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let circuit_breaker_trips: u64 =
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND circuit_breaker_tripped = 1"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let hysteresis_blocks: u64 =
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM adaptive_eco_events WHERE {time_filter}
                     AND hysteresis_blocked = 1"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let eco_compression_turns: u64 =
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM eco_compression_events WHERE {time_filter}
                     AND savings_pct > 0"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT semantic_preservation_score FROM eco_compression_events
                 WHERE {time_filter} AND semantic_preservation_score IS NOT NULL
                 ORDER BY semantic_preservation_score"
            ))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let scores: Vec<f64> = stmt
            .query_map([], |row| row.get::<_, f64>(0))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let n = scores.len();
        let (compression_semantic_p50, compression_semantic_p95, compression_semantic_mean) =
            if n == 0 {
                (None, None, None)
            } else {
                let mean = Some(scores.iter().sum::<f64>() / n as f64);
                let p50 = Some(scores[n / 2]);
                let p95_idx = (((n.saturating_sub(1)) as f64 * 0.95).round() as usize).min(n - 1);
                let p95 = Some(scores[p95_idx]);
                (p50, p95, mean)
            };

        let shadow_mismatch_rate = if adaptive_eco_events > 0 {
            shadow_mismatch_turns as f64 / adaptive_eco_events as f64
        } else {
            0.0
        };

        let effective_mode_flip_turns: u64 =
            conn.query_row(
                &format!(
                    "SELECT COUNT(*) FROM (
                        SELECT
                            LAG(effective_mode) OVER (
                                PARTITION BY agent_id ORDER BY timestamp
                            ) AS prev_mode,
                            effective_mode
                        FROM adaptive_eco_events
                        WHERE {time_filter}
                    ) WHERE prev_mode IS NOT NULL AND prev_mode != effective_mode"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let effective_mode_transition_slots: u64 =
            conn.query_row(
                &format!(
                    "SELECT COALESCE(SUM(cnt - 1), 0) FROM (
                        SELECT COUNT(*) AS cnt FROM adaptive_eco_events
                        WHERE {time_filter}
                        GROUP BY agent_id
                    )"
                ),
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;

        let effective_mode_flip_rate = if effective_mode_transition_slots > 0 {
            effective_mode_flip_turns as f64 / effective_mode_transition_slots as f64
        } else {
            0.0
        };

        let mut stmt_conf = conn
            .prepare(&format!(
                "SELECT adaptive_confidence FROM adaptive_eco_events
                 WHERE {time_filter} AND adaptive_confidence IS NOT NULL
                 ORDER BY adaptive_confidence"
            ))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let confidences: Vec<f64> = stmt_conf
            .query_map([], |row| row.get::<_, f64>(0))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let nc = confidences.len();
        let adaptive_confidence_samples = nc as u64;
        let (adaptive_confidence_p50, adaptive_confidence_p95, adaptive_confidence_mean) =
            if nc == 0 {
                (None, None, None)
            } else {
                let mean = Some(confidences.iter().sum::<f64>() / nc as f64);
                let p50 = Some(confidences[nc / 2]);
                let p95_idx = (((nc.saturating_sub(1)) as f64 * 0.95).round() as usize).min(nc - 1);
                let p95 = Some(confidences[p95_idx]);
                (p50, p95, mean)
            };

        let mut adaptive_confidence_bucket_low: u64 = 0;
        let mut adaptive_confidence_bucket_mid: u64 = 0;
        let mut adaptive_confidence_bucket_high: u64 = 0;
        for v in &confidences {
            if *v < 0.33 {
                adaptive_confidence_bucket_low += 1;
            } else if *v < 0.66 {
                adaptive_confidence_bucket_mid += 1;
            } else {
                adaptive_confidence_bucket_high += 1;
            }
        }

        Ok(AdaptiveEcoReplayReport {
            window: window_str,
            adaptive_eco_events,
            shadow_mismatch_turns,
            shadow_mismatch_rate,
            circuit_breaker_trips,
            hysteresis_blocks,
            eco_compression_turns,
            compression_semantic_p50,
            compression_semantic_p95,
            compression_semantic_mean,
            effective_mode_flip_turns,
            effective_mode_transition_slots,
            effective_mode_flip_rate,
            adaptive_confidence_samples,
            adaptive_confidence_p50,
            adaptive_confidence_p95,
            adaptive_confidence_mean,
            adaptive_confidence_bucket_low,
            adaptive_confidence_bucket_mid,
            adaptive_confidence_bucket_high,
        })
    }

    /// Record a prompt-compression telemetry event.
    pub fn record_compression(&self, record: &CompressionUsageRecord) -> OpenFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let input_saved = record.input_tokens_saturated();
        // Use the kernel snapshot as-is. `0.0` is valid (OpenRouter `:free`, local $0, catalog
        // explicit zeros). We must *not* overlay `estimate_input_per_million` here: that
        // heuristic defaults to $1/M for unknown ids and would inflate "USD not spent" and
        // `billed_input_cost_usd` for free tier — especially with whole-prompt `input_tokens_saved`
        // (M1). Pre-v15 rows with missing price are repaired by `backfill_compression_pricing`.
        let price = record.input_price_per_million_usd;
        let mut usd = record.est_input_cost_saved_usd;
        if usd == 0.0 && input_saved > 0 && price > 0.0 {
            usd = (input_saved as f64 / 1_000_000.0) * price;
        }
        let billed_input_tokens = record.billed_input_tokens;
        let billed_input_cost_usd = if record.billed_input_cost_usd > 0.0 {
            record.billed_input_cost_usd
        } else if billed_input_tokens > 0 && price > 0.0 {
            (billed_input_tokens as f64 / 1_000_000.0) * price
        } else {
            0.0
        };
        conn.execute(
            "INSERT INTO eco_compression_events (
                id, agent_id, timestamp, mode, model,
                original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                input_tokens_saved, input_price_per_million_usd, est_input_cost_saved_usd,
                provider, billed_input_tokens, billed_input_cost_usd
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.mode.to_ascii_lowercase(),
                record.model.as_str(),
                record.original_tokens_est as i64,
                record.compressed_tokens_est as i64,
                record.savings_pct as i64,
                record.semantic_preservation_score.map(|v| v as f64),
                input_saved as i64,
                price,
                usd,
                record.provider.as_str(),
                billed_input_tokens as i64,
                billed_input_cost_usd,
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Idempotent one-shot repair of historical `eco_compression_events` rows that were persisted
    /// **before schema v15** (or by a code path that didn't snapshot pricing).
    ///
    /// Symptoms this fixes on the dashboard:
    /// - `compression_savings.estimated_compression_cost_saved_usd` stuck near `$0.00` despite
    ///   thousands of compressed turns ("USD NOT SPENT (EST.)" reads `<$0.01`).
    /// - `by_provider_model[]` rows missing `provider`, weighted price = 0.
    ///
    /// For every row where we have evidence the turn happened (`input_tokens_saved > 0` OR
    /// `billed_input_tokens > 0` OR `original_tokens_est > 0`) AND any of the priced fields are
    /// zero / provider is blank, we look up the model in `lookup` and:
    ///
    /// 1. Set `input_price_per_million_usd` to the catalog snapshot when missing.
    /// 2. Recompute `est_input_cost_saved_usd = (input_tokens_saved / 1e6) * price` if zero.
    /// 3. Recompute `billed_input_cost_usd = (billed_input_tokens / 1e6) * price` if zero.
    /// 4. Fill `provider` from the catalog when blank.
    ///
    /// Already-priced rows are NOT mutated — once a turn captures a pricing snapshot we keep it
    /// (catalog prices change over time; we want history to reflect the price at insert time).
    /// Returns the number of rows actually updated.
    pub fn backfill_compression_pricing<F>(&self, lookup: F) -> OpenFangResult<u64>
    where
        F: Fn(&str) -> Option<CompressionPricingSnapshot>,
    {
        let mut conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        // Snapshot of repair candidates. We materialize before opening the write transaction so
        // we don't hold the SELECT statement open across UPDATEs (rusqlite borrow rules).
        let candidates: Vec<(String, String, String, i64, i64, f64, f64, f64)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, model, COALESCE(provider, ''),
                            COALESCE(input_tokens_saved, 0),
                            COALESCE(billed_input_tokens, 0),
                            COALESCE(input_price_per_million_usd, 0.0),
                            COALESCE(est_input_cost_saved_usd, 0.0),
                            COALESCE(billed_input_cost_usd, 0.0)
                     FROM eco_compression_events
                     WHERE COALESCE(input_price_per_million_usd, 0.0) <= 0.0
                        OR (COALESCE(input_tokens_saved, 0) > 0
                            AND COALESCE(est_input_cost_saved_usd, 0.0) <= 0.0)
                        OR (COALESCE(billed_input_tokens, 0) > 0
                            AND COALESCE(billed_input_cost_usd, 0.0) <= 0.0)
                        OR COALESCE(provider, '') = ''",
                )
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, f64>(6)?,
                        row.get::<_, f64>(7)?,
                    ))
                })
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| OpenFangError::Memory(e.to_string()))?);
            }
            out
        };

        if candidates.is_empty() {
            return Ok(0);
        }

        let tx = conn
            .transaction()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let mut updated: u64 = 0;
        for (id, model, provider, saved_i, billed_i, mut price, mut est_saved, mut billed_cost) in
            candidates
        {
            if is_marginal_free_model_id(&model) {
                continue;
            }
            let saved = saved_i.max(0) as u64;
            let billed = billed_i.max(0) as u64;
            let snapshot = match lookup(&model) {
                Some(s) => s,
                None => continue,
            };

            let mut changed = false;
            if price <= 0.0 && snapshot.input_per_million_usd > 0.0 {
                price = snapshot.input_per_million_usd;
                changed = true;
            }
            if est_saved <= 0.0 && saved > 0 && price > 0.0 {
                est_saved = (saved as f64 / 1_000_000.0) * price;
                changed = true;
            }
            if billed_cost <= 0.0 && billed > 0 && price > 0.0 {
                billed_cost = (billed as f64 / 1_000_000.0) * price;
                changed = true;
            }
            let new_provider = if provider.is_empty() {
                snapshot.provider.clone()
            } else {
                provider.clone()
            };
            if new_provider != provider {
                changed = true;
            }
            if !changed {
                continue;
            }
            tx.execute(
                "UPDATE eco_compression_events
                    SET input_price_per_million_usd = ?1,
                        est_input_cost_saved_usd = ?2,
                        billed_input_cost_usd = ?3,
                        provider = ?4
                  WHERE id = ?5",
                rusqlite::params![price, est_saved, billed_cost, new_provider, id],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            updated += 1;
        }
        tx.commit()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(updated)
    }

    /// Idempotent one-shot repair: backfills `billed_input_tokens` (and the matching
    /// `billed_input_cost_usd`) on historical compression rows by joining to the closest
    /// `usage_events` row for the same agent within ±60 seconds of the compression event.
    ///
    /// Why this works: every compression turn writes a `usage_events` row a few milliseconds
    /// before / after the `eco_compression_events` row in the same kernel turn-handler. So we
    /// can recover the **provider-billed** input token count for pre-v15 rows that only have
    /// the heuristic `compressed_tokens_est`.
    ///
    /// Returns the number of rows updated. Should be called **after**
    /// [`UsageStore::backfill_compression_pricing`] so the cost recomputation has a price to
    /// multiply against.
    pub fn backfill_compression_billed_tokens(&self) -> OpenFangResult<u64> {
        let mut conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        // SQLite cross-version note: a correlated UPDATE that references the target table by
        // name inside the subquery (e.g. `eco_compression_events.timestamp`) is rejected by
        // some bundled rusqlite/SQLite versions. We therefore do the join in Rust — collect
        // candidate (id, agent_id, timestamp) tuples first, look up the closest usage event in
        // a follow-up query per row, then UPDATE in a transaction.
        let candidates: Vec<(String, String, String)> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id, agent_id, timestamp
                       FROM eco_compression_events
                      WHERE COALESCE(billed_input_tokens, 0) = 0",
                )
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(|e| OpenFangError::Memory(e.to_string()))?);
            }
            out
        };

        if candidates.is_empty() {
            return Ok(0);
        }

        let tx = conn
            .transaction()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut copied: u64 = 0;
        for (row_id, agent_id, ts) in candidates {
            // Closest usage_events.input_tokens for this agent within ±60s of the comp row.
            let billed: Option<i64> = tx
                .query_row(
                    "SELECT input_tokens FROM usage_events
                      WHERE agent_id = ?1
                        AND ABS(julianday(timestamp) - julianday(?2)) < (60.0 / 86400.0)
                        AND input_tokens > 0
                      ORDER BY ABS(julianday(timestamp) - julianday(?2))
                      LIMIT 1",
                    rusqlite::params![agent_id, ts],
                    |r| r.get::<_, i64>(0),
                )
                .ok();
            let Some(b) = billed.filter(|n| *n > 0) else {
                continue;
            };
            tx.execute(
                "UPDATE eco_compression_events
                    SET billed_input_tokens = ?1
                  WHERE id = ?2",
                rusqlite::params![b, row_id],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            copied += 1;
        }

        // Recompute billed_input_cost_usd for any rows that now have tokens + a price snapshot
        // but were never priced. This statement is target-only (no correlated subquery) so it
        // works on every SQLite version we ship with.
        let _ = tx
            .execute(
                "UPDATE eco_compression_events
                    SET billed_input_cost_usd =
                        (CAST(billed_input_tokens AS REAL) / 1000000.0)
                        * input_price_per_million_usd
                  WHERE COALESCE(billed_input_tokens, 0) > 0
                    AND COALESCE(input_price_per_million_usd, 0.0) > 0.0
                    AND COALESCE(billed_input_cost_usd, 0.0) <= 0.0",
                [],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        tx.commit()
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        Ok(copied)
    }

    /// One-shot repair: rows whose model id contains a **:free** route (OpenRouter-style) should
    /// have **$0** cost in analytics, not a legacy $1/M unknown-model fallback. Idempotent: safe
    /// to re-run; only touches `cost_usd` on `usage_events` and the dollar/price fields on
    /// `eco_compression_events` for matching models.
    ///
    /// Return `(usage_events rows updated, eco_compression_events rows updated)`.
    pub fn backfill_marginal_free_tier_costs(
        &self,
    ) -> OpenFangResult<(u64, u64)> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let n_usage = conn
            .execute(
                "UPDATE usage_events
                    SET cost_usd = 0.0
                  WHERE lower(model) LIKE '%:free%'",
                [],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;
        let n_comp = conn
            .execute(
                "UPDATE eco_compression_events
                    SET input_price_per_million_usd = 0.0,
                        est_input_cost_saved_usd = 0.0,
                        billed_input_cost_usd = 0.0
                  WHERE lower(model) LIKE '%:free%'",
                [],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))? as u64;
        Ok((n_usage, n_comp))
    }

    /// Query total cost in the last hour for an agent.
    pub fn query_hourly(&self, agent_id: AgentId) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', '-1 hour')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost today for an agent.
    pub fn query_daily(&self, agent_id: AgentId) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of day')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost in the current calendar month for an agent.
    pub fn query_monthly(&self, agent_id: AgentId) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE agent_id = ?1 AND timestamp > datetime('now', 'start of month')",
                rusqlite::params![agent_id.0.to_string()],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost across all agents for the current hour.
    pub fn query_global_hourly(&self) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', '-1 hour')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query total cost across all agents for the current calendar month.
    pub fn query_global_monthly(&self) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', 'start of month')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Query usage summary, optionally filtered by agent.
    pub fn query_summary(&self, agent_id: Option<AgentId>) -> OpenFangResult<UsageSummary> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match agent_id {
            Some(aid) => (
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
                 FROM usage_events WHERE agent_id = ?1",
                vec![Box::new(aid.0.to_string())],
            ),
            None => (
                "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cost_usd), 0.0), COUNT(*), COALESCE(SUM(tool_calls), 0)
                 FROM usage_events",
                vec![],
            ),
        };

        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();

        let summary = conn
            .query_row(sql, params_refs.as_slice(), |row| {
                Ok(UsageSummary {
                    total_input_tokens: row.get::<_, i64>(0)? as u64,
                    total_output_tokens: row.get::<_, i64>(1)? as u64,
                    total_cost_usd: row.get(2)?,
                    call_count: row.get::<_, i64>(3)? as u64,
                    total_tool_calls: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        Ok(summary)
    }

    /// Query usage grouped by model.
    pub fn query_by_model(&self) -> OpenFangResult<Vec<ModelUsage>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(
                "SELECT model, COALESCE(SUM(cost_usd), 0.0), COALESCE(SUM(input_tokens), 0),
                        COALESCE(SUM(output_tokens), 0), COUNT(*)
                 FROM usage_events GROUP BY model ORDER BY SUM(cost_usd) DESC",
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ModelUsage {
                    model: row.get(0)?,
                    total_cost_usd: row.get(1)?,
                    total_input_tokens: row.get::<_, i64>(2)? as u64,
                    total_output_tokens: row.get::<_, i64>(3)? as u64,
                    call_count: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| OpenFangError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Query daily usage breakdown for the last N days.
    pub fn query_daily_breakdown(&self, days: u32) -> OpenFangResult<Vec<DailyBreakdown>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;

        let mut stmt = conn
            .prepare(&format!(
                "SELECT date(timestamp) as day,
                            COALESCE(SUM(cost_usd), 0.0),
                            COALESCE(SUM(input_tokens) + SUM(output_tokens), 0),
                            COUNT(*)
                     FROM usage_events
                     WHERE timestamp > datetime('now', '-{days} days')
                     GROUP BY day
                     ORDER BY day ASC"
            ))
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(DailyBreakdown {
                    date: row.get(0)?,
                    cost_usd: row.get(1)?,
                    tokens: row.get::<_, i64>(2)? as u64,
                    calls: row.get::<_, i64>(3)? as u64,
                })
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| OpenFangError::Memory(e.to_string()))?);
        }
        Ok(results)
    }

    /// Query the timestamp of the earliest usage event.
    pub fn query_first_event_date(&self) -> OpenFangResult<Option<String>> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let result: Option<String> = conn
            .query_row("SELECT MIN(timestamp) FROM usage_events", [], |row| {
                row.get(0)
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(result)
    }

    /// Query today's total cost across all agents.
    pub fn query_today_cost(&self) -> OpenFangResult<f64> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let cost: f64 = conn
            .query_row(
                "SELECT COALESCE(SUM(cost_usd), 0.0) FROM usage_events
                 WHERE timestamp > datetime('now', 'start of day')",
                [],
                |row| row.get(0),
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(cost)
    }

    /// Delete usage events older than the given number of days.
    pub fn cleanup_old(&self, days: u32) -> OpenFangResult<usize> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let deleted = conn
            .execute(
                &format!(
                    "DELETE FROM usage_events WHERE timestamp < datetime('now', '-{days} days')"
                ),
                [],
            )
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let _ = conn.execute(
            &format!(
                "DELETE FROM quota_block_events WHERE timestamp < datetime('now', '-{days} days')"
            ),
            [],
        );
        Ok(deleted)
    }

    /// Query compression metrics by mode and by agent.
    ///
    /// `window_days`:
    /// - `Some(N)` => only rows newer than `now - N days`
    /// - `None` => all rows
    pub fn query_compression_summary(
        &self,
        window_days: Option<u32>,
    ) -> OpenFangResult<CompressionSummary> {
        #[derive(Default)]
        struct Agg {
            turns: u64,
            compressed_turns: u64,
            original_tokens: u64,
            compressed_tokens: u64,
            savings_samples: Vec<u8>,
            semantic_samples: Vec<f64>,
        }
        fn pctl(samples: &[u8], p: f64) -> u8 {
            if samples.is_empty() {
                return 0;
            }
            let mut s = samples.to_vec();
            s.sort_unstable();
            let idx = ((s.len().saturating_sub(1) as f64) * p).round() as usize;
            s[idx.min(s.len().saturating_sub(1))]
        }
        fn finalize(a: Agg) -> CompressionModeSummary {
            let savings_mean = if a.savings_samples.is_empty() {
                0.0
            } else {
                a.savings_samples.iter().map(|v| *v as u64).sum::<u64>() as f64
                    / a.savings_samples.len() as f64
            };
            let reduction_pct = if a.original_tokens == 0 {
                0.0
            } else {
                100.0 - ((a.compressed_tokens as f64 * 100.0) / a.original_tokens.max(1) as f64)
            };
            let semantic_mean = if a.semantic_samples.is_empty() {
                None
            } else {
                Some(a.semantic_samples.iter().sum::<f64>() / a.semantic_samples.len() as f64)
            };
            let semantic_p50 = if a.semantic_samples.is_empty() {
                None
            } else {
                let mut s = a.semantic_samples.clone();
                s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                let idx = ((s.len().saturating_sub(1) as f64) * 0.50).round() as usize;
                Some(s[idx.min(s.len().saturating_sub(1))])
            };
            let semantic_p95 = if a.semantic_samples.is_empty() {
                None
            } else {
                let mut s = a.semantic_samples.clone();
                s.sort_by(|x, y| x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal));
                let idx = ((s.len().saturating_sub(1) as f64) * 0.95).round() as usize;
                Some(s[idx.min(s.len().saturating_sub(1))])
            };
            CompressionModeSummary {
                turns: a.turns,
                compressed_turns: a.compressed_turns,
                compression_hit_rate_pct: if a.turns == 0 {
                    0.0
                } else {
                    a.compressed_turns as f64 * 100.0 / a.turns as f64
                },
                sample_count: a.savings_samples.len(),
                savings_pct_p50: pctl(&a.savings_samples, 0.50),
                savings_pct_p95: pctl(&a.savings_samples, 0.95),
                savings_pct_mean: savings_mean,
                semantic_score_mean: semantic_mean,
                semantic_score_p50: semantic_p50,
                semantic_score_p95: semantic_p95,
                semantic_score_samples: a.semantic_samples.len(),
                estimated_original_tokens: a.original_tokens,
                estimated_compressed_tokens: a.compressed_tokens,
                estimated_token_reduction_pct: reduction_pct,
            }
        }

        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        #[derive(Default)]
        struct PmAgg {
            turns: u64,
            original: u64,
            compressed: u64,
            billed_input: u64,
            input_saved: u64,
            est_saved_usd: f64,
            billed_cost_usd: f64,
            // weighted average input price ($/1M) by input tokens saved
            price_weight: f64,
            price_weight_tokens: u64,
        }

        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match window_days {
            Some(days) => (
                "SELECT agent_id, mode, original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                        COALESCE(est_input_cost_saved_usd, 0.0),
                        COALESCE(provider, ''), COALESCE(model, ''),
                        COALESCE(input_tokens_saved, 0),
                        COALESCE(input_price_per_million_usd, 0.0),
                        COALESCE(billed_input_tokens, 0),
                        COALESCE(billed_input_cost_usd, 0.0)
                 FROM eco_compression_events
                 WHERE timestamp > datetime('now', ?1)",
                vec![Box::new(format!("-{} days", days))],
            ),
            None => (
                "SELECT agent_id, mode, original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                        COALESCE(est_input_cost_saved_usd, 0.0),
                        COALESCE(provider, ''), COALESCE(model, ''),
                        COALESCE(input_tokens_saved, 0),
                        COALESCE(input_price_per_million_usd, 0.0),
                        COALESCE(billed_input_tokens, 0),
                        COALESCE(billed_input_cost_usd, 0.0)
                 FROM eco_compression_events",
                vec![],
            ),
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn
            .prepare(sql)
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map(params_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u8,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, f64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, i64>(9)? as u64,
                    row.get::<_, f64>(10)?,
                    row.get::<_, i64>(11)? as u64,
                    row.get::<_, f64>(12)?,
                ))
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut mode_agg: HashMap<String, Agg> = HashMap::new();
        let mut agent_mode_agg: HashMap<(String, String), Agg> = HashMap::new();
        let mut pm_agg: HashMap<(String, String), PmAgg> = HashMap::new();
        let mut total_compression_tokens_saved: u64 = 0;
        let mut per_row_comp_usd: f64 = 0.0;
        let mut original_input_tokens_total: u64 = 0;
        let mut billed_input_tokens_total: u64 = 0;
        let mut billed_input_cost_usd_total: f64 = 0.0;
        for row in rows {
            let (
                agent_id,
                mode_raw,
                original,
                compressed,
                savings_pct,
                semantic_score,
                row_usd,
                provider,
                model,
                input_saved,
                input_price_per_million_usd,
                billed_input_tokens,
                billed_input_cost_usd,
            ) = row.map_err(|e| OpenFangError::Memory(e.to_string()))?;
            per_row_comp_usd += row_usd;
            original_input_tokens_total = original_input_tokens_total.saturating_add(original);
            billed_input_tokens_total =
                billed_input_tokens_total.saturating_add(billed_input_tokens);
            billed_input_cost_usd_total += billed_input_cost_usd;
            if original > compressed {
                total_compression_tokens_saved =
                    total_compression_tokens_saved.saturating_add(original - compressed);
            }
            let mode = mode_raw.to_ascii_lowercase();
            {
                let a = mode_agg.entry(mode.clone()).or_default();
                a.turns = a.turns.saturating_add(1);
                if savings_pct > 0 {
                    a.compressed_turns = a.compressed_turns.saturating_add(1);
                }
                a.original_tokens = a.original_tokens.saturating_add(original);
                a.compressed_tokens = a.compressed_tokens.saturating_add(compressed);
                a.savings_samples.push(savings_pct);
                if let Some(v) = semantic_score {
                    a.semantic_samples.push(v);
                }
            }
            {
                let a = agent_mode_agg.entry((agent_id, mode)).or_default();
                a.turns = a.turns.saturating_add(1);
                if savings_pct > 0 {
                    a.compressed_turns = a.compressed_turns.saturating_add(1);
                }
                a.original_tokens = a.original_tokens.saturating_add(original);
                a.compressed_tokens = a.compressed_tokens.saturating_add(compressed);
                a.savings_samples.push(savings_pct);
                if let Some(v) = semantic_score {
                    a.semantic_samples.push(v);
                }
            }
            // Per-(provider, model) rollup keyed by lowercase ids.
            let pkey = (provider.to_ascii_lowercase(), model.to_ascii_lowercase());
            let pm = pm_agg.entry(pkey).or_default();
            pm.turns = pm.turns.saturating_add(1);
            pm.original = pm.original.saturating_add(original);
            pm.compressed = pm.compressed.saturating_add(compressed);
            pm.input_saved = pm.input_saved.saturating_add(input_saved);
            pm.billed_input = pm.billed_input.saturating_add(billed_input_tokens);
            pm.est_saved_usd += row_usd;
            pm.billed_cost_usd += billed_input_cost_usd;
            if input_price_per_million_usd > 0.0 && input_saved > 0 {
                pm.price_weight += input_price_per_million_usd * input_saved as f64;
                pm.price_weight_tokens = pm.price_weight_tokens.saturating_add(input_saved);
            }
        }

        let mut modes: BTreeMap<String, CompressionModeSummary> = BTreeMap::new();
        for (mode, agg) in mode_agg {
            modes.insert(mode, finalize(agg));
        }

        let mut agent_grouped: BTreeMap<String, BTreeMap<String, CompressionModeSummary>> =
            BTreeMap::new();
        for ((agent_id, mode), agg) in agent_mode_agg {
            agent_grouped
                .entry(agent_id)
                .or_default()
                .insert(mode, finalize(agg));
        }
        let agents = agent_grouped
            .into_iter()
            .map(|(agent_id, modes)| CompressionAgentSummary { agent_id, modes })
            .collect();

        let (
            cache_read_tokens,
            estimated_cache_cost_saved_usd,
            weighted_input_rate_sum,
            weighted_rate_tokens,
        ): (u64, f64, f64, u64) = {
            let (usage_sql, usage_params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) =
                match window_days {
                    Some(days) => (
                        "SELECT model, input_tokens, cache_read_input_tokens FROM usage_events
                         WHERE timestamp > datetime('now', ?1)",
                        vec![Box::new(format!("-{} days", days))],
                    ),
                    None => (
                        "SELECT model, input_tokens, cache_read_input_tokens FROM usage_events",
                        vec![],
                    ),
                };
            let usage_params_refs: Vec<&dyn rusqlite::types::ToSql> =
                usage_params.iter().map(|p| p.as_ref()).collect();
            let mut usage_stmt = conn
                .prepare(usage_sql)
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let usage_rows = usage_stmt
                .query_map(usage_params_refs.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                    ))
                })
                .map_err(|e| OpenFangError::Memory(e.to_string()))?;
            let mut cache_tokens: u64 = 0;
            let mut cache_saved: f64 = 0.0;
            let mut rate_sum: f64 = 0.0;
            let mut rate_tokens: u64 = 0;
            for row in usage_rows {
                let (model, input_tokens, cache_read) =
                    row.map_err(|e| OpenFangError::Memory(e.to_string()))?;
                let input_rate = estimate_input_per_million(&model) / 1_000_000.0;
                if input_tokens > 0 {
                    rate_sum += input_rate * input_tokens as f64;
                    rate_tokens = rate_tokens.saturating_add(input_tokens);
                }
                if cache_read > 0 {
                    cache_tokens = cache_tokens.saturating_add(cache_read);
                    cache_saved += input_rate * cache_read as f64 * cache_discount_factor(&model);
                }
            }
            (cache_tokens, cache_saved, rate_sum, rate_tokens)
        };
        let avg_input_rate_per_token = if weighted_rate_tokens == 0 {
            0.0
        } else {
            weighted_input_rate_sum / weighted_rate_tokens as f64
        };
        let estimated_compression_cost_saved_usd = if per_row_comp_usd > 1e-9 {
            per_row_comp_usd
        } else {
            total_compression_tokens_saved as f64 * avg_input_rate_per_token
        };
        let estimated_total_input_tokens_saved =
            total_compression_tokens_saved.saturating_add(cache_read_tokens);
        let estimated_total_cost_saved_usd =
            estimated_cache_cost_saved_usd + estimated_compression_cost_saved_usd;

        let adaptive_eco = match (
            self.query_adaptive_eco_summary(window_days),
            self.query_adaptive_eco_replay_report(window_days),
        ) {
            (Ok(summary), Ok(replay)) => Some(CompressionAdaptiveEcoBundle { summary, replay }),
            _ => None,
        };

        let mut by_provider_model: Vec<CompressionProviderModelRollup> = pm_agg
            .into_iter()
            .map(|((provider, model), pm)| {
                let avg_price = if pm.price_weight_tokens > 0 {
                    pm.price_weight / pm.price_weight_tokens as f64
                } else {
                    0.0
                };
                CompressionProviderModelRollup {
                    provider,
                    model,
                    turns: pm.turns,
                    original_input_tokens: pm.original,
                    compressed_input_tokens: pm.compressed,
                    billed_input_tokens: pm.billed_input,
                    input_tokens_saved: pm.input_saved,
                    input_price_per_million_usd: avg_price,
                    est_input_cost_saved_usd: pm.est_saved_usd,
                    billed_input_cost_usd: pm.billed_cost_usd,
                }
            })
            .collect();
        by_provider_model.sort_by(|a, b| {
            b.est_input_cost_saved_usd
                .partial_cmp(&a.est_input_cost_saved_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.provider.cmp(&b.provider))
                .then_with(|| a.model.cmp(&b.model))
        });

        Ok(CompressionSummary {
            window: window_days
                .map(|d| format!("{d}d"))
                .unwrap_or_else(|| "all".to_string()),
            modes,
            agents,
            estimated_compression_tokens_saved: total_compression_tokens_saved,
            cache_read_input_tokens: cache_read_tokens,
            estimated_total_input_tokens_saved,
            estimated_cache_cost_saved_usd,
            estimated_compression_cost_saved_usd,
            estimated_total_cost_saved_usd,
            original_input_tokens_total,
            billed_input_tokens_total,
            billed_input_cost_usd_total,
            by_provider_model,
            adaptive_eco,
        })
    }
}

fn estimate_input_per_million(model: &str) -> f64 {
    let m = model.to_ascii_lowercase();
    // OpenRouter and similar free-tier route suffix — never impute a positive $/1M.
    if m.contains(":free") {
        return 0.0;
    }
    if m.contains("haiku") {
        0.25
    } else if m.contains("opus-4-6") || m.contains("claude-opus-4-6") {
        5.0
    } else if m.contains("opus") {
        15.0
    } else if m.contains("sonnet") {
        3.0
    } else if m.contains("gpt-4o-mini") {
        0.15
    } else if m.contains("gpt-4o") {
        2.50
    } else if m.contains("gpt-4.1-mini") {
        0.40
    } else if m.contains("gpt-4.1") {
        2.00
    } else if m.contains("gemini-2.5-pro") {
        1.25
    } else if m.contains("gemini-2.5-flash") {
        0.15
    } else if m.contains("deepseek") {
        0.27
    } else {
        1.0
    }
}

fn cache_discount_factor(model: &str) -> f64 {
    let m = model.to_ascii_lowercase();
    if m.contains("anthropic/") || m.contains("claude") {
        0.90
    } else if m.contains("openai/gpt-4o") || m.contains("gpt-4o") {
        0.50
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::open_in_memory_pool;
    use openfang_types::adaptive_eco::EcoCounterfactualReceipt;
    use openfang_types::config::MemoryConfig;

    fn setup() -> UsageStore {
        let pool = open_in_memory_pool(&MemoryConfig::default()).unwrap();
        UsageStore::new(pool)
    }

    #[test]
    fn test_record_and_query_summary() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                model: "claude-haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 2,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id,
                model: "claude-sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        let summary = store.query_summary(Some(agent_id)).unwrap();
        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_input_tokens, 600);
        assert_eq!(summary.total_output_tokens, 250);
        assert!((summary.total_cost_usd - 0.011).abs() < 0.0001);
        assert_eq!(summary.total_tool_calls, 3);
    }

    #[test]
    fn test_query_summary_all_agents() {
        let store = setup();
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id: a1,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        store
            .record(&UsageRecord {
                agent_id: a2,
                model: "sonnet".to_string(),
                input_tokens: 200,
                output_tokens: 100,
                cost_usd: 0.005,
                tool_calls: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 2);
        assert_eq!(summary.total_input_tokens, 300);
    }

    #[test]
    fn test_query_by_model() {
        let store = setup();
        let agent_id = AgentId::new();

        for _ in 0..3 {
            store
                .record(&UsageRecord {
                    agent_id,
                    model: "haiku".to_string(),
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: 0.001,
                    tool_calls: 0,
                    cache_creation_input_tokens: 0,
                    cache_read_input_tokens: 0,
                })
                .unwrap();
        }

        store
            .record(&UsageRecord {
                agent_id,
                model: "sonnet".to_string(),
                input_tokens: 500,
                output_tokens: 200,
                cost_usd: 0.01,
                tool_calls: 1,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        let by_model = store.query_by_model().unwrap();
        assert_eq!(by_model.len(), 2);
        // sonnet should be first (highest cost)
        assert_eq!(by_model[0].model, "sonnet");
        assert_eq!(by_model[1].model, "haiku");
        assert_eq!(by_model[1].call_count, 3);
    }

    #[test]
    fn test_query_hourly() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.05,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        let hourly = store.query_hourly(agent_id).unwrap();
        assert!((hourly - 0.05).abs() < 0.001);
    }

    #[test]
    fn test_query_daily() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.123,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        let daily = store.query_daily(agent_id).unwrap();
        assert!((daily - 0.123).abs() < 0.001);
    }

    #[test]
    fn test_cleanup_old() {
        let store = setup();
        let agent_id = AgentId::new();

        store
            .record(&UsageRecord {
                agent_id,
                model: "haiku".to_string(),
                input_tokens: 100,
                output_tokens: 50,
                cost_usd: 0.001,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        // Cleanup events older than 1 day should not remove today's events
        let deleted = store.cleanup_old(1).unwrap();
        assert_eq!(deleted, 0);

        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 1);
    }

    #[test]
    fn test_empty_summary() {
        let store = setup();
        let summary = store.query_summary(None).unwrap();
        assert_eq!(summary.call_count, 0);
        assert_eq!(summary.total_cost_usd, 0.0);
    }

    #[test]
    fn test_record_and_query_quota_block_summary() {
        let store = setup();
        let agent_id = AgentId::new();
        store
            .record_quota_block(&QuotaBlockRecord {
                agent_id,
                reason: "hourly_llm_tokens".to_string(),
                est_input_tokens: 1000,
                est_output_tokens: 512,
                est_cost_usd: 0.02,
            })
            .unwrap();
        store
            .record_quota_block(&QuotaBlockRecord {
                agent_id,
                reason: "global_daily_usd".to_string(),
                est_input_tokens: 500,
                est_output_tokens: 512,
                est_cost_usd: 0.01,
            })
            .unwrap();

        let s = store.query_quota_block_summary(None).unwrap();
        assert_eq!(s.block_count, 2);
        assert_eq!(s.total_est_input_tokens, 1500);
        assert_eq!(s.total_est_output_tokens, 1024);
        assert!((s.total_est_cost_usd - 0.03).abs() < 0.0001);
        assert_eq!(s.by_reason.get("hourly_llm_tokens").unwrap().count, 1);
    }

    #[test]
    fn test_record_and_query_compression_summary() {
        let store = setup();
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a1,
                mode: "balanced".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                provider: "anthropic".to_string(),
                original_tokens_est: 100,
                compressed_tokens_est: 60,
                input_tokens_saved: 40,
                input_price_per_million_usd: 3.0,
                est_input_cost_saved_usd: 40.0 * 3.0 / 1_000_000.0,
                billed_input_tokens: 60,
                billed_input_cost_usd: 60.0 * 3.0 / 1_000_000.0,
                savings_pct: 40,
                semantic_preservation_score: Some(0.92),
            })
            .unwrap();
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a1,
                mode: "balanced".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                provider: "anthropic".to_string(),
                original_tokens_est: 120,
                compressed_tokens_est: 90,
                input_tokens_saved: 30,
                input_price_per_million_usd: 3.0,
                est_input_cost_saved_usd: 30.0 * 3.0 / 1_000_000.0,
                billed_input_tokens: 90,
                billed_input_cost_usd: 90.0 * 3.0 / 1_000_000.0,
                savings_pct: 25,
                semantic_preservation_score: None,
            })
            .unwrap();
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a2,
                mode: "off".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                provider: "anthropic".to_string(),
                original_tokens_est: 80,
                compressed_tokens_est: 80,
                input_tokens_saved: 0,
                input_price_per_million_usd: 3.0,
                est_input_cost_saved_usd: 0.0,
                billed_input_tokens: 80,
                billed_input_cost_usd: 80.0 * 3.0 / 1_000_000.0,
                savings_pct: 0,
                semantic_preservation_score: None,
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: a1,
                model: "claude-sonnet-4-6".to_string(),
                input_tokens: 1000,
                output_tokens: 100,
                cost_usd: 0.0,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 500,
            })
            .unwrap();

        let summary = store.query_compression_summary(None).unwrap();
        let balanced = summary.modes.get("balanced").expect("balanced mode");
        assert_eq!(balanced.turns, 2);
        assert_eq!(balanced.compressed_turns, 2);
        assert_eq!(balanced.savings_pct_p50, 40);
        assert!(balanced.semantic_score_mean.unwrap_or_default() > 0.9);
        assert!(balanced.semantic_score_p50.unwrap_or_default() > 0.9);
        assert!(balanced.semantic_score_p95.unwrap_or_default() > 0.9);

        let off = summary.modes.get("off").expect("off mode");
        assert_eq!(off.turns, 1);
        assert_eq!(off.compressed_turns, 0);
        assert_eq!(off.savings_pct_p95, 0);
        assert_eq!(summary.agents.len(), 2);
        assert_eq!(summary.estimated_compression_tokens_saved, 70);
        assert_eq!(summary.cache_read_input_tokens, 500);
        assert_eq!(summary.estimated_total_input_tokens_saved, 570);
        assert!(summary.estimated_cache_cost_saved_usd > 0.0);
        assert!(summary.estimated_compression_cost_saved_usd > 0.0);
        assert!(summary.estimated_total_cost_saved_usd > 0.0);
        // Pre-compression input baseline + provider-billed input both persisted.
        assert_eq!(summary.original_input_tokens_total, 300);
        assert_eq!(summary.billed_input_tokens_total, 230);
        assert!(summary.billed_input_cost_usd_total > 0.0);
        // by_provider_model rolls up to one entry (anthropic / claude-sonnet-4-6).
        assert_eq!(summary.by_provider_model.len(), 1);
        let pm = &summary.by_provider_model[0];
        assert_eq!(pm.provider, "anthropic");
        assert_eq!(pm.model, "claude-sonnet-4-6");
        assert_eq!(pm.turns, 3);
        assert_eq!(pm.original_input_tokens, 300);
        assert_eq!(pm.billed_input_tokens, 230);
        assert_eq!(pm.input_tokens_saved, 70);
        assert!((pm.input_price_per_million_usd - 3.0).abs() < 1e-9);
        assert!(summary.adaptive_eco.is_some());
        let bundle = summary.adaptive_eco.as_ref().unwrap();
        assert_eq!(bundle.summary.window, "all");
        assert_eq!(bundle.replay.window, "all");
    }

    #[test]
    fn test_query_compression_summary_window_label() {
        let store = setup();
        let a1 = AgentId::new();
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a1,
                mode: "balanced".to_string(),
                model: "claude-3-5-sonnet".to_string(),
                provider: "anthropic".to_string(),
                original_tokens_est: 50,
                compressed_tokens_est: 40,
                input_tokens_saved: 10,
                input_price_per_million_usd: 3.0,
                est_input_cost_saved_usd: 10.0 * 3.0 / 1_000_000.0,
                billed_input_tokens: 40,
                billed_input_cost_usd: 40.0 * 3.0 / 1_000_000.0,
                savings_pct: 20,
                semantic_preservation_score: None,
            })
            .unwrap();

        let s7 = store.query_compression_summary(Some(7)).unwrap();
        assert_eq!(s7.window, "7d");
        assert!(s7.modes.contains_key("balanced"));

        let s_all = store.query_compression_summary(None).unwrap();
        assert_eq!(s_all.window, "all");
    }

    /// Verifies the v15 fallback-attribution contract: when a turn is serviced by a fallback
    /// model (different `provider` + `model` from the requested ones, e.g. an OpenRouter free
    /// model after a primary 429), `record_compression` snapshots the *actually-used* provider /
    /// model + their pricing. `query_compression_summary().by_provider_model` must then split
    /// across providers so dashboards attribute cost to the model that really billed the call.
    #[test]
    fn test_compression_records_actual_provider_when_fallback() {
        let store = setup();
        let agent = AgentId::new();

        // Primary turn: requested model (anthropic / claude-sonnet-4-6) actually billed.
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: agent,
                mode: "balanced".to_string(),
                model: "claude-sonnet-4-6".to_string(),
                provider: "anthropic".to_string(),
                original_tokens_est: 200,
                compressed_tokens_est: 120,
                input_tokens_saved: 80,
                input_price_per_million_usd: 3.0,
                est_input_cost_saved_usd: 80.0 * 3.0 / 1_000_000.0,
                billed_input_tokens: 120,
                billed_input_cost_usd: 120.0 * 3.0 / 1_000_000.0,
                savings_pct: 40,
                semantic_preservation_score: Some(0.93),
            })
            .unwrap();

        // Fallback turn: primary 429 forced OpenRouter free-tier; provider/model differ from manifest.
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: agent,
                mode: "balanced".to_string(),
                model: "stepfun/step-3.5-flash:free".to_string(),
                provider: "openrouter".to_string(),
                original_tokens_est: 200,
                compressed_tokens_est: 130,
                input_tokens_saved: 70,
                input_price_per_million_usd: 0.0,
                est_input_cost_saved_usd: 0.0,
                billed_input_tokens: 130,
                billed_input_cost_usd: 0.0,
                savings_pct: 35,
                semantic_preservation_score: Some(0.88),
            })
            .unwrap();

        let summary = store.query_compression_summary(None).unwrap();

        // Aggregated totals span both providers.
        assert_eq!(summary.original_input_tokens_total, 400);
        assert_eq!(summary.billed_input_tokens_total, 250);
        assert!(summary.billed_input_cost_usd_total > 0.0);
        assert_eq!(summary.estimated_compression_tokens_saved, 150);

        // by_provider_model must split: one row per (provider, model). This is the contract that
        // makes fallback attribution observable in the dashboard.
        assert_eq!(summary.by_provider_model.len(), 2);

        let anth = summary
            .by_provider_model
            .iter()
            .find(|r| r.provider == "anthropic")
            .expect("anthropic rollup");
        assert_eq!(anth.model, "claude-sonnet-4-6");
        assert_eq!(anth.turns, 1);
        assert_eq!(anth.original_input_tokens, 200);
        assert_eq!(anth.billed_input_tokens, 120);
        assert_eq!(anth.input_tokens_saved, 80);
        assert!((anth.input_price_per_million_usd - 3.0).abs() < 1e-9);
        assert!(anth.billed_input_cost_usd > 0.0);

        let or = summary
            .by_provider_model
            .iter()
            .find(|r| r.provider == "openrouter")
            .expect("openrouter rollup");
        assert_eq!(or.model, "stepfun/step-3.5-flash:free");
        assert_eq!(or.turns, 1);
        assert_eq!(or.original_input_tokens, 200);
        assert_eq!(or.billed_input_tokens, 130);
        assert_eq!(or.input_tokens_saved, 70);
        // Caller passed $0/M (true marginal price for OpenRouter `:free`). The row must not be
        // "upgraded" to a positive heuristic — that would inflate est. $ not spent for free tier.
        assert!((or.input_price_per_million_usd - 0.0).abs() < 1e-9);
        assert!(or.billed_input_cost_usd <= 0.0 + 1e-9);
    }

    /// Verifies the v15+ historical-data repair contract.
    ///
    /// Simulates a pre-v15 row: `provider = ''`, `input_price_per_million_usd = 0`,
    /// `est_input_cost_saved_usd = 0`, `billed_input_tokens = 0`, `billed_input_cost_usd = 0` —
    /// the exact shape of every compression event written before pricing was snapshotted on
    /// each row. Then runs both backfills and asserts the row is now fully priced AND the
    /// dashboard rollup (`compression_savings`) reflects the recovered USD.
    #[test]
    fn test_backfill_compression_pricing_repairs_pre_v15_rows() {
        let store = setup();
        let agent = AgentId::new();

        // Direct INSERT to bypass record_compression()'s heuristic price fallback — we want a
        // genuine pre-v15 row shape (NULL provider, zero pricing, zero saved-USD).
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO eco_compression_events (
                id, agent_id, timestamp, mode, model,
                original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                input_tokens_saved, input_price_per_million_usd, est_input_cost_saved_usd,
                provider, billed_input_tokens, billed_input_cost_usd
            ) VALUES (?1, ?2, datetime('now'), ?3, ?4, ?5, ?6, ?7, NULL, ?8, 0.0, 0.0, '', 0, 0.0)",
            rusqlite::params![
                "row-pre-v15".to_string(),
                agent.0.to_string(),
                "balanced",
                "claude-sonnet-4-6",
                500i64,
                300i64,
                40i64,
                200i64,
            ],
        )
        .unwrap();
        drop(conn);

        // Pre-condition: rollup shows zero saved USD, blank provider.
        let before = store.query_compression_summary(None).unwrap();
        assert!(before.estimated_compression_cost_saved_usd <= 0.0);
        let before_rollup = before
            .by_provider_model
            .iter()
            .find(|r| r.model == "claude-sonnet-4-6")
            .expect("model rollup");
        assert_eq!(before_rollup.provider, "");
        assert!((before_rollup.input_price_per_million_usd - 0.0).abs() < 1e-9);

        // Catalog says: claude-sonnet-4-6 → anthropic, $3/M input.
        let lookup = |model: &str| -> Option<CompressionPricingSnapshot> {
            if model == "claude-sonnet-4-6" {
                Some(CompressionPricingSnapshot {
                    provider: "anthropic".to_string(),
                    input_per_million_usd: 3.0,
                })
            } else {
                None
            }
        };

        let updated = store.backfill_compression_pricing(lookup).unwrap();
        assert_eq!(updated, 1, "exactly one pre-v15 row should be repaired");

        // Idempotency: a second run is a no-op (no rows match the WHERE filter anymore).
        let updated_again = store.backfill_compression_pricing(lookup).unwrap();
        assert_eq!(
            updated_again, 0,
            "second run must be a no-op (already-priced rows are not re-priced)"
        );

        let after = store.query_compression_summary(None).unwrap();
        assert!(
            after.estimated_compression_cost_saved_usd > 0.0,
            "saved USD should be recovered after backfill (got {})",
            after.estimated_compression_cost_saved_usd
        );
        let after_rollup = after
            .by_provider_model
            .iter()
            .find(|r| r.model == "claude-sonnet-4-6")
            .expect("model rollup");
        assert_eq!(after_rollup.provider, "anthropic");
        assert!((after_rollup.input_price_per_million_usd - 3.0).abs() < 1e-9);
        // 200 saved tokens * $3/M = $0.0006
        let expected_saved = (200.0_f64 / 1_000_000.0) * 3.0;
        assert!((after_rollup.est_input_cost_saved_usd - expected_saved).abs() < 1e-9);
    }

    /// Verifies that `backfill_compression_billed_tokens` recovers `billed_input_tokens` (and the
    /// matching cost) for pre-v15 rows by joining to the nearest `usage_events` row for the same
    /// agent. The kernel writes both within the same turn, so a ±60s window always finds them.
    #[test]
    fn test_backfill_compression_billed_tokens_from_usage_events() {
        let store = setup();
        let agent = AgentId::new();

        // The provider-billed input for this turn (what we want to recover into the comp row).
        store
            .record(&UsageRecord {
                agent_id: agent,
                model: "claude-sonnet-4-6".to_string(),
                input_tokens: 1234,
                output_tokens: 256,
                cost_usd: 0.0,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();

        // Pre-v15 compression row with a price snapshot already (so the backfill can recompute
        // billed_input_cost_usd) but with billed_input_tokens / billed_input_cost_usd zero.
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO eco_compression_events (
                id, agent_id, timestamp, mode, model,
                original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                input_tokens_saved, input_price_per_million_usd, est_input_cost_saved_usd,
                provider, billed_input_tokens, billed_input_cost_usd
            ) VALUES (?1, ?2, datetime('now'), 'balanced', ?3, 600, 400, 33, NULL,
                      200, 3.0, 0.0006, 'anthropic', 0, 0.0)",
            rusqlite::params![
                "row-needs-billed".to_string(),
                agent.0.to_string(),
                "claude-sonnet-4-6",
            ],
        )
        .unwrap();
        drop(conn);

        let copied = store.backfill_compression_billed_tokens().unwrap();
        assert_eq!(copied, 1, "one row should have billed_input_tokens copied in");

        let after = store.query_compression_summary(None).unwrap();
        assert_eq!(
            after.billed_input_tokens_total, 1234,
            "billed_input_tokens_total must reflect provider-reported input from usage_events"
        );
        assert!(
            after.billed_input_cost_usd_total > 0.0,
            "billed_input_cost_usd_total must be recomputed once tokens + price are both present"
        );

        // Idempotency: a second run touches no compression rows that already have billed values.
        let copied_again = store.backfill_compression_billed_tokens().unwrap();
        assert_eq!(copied_again, 0);
    }

    #[test]
    fn test_backfill_marginal_free_tier_costs() {
        let store = setup();
        let agent = AgentId::new();
        store
            .record(&UsageRecord {
                agent_id: agent,
                model: "qwen/qwen3.6-plus:free".to_string(),
                input_tokens: 1_000_000,
                output_tokens: 100_000,
                cost_usd: 19.25,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();
        store
            .record(&UsageRecord {
                agent_id: agent,
                model: "claude-sonnet-4-6".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                cost_usd: 0.10,
                tool_calls: 0,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            })
            .unwrap();
        let conn = store.pool.get().unwrap();
        conn.execute(
            "INSERT INTO eco_compression_events (
                id, agent_id, timestamp, mode, model,
                original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score,
                input_tokens_saved, input_price_per_million_usd, est_input_cost_saved_usd,
                provider, billed_input_tokens, billed_input_cost_usd
            ) VALUES ('cfree', ?1, datetime('now'), 'efficient', 'nvidia/nemotron:free',
                      1000, 500, 50, NULL, 500, 1.0, 0.0005, 'openrouter', 1000, 0.001)",
            rusqlite::params![agent.0.to_string()],
        )
        .unwrap();
        drop(conn);

        let (n_u, n_c) = store.backfill_marginal_free_tier_costs().unwrap();
        assert_eq!(n_u, 1);
        assert_eq!(n_c, 1);

        let s = store.query_summary(None).unwrap();
        assert!((s.total_cost_usd - 0.10).abs() < 1e-6, "free route cost should be zeroed");
        let conn = store.pool.get().unwrap();
        let c: f64 = conn
            .query_row(
                "SELECT cost_usd FROM usage_events WHERE model LIKE '%:free%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(c, 0.0);
        let (p, e, b): (f64, f64, f64) = conn
            .query_row(
                "SELECT input_price_per_million_usd, est_input_cost_saved_usd, billed_input_cost_usd
                 FROM eco_compression_events WHERE id = 'cfree'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(p, 0.0);
        assert_eq!(e, 0.0);
        assert_eq!(b, 0.0);
    }

    #[test]
    fn test_adaptive_eco_roundtrip_and_summary() {
        let store = setup();
        let aid = AgentId::new();
        store
            .record_adaptive_eco(&AdaptiveEcoUsageRecord {
                agent_id: aid,
                effective_mode: "balanced".to_string(),
                recommended_mode: "off".to_string(),
                base_mode_before_circuit: Some("balanced".to_string()),
                circuit_breaker_tripped: false,
                hysteresis_blocked: true,
                shadow_only: true,
                enforce: false,
                provider: "openrouter".to_string(),
                model: "x".to_string(),
                cache_capability: "routed".to_string(),
                input_price_per_million: None,
                reason_codes: vec!["shadow_only:enforce_off".to_string()],
                semantic_preservation_score: Some(0.91),
                adaptive_confidence: Some(0.77),
                counterfactual: None,
            })
            .unwrap();
        let s = store.query_adaptive_eco_summary(None).unwrap();
        assert_eq!(s.events, 1);
        assert_eq!(s.shadow_mismatch_turns, 1);
        assert_eq!(s.hysteresis_blocks, 1);
    }

    #[test]
    fn test_adaptive_eco_counterfactual_json_roundtrip() {
        let store = setup();
        let aid = AgentId::new();
        let cf = EcoCounterfactualReceipt {
            applied_mode: "balanced".to_string(),
            original_tokens_est: 100,
            applied_compressed_tokens_est: 72,
            vs_off_tokens_saved: 12,
            vs_off_savings_pct: 12,
            recommended_mode: Some("aggressive".to_string()),
            recommended_compressed_tokens_est: Some(60),
            tokens_saved_delta_recommended_minus_applied: Some(12),
            balanced_compressed_tokens_est: Some(70),
            aggressive_extra_tokens_saved_vs_balanced: Some(10),
        };
        store
            .record_adaptive_eco(&AdaptiveEcoUsageRecord {
                agent_id: aid,
                effective_mode: "balanced".to_string(),
                recommended_mode: "aggressive".to_string(),
                base_mode_before_circuit: None,
                circuit_breaker_tripped: false,
                hysteresis_blocked: false,
                shadow_only: true,
                enforce: false,
                provider: "openrouter".to_string(),
                model: "x".to_string(),
                cache_capability: "routed".to_string(),
                input_price_per_million: Some(1.5),
                reason_codes: vec!["adaptive_eco:v1".to_string()],
                semantic_preservation_score: Some(0.9),
                adaptive_confidence: Some(0.66),
                counterfactual: Some(cf.clone()),
            })
            .unwrap();
        let json = store
            .test_last_adaptive_eco_counterfactual_json(aid)
            .unwrap()
            .expect("counterfactual_json persisted");
        let back: EcoCounterfactualReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(back.applied_mode, cf.applied_mode);
        assert_eq!(back.vs_off_tokens_saved, cf.vs_off_tokens_saved);
        assert_eq!(back.recommended_mode, cf.recommended_mode);
        assert_eq!(
            back.tokens_saved_delta_recommended_minus_applied,
            cf.tokens_saved_delta_recommended_minus_applied
        );
    }

    #[test]
    fn test_adaptive_eco_replay_mode_flips_and_confidence_distribution() {
        use std::thread;
        use std::time::Duration;

        let store = setup();
        let aid = AgentId::new();
        let mk = |effective_mode: &str, conf: f32| AdaptiveEcoUsageRecord {
            agent_id: aid,
            effective_mode: effective_mode.to_string(),
            recommended_mode: "balanced".to_string(),
            base_mode_before_circuit: None,
            circuit_breaker_tripped: false,
            hysteresis_blocked: false,
            shadow_only: true,
            enforce: false,
            provider: "openrouter".to_string(),
            model: "x".to_string(),
            cache_capability: "routed".to_string(),
            input_price_per_million: None,
            reason_codes: vec![],
            semantic_preservation_score: None,
            adaptive_confidence: Some(conf),
            counterfactual: None,
        };

        store.record_adaptive_eco(&mk("balanced", 0.10)).unwrap();
        thread::sleep(Duration::from_millis(20));
        store.record_adaptive_eco(&mk("aggressive", 0.50)).unwrap();
        thread::sleep(Duration::from_millis(20));
        store.record_adaptive_eco(&mk("aggressive", 0.80)).unwrap();

        let r = store.query_adaptive_eco_replay_report(None).unwrap();
        assert_eq!(r.adaptive_eco_events, 3);
        assert_eq!(r.effective_mode_flip_turns, 1);
        assert_eq!(r.effective_mode_transition_slots, 2);
        assert!((r.effective_mode_flip_rate - 0.5).abs() < 1e-9);
        assert_eq!(r.adaptive_confidence_samples, 3);
        assert_eq!(r.adaptive_confidence_bucket_low, 1);
        assert_eq!(r.adaptive_confidence_bucket_mid, 1);
        assert_eq!(r.adaptive_confidence_bucket_high, 1);
    }
}
