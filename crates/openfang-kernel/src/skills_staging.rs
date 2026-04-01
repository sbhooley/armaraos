//! Staging paths and Markdown drafts for learning / skill-mint flows.
//!
//! Drafts live under `<home>/skills/staging/` until promoted to `skills/` (manual or future API).

use std::path::{Path, PathBuf};

use openfang_types::learning_frame::{LearningFrameV1, LearningOp, LearningOutcome, LearningScope};
use uuid::Uuid;

/// User messages starting with this prefix (ASCII case-insensitive) trigger a skill draft after `POST .../message`.
pub const LEARN_SKILL_PREFIX: &str = "[learn]";

const INTENT_FROM_PREFIX_MAX: usize = 512;
const EPISODE_USER_MAX: usize = 12_000;
const EPISODE_ASSIST_MAX: usize = 24_000;

/// If the message starts with [`LEARN_SKILL_PREFIX`], returns the intent line (rest of message, truncated).
pub fn learn_prefixed_intent(message: &str) -> Option<String> {
    let t = message.trim_start();
    let p = LEARN_SKILL_PREFIX;
    if t.len() < p.len() {
        return None;
    }
    if !t[..p.len()].eq_ignore_ascii_case(p) {
        return None;
    }
    let rest = t[p.len()..].trim();
    let intent = if rest.is_empty() {
        "User-requested skill capture".to_string()
    } else {
        openfang_types::truncate_str(rest, INTENT_FROM_PREFIX_MAX).to_string()
    };
    Some(intent)
}

/// Build a [`LearningFrameV1`] from an API chat turn (opt-in via [`learn_prefixed_intent`]).
pub fn frame_from_agent_learn_turn(
    intent: String,
    full_user_message: &str,
    assistant_response: &str,
    agent_id: &str,
    silent: bool,
) -> LearningFrameV1 {
    let mut frame = LearningFrameV1::minimal(
        LearningOp::SkillMint,
        format!("api-{}", Uuid::new_v4()),
        intent,
        if silent {
            LearningOutcome::Partial
        } else {
            LearningOutcome::Ok
        },
        String::new(),
    );
    let u = openfang_types::truncate_str(full_user_message.trim(), EPISODE_USER_MAX);
    let a = openfang_types::truncate_str(assistant_response.trim(), EPISODE_ASSIST_MAX);
    let mut episode = format!("## User message\n\n{u}\n\n## Assistant\n\n{a}");
    if episode.len() > openfang_types::learning_frame::DEFAULT_MAX_EPISODE_BYTES {
        episode = openfang_types::truncate_str(
            &episode,
            openfang_types::learning_frame::DEFAULT_MAX_EPISODE_BYTES,
        )
        .to_string();
    }
    frame.episode = episode;
    frame.scope = Some(LearningScope {
        agent_id: Some(agent_id.to_string()),
        hand_id: None,
        channel: None,
    });
    frame
}

/// `<home>/skills/staging`
pub fn skills_staging_dir(home: &Path) -> PathBuf {
    home.join("skills").join("staging")
}

fn sanitize_filename_part(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Render a deterministic SKILL-style Markdown body (matches `programs/skill-mint-stub/skill_mint_stub.ainl`).
pub fn render_skill_draft_markdown(frame: &LearningFrameV1) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&frame.intent);
    out.push_str("\n\n## Meta\n\n");
    out.push_str("- run_id: ");
    out.push_str(&frame.run_id);
    out.push_str("\n- frame_version: ");
    out.push_str(&frame.frame_version);
    out.push_str("\n- op: ");
    out.push_str(frame.op.as_str());
    out.push_str("\n- outcome: ");
    out.push_str(frame.outcome.as_str());
    out.push_str("\n- tier: ");
    out.push_str(frame.tier.as_str());
    if let Some(scope) = &frame.scope {
        if let Some(p) = &scope.hand_id {
            out.push_str("\n- hand_id: ");
            out.push_str(p);
        }
        if let Some(p) = &scope.agent_id {
            out.push_str("\n- agent_id: ");
            out.push_str(p);
        }
        if let Some(p) = &scope.channel {
            out.push_str("\n- channel: ");
            out.push_str(p);
        }
    }
    if let Some(note) = &frame.user_note {
        if !note.is_empty() {
            out.push_str("\n\n## User note\n\n");
            out.push_str(note);
        }
    }
    if let Some(refs) = &frame.refs {
        if refs.trace_uri.is_some() || refs.bundle_path.is_some() || refs.prior_skill_id.is_some() {
            out.push_str("\n\n## Refs\n\n");
            if let Some(ref u) = refs.trace_uri {
                out.push_str("- trace_uri: ");
                out.push_str(u);
                out.push('\n');
            }
            if let Some(ref u) = refs.bundle_path {
                out.push_str("- bundle_path: ");
                out.push_str(u);
                out.push('\n');
            }
            if let Some(ref u) = refs.prior_skill_id {
                out.push_str("- prior_skill_id: ");
                out.push_str(u);
                out.push('\n');
            }
        }
    }
    if !frame.tags.is_empty() {
        out.push_str("\n## Tags\n\n");
        out.push_str(&frame.tags.join(", "));
        out.push('\n');
    }
    out.push_str("\n## Episode\n\n");
    out.push_str(&frame.episode);
    out.push('\n');
    out
}

/// Validate frame, then write `draft-<run_id>-<timestamp>.md` under [`skills_staging_dir`].
pub fn write_skill_draft_markdown(home: &Path, frame: &LearningFrameV1) -> Result<PathBuf, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    frame
        .validate_defaults()
        .map_err(|e: openfang_types::learning_frame::LearningFrameError| e.to_string())?;
    let dir = skills_staging_dir(home);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create skills staging: {e}"))?;
    let slug = sanitize_filename_part(&frame.run_id);
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("draft-{slug}-{ts}.md"));
    let body = render_skill_draft_markdown(frame);
    std::fs::write(&path, body).map_err(|e| format!("write skill draft: {e}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfang_types::learning_frame::{LearningFrameV1, LearningOp, LearningOutcome};

    #[test]
    fn learn_prefix_detected() {
        assert_eq!(
            learn_prefixed_intent("[learn] how to reboot").as_deref(),
            Some("how to reboot")
        );
        assert_eq!(
            learn_prefixed_intent("  [LEARN]  ").as_deref(),
            Some("User-requested skill capture")
        );
        assert!(learn_prefixed_intent("hello").is_none());
    }

    #[test]
    fn frame_from_turn_has_agent_scope() {
        let f = frame_from_agent_learn_turn(
            "My intent".into(),
            "[learn] x",
            "reply",
            "agent-uuid",
            false,
        );
        assert_eq!(
            f.scope.as_ref().and_then(|s| s.agent_id.as_deref()),
            Some("agent-uuid")
        );
        assert!(f.episode.contains("## User message"));
    }

    #[test]
    fn render_contains_intent_and_episode() {
        let f = LearningFrameV1::minimal(
            LearningOp::SkillMint,
            "run-1",
            "Do the thing",
            LearningOutcome::Ok,
            "Used tools; success.",
        );
        let s = render_skill_draft_markdown(&f);
        assert!(s.contains("# Do the thing"));
        assert!(s.contains("run-1"));
        assert!(s.contains("Used tools; success."));
        assert!(s.contains("skill_mint"));
    }
}
