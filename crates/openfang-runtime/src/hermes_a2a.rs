//! Resolve a local Hermes A2A server base URL from `HERMES_HOME` / `~/.hermes/a2a.json`.
//!
//! Hermes (or any compatible host) writes `a2a.json` with `base_url` pointing at the HTTP
//! origin used for Agent Card discovery (`{base_url}/.well-known/agent.json`). ArmaraOS
//! tools read this file so agents can reach loopback peers without pasting URLs through
//! `web_fetch` SSRF checks on arbitrary user input.

use serde::Deserialize;
use std::path::{Path, PathBuf};

const HERMES_A2A_FILE: &str = "a2a.json";

#[derive(Debug, Deserialize)]
struct HermesA2aFile {
    base_url: String,
    /// `auto` (default) | `armaraos_jsonrpc` | `a2a_http` — see `HermesSendBinding`.
    #[serde(default)]
    send_binding: Option<String>,
}

/// How to deliver user text to a peer described by `a2a.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HermesSendBinding {
    /// Try ArmaraOS JSON-RPC `tasks/send` to `AgentCard.url`, then HTTP `message:send` candidates.
    Auto,
    /// JSON-RPC `tasks/send` only (ArmaraOS / legacy OpenFang-shaped peers).
    ArmaraosJsonRpc,
    /// HTTP+JSON `POST …/message:send` only (Linux Foundation A2A HTTP binding).
    A2aHttp,
}

impl HermesSendBinding {
    pub fn from_json(raw: Option<&str>) -> Self {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None => Self::Auto,
            Some(s) => {
                let sl = s.to_ascii_lowercase();
                match sl.as_str() {
                    "armaraos" | "armaraos_jsonrpc" | "jsonrpc" | "tasks_send" => Self::ArmaraosJsonRpc,
                    "a2a_http" | "http" | "http+json" | "message_send" => Self::A2aHttp,
                    "auto" => Self::Auto,
                    _ => Self::Auto,
                }
            }
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::ArmaraosJsonRpc => "armaraos_jsonrpc",
            Self::A2aHttp => "a2a_http",
        }
    }
}

/// Parsed `a2a.json` for Hermes / operator-local A2A routing.
#[derive(Debug, Clone)]
pub struct HermesA2aConfig {
    pub base_url: String,
    pub send_binding: HermesSendBinding,
    pub config_path: PathBuf,
}

/// Hermes data root: `HERMES_HOME` when set, else `~/.hermes`.
pub fn hermes_root() -> PathBuf {
    std::env::var("HERMES_HOME")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".hermes")
        })
}

/// Path to the Hermes A2A config file (`a2a.json`).
pub fn hermes_a2a_config_path() -> PathBuf {
    hermes_root().join(HERMES_A2A_FILE)
}

fn blocked_metadata_host(host: &str) -> bool {
    let h = host.trim().to_ascii_lowercase();
    matches!(
        h.as_str(),
        "metadata.google.internal"
            | "metadata.aws.internal"
            | "instance-data"
            | "169.254.169.254"
            | "100.100.100.200"
            | "192.0.0.192"
    )
}

/// Validate `base_url` from operator-controlled `a2a.json` (http(s) only; block cloud metadata literals).
pub fn validate_hermes_base_url(url: &str) -> Result<(), String> {
    let u = reqwest::Url::parse(url.trim()).map_err(|e| format!("invalid base_url: {e}"))?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err("Hermes base_url must use http or https".to_string());
    }
    let Some(host) = u.host_str() else {
        return Err("Hermes base_url must include a host".to_string());
    };
    if blocked_metadata_host(host) {
        return Err(format!("Hermes base_url host {host:?} is not allowed"));
    }
    Ok(())
}

/// Load and validate `{root}/a2a.json`.
pub fn load_hermes_a2a_config_from_root(root: &Path) -> Result<HermesA2aConfig, String> {
    let path = root.join(HERMES_A2A_FILE);
    let raw = std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "Hermes A2A config not found at {}: {e}. Create this file with {{\"base_url\":\"http://127.0.0.1:<port>\"}} (origin only, no path).",
            path.display()
        )
    })?;
    let cfg: HermesA2aFile = serde_json::from_str(&raw)
        .map_err(|e| format!("Invalid JSON in {}: {e}", path.display()))?;
    let base = cfg.base_url.trim().trim_end_matches('/').to_string();
    if base.is_empty() {
        return Err(format!("{}: missing or empty base_url", path.display()));
    }
    validate_hermes_base_url(&base)?;
    let send_binding = HermesSendBinding::from_json(cfg.send_binding.as_deref());
    Ok(HermesA2aConfig {
        base_url: base,
        send_binding,
        config_path: path,
    })
}

/// Load `HERMES_HOME/a2a.json` (or `~/.hermes/a2a.json`).
pub fn load_hermes_a2a_config() -> Result<HermesA2aConfig, String> {
    load_hermes_a2a_config_from_root(&hermes_root())
}

/// Load and validate `base_url` from `{root}/a2a.json`.
pub fn load_hermes_a2a_base_url_from_root(root: &Path) -> Result<(String, PathBuf), String> {
    let c = load_hermes_a2a_config_from_root(root)?;
    Ok((c.base_url, c.config_path))
}

/// Load and validate `base_url` from `HERMES_HOME/a2a.json` (or `~/.hermes/a2a.json`).
pub fn load_hermes_a2a_base_url() -> Result<(String, PathBuf), String> {
    load_hermes_a2a_base_url_from_root(&hermes_root())
}

/// Ensure the Agent Card JSON-RPC URL is same-origin with the trusted Hermes `base_url`.
pub fn assert_hermes_rpc_matches_base(base: &str, rpc: &str) -> Result<(), String> {
    let b = reqwest::Url::parse(base.trim_end_matches('/'))
        .map_err(|e| format!("invalid hermes base_url: {e}"))?;
    let r = reqwest::Url::parse(rpc.trim()).map_err(|e| format!("invalid agent card url: {e}"))?;
    if b.scheme() != r.scheme() {
        return Err("Agent card url scheme must match Hermes base_url".to_string());
    }
    if b.host_str() != r.host_str() {
        return Err(format!(
            "Agent card url host {:?} must match Hermes base_url host {:?}",
            r.host_str(),
            b.host_str()
        ));
    }
    if b.port_or_known_default() != r.port_or_known_default() {
        return Err(
            "Agent card url port must match Hermes base_url port (same-origin required)"
                .to_string(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn validate_rejects_metadata_host() {
        assert!(validate_hermes_base_url("http://169.254.169.254/").is_err());
        assert!(validate_hermes_base_url("http://metadata.google.internal/").is_err());
    }

    #[test]
    fn load_from_custom_hermes_home() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a2a.json");
        std::fs::write(
            &path,
            r#"{"base_url":"http://127.0.0.1:9999","send_binding":"a2a_http"}"#,
        )
        .unwrap();
        let c = load_hermes_a2a_config_from_root(dir.path()).expect("load");
        assert_eq!(c.base_url, "http://127.0.0.1:9999");
        assert_eq!(c.send_binding, HermesSendBinding::A2aHttp);
        assert_eq!(c.config_path, path);
    }

    #[test]
    fn assert_same_origin_ok() {
        assert_hermes_rpc_matches_base("http://127.0.0.1:8765", "http://127.0.0.1:8765/a2a/tasks")
            .unwrap();
    }

    #[test]
    fn assert_same_origin_rejects_cross_host() {
        assert!(
            assert_hermes_rpc_matches_base("http://127.0.0.1:1", "http://127.0.0.2:1").is_err()
        );
    }
}
