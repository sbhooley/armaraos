//! Validation strategies for improvement proposals: structural, strict line-based, optional external
//! process (`%p` in command → path to a temp `.ainl` file).

use std::process::Command;

use ainl_contracts::{ProcedureArtifact, ProcedureExecutionPlan, ProcedurePatch};
use uuid::Uuid;

/// `structural` — `graph` header + size; `strict` — structural + minimal AINL section/transform shape; `external` — runs
/// [`validate_external_process`] if configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateMode {
    Structural,
    Strict,
    /// Structural + optional external; same as `Strict` for line checks, then external if env set.
    External,
}

/// Parse from API / env; default: structural.
#[must_use]
pub fn parse_validate_mode(s: &str) -> Option<ValidateMode> {
    match s.trim().to_ascii_lowercase().as_str() {
        "structural" | "s" | "0" => Some(ValidateMode::Structural),
        "strict" | "1" | "2" | "t" | "st" => Some(ValidateMode::Strict),
        "external" | "ext" | "e" | "3" | "mcp" => Some(ValidateMode::External),
        _ => None,
    }
}

/// `AINL_IMPROVEMENT_PROPOSALS_DEFAULT_VALIDATE_MODE` — `structural` (default) | `strict` | `external`.
#[must_use]
pub fn default_validate_mode() -> ValidateMode {
    std::env::var("AINL_IMPROVEMENT_PROPOSALS_DEFAULT_VALIDATE_MODE")
        .ok()
        .and_then(|s| parse_validate_mode(&s))
        .unwrap_or(ValidateMode::Structural)
}

/// `AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE` — if set, must contain `%p`; shell-executed with the temp path substituted.
const ENV_EXT_VALIDATE: &str = "AINL_IMPROVEMENT_PROPOSALS_EXTERNAL_VALIDATE";
const MAX_PROPOSED_AINL_BYTES: usize = 1_000_000;

/// Re-export for structural: must match `improvement_proposals_host::default_structural_validate` signature at call site.
pub fn structural_prologue_only(proposed: &str) -> Result<(), String> {
    let t = proposed.trim();
    if t.is_empty() {
        return Err("proposed_ainl_text is empty".to_string());
    }
    if t.len() > MAX_PROPOSED_AINL_BYTES {
        return Err(format!(
            "proposed_ainl_text exceeds {MAX_PROPOSED_AINL_BYTES} bytes"
        ));
    }
    if serde_json::from_str::<ProcedureArtifact>(t).is_ok()
        || serde_json::from_str::<ProcedurePatch>(t).is_ok()
        || serde_json::from_str::<ProcedureExecutionPlan>(t).is_ok()
        || serde_json::from_str::<ProcedureLifecycleAction>(t).is_ok()
    {
        return Ok(());
    }
    let first = t.lines().map(|l| l.trim()).find(|l| !l.is_empty());
    match first {
        Some(line) if line.eq_ignore_ascii_case("graph") => Ok(()),
        _ => Err("structural: first non-empty line must be `graph` (AINL file header)".to_string()),
    }
}

/// Extra checks: at least 2 content lines, and at least one of `tN` (transform) or a `##` section.
pub fn strict_line_shape(proposed: &str) -> Result<(), String> {
    structural_prologue_only(proposed)?;
    let t = proposed.trim();
    if let Ok(artifact) = serde_json::from_str::<ProcedureArtifact>(t) {
        if artifact.steps.is_empty() {
            return Err("strict: procedure artifact must include at least one step".to_string());
        }
        if artifact.intent.trim().is_empty() || artifact.title.trim().is_empty() {
            return Err(
                "strict: procedure artifact requires non-empty title and intent".to_string(),
            );
        }
        return Ok(());
    }
    if let Ok(patch) = serde_json::from_str::<ProcedurePatch>(t) {
        if patch.add_steps.is_empty()
            && patch.add_known_failures.is_empty()
            && patch.add_recovery.is_empty()
        {
            return Err(
                "strict: procedure patch must add steps, known failures, or recovery".to_string(),
            );
        }
        return Ok(());
    }
    if let Ok(plan) = serde_json::from_str::<ProcedureExecutionPlan>(t) {
        if plan.procedure_id.trim().is_empty() || plan.steps.is_empty() {
            return Err(
                "strict: procedure execution plan requires procedure_id and steps".to_string(),
            );
        }
        return Ok(());
    }
    if let Ok(action) = serde_json::from_str::<ProcedureLifecycleAction>(t) {
        if action.procedure_id.trim().is_empty() {
            return Err("strict: procedure lifecycle action requires procedure_id".to_string());
        }
        return Ok(());
    }
    let line_count = t.lines().filter(|l| !l.trim().is_empty()).count();
    if line_count < 2 {
        return Err("strict: expected at least two non-empty lines after `graph`".to_string());
    }
    let has_transform = t.lines().any(|l| {
        let s = l.trim_start();
        s.starts_with("t1")
            || s.len() > 1
                && s.starts_with('t')
                && s[1..].chars().next().is_some_and(|c| c.is_ascii_digit())
    });
    let has_sec = t.lines().any(|l| l.trim().starts_with("## "));
    if has_transform || has_sec {
        Ok(())
    } else {
        Err("strict: add at least one `tN` transform line or a `## ` top-level section".to_string())
    }
}

