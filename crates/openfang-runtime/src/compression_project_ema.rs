//! On-disk EMA for per-`project_id` compression quality / savings (Phase 5).
//!
//! When `metadata["project_id"]` is set on the agent manifest and
//! `AINL_COMPRESSION_PROJECT_EMA=1` (truthy), each qualifying turn appends a row to
//! `~/.armaraos/agents/<agent_id>/compression_project_profiles.json`.
//!
//! Hosts may later read this file to bias adaptive policy; today we persist + expose
//! `GET /api/compression/project-profiles` for operators.

use std::collections::HashMap;
use std::fs;
use std::io::{Error, ErrorKind, Result};
use std::path::{Path, PathBuf};

use ainl_compression::Compressed;
use openfang_types::agent::AgentManifest;
use openfang_types::config::openfang_home_dir;
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "compression_project_profiles.json";
const EMA_ALPHA: f64 = 0.2;

/// Persisted file shape (versioned for future migrations).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompressionProjectProfilesFile {
    pub version: u32,
    /// Key: `project_id` string from manifest metadata.
    pub projects: HashMap<String, ProjectEmaEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct CachePolicySnapshot {
    /// Last `effective_ttl_with_hysteresis` seconds (operator visibility).
    pub last_effective_ttl_secs: u64,
    /// Same-key streak fed into TTL stretch.
    pub last_streak: u32,
    /// `ainl_compression` content-classifier label before merge.
    #[serde(default)]
    pub last_content_recommendation: String,
    pub updated_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEmaEntry {
    /// EMA of observed savings ratio `tokens_saved / original_tokens` (0..=1).
    pub savings_ema: f64,
    /// EMA of semantic preservation score (0..=1).
    pub semantic_ema: f64,
    /// Number of updates applied to this project key.
    pub observations: u64,
    /// Last `efficient_mode` label applied for this turn (`off` / `balanced` / `aggressive`).
    #[serde(default)]
    pub last_applied_mode: String,
    /// Unix seconds when this row was last updated.
    #[serde(default)]
    pub updated_at_unix: i64,
    /// Richer per-project cache / adaptive hints (see Phase 5 in `SELF_LEARNING_INTEGRATION_MAP.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache: Option<CachePolicySnapshot>,
}

#[inline]
fn env_enabled() -> bool {
    std::env::var("AINL_COMPRESSION_PROJECT_EMA")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

#[must_use]
pub fn profiles_path_for_agent(agent_id: &str) -> PathBuf {
    openfang_home_dir()
        .join("agents")
        .join(agent_id.trim())
        .join(FILE_NAME)
}

/// Read and parse the JSON file, or an empty object if missing.
pub fn load_file(agent_id: &str) -> Result<CompressionProjectProfilesFile> {
    let p = profiles_path_for_agent(agent_id);
    if !p.is_file() {
        return Ok(CompressionProjectProfilesFile {
            version: 1,
            projects: HashMap::new(),
        });
    }
    let s = fs::read_to_string(&p)?;
    serde_json::from_str(&s).map_err(|e| Error::new(ErrorKind::InvalidData, e))
}

fn write_file_atomic(path: &Path, data: &CompressionProjectProfilesFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json =
        serde_json::to_string_pretty(data).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Update EMA for one project key and persist.
pub fn record_turn(
    agent_id: &str,
    project_id: &str,
    compressed: &Compressed,
    semantic: f32,
    applied_mode_label: &str,
) -> Result<()> {
    if !env_enabled() {
        return Ok(());
    }
    let pid = project_id.trim();
    if pid.is_empty() {
        return Ok(());
    }
    if compressed.original_tokens == 0 {
        return Ok(());
    }
    let savings_ratio =
        (compressed.tokens_saved() as f64) / (compressed.original_tokens as f64).max(1.0);
    let sem = f64::from(semantic).clamp(0.0, 1.0);

    let path = profiles_path_for_agent(agent_id);
    let mut file = load_file(agent_id).unwrap_or_else(|_| CompressionProjectProfilesFile {
        version: 1,
        projects: HashMap::new(),
    });
    if file.version == 0 {
        file.version = 1;
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    file.projects
        .entry(pid.to_string())
        .and_modify(|e| {
            e.savings_ema = EMA_ALPHA * savings_ratio + (1.0 - EMA_ALPHA) * e.savings_ema;
            e.semantic_ema = EMA_ALPHA * sem + (1.0 - EMA_ALPHA) * e.semantic_ema;
            e.observations = e.observations.saturating_add(1);
            e.last_applied_mode = applied_mode_label.to_string();
            e.updated_at_unix = now;
        })
        .or_insert_with(|| ProjectEmaEntry {
            savings_ema: savings_ratio,
            semantic_ema: sem,
            observations: 1,
            last_applied_mode: applied_mode_label.to_string(),
            updated_at_unix: now,
            cache: None,
        });

    write_file_atomic(&path, &file)
}

/// Operator / CLI override: merge or create a per-`project_id` row and persist (does not require
/// `AINL_COMPRESSION_PROJECT_EMA=1` — used for hand-tuned baselines and scripting).
pub fn operator_merge_project_entry(
    agent_id: &str,
    project_id: &str,
    savings_ema: Option<f64>,
    semantic_ema: Option<f64>,
    last_applied_mode: Option<&str>,
) -> Result<()> {
    let pid = project_id.trim();
    if pid.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "project_id must be non-empty",
        ));
    }
    let path = profiles_path_for_agent(agent_id);
    let mut file = load_file(agent_id)?;
    if file.version == 0 {
        file.version = 1;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    file.projects
        .entry(pid.to_string())
        .and_modify(|e| {
            if let Some(s) = savings_ema {
                e.savings_ema = s.clamp(0.0, 1.0);
            }
            if let Some(s) = semantic_ema {
                e.semantic_ema = s.clamp(0.0, 1.0);
            }
            if let Some(m) = last_applied_mode {
                e.last_applied_mode = m.trim().to_string();
            }
            e.observations = e.observations.saturating_add(1);
            e.updated_at_unix = now;
        })
        .or_insert_with(|| {
            let savings = savings_ema.unwrap_or(0.0).clamp(0.0, 1.0);
            let sem = semantic_ema.unwrap_or(0.0).clamp(0.0, 1.0);
            ProjectEmaEntry {
                savings_ema: savings,
                semantic_ema: sem,
                observations: 1,
                last_applied_mode: last_applied_mode.unwrap_or("").trim().to_string(),
                updated_at_unix: now,
                cache: None,
            }
        });
    write_file_atomic(&path, &file)
}

/// If manifest has `project_id` and mode is not off, update EMA (best-effort, logs nothing on err).
pub fn maybe_record_from_turn(
    agent_id: &str,
    manifest: &AgentManifest,
    mode: crate::prompt_compressor::EfficientMode,
    compressed: &Compressed,
    semantic: Option<f32>,
) {
    if matches!(mode, crate::prompt_compressor::EfficientMode::Off) {
        return;
    }
    let Some(semantic) = semantic else {
        return;
    };
    let Some(pid) = manifest
        .metadata
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
    else {
        return;
    };
    if pid.is_empty() {
        return;
    }
    let label = match mode {
        crate::prompt_compressor::EfficientMode::Off => "off",
        crate::prompt_compressor::EfficientMode::Balanced => "balanced",
        crate::prompt_compressor::EfficientMode::Aggressive => "aggressive",
    };
    if let Err(e) = record_turn(agent_id, pid, compressed, semantic, label) {
        tracing::debug!(%agent_id, ?e, "compression_project_ema: persist failed (non-fatal)");
    }
}

/// Merge adaptive **cache** telemetry into the on-disk per-project row (best-effort).
pub fn maybe_record_cache_from_adaptive_snapshot(
    agent_id: &str,
    manifest: &AgentManifest,
    snap: &openfang_types::adaptive_eco::AdaptiveEcoTurnSnapshot,
) {
    if !env_enabled() {
        return;
    }
    if !crate::eco_mode_resolver::env_ainl_adaptive_compression() {
        return;
    }
    let Some(pid) = manifest
        .metadata
        .get("project_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
    else {
        return;
    };
    if pid.is_empty() {
        return;
    }
    let ttl = snap.cache_effective_ttl_secs.unwrap_or(0);
    let streak = snap.cache_prompt_streak.unwrap_or(0);
    let cr = snap.content_recommendation.as_deref().unwrap_or("");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let path = profiles_path_for_agent(agent_id);
    let mut file = match load_file(agent_id) {
        Ok(f) => f,
        Err(e) => {
            tracing::debug!(%agent_id, ?e, "compression_project_ema: load for cache merge failed");
            return;
        }
    };
    if file.version == 0 {
        file.version = 1;
    }
    file.projects
        .entry(pid.to_string())
        .and_modify(|e| {
            e.cache = Some(CachePolicySnapshot {
                last_effective_ttl_secs: ttl,
                last_streak: streak,
                last_content_recommendation: cr.to_string(),
                updated_at_unix: now,
            });
            e.updated_at_unix = now;
        })
        .or_insert_with(|| ProjectEmaEntry {
            savings_ema: 0.0,
            semantic_ema: 0.0,
            observations: 0,
            last_applied_mode: String::new(),
            updated_at_unix: now,
            cache: Some(CachePolicySnapshot {
                last_effective_ttl_secs: ttl,
                last_streak: streak,
                last_content_recommendation: cr.to_string(),
                updated_at_unix: now,
            }),
        });
    if let Err(e) = write_file_atomic(&path, &file) {
        tracing::debug!(%agent_id, ?e, "compression_project_ema: cache merge write failed (non-fatal)");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_sample_file() {
        let mut f = CompressionProjectProfilesFile {
            version: 1,
            projects: HashMap::new(),
        };
        f.projects.insert(
            "p1".into(),
            ProjectEmaEntry {
                savings_ema: 0.5,
                semantic_ema: 0.9,
                observations: 2,
                last_applied_mode: "balanced".into(),
                updated_at_unix: 1,
                cache: None,
            },
        );
        let s = serde_json::to_string(&f).expect("ser");
        let g: CompressionProjectProfilesFile = serde_json::from_str(&s).expect("de");
        assert_eq!(g.projects["p1"].observations, 2);
    }
}
