//! Integration installer — one-click add/remove flow.
//!
//! Handles the complete flow: template lookup → credential resolution →
//! OAuth if needed → write to integrations.toml → hot-reload daemon.

use crate::credentials::CredentialResolver;
use crate::registry::IntegrationRegistry;
use crate::{
    ExtensionError, ExtensionResult, InstalledIntegration, IntegrationStatus, IntegrationTemplate,
};
use chrono::Utc;
use std::collections::HashMap;
use tracing::{info, warn};
use zeroize::Zeroizing;

/// Result of an installation attempt.
#[derive(Debug)]
pub struct InstallResult {
    /// Integration ID.
    pub id: String,
    /// Final status.
    pub status: IntegrationStatus,
    /// Number of MCP tools that will be available.
    pub tool_count: usize,
    /// Message to display to the user.
    pub message: String,
}

/// Install an integration.
///
/// `secrets` are stored in the vault when possible (typically `is_secret` required env vars).
/// `config` holds non-secret values (`is_secret = false` required env vars) plus integration-specific
/// keys such as `allowed_paths` for the filesystem preset.
pub fn install_integration(
    registry: &mut IntegrationRegistry,
    resolver: &mut CredentialResolver,
    id: &str,
    secrets: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> ExtensionResult<InstallResult> {
    // 1. Look up template
    let template = registry
        .get_template(id)
        .ok_or_else(|| ExtensionError::NotFound(id.to_string()))?
        .clone();

    // Check not already installed
    if registry.is_installed(id) {
        return Err(ExtensionError::AlreadyInstalled(id.to_string()));
    }

    // 1b. Validate unknown keys (typos should fail fast)
    validate_install_maps(&template, id, secrets, config)?;

    // 2. Store provided secrets in vault
    for (key, value) in secrets {
        if value.trim().is_empty() {
            continue;
        }
        if let Err(e) = resolver.store_in_vault(key, Zeroizing::new(value.clone())) {
            warn!("Could not store {} in vault: {}", key, e);
        }
    }

    // 3. Determine credential completeness
    let required_keys: Vec<&str> = template
        .required_env
        .iter()
        .map(|e| e.name.as_str())
        .collect();

    let mut actually_missing: Vec<String> = Vec::new();
    for key in &required_keys {
        if id == "google-workspace-mcp" && *key == "GOOGLE_OAUTH_CLIENT_SECRET" {
            // Optional for PKCE-only OAuth clients; do not block Ready status.
            continue;
        }
        let meta = template
            .required_env
            .iter()
            .find(|e| e.name == *key)
            .expect("required key must exist in template");
        let has = if meta.is_secret {
            resolver.has_credential(key)
                || secrets
                    .get(*key)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
        } else {
            resolver.has_credential(key)
                || config
                    .get(*key)
                    .map(|s| !s.trim().is_empty())
                    .unwrap_or(false)
        };
        if !has {
            actually_missing.push((*key).to_string());
        }
    }

    // Filesystem preset requires at least one allowed directory.
    if id == "filesystem" {
        let paths = config.get("allowed_paths").map(|s| s.trim()).unwrap_or("");
        if paths.is_empty() {
            actually_missing.push("allowed_paths".to_string());
        }
    }

    let status = if actually_missing.is_empty() {
        IntegrationStatus::Ready
    } else {
        IntegrationStatus::Setup
    };

    // 4. Determine OAuth provider
    let oauth_provider = template.oauth.as_ref().map(|o| o.provider.clone());

    // 5. Merge install record (persist non-secret config + integration-specific keys)
    let mut merged_config = config.clone();
    // Persist non-secret required env vars into config for MCP stdio env injection
    for e in &template.required_env {
        if e.is_secret {
            continue;
        }
        if let Some(v) = config.get(&e.name) {
            merged_config.insert(e.name.clone(), v.clone());
        }
    }

    let entry = InstalledIntegration {
        id: id.to_string(),
        installed_at: Utc::now(),
        enabled: true,
        oauth_provider,
        config: merged_config,
    };
    registry.install(entry)?;

    // 6. Build result message
    let message = match &status {
        IntegrationStatus::Ready => {
            format!(
                "{} added. MCP tools will be available as mcp_{}_*.",
                template.name, id
            )
        }
        IntegrationStatus::Setup => {
            let missing_labels: Vec<String> = actually_missing
                .iter()
                .filter_map(|key| {
                    if key == "allowed_paths" {
                        return Some("Allowed directories (allowed_paths)".to_string());
                    }
                    template
                        .required_env
                        .iter()
                        .find(|e| e.name == *key)
                        .map(|e| format!("{} ({})", e.label, e.name))
                })
                .collect();
            format!(
                "{} installed but needs credentials: {}",
                template.name,
                missing_labels.join(", ")
            )
        }
        _ => format!("{} installed.", template.name),
    };

    info!("{}", message);

    Ok(InstallResult {
        id: id.to_string(),
        status,
        tool_count: 0,
        message,
    })
}

/// Install a **custom** MCP integration created from the dashboard (user-defined template).
///
/// Registers the template in the registry, then runs the same credential + `integrations.toml`
/// path as [`install_integration`]. Rolls back the in-memory template if installation fails.
pub fn install_custom_mcp(
    registry: &mut IntegrationRegistry,
    resolver: &mut CredentialResolver,
    template: IntegrationTemplate,
    secrets: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> ExtensionResult<InstallResult> {
    if crate::bundled::is_bundled_id(&template.id) {
        return Err(ExtensionError::InvalidIntegrationId(
            "id is reserved for a built-in integration".to_string(),
        ));
    }
    if registry.is_installed(&template.id) {
        return Err(ExtensionError::AlreadyInstalled(template.id.clone()));
    }

    let tid = template.id.clone();
    registry.insert_custom_template_for_install(template)?;

    match install_integration(registry, resolver, &tid, secrets, config) {
        Ok(r) => Ok(r),
        Err(e) => {
            registry.rollback_custom_template_insert(&tid);
            Err(e)
        }
    }
}

fn validate_install_maps(
    template: &crate::IntegrationTemplate,
    id: &str,
    secrets: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> ExtensionResult<()> {
    validate_user_supplied_keys(template, id, secrets, config)
}

/// Validate keys for raw dashboard payloads (`env` + `config` objects) before splitting secrets.
pub fn validate_user_supplied_keys(
    template: &crate::IntegrationTemplate,
    id: &str,
    env_like: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> ExtensionResult<()> {
    let mut allowed = std::collections::HashSet::new();
    for e in &template.required_env {
        allowed.insert(e.name.clone());
    }
    allowed.insert("allowed_paths".to_string());

    for k in env_like.keys().chain(config.keys()) {
        if !allowed.contains(k) {
            return Err(ExtensionError::TomlParse(format!(
                "Unknown field '{k}' for integration '{id}'"
            )));
        }
    }
    Ok(())
}

/// Validate dashboard / API payloads before persisting installs.
///
/// Returns a map of field name → error message (empty map means OK).
pub fn integration_payload_field_errors(
    template: &crate::IntegrationTemplate,
    id: &str,
    secrets: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut errors = HashMap::new();
    if let Err(e) = validate_install_maps(template, id, secrets, config) {
        errors.insert("_general".to_string(), e.to_string());
        return errors;
    }

    for e in &template.required_env {
        let v_secret = secrets.get(&e.name).map(|s| s.as_str()).unwrap_or("");
        let v_cfg = config.get(&e.name).map(|s| s.as_str()).unwrap_or("");
        if e.is_secret {
            if v_secret.trim().is_empty() {
                // `google-workspace-mcp` supports public OAuth 2.1 (PKCE) with no client secret;
                // workspace-mcp reads other env in that case.
                if id == "google-workspace-mcp" && e.name == "GOOGLE_OAUTH_CLIENT_SECRET" {
                    continue;
                }
                errors.insert(e.name.clone(), format!("{} is required", e.label));
            }
        } else if v_cfg.trim().is_empty() && v_secret.trim().is_empty() {
            // Non-secret values should be provided via `config` (or legacy `env` split),
            // but we still treat empty as invalid for dashboard validation.
            errors.insert(e.name.clone(), format!("{} is required", e.label));
        }
    }

    if id == "filesystem" {
        let paths = config.get("allowed_paths").map(|s| s.trim()).unwrap_or("");
        if paths.is_empty() {
            errors.insert(
                "allowed_paths".to_string(),
                "Enter at least one absolute directory path (comma-separated).".to_string(),
            );
        }
    }

    errors
}

/// Remove an installed integration.
pub fn remove_integration(registry: &mut IntegrationRegistry, id: &str) -> ExtensionResult<String> {
    let template = registry.get_template(id);
    let name = template
        .map(|t| t.name.clone())
        .unwrap_or_else(|| id.to_string());

    registry.uninstall(id)?;
    let msg = format!("{name} removed.");
    info!("{msg}");
    Ok(msg)
}

/// List all integrations with their status.
pub fn list_integrations(
    registry: &IntegrationRegistry,
    resolver: &CredentialResolver,
) -> Vec<IntegrationListEntry> {
    let mut entries = Vec::new();
    for template in registry.list_templates() {
        let installed = registry.get_installed(&template.id);
        let status = match installed {
            Some(inst) if !inst.enabled => IntegrationStatus::Disabled,
            Some(inst) => {
                let required_keys: Vec<&str> = template
                    .required_env
                    .iter()
                    .map(|e| e.name.as_str())
                    .collect();
                let mut missing = false;
                for key in &required_keys {
                    let meta = template
                        .required_env
                        .iter()
                        .find(|e| e.name == *key)
                        .unwrap();
                    let ok = if meta.is_secret {
                        resolver.has_credential(key)
                    } else {
                        resolver.has_credential(key)
                            || inst
                                .config
                                .get(*key)
                                .map(|s| !s.trim().is_empty())
                                .unwrap_or(false)
                    };
                    if !ok {
                        missing = true;
                        break;
                    }
                }
                if template.id == "filesystem" {
                    let paths = inst
                        .config
                        .get("allowed_paths")
                        .map(|s| s.trim())
                        .unwrap_or("");
                    if paths.is_empty() {
                        missing = true;
                    }
                }
                if missing {
                    IntegrationStatus::Setup
                } else {
                    IntegrationStatus::Ready
                }
            }
            None => IntegrationStatus::Available,
        };

        entries.push(IntegrationListEntry {
            id: template.id.clone(),
            name: template.name.clone(),
            icon: template.icon.clone(),
            category: template.category.to_string(),
            status,
            description: template.description.clone(),
        });
    }
    entries
}

/// Flat list entry for display.
#[derive(Debug, Clone)]
pub struct IntegrationListEntry {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub category: String,
    pub status: IntegrationStatus,
    pub description: String,
}

/// Search available integrations.
pub fn search_integrations(
    registry: &IntegrationRegistry,
    query: &str,
) -> Vec<IntegrationListEntry> {
    registry
        .search(query)
        .into_iter()
        .map(|t| {
            let installed = registry.get_installed(&t.id);
            let status = match installed {
                Some(inst) if !inst.enabled => IntegrationStatus::Disabled,
                Some(_) => IntegrationStatus::Ready,
                None => IntegrationStatus::Available,
            };
            IntegrationListEntry {
                id: t.id.clone(),
                name: t.name.clone(),
                icon: t.icon.clone(),
                category: t.category.to_string(),
                status,
                description: t.description.clone(),
            }
        })
        .collect()
}

/// Generate scaffold files for a new custom integration.
pub fn scaffold_integration(dir: &std::path::Path) -> ExtensionResult<String> {
    let template = r#"# Custom Integration Template
# Place this in ~/.openfang/integrations/ or use `openfang add --custom <path>`

id = "my-integration"
name = "My Integration"
description = "A custom MCP server integration"
category = "devtools"
icon = "🔧"
tags = ["custom"]

[transport]
type = "stdio"
command = "npx"
args = ["my-mcp-server"]

[[required_env]]
name = "MY_API_KEY"
label = "API Key"
help = "Get your API key from https://example.com/api-keys"
is_secret = true

[health_check]
interval_secs = 60
unhealthy_threshold = 3

setup_instructions = """
1. Install the MCP server: npm install -g my-mcp-server
2. Get your API key from https://example.com/api-keys
3. Run: openfang add my-integration --key=<your-key>
"""
"#;
    let path = dir.join("integration.toml");
    std::fs::create_dir_all(dir)?;
    std::fs::write(&path, template)?;
    Ok(format!(
        "Integration template created at {}",
        path.display()
    ))
}

/// Generate scaffold files for a new skill.
pub fn scaffold_skill(dir: &std::path::Path) -> ExtensionResult<String> {
    let skill_toml = r#"name = "my-skill"
description = "A custom skill"
version = "0.1.0"
runtime = "prompt_only"
"#;
    let skill_md = r#"---
name: my-skill
description: A custom skill
version: 0.1.0
runtime: prompt_only
---

# My Skill

You are an expert at [domain]. When the user asks about [topic], provide [behavior].

## Guidelines

- Be concise and accurate
- Cite sources when possible
"#;
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("skill.toml"), skill_toml)?;
    std::fs::write(dir.join("SKILL.md"), skill_md)?;
    Ok(format!("Skill scaffold created at {}", dir.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::IntegrationRegistry;

    #[test]
    fn install_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();

        let mut resolver = CredentialResolver::new(None, None);

        // Install github (will be Setup status since no token)
        let result = install_integration(
            &mut registry,
            &mut resolver,
            "github",
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.id, "github");
        // Status depends on whether GITHUB_PERSONAL_ACCESS_TOKEN is in env
        assert!(
            result.status == IntegrationStatus::Ready || result.status == IntegrationStatus::Setup
        );

        // Remove
        let msg = remove_integration(&mut registry, "github").unwrap();
        assert!(msg.contains("GitHub"));
        assert!(!registry.is_installed("github"));
    }

    #[test]
    fn install_with_key() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();

        let mut resolver = CredentialResolver::new(None, None);

        // Provide key directly
        let mut keys = HashMap::new();
        keys.insert("NOTION_TOKEN".to_string(), "ntn_test_key_123".to_string());

        let result = install_integration(
            &mut registry,
            &mut resolver,
            "notion",
            &keys,
            &HashMap::new(),
        )
        .unwrap();
        assert_eq!(result.id, "notion");
    }

    #[test]
    fn install_already_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();

        let mut resolver = CredentialResolver::new(None, None);

        install_integration(
            &mut registry,
            &mut resolver,
            "github",
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap();
        let err = install_integration(
            &mut registry,
            &mut resolver,
            "github",
            &HashMap::new(),
            &HashMap::new(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("already"));
    }

    #[test]
    fn remove_not_installed() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();
        let err = remove_integration(&mut registry, "github").unwrap_err();
        assert!(err.to_string().contains("not installed"));
    }

    #[test]
    fn list_integrations_all() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();
        let resolver = CredentialResolver::new(None, None);

        let list = list_integrations(&registry, &resolver);
        assert_eq!(list.len(), 28);
        assert!(list
            .iter()
            .all(|e| e.status == IntegrationStatus::Available));
    }

    #[test]
    fn search_integrations_query() {
        let dir = tempfile::tempdir().unwrap();
        let mut registry = IntegrationRegistry::new(dir.path());
        registry.load_bundled();

        let results = search_integrations(&registry, "git");
        assert!(results.iter().any(|e| e.id == "github"));
        assert!(results.iter().any(|e| e.id == "gitlab"));
    }

    #[test]
    fn scaffold_integration_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my-integration");
        let msg = scaffold_integration(&sub).unwrap();
        assert!(sub.join("integration.toml").exists());
        assert!(msg.contains("integration.toml"));
    }

    #[test]
    fn scaffold_skill_creates_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("my-skill");
        let msg = scaffold_skill(&sub).unwrap();
        assert!(sub.join("skill.toml").exists());
        assert!(sub.join("SKILL.md").exists());
        assert!(msg.contains("my-skill"));
    }
}