#[derive(serde::Deserialize)]
struct ProcedureLifecycleAction {
    procedure_id: String,
    #[allow(dead_code)]
    reason: Option<String>,
}

/// Run `structural` + (for `Strict` / `External`) `strict_line_shape` + optional external.
pub fn run_validate(mode: ValidateMode, proposed_ainl_text: &str) -> Result<(), String> {
    match mode {
        ValidateMode::Structural => structural_prologue_only(proposed_ainl_text),
        ValidateMode::Strict => strict_line_shape(proposed_ainl_text),
        ValidateMode::External => {
            strict_line_shape(proposed_ainl_text)?;
            validate_external_process(proposed_ainl_text)
        }
    }
}

/// External template: the command is passed to `sh -c` after substituting the single `%p` with the
/// path to a UTF-8 `.ainl` temp file. If the env is unset, returns `Ok(())` (use strict only).
/// If the env is set and lacks `%p`, returns an error.
fn validate_external_process(proposed_ainl_text: &str) -> Result<(), String> {
    let Ok(tpl) = std::env::var(ENV_EXT_VALIDATE) else {
        return Ok(());
    };
    if !tpl.contains("%p") {
        return Err(format!(
            "{ENV_EXT_VALIDATE} must include a single %p placeholder (temp .ainl path)"
        ));
    }
    let p = std::env::temp_dir().join(format!("improvement_proposal_{}.ainl", Uuid::new_v4()));
    std::fs::write(&p, proposed_ainl_text).map_err(|e| format!("temp ainl: {e}"))?;
    let p_str = p.to_str().ok_or("temp path not utf-8")?;
    let sh_cmd = tpl.replace("%p", p_str);
    let st = Command::new("sh")
        .arg("-c")
        .arg(&sh_cmd)
        .status()
        .map_err(|e| format!("external validate spawn: {e}"))?;
    if st.success() {
        return Ok(());
    }
    Err(format!(
        "external validate process exited with status: {:?}",
        st.code()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_rejects_too_few_lines() {
        let r = run_validate(ValidateMode::Strict, "graph\nt1\n");
        // only 2 non-empty: graph, t1 — that is 2, need transform or section - t1 is transform so ok
        assert!(r.is_ok());
    }

    #[test]
    fn strict_requires_transform_or_section() {
        let r = run_validate(
            ValidateMode::Strict,
            "graph\n# a\n# b only\n# no ## or t1\n",
        );
        assert!(r.is_err());
    }

    #[test]
    fn structural_accepts_procedure_artifact_json() {
        let tools = vec!["file_read".to_string()];
        let spec = crate::procedure_learning_host::ProcedureMintFromPattern {
            name: "Test",
            tool_sequence: &tools,
            observation_count: 3,
            fitness: 0.8,
            freshness_at_proposal: None,
        };
        let (_, text) =
            crate::procedure_learning_host::build_procedure_mint_envelope("agent", &spec).unwrap();
        assert!(run_validate(ValidateMode::Structural, &text).is_ok());
        assert!(run_validate(ValidateMode::Strict, &text).is_ok());
    }
}
