//! JSON frame contract for AINL learning graphs (`ainl run` / `POST /run` `frame` body).
//!
//! Spec: `docs/learning-frame-v1.md` and `schemas/learning-frame-v1.schema.json`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Current `frame_version` value accepted by [`LearningFrameV1::validate`].
pub const LEARNING_FRAME_VERSION: &str = "1";

/// Default cap for [`LearningFrameV1::episode`] (bytes). Hosts should truncate before send.
pub const DEFAULT_MAX_EPISODE_BYTES: usize = 32 * 1024;

/// Default cap for total serialized JSON size (matches typical `max_frame_bytes` budgets).
pub const DEFAULT_MAX_FRAME_BYTES: usize = 256 * 1024;

/// What the learning graph is being asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningOp {
    Capture,
    Consolidate,
    SkillMint,
    Promote,
    Evolve,
}

impl LearningOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            LearningOp::Capture => "capture",
            LearningOp::Consolidate => "consolidate",
            LearningOp::SkillMint => "skill_mint",
            LearningOp::Promote => "promote",
            LearningOp::Evolve => "evolve",
        }
    }
}

/// High-level result of the episode being learned from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningOutcome {
    Ok,
    Fail,
    Partial,
}

impl LearningOutcome {
    pub const fn as_str(self) -> &'static str {
        match self {
            LearningOutcome::Ok => "ok",
            LearningOutcome::Fail => "fail",
            LearningOutcome::Partial => "partial",
        }
    }
}

/// Trust / capability mode for adapter gates (host-enforced).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LearningTier {
    Unattended,
    #[default]
    Assisted,
    User,
}

impl LearningTier {
    pub const fn as_str(self) -> &'static str {
        match self {
            LearningTier::Unattended => "unattended",
            LearningTier::Assisted => "assisted",
            LearningTier::User => "user",
        }
    }
}

fn default_frame_version() -> String {
    LEARNING_FRAME_VERSION.to_string()
}

/// Optional routing context (hand, agent, channel).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningScope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Pointers to large blobs kept out of the frame.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningRefs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prior_skill_id: Option<String>,
}

/// Small attachment list (paths remain host-controlled).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningArtifact {
    pub kind: String,
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Version 1 envelope for learning / skill pipelines (flat keys for AINL `frame` resolution).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LearningFrameV1 {
    #[serde(default = "default_frame_version")]
    pub frame_version: String,
    pub op: LearningOp,
    pub run_id: String,
    pub intent: String,
    pub outcome: LearningOutcome,
    pub episode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<LearningScope>,
    #[serde(default)]
    pub tier: LearningTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<LearningRefs>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<LearningArtifact>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub extra: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LearningFrameError {
    #[error("frame_version must be \"1\", got {0:?}")]
    UnsupportedVersion(String),
    #[error("run_id must be non-empty")]
    EmptyRunId,
    #[error("intent must be non-empty")]
    EmptyIntent,
    #[error("episode exceeds max length ({len} > {max} bytes)")]
    EpisodeTooLarge { len: usize, max: usize },
    #[error("serialized frame exceeds max length ({len} > {max} bytes)")]
    FrameTooLarge { len: usize, max: usize },
}

impl LearningFrameV1 {
    /// Build a minimal frame for tests or host bootstrap.
    pub fn minimal(
        op: LearningOp,
        run_id: impl Into<String>,
        intent: impl Into<String>,
        outcome: LearningOutcome,
        episode: impl Into<String>,
    ) -> Self {
        Self {
            frame_version: LEARNING_FRAME_VERSION.to_string(),
            op,
            run_id: run_id.into(),
            intent: intent.into(),
            outcome,
            episode: episode.into(),
            scope: None,
            tier: LearningTier::default(),
            user_note: None,
            refs: None,
            artifacts: vec![],
            tags: vec![],
            locale: None,
            created_at: None,
            extra: json!({}),
        }
    }

    /// Enforce version and size rules before `ainl run`.
    pub fn validate(
        &self,
        max_episode_bytes: usize,
        max_frame_bytes: usize,
    ) -> Result<(), LearningFrameError> {
        if self.frame_version != LEARNING_FRAME_VERSION {
            return Err(LearningFrameError::UnsupportedVersion(
                self.frame_version.clone(),
            ));
        }
        if self.run_id.trim().is_empty() {
            return Err(LearningFrameError::EmptyRunId);
        }
        if self.intent.trim().is_empty() {
            return Err(LearningFrameError::EmptyIntent);
        }
        let elen = self.episode.len();
        if elen > max_episode_bytes {
            return Err(LearningFrameError::EpisodeTooLarge {
                len: elen,
                max: max_episode_bytes,
            });
        }
        let len = serde_json::to_vec(self)
            .map(|v| v.len())
            .unwrap_or(max_frame_bytes.saturating_add(1));
        if len > max_frame_bytes {
            return Err(LearningFrameError::FrameTooLarge {
                len,
                max: max_frame_bytes,
            });
        }
        Ok(())
    }

    /// Convenience: default caps from [`DEFAULT_MAX_EPISODE_BYTES`] and [`DEFAULT_MAX_FRAME_BYTES`].
    pub fn validate_defaults(&self) -> Result<(), LearningFrameError> {
        self.validate(DEFAULT_MAX_EPISODE_BYTES, DEFAULT_MAX_FRAME_BYTES)
    }

    /// Serialize for [`crate::scheduler::CronAction::AinlRun`] `frame` (after validation).
    pub fn to_cron_json_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip_minimal() {
        let f = LearningFrameV1::minimal(
            LearningOp::SkillMint,
            "run-001",
            "Automate my weekly report",
            LearningOutcome::Ok,
            "Used tools A, B; user approved step 2.",
        );
        let j = serde_json::to_string(&f).unwrap();
        let back: LearningFrameV1 = serde_json::from_str(&j).unwrap();
        assert_eq!(f, back);
        assert!(j.contains("\"op\":\"skill_mint\""));
        assert!(j.contains("\"outcome\":\"ok\""));
    }

    #[test]
    fn validate_defaults_ok() {
        let f = LearningFrameV1::minimal(
            LearningOp::Capture,
            "r1",
            "intent",
            LearningOutcome::Partial,
            "episode",
        );
        f.validate_defaults().unwrap();
    }

    #[test]
    fn validate_rejects_empty_run_id() {
        let f = LearningFrameV1::minimal(
            LearningOp::Capture,
            "  ",
            "intent",
            LearningOutcome::Fail,
            "e",
        );
        assert!(matches!(
            f.validate_defaults(),
            Err(LearningFrameError::EmptyRunId)
        ));
    }

    #[test]
    fn validate_rejects_bad_version() {
        let mut f =
            LearningFrameV1::minimal(LearningOp::Consolidate, "r", "i", LearningOutcome::Ok, "e");
        f.frame_version = "2".into();
        assert!(matches!(
            f.validate_defaults(),
            Err(LearningFrameError::UnsupportedVersion(_))
        ));
    }
}
