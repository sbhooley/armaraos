//! [`RecommendedNextTools`](ainl_contracts::RecommendedNextTools) state machine for MCP responses.

use ainl_contracts::{RecommendedNextTools, RecommendedToolStep, CONTRACT_SCHEMA_VERSION};

/// Phase of the authoring loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoringPhase {
    AfterEdit,
    AfterValidateOk,
    AfterCompileOk,
    AfterImpactOk,
    ReadyToRun,
}

/// Build next tools from phase (deterministic).
#[must_use]
pub fn recommend_next_tools(phase: AuthoringPhase, strict_impact: bool) -> RecommendedNextTools {
    let mut steps: Vec<RecommendedToolStep> = Vec::new();
    match phase {
        AuthoringPhase::AfterEdit => {
            steps.push(step("ainl_validate", "Validate before compile"));
        }
        AuthoringPhase::AfterValidateOk => {
            steps.push(step("ainl_compile", "IR for diff/impact"));
            if strict_impact {
                steps.push(step(
                    "ainl_ir_diff",
                    "Compare IR vs previous (impact awareness)",
                ));
            }
        }
        AuthoringPhase::AfterCompileOk => {
            if strict_impact {
                steps.push(step("ainl_ir_diff", "Confirm blast radius on IR delta"));
            }
            steps.push(step("ainl_run", "Execute with registered adapters"));
        }
        AuthoringPhase::AfterImpactOk => {
            steps.push(step("ainl_run", "Execute"));
        }
        AuthoringPhase::ReadyToRun => {
            steps.push(step("ainl_run", "Execute"));
        }
    }
    RecommendedNextTools {
        schema_version: CONTRACT_SCHEMA_VERSION,
        steps,
    }
}

fn step(tool: &str, reason: &str) -> RecommendedToolStep {
    RecommendedToolStep {
        tool: tool.to_string(),
        reason: Some(reason.to_string()),
    }
}

/// Full golden chain for docs and MCP resources.
#[must_use]
pub fn golden_chain() -> RecommendedNextTools {
    RecommendedNextTools::golden_default_chain()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn after_edit_requests_validate() {
        let r = recommend_next_tools(AuthoringPhase::AfterEdit, false);
        assert_eq!(
            r.steps.first().map(|s| s.tool.as_str()),
            Some("ainl_validate")
        );
    }
}
