//! FangHub marketplace client — install skills from the registry.
//!
//! For Phase 1, uses GitHub releases as the registry backend.
//! Each skill is a GitHub repo with releases containing the skill bundle.

use crate::openclaw_compat;
use crate::verify::{SkillVerifier, WarningSeverity};
use crate::SkillError;
use sha2::{Digest, Sha256};
use std::path::Path;
use tracing::{info, warn};

/// FangHub registry configuration.
#[derive(Debug, Clone)]
pub struct MarketplaceConfig {
    /// Base URL for the registry API.
    pub registry_url: String,
    /// GitHub organization for community skills.
    pub github_org: String,
}

impl Default for MarketplaceConfig {
    fn default() -> Self {
        Self {
            registry_url: "https://api.github.com".to_string(),
            github_org: "openfang-skills".to_string(),
        }
    }
}

/// Client for the FangHub marketplace.
pub struct MarketplaceClient {
    config: MarketplaceConfig,
    http: reqwest::Client,
}

impl MarketplaceClient {
    /// Create a new marketplace client.
    pub fn new(config: MarketplaceConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .user_agent("openfang-skills/0.1")
                .build()
                .expect("Failed to build HTTP client"),
        }
    }

    /// Search for skills by query string.
    pub async fn search(&self, query: &str) -> Result<Vec<SkillSearchResult>, SkillError> {
        let url = format!(
            "{}/search/repositories?q={}+org:{}&sort=stars",
            self.config.registry_url, query, self.config.github_org
        );

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Search request failed: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::Network(format!(
                "Search returned status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse search response: {e}")))?;

        let results = body["items"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .map(|item| SkillSearchResult {
                        name: item["name"].as_str().unwrap_or("").to_string(),
                        description: item["description"].as_str().unwrap_or("").to_string(),
                        stars: item["stargazers_count"].as_u64().unwrap_or(0),
                        url: item["html_url"].as_str().unwrap_or("").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(results)
    }

    /// Install a skill from a GitHub repo by name.
    ///
    /// Security pipeline (mirrors the ClawHub path):
    /// 1. Fetch latest release metadata from GitHub
    /// 2. Download the skill bundle and compute SHA256
    /// 3. Extract zip (with zip-slip protection) or save raw content
    /// 4. If SKILL.md format: run prompt injection scan — block on Critical
    /// 5. If skill.toml present: run manifest security scan — block on Critical
    /// 6. Write marketplace_meta.json with computed hash for integrity record
    pub async fn install(&self, skill_name: &str, target_dir: &Path) -> Result<String, SkillError> {
        let repo = format!("{}/{}", self.config.github_org, skill_name);
        let url = format!(
            "{}/repos/{}/releases/latest",
            self.config.registry_url, repo
        );

        info!("Fetching skill info from {url}");

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Fetch release: {e}")))?;

        if !resp.status().is_success() {
            return Err(SkillError::NotFound(format!(
                "Skill '{skill_name}' not found in marketplace (status {})",
                resp.status()
            )));
        }

        let release: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| SkillError::Network(format!("Parse release: {e}")))?;

        let version = release["tag_name"]
            .as_str()
            .unwrap_or("unknown")
            .to_string();

        // Find the tarball asset
        let tarball_url = release["tarball_url"]
            .as_str()
            .ok_or_else(|| SkillError::Network("No tarball URL in release".to_string()))?;

        info!("Downloading skill {skill_name} {version}...");

        let skill_dir = target_dir.join(skill_name);
        std::fs::create_dir_all(&skill_dir)?;

        // Download the bundle
        let tar_resp = self
            .http
            .get(tarball_url)
            .send()
            .await
            .map_err(|e| SkillError::Network(format!("Download tarball: {e}")))?;

        if !tar_resp.status().is_success() {
            let _ = std::fs::remove_dir_all(&skill_dir);
            return Err(SkillError::Network(format!(
                "Download failed: {}",
                tar_resp.status()
            )));
        }

        let bytes = tar_resp
            .bytes()
            .await
            .map_err(|e| SkillError::Network(format!("Read download body: {e}")))?;

        // Step 2: Compute and log SHA256 for integrity record.
        let sha256 = {
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            hex::encode(hasher.finalize())
        };
        info!(skill_name, sha256 = %sha256, version = %version, "Downloaded FangHub skill");

        // Step 3: Extract if zip, otherwise save raw.
        let content_str = String::from_utf8_lossy(&bytes);
        let is_skillmd = content_str.trim_start().starts_with("---");

        if is_skillmd {
            std::fs::write(skill_dir.join("SKILL.md"), &*bytes)?;
        } else if bytes.len() >= 4 && bytes[0] == 0x50 && bytes[1] == 0x4b {
            let cursor = std::io::Cursor::new(&*bytes);
            match zip::ZipArchive::new(cursor) {
                Ok(mut archive) => {
                    for i in 0..archive.len() {
                        let mut file = match archive.by_index(i) {
                            Ok(f) => f,
                            Err(e) => {
                                warn!(index = i, error = %e, "Skipping zip entry");
                                continue;
                            }
                        };
                        // Zip-slip protection: skip entries with unsafe paths.
                        let Some(enclosed_name) = file.enclosed_name() else {
                            warn!("Skipping zip entry with unsafe path");
                            continue;
                        };
                        let out_path = skill_dir.join(enclosed_name);
                        if file.is_dir() {
                            std::fs::create_dir_all(&out_path)?;
                        } else {
                            if let Some(parent) = out_path.parent() {
                                std::fs::create_dir_all(parent)?;
                            }
                            let mut out_file = std::fs::File::create(&out_path)?;
                            std::io::copy(&mut file, &mut out_file)?;
                        }
                    }
                }
                Err(e) => {
                    warn!(skill_name, error = %e, "Failed to read zip, saving raw bytes");
                    std::fs::write(skill_dir.join("skill.tar.gz"), &*bytes)?;
                }
            }
        } else {
            std::fs::write(skill_dir.join("skill.tar.gz"), &*bytes)?;
        }

        // Step 4: Prompt injection scan for SKILL.md format.
        if openclaw_compat::detect_skillmd(&skill_dir) {
            match openclaw_compat::convert_skillmd(&skill_dir) {
                Ok(converted) => {
                    let prompt_warnings =
                        SkillVerifier::scan_prompt_content(&converted.prompt_context);
                    if prompt_warnings
                        .iter()
                        .any(|w| w.severity == WarningSeverity::Critical)
                    {
                        let critical_msgs: Vec<_> = prompt_warnings
                            .iter()
                            .filter(|w| w.severity == WarningSeverity::Critical)
                            .map(|w| w.message.clone())
                            .collect();
                        let _ = std::fs::remove_dir_all(&skill_dir);
                        return Err(SkillError::SecurityBlocked(format!(
                            "FangHub skill '{skill_name}' blocked — prompt injection detected: {}",
                            critical_msgs.join("; ")
                        )));
                    }
                    for w in &prompt_warnings {
                        warn!(skill_name, "[{:?}] {}", w.severity, w.message);
                    }
                }
                Err(e) => {
                    warn!(skill_name, error = %e, "Could not parse SKILL.md for security scan");
                }
            }
        }

        // Step 5: Manifest security scan if skill.toml exists.
        let manifest_path = skill_dir.join("skill.toml");
        if manifest_path.exists() {
            match std::fs::read_to_string(&manifest_path)
                .ok()
                .and_then(|s| toml::from_str::<crate::SkillManifest>(&s).ok())
            {
                Some(manifest) => {
                    let manifest_warnings = SkillVerifier::security_scan(&manifest);
                    if manifest_warnings
                        .iter()
                        .any(|w| w.severity == WarningSeverity::Critical)
                    {
                        let critical_msgs: Vec<_> = manifest_warnings
                            .iter()
                            .filter(|w| w.severity == WarningSeverity::Critical)
                            .map(|w| w.message.clone())
                            .collect();
                        let _ = std::fs::remove_dir_all(&skill_dir);
                        return Err(SkillError::SecurityBlocked(format!(
                            "FangHub skill '{skill_name}' manifest blocked: {}",
                            critical_msgs.join("; ")
                        )));
                    }
                    for w in &manifest_warnings {
                        warn!(skill_name, "[{:?}] {}", w.severity, w.message);
                    }
                }
                None => {
                    warn!(
                        skill_name,
                        "Could not parse skill.toml for manifest security scan"
                    );
                }
            }
        }

        // Step 6: Write metadata with computed hash for integrity tracking.
        let meta = serde_json::json!({
            "name": skill_name,
            "version": version,
            "source": tarball_url,
            "sha256": sha256,
            "installed_at": chrono::Utc::now().to_rfc3339(),
        });
        std::fs::write(
            skill_dir.join("marketplace_meta.json"),
            serde_json::to_string_pretty(&meta).unwrap_or_default(),
        )?;

        info!("Installed FangHub skill: {skill_name} {version}");
        Ok(version)
    }
}

/// A search result from the marketplace.
#[derive(Debug, Clone)]
pub struct SkillSearchResult {
    /// Skill name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Star count.
    pub stars: u64,
    /// Repository URL.
    pub url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = MarketplaceConfig::default();
        assert!(config.registry_url.contains("github"));
        assert_eq!(config.github_org, "openfang-skills");
    }

    #[test]
    fn test_client_creation() {
        let client = MarketplaceClient::new(MarketplaceConfig::default());
        assert_eq!(client.config.github_org, "openfang-skills");
    }
}
