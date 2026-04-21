//! Repair common `~/.armaraos/config.toml` corruption from older AINL MCP bootstrap merges.
//!
//! Stale **multiline** `env = [ ... ]` tails sometimes remained after collapsing `env` to a single
//! line, producing invalid TOML (quoted rows / a stray `]` after a completed `env = [...]`).
//! Python `tooling/mcp_host_install.py` fixes this at source; the kernel also repairs on load so
//! embedded/desktop builds are safe before a new `ainativelang` wheel ships.

/// Remove stale lines after a completed `env = [ ... ]` inside each `[[mcp_servers]]` block.
pub(crate) fn repair_stale_mcp_env_continuations(raw: &str) -> String {
    let ends_with_newline = raw.ends_with('\n');
    let lines: Vec<&str> = raw.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == "[[mcp_servers]]" {
            out.push(lines[i].to_string());
            i += 1;
            let start = i;
            while i < lines.len() && lines[i].trim() != "[[mcp_servers]]" {
                i += 1;
            }
            let segment = &lines[start..i];
            out.extend(repair_mcp_servers_segment(segment));
            continue;
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut s = out.join("\n");
    if ends_with_newline && !s.ends_with('\n') {
        s.push('\n');
    }
    s
}

fn bracket_depth_delta(s: &str) -> i32 {
    s.chars().filter(|c| *c == '[').count() as i32
        - s.chars().filter(|c| *c == ']').count() as i32
}

/// First line index *after* the `env = ...` value (possibly multiline) for bracket balance.
fn env_array_balanced_end(lines: &[&str], env_idx: usize) -> usize {
    let first = lines[env_idx];
    let rhs = first.split_once('=').map(|(_, r)| r).unwrap_or("");
    let mut depth = bracket_depth_delta(rhs);
    let mut end = env_idx + 1;
    while depth > 0 && end < lines.len() {
        depth += bracket_depth_delta(lines[end]);
        end += 1;
    }
    end
}

fn env_assignment_rhs(lines: &[&str], env_idx: usize, balanced_end: usize) -> String {
    lines[env_idx..balanced_end]
        .join("\n")
        .split_once('=')
        .map(|(_, r)| r.trim().to_string())
        .unwrap_or_default()
}

fn parse_json_string_array(rhs: &str) -> Option<Vec<String>> {
    serde_json::from_str::<Vec<String>>(rhs.trim()).ok()
}

fn is_stale_env_continuation_line(line: &str) -> bool {
    let s = line.trim().trim_end_matches('\r');
    if s.is_empty() {
        return false;
    }
    if s == "]" {
        return true;
    }
    if s.starts_with('"') {
        if s.ends_with(',') {
            return true;
        }
        if s.len() >= 2 && s.ends_with('"') {
            return true;
        }
    }
    false
}

fn repair_mcp_servers_segment(segment: &[&str]) -> Vec<String> {
    let env_idx = segment
        .iter()
        .position(|l| l.trim_start().starts_with("env = "));
    let Some(env_idx) = env_idx else {
        return segment.iter().map(|s| (*s).to_string()).collect();
    };
    let balanced_end = env_array_balanced_end(segment, env_idx);
    let rhs = env_assignment_rhs(segment, env_idx, balanced_end);
    if parse_json_string_array(&rhs).is_none() {
        return segment.iter().map(|s| (*s).to_string()).collect();
    }
    let mut j = balanced_end;
    while j < segment.len() && is_stale_env_continuation_line(segment[j]) {
        j += 1;
    }
    if j == balanced_end {
        return segment.iter().map(|s| (*s).to_string()).collect();
    }
    let mut out = Vec::with_capacity(segment.len() - (j - balanced_end));
    out.extend(segment[..balanced_end].iter().map(|s| (*s).to_string()));
    out.extend(segment[j..].iter().map(|s| (*s).to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::repair_stale_mcp_env_continuations;

    #[test]
    fn repairs_duplicate_env_tail_after_single_line_array() {
        let broken = r#"config_schema_version = 1

[[mcp_servers]]
env = ["AINL_MCP_EXPOSURE_PROFILE", "AINL_MCP_RESOURCES", "AINL_MCP_RESOURCES_EXCLUDE", "AINL_MCP_TOOLS", "AINL_MCP_TOOLS_EXCLUDE"]
    "AINL_MCP_EXPOSURE_PROFILE",
    "AINL_MCP_RESOURCES",
    "AINL_MCP_RESOURCES_EXCLUDE",
    "AINL_MCP_TOOLS",
    "AINL_MCP_TOOLS_EXCLUDE",
]
name = "ainl"
timeout_secs = 30

[mcp_servers.transport]
type = "stdio"
command = "/path/ainl-mcp"
args = []
"#;
        let fixed = repair_stale_mcp_env_continuations(broken);
        assert!(
            !fixed.contains("\n    \"AINL_MCP_EXPOSURE_PROFILE\",\n"),
            "expected orphan quoted lines removed: {fixed}"
        );
        assert!(
            toml::from_str::<toml::Value>(&fixed).is_ok(),
            "repaired TOML should parse: {fixed}"
        );
    }
}
