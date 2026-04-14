//! Tool name → canonical semantic tool tags.

use crate::tag::{SemanticTag, TagNamespace};

fn sanitize_tool_slug(name: &str) -> String {
    let mut s = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    while s.contains("__") {
        s = s.replace("__", "_");
    }
    s.trim_matches('_').to_string()
}

fn map_tool_name(raw: &str) -> SemanticTag {
    let t = raw.trim();
    let l = t.to_ascii_lowercase();

    let (value, confidence) = if l == "python_repl"
        || l == "python"
        || l == "python3"
        || l.contains("python_repl")
    {
        ("python_repl", 0.85_f32)
    } else if l == "bash" || l == "shell" || l == "sh" || l.contains("shell") {
        ("bash", 0.85_f32)
    } else if l == "search_web" || l == "web_search" || l.contains("web_search") {
        ("search_web", 0.85_f32)
    } else if l == "web_fetch" || l == "fetch" || l.contains("web_fetch") {
        ("web_fetch", 0.85_f32)
    } else if l == "file_write" || l == "write_file" || l.contains("file_write") {
        ("file_write", 0.85_f32)
    } else if l == "file_read" || l.contains("file_read") {
        ("file_read", 0.85_f32)
    } else if l == "ainl_mcp" || l == "mcp" || l.contains("mcp") {
        ("mcp", 0.85_f32)
    } else if l == "cli" {
        ("cli", 0.85_f32)
    } else {
        let slug = sanitize_tool_slug(t);
        let slug = if slug.is_empty() {
            "unknown".to_string()
        } else {
            slug
        };
        return SemanticTag {
            namespace: TagNamespace::Tool,
            value: slug,
            confidence: 0.5,
        };
    };

    SemanticTag {
        namespace: TagNamespace::Tool,
        value: value.to_string(),
        confidence,
    }
}

/// Maps tool name strings to canonical [`TagNamespace::Tool`] tags. Unknown tools are retained with
/// a sanitized slug and confidence `0.5`.
pub fn tag_tool_names(tools: &[String]) -> Vec<SemanticTag> {
    tools
        .iter()
        .filter(|t| !t.trim().is_empty())
        .map(|t| map_tool_name(t))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_shell_exec_to_bash() {
        let v = tag_tool_names(&["shell_exec".into()]);
        assert_eq!(v[0].value, "bash");
    }

    #[test]
    fn maps_sh_to_bash() {
        let v = tag_tool_names(&["sh".into()]);
        assert_eq!(v[0].value, "bash");
    }

    #[test]
    fn unknown_tool_kept() {
        let v = tag_tool_names(&["my_custom_tool".into()]);
        assert_eq!(v[0].value, "my_custom_tool");
        assert!((v[0].confidence - 0.5).abs() < f32::EPSILON);
    }
}
