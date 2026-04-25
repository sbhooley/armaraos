//! Workspace action contract loader for `armaraos.toml`.
//!
//! This is the declarative layer behind `workspace_action`: the model picks a named action,
//! while the runtime owns how it is executed (runner selection, env merge, daemon health checks).

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Root contract file shape.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct WorkspaceContract {
    /// Named actions exposed to the assistant.
    #[serde(default)]
    pub actions: BTreeMap<String, WorkspaceAction>,
}

/// One declarative action entry under `[actions.<name>]`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct WorkspaceAction {
    /// Human-readable intent shown in `workspace_actions_list`.
    pub description: Option<String>,
    /// Script path (relative to workspace, or absolute under allowed prefixes).
    pub script: String,
    /// Static default args.
    #[serde(default)]
    pub args: Vec<String>,
    /// Static default env vars.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Optional cwd override.
    pub cwd: Option<String>,
    /// Optional language hint (`python`, `typescript`, ...).
    pub language: Option<String>,
    /// Optional mode (`oneshot` or `daemon`).
    pub mode: Option<String>,
    /// Optional oneshot timeout.
    pub timeout_seconds: Option<u64>,
    /// Optional daemon health check.
    pub health_check: Option<WorkspaceActionHealthCheck>,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, Default)]
pub struct WorkspaceActionHealthCheck {
    pub url: String,
    pub timeout_seconds: Option<u64>,
    pub expect_status: Option<u16>,
}

/// Summary row returned to the model for `workspace_actions_list`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct WorkspaceActionSummary {
    pub name: String,
    pub description: Option<String>,
    pub script: String,
    pub mode: Option<String>,
    pub language: Option<String>,
}

/// Return the canonical contract path: `<workspace>/armaraos.toml`.
#[must_use]
pub fn contract_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join("armaraos.toml")
}

/// Load and validate the workspace action contract.
pub fn load_workspace_contract(workspace_root: &Path) -> Result<WorkspaceContract, String> {
    let path = contract_path(workspace_root);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "workspace action contract not found at `{}`: {e}",
            path.display()
        )
    })?;
    let parsed: WorkspaceContract =
        toml::from_str(&raw).map_err(|e| format!("invalid `{}` TOML: {e}", path.display()))?;
    validate_contract(&parsed, &path)?;
    Ok(parsed)
}

/// Like [`load_workspace_contract`], but returns an empty contract when file is missing.
pub fn load_workspace_contract_or_empty(
    workspace_root: &Path,
) -> Result<WorkspaceContract, String> {
    let path = contract_path(workspace_root);
    if !path.exists() {
        return Ok(WorkspaceContract::default());
    }
    load_workspace_contract(workspace_root)
}

/// Save contract TOML back to `<workspace>/armaraos.toml`.
pub fn save_workspace_contract(
    workspace_root: &Path,
    contract: &WorkspaceContract,
) -> Result<(), String> {
    let path = contract_path(workspace_root);
    validate_contract(contract, &path)?;
    let text = toml::to_string_pretty(contract)
        .map_err(|e| format!("failed to serialize `{}`: {e}", path.display()))?;
    std::fs::write(&path, text).map_err(|e| format!("failed to write `{}`: {e}", path.display()))
}

