//! Dashboard **Add custom MCP server** payloads — validate, build templates, split secrets vs config.

use crate::bundled;
use crate::{
    HealthCheckConfig, IntegrationCategory, IntegrationTemplate, McpTransportTemplate,
    RequiredEnvVar,
};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Successful parse: template + vault secrets + non-secret config map.
pub type CustomMcpParsed = (
    IntegrationTemplate,
    HashMap<String, String>,
    HashMap<String, String>,
);

/// Validate integration id slug: `^[a-z][a-z0-9_-]{0,62}$`.
pub fn validate_custom_id_slug(id: &str) -> Result<(), String> {
    if id.is_empty() {
        return Err("id is required".to_string());
    }
    if id.len() > 63 {
        return Err("id must be at most 63 characters".to_string());
    }
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return Err("id is required".to_string());
    };
    if !first.is_ascii_lowercase() {
        return Err("id must start with a lowercase ASCII letter".to_string());
    }
    for c in chars {
        if !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_' && c != '-' {
            return Err(
                "id may only contain lowercase letters, digits, hyphens, and underscores"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn transport_from_json(
    t: &Value,
    errs: &mut HashMap<String, String>,
) -> Option<McpTransportTemplate> {
    let ty = t
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_lowercase();
    match ty.as_str() {
        "stdio" => {
            let cmd = t
                .get("command")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(command) = cmd.map(|s| s.to_string()) else {
                errs.insert(
                    "transport.command".to_string(),
                    "stdio transport requires non-empty command".to_string(),
                );
                return None;
            };
            let mut args: Vec<String> = Vec::new();
            if let Some(arr) = t.get("args").and_then(|v| v.as_array()) {
                for (i, item) in arr.iter().enumerate() {
                    let Some(s) = item.as_str() else {
                        errs.insert(
                            format!("transport.args.{i}"),
                            "Each arg must be a string".to_string(),
                        );
                        continue;
                    };
                    args.push(s.to_string());
                }
            }
            Some(McpTransportTemplate::Stdio { command, args })
        }
        "sse" => {
            let url = t
                .get("url")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(url) = url.map(|s| s.to_string()) else {
                errs.insert(
                    "transport.url".to_string(),
                    "sse transport requires non-empty url".to_string(),
                );
                return None;
            };
            Some(McpTransportTemplate::Sse { url })
        }
        "http" => {
            let url = t
                .get("url")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(url) = url.map(|s| s.to_string()) else {
                errs.insert(
                    "transport.url".to_string(),
                    "http transport requires non-empty url".to_string(),
                );
                return None;
            };
            Some(McpTransportTemplate::Http { url })
        }
        "" => {
            errs.insert(
                "transport.type".to_string(),
                "transport.type is required (stdio, sse, or http)".to_string(),
            );
            None
        }
        other => {
            errs.insert(
                "transport.type".to_string(),
                format!("unknown transport type '{other}' (use stdio, sse, or http)"),
            );
            None
        }
    }
}

/// Parse JSON body into a template plus secret/config maps for [`crate::installer::install_integration`].
///
/// On success, `template` has no credential values — only `required_env` metadata.
pub fn parse_custom_mcp_payload(v: &Value) -> Result<CustomMcpParsed, HashMap<String, String>> {
    let mut errs = HashMap::new();

    let id = match v.get("id").and_then(|x| x.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            errs.insert(
                "id".to_string(),
                "Required: unique id (e.g. my-github-tool)".to_string(),
            );
            String::new()
        }
    };

    if !id.is_empty() {
        if let Err(msg) = validate_custom_id_slug(&id) {
            errs.insert("id".to_string(), msg);
        }
        if bundled::is_bundled_id(&id) {
            errs.insert(
                "id".to_string(),
                "This id is reserved for a built-in integration. Pick another id.".to_string(),
            );
        }
    }

    let name = match v.get("name").and_then(|x| x.as_str()) {
        Some(s) if !s.trim().is_empty() => s.trim().to_string(),
        _ => {
            errs.insert("name".to_string(), "Display name is required".to_string());
            String::new()
        }
    };

    let icon = v
        .get("icon")
        .and_then(|x| x.as_str())
        .unwrap_or("🔌")
        .to_string();

    let description = v
        .get("description")
        .and_then(|x| x.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Custom MCP server (added from the dashboard).".to_string());

    let transport = match v.get("transport") {
        Some(t) if t.is_object() => transport_from_json(t, &mut errs),
        _ => {
            errs.insert(
                "transport".to_string(),
                "transport object is required (type, command/args or url)".to_string(),
            );
            None
        }
    };

    let mut required_env: Vec<RequiredEnvVar> = Vec::new();
    let mut secrets: HashMap<String, String> = HashMap::new();
    let mut cfg: HashMap<String, String> = HashMap::new();

    match v.get("env") {
        None | Some(Value::Null) => {}
        Some(Value::Array(arr)) => {
            let mut seen: HashSet<String> = HashSet::new();
            for (i, row) in arr.iter().enumerate() {
                let Some(obj) = row.as_object() else {
                    errs.insert(
                        format!("env.{i}"),
                        "Each env entry must be an object".to_string(),
                    );
                    continue;
                };
                let env_name = obj
                    .get("name")
                    .and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let Some(env_name) = env_name.map(|s| s.to_string()) else {
                    errs.insert(
                        format!("env.{i}.name"),
                        "Environment variable name is required".to_string(),
                    );
                    continue;
                };
                if !seen.insert(env_name.clone()) {
                    errs.insert(
                        format!("env.{i}.name"),
                        format!("Duplicate environment variable name '{}'", env_name),
                    );
                    continue;
                }
                let is_secret = obj
                    .get("is_secret")
                    .and_then(|x| x.as_bool())
                    .unwrap_or(true);
                let label = obj
                    .get("label")
                    .and_then(|x| x.as_str())
                    .unwrap_or(&env_name)
                    .to_string();
                let help = obj
                    .get("help")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Set this variable for the MCP server process.")
                    .to_string();
                let value = obj
                    .get("value")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();

                required_env.push(RequiredEnvVar {
                    name: env_name.clone(),
                    label,
                    help,
                    is_secret,
                    get_url: None,
                });

                if is_secret {
                    secrets.insert(env_name, value);
                } else {
                    cfg.insert(env_name, value);
                }
            }
        }
        _ => {
            errs.insert(
                "env".to_string(),
                "env must be an array of objects".to_string(),
            );
        }
    }

    let mut mcp_headers: Vec<String> = Vec::new();
    match v.get("headers") {
        None | Some(Value::Null) => {}
        Some(Value::Array(arr)) => {
            for (i, item) in arr.iter().enumerate() {
                let Some(line) = item.as_str() else {
                    errs.insert(
                        format!("headers.{i}"),
                        "Each header must be a string (e.g. \"Authorization: Bearer …\")"
                            .to_string(),
                    );
                    continue;
                };
                let t = line.trim();
                if !t.is_empty() {
                    mcp_headers.push(t.to_string());
                }
            }
        }
        _ => {
            errs.insert(
                "headers".to_string(),
                "headers must be an array of strings".to_string(),
            );
        }
    }

    let mcp_timeout_secs = match v.get("timeout_secs") {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) => n.as_u64(),
        _ => {
            errs.insert(
                "timeout_secs".to_string(),
                "timeout_secs must be a positive integer or omitted".to_string(),
            );
            None
        }
    };

    if let Some(t) = mcp_timeout_secs {
        if t == 0 || t > 600 {
            errs.insert(
                "timeout_secs".to_string(),
                "timeout_secs must be between 1 and 600".to_string(),
            );
        }
    }

    if !errs.is_empty() {
        return Err(errs);
    }

    let transport = transport.expect("validated transport");

    let template = IntegrationTemplate {
        id: id.clone(),
        name,
        description,
        category: IntegrationCategory::DevTools,
        icon,
        transport,
        required_env,
        oauth: None,
        tags: vec!["custom".to_string()],
        setup_instructions: String::new(),
        health_check: HealthCheckConfig::default(),
        mcp_headers,
        mcp_timeout_secs,
    };

    Ok((template, secrets, cfg))
}

/// Field-level validation for install flow (required env values, etc.).
pub fn custom_mcp_field_errors(
    template: &IntegrationTemplate,
    id: &str,
    secrets: &HashMap<String, String>,
    config: &HashMap<String, String>,
) -> HashMap<String, String> {
    crate::installer::integration_payload_field_errors(template, id, secrets, config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_stdio_minimal() {
        let v = json!({
            "id": "my-tool",
            "name": "My Tool",
            "transport": { "type": "stdio", "command": "npx", "args": ["-y", "@foo/bar"] },
            "env": []
        });
        let (t, sec, cfg) = parse_custom_mcp_payload(&v).unwrap();
        assert_eq!(t.id, "my-tool");
        assert!(sec.is_empty());
        assert!(cfg.is_empty());
    }

    #[test]
    fn rejects_bundled_id() {
        let v = json!({
            "id": "github",
            "name": "X",
            "transport": { "type": "stdio", "command": "npx" }
        });
        assert!(parse_custom_mcp_payload(&v).is_err());
    }
}
