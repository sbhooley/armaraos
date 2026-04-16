//! Usage tracking store — records LLM usage events for cost monitoring.

use crate::MemorySqlitePool;
use chrono::Utc;
use openfang_types::adaptive_eco::{AdaptiveEcoUsageRecord, AdaptiveEcoUsageSummary};
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionUsageRecord {
    pub agent_id: AgentId,
    pub mode: String,
    pub original_tokens_est: u64,
    pub compressed_tokens_est: u64,
    pub savings_pct: u8,
    pub semantic_preservation_score: Option<f32>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionAgentSummary {
    pub agent_id: String,
    pub modes: BTreeMap<String, CompressionModeSummary>,
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
        conn.execute(
            "INSERT INTO adaptive_eco_events (
                id, agent_id, timestamp,
                effective_mode, recommended_mode, base_mode_before_circuit,
                circuit_breaker_tripped, hysteresis_blocked,
                shadow_only, enforce,
                provider, model, cache_capability, input_price_per_million,
                reason_codes_json, semantic_preservation_score
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
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

    /// Record a prompt-compression telemetry event.
    pub fn record_compression(&self, record: &CompressionUsageRecord) -> OpenFangResult<()> {
        let conn = self
            .pool
            .get()
            .map_err(|e| OpenFangError::Internal(e.to_string()))?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO eco_compression_events (
                id, agent_id, timestamp, mode,
                original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                id,
                record.agent_id.0.to_string(),
                now,
                record.mode.to_ascii_lowercase(),
                record.original_tokens_est as i64,
                record.compressed_tokens_est as i64,
                record.savings_pct as i64,
                record.semantic_preservation_score.map(|v| v as f64),
            ],
        )
        .map_err(|e| OpenFangError::Memory(e.to_string()))?;
        Ok(())
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
        let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match window_days {
            Some(days) => (
                "SELECT agent_id, mode, original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score
                 FROM eco_compression_events
                 WHERE timestamp > datetime('now', ?1)",
                vec![Box::new(format!("-{} days", days))],
            ),
            None => (
                "SELECT agent_id, mode, original_tokens_est, compressed_tokens_est, savings_pct, semantic_preservation_score
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
                ))
            })
            .map_err(|e| OpenFangError::Memory(e.to_string()))?;

        let mut mode_agg: HashMap<String, Agg> = HashMap::new();
        let mut agent_mode_agg: HashMap<(String, String), Agg> = HashMap::new();
        let mut total_compression_tokens_saved: u64 = 0;
        for row in rows {
            let (agent_id, mode_raw, original, compressed, savings_pct, semantic_score) =
                row.map_err(|e| OpenFangError::Memory(e.to_string()))?;
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

        let (cache_read_tokens, estimated_cache_cost_saved_usd, weighted_input_rate_sum, weighted_rate_tokens): (
            u64,
            f64,
            f64,
            u64,
        ) = {
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
        let estimated_compression_cost_saved_usd =
            total_compression_tokens_saved as f64 * avg_input_rate_per_token;
        let estimated_total_input_tokens_saved =
            total_compression_tokens_saved.saturating_add(cache_read_tokens);
        let estimated_total_cost_saved_usd =
            estimated_cache_cost_saved_usd + estimated_compression_cost_saved_usd;

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
        })
    }
}

fn estimate_input_per_million(model: &str) -> f64 {
    let m = model.to_ascii_lowercase();
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
    fn test_record_and_query_compression_summary() {
        let store = setup();
        let a1 = AgentId::new();
        let a2 = AgentId::new();

        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a1,
                mode: "balanced".to_string(),
                original_tokens_est: 100,
                compressed_tokens_est: 60,
                savings_pct: 40,
                semantic_preservation_score: Some(0.92),
            })
            .unwrap();
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a1,
                mode: "balanced".to_string(),
                original_tokens_est: 120,
                compressed_tokens_est: 90,
                savings_pct: 25,
                semantic_preservation_score: None,
            })
            .unwrap();
        store
            .record_compression(&CompressionUsageRecord {
                agent_id: a2,
                mode: "off".to_string(),
                original_tokens_est: 80,
                compressed_tokens_est: 80,
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
            })
            .unwrap();
        let s = store.query_adaptive_eco_summary(None).unwrap();
        assert_eq!(s.events, 1);
        assert_eq!(s.shadow_mismatch_turns, 1);
        assert_eq!(s.hysteresis_blocks, 1);
    }
}