/// Create or update a named action and persist contract.
pub fn upsert_workspace_action(
    workspace_root: &Path,
    action_name: &str,
    action: WorkspaceAction,
) -> Result<WorkspaceContract, String> {
    let mut contract = load_workspace_contract_or_empty(workspace_root)?;
    let path = contract_path(workspace_root);
    validate_action_entry(action_name, &action, &path)?;
    contract.actions.insert(action_name.to_string(), action);
    save_workspace_contract(workspace_root, &contract)?;
    Ok(contract)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteWorkspaceActionOutcome {
    Deleted,
    DeletedAndRemovedContract,
}

/// Delete named action and persist. If last action is removed, deletes `armaraos.toml`.
pub fn delete_workspace_action(
    workspace_root: &Path,
    action_name: &str,
) -> Result<DeleteWorkspaceActionOutcome, String> {
    let mut contract = load_workspace_contract(workspace_root)?;
    if contract.actions.remove(action_name).is_none() {
        return Err(format!(
            "workspace action `{action_name}` not found in `{}`",
            contract_path(workspace_root).display()
        ));
    }
    if contract.actions.is_empty() {
        let p = contract_path(workspace_root);
        std::fs::remove_file(&p)
            .map_err(|e| format!("failed to remove empty contract `{}`: {e}", p.display()))?;
        return Ok(DeleteWorkspaceActionOutcome::DeletedAndRemovedContract);
    }
    save_workspace_contract(workspace_root, &contract)?;
    Ok(DeleteWorkspaceActionOutcome::Deleted)
}

fn validate_contract(contract: &WorkspaceContract, path: &Path) -> Result<(), String> {
    if contract.actions.is_empty() {
        return Err(format!(
            "`{}` has no [actions.<name>] entries",
            path.display()
        ));
    }
    for (name, action) in &contract.actions {
        validate_action_entry(name, action, path)?;
    }
    Ok(())
}

fn validate_action_entry(name: &str, action: &WorkspaceAction, path: &Path) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err(format!(
            "`{}` contains an empty action name",
            path.display()
        ));
    }
    if action.script.trim().is_empty() {
        return Err(format!(
            "action `{name}` in `{}` must set `script`",
            path.display()
        ));
    }
    if let Some(mode) = &action.mode {
        let m = mode.trim().to_ascii_lowercase();
        if m != "oneshot" && m != "daemon" {
            return Err(format!(
                "action `{name}` in `{}` has invalid mode `{mode}` (expected oneshot|daemon)",
                path.display()
            ));
        }
    }
    if let Some(hc) = &action.health_check {
        if hc.url.trim().is_empty() {
            return Err(format!(
                "action `{name}` in `{}` has empty health_check.url",
                path.display()
            ));
        }
    }
    Ok(())
}

/// Build stable summaries for listing.
#[must_use]
pub fn summarize_actions(contract: &WorkspaceContract) -> Vec<WorkspaceActionSummary> {
    contract
        .actions
        .iter()
        .map(|(name, action)| WorkspaceActionSummary {
            name: name.clone(),
            description: action.description.clone(),
            script: action.script.clone(),
            mode: action.mode.clone(),
            language: action.language.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_contract() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = contract_path(tmp.path());
        std::fs::write(
            &p,
            r#"
[actions.gateway]
description = "Start gateway server"
script = "src/gateway.ts"
mode = "daemon"
args = ["--port", "8080"]
language = "typescript"

[actions.gateway.env]
PORT = "8080"

[actions.gateway.health_check]
url = "http://127.0.0.1:8080/health"
timeout_seconds = 15
expect_status = 200
"#,
        )
        .unwrap();

        let c = load_workspace_contract(tmp.path()).unwrap();
        assert!(c.actions.contains_key("gateway"));
        let s = summarize_actions(&c);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "gateway");
    }

    #[test]
    fn rejects_invalid_mode() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = contract_path(tmp.path());
        std::fs::write(
            &p,
            r#"
[actions.bad]
script = "x.py"
mode = "forever"
"#,
        )
        .unwrap();
        let err = load_workspace_contract(tmp.path()).unwrap_err();
        assert!(err.contains("invalid mode"), "{err}");
    }

    #[test]
    fn upsert_creates_contract_when_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = upsert_workspace_action(
            tmp.path(),
            "gateway",
            WorkspaceAction {
                script: "gateway.py".to_string(),
                mode: Some("daemon".to_string()),
                ..WorkspaceAction::default()
            },
        )
        .unwrap();
        assert!(c.actions.contains_key("gateway"));
        assert!(contract_path(tmp.path()).exists());
    }

    #[test]
    fn delete_last_action_removes_contract_file() {
        let tmp = tempfile::TempDir::new().unwrap();
        upsert_workspace_action(
            tmp.path(),
            "gateway",
            WorkspaceAction {
                script: "gateway.py".to_string(),
                ..WorkspaceAction::default()
            },
        )
        .unwrap();
        let outcome = delete_workspace_action(tmp.path(), "gateway").unwrap();
        assert_eq!(
            outcome,
            DeleteWorkspaceActionOutcome::DeletedAndRemovedContract
        );
        assert!(!contract_path(tmp.path()).exists());
    }
}
