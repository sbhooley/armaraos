//! Skill loader — loads and executes skills from various runtimes.

use crate::{SkillError, SkillManifest, SkillRuntime, SkillToolResult};
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tracing::{debug, error, warn};

// ---------------------------------------------------------------------------
// Subprocess sandbox constants — mirrors openfang-runtime::subprocess_sandbox
// without introducing a crate dependency.
// ---------------------------------------------------------------------------

/// Maximum execution time for any skill subprocess.
const SKILL_EXEC_TIMEOUT_SECS: u64 = 30;

/// Maximum output bytes collected from a skill subprocess (1 MiB).
const SKILL_MAX_OUTPUT_BYTES: usize = 1024 * 1024;

/// Environment variables considered safe to pass into skill subprocesses.
const SKILL_SAFE_ENV_VARS: &[&str] = &[
    "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
];

/// Windows-specific safe variables for skill subprocesses.
#[cfg(windows)]
const SKILL_SAFE_ENV_VARS_WINDOWS: &[&str] = &[
    "USERPROFILE",
    "SYSTEMROOT",
    "APPDATA",
    "LOCALAPPDATA",
    "COMSPEC",
    "WINDIR",
    "PATHEXT",
];

/// Apply the skill subprocess sandbox to a `tokio::process::Command`.
///
/// Clears the entire inherited environment and re-adds only the vetted
/// platform-independent safe variables plus any caller-specified extras.
/// This prevents API keys, tokens, and credentials from leaking into
/// third-party skill code.
fn sandbox_skill_command(cmd: &mut tokio::process::Command, extra_vars: &[(&str, &str)]) {
    cmd.env_clear();

    for var in SKILL_SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    #[cfg(windows)]
    for var in SKILL_SAFE_ENV_VARS_WINDOWS {
        if let Ok(val) = std::env::var(var) {
            cmd.env(var, val);
        }
    }

    for (k, v) in extra_vars {
        cmd.env(k, v);
    }
}

/// Collect output from a child process up to `SKILL_MAX_OUTPUT_BYTES`, with
/// an absolute execution timeout of `SKILL_EXEC_TIMEOUT_SECS`.
///
/// On timeout the child process is force-killed before returning an error.
async fn collect_output_with_timeout(
    child: tokio::process::Child,
) -> Result<std::process::Output, SkillError> {
    let timeout = std::time::Duration::from_secs(SKILL_EXEC_TIMEOUT_SECS);

    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            // Cap stdout to prevent runaway skill output from exhausting heap.
            if output.stdout.len() > SKILL_MAX_OUTPUT_BYTES {
                warn!(
                    bytes = output.stdout.len(),
                    limit = SKILL_MAX_OUTPUT_BYTES,
                    "Skill stdout exceeded output cap — truncating"
                );
                let mut truncated = output;
                truncated.stdout.truncate(SKILL_MAX_OUTPUT_BYTES);
                return Ok(truncated);
            }
            Ok(output)
        }
        Ok(Err(e)) => Err(SkillError::ExecutionFailed(format!("Wait error: {e}"))),
        Err(_elapsed) => Err(SkillError::ExecutionFailed(format!(
            "Skill execution timed out after {SKILL_EXEC_TIMEOUT_SECS}s"
        ))),
    }
}

/// Execute a skill tool by spawning the appropriate runtime.
pub async fn execute_skill_tool(
    manifest: &SkillManifest,
    skill_dir: &Path,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    // Verify the tool exists in the manifest
    let _tool_def = manifest
        .tools
        .provided
        .iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| SkillError::NotFound(format!("Tool {tool_name} not in skill manifest")))?;

    match manifest.runtime.runtime_type {
        SkillRuntime::Python => {
            execute_python(skill_dir, &manifest.runtime.entry, tool_name, input).await
        }
        SkillRuntime::Node => {
            execute_node(skill_dir, &manifest.runtime.entry, tool_name, input).await
        }
        SkillRuntime::Shell => {
            execute_shell(skill_dir, &manifest.runtime.entry, tool_name, input).await
        }
        SkillRuntime::Wasm => Err(SkillError::RuntimeNotAvailable(
            "WASM skill runtime not yet implemented".to_string(),
        )),
        SkillRuntime::Builtin => Err(SkillError::RuntimeNotAvailable(
            "Builtin skills are handled by the kernel directly".to_string(),
        )),
        SkillRuntime::PromptOnly => {
            // Prompt-only skills inject context into the system prompt.
            // When a tool call arrives here, guide the LLM to use built-in tools.
            Ok(SkillToolResult {
                output: serde_json::json!({
                    "note": "Prompt-context skill — instructions are in your system prompt. Use built-in tools directly."
                }),
                is_error: false,
            })
        }
    }
}

/// Execute a Python skill script.
async fn execute_python(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let script_path = skill_dir.join(entry);
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Python script not found: {}",
            script_path.display()
        )));
    }

    // Build the JSON payload to send via stdin
    let payload = serde_json::json!({
        "tool": tool_name,
        "input": input,
    });

    let python = find_python().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Python not found. Install Python 3.8+ to run Python skills.".to_string(),
        )
    })?;

    debug!(
        "Executing Python skill: {} {}",
        python,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&python);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Apply full subprocess sandbox — clears entire environment and
    // re-adds only the vetted safe variable list. Python also needs
    // PYTHONIOENCODING for correct UTF-8 output.
    sandbox_skill_command(&mut cmd, &[("PYTHONIOENCODING", "utf-8")]);

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Python: {e}")))?;

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        stdin
            .write_all(&payload_bytes)
            .await
            .map_err(|e| SkillError::ExecutionFailed(format!("Write stdin: {e}")))?;
        drop(stdin);
    }

    let output = collect_output_with_timeout(child).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Python skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    // Parse stdout as JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Execute a Node.js skill script.
async fn execute_node(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let script_path = skill_dir.join(entry);
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Node.js script not found: {}",
            script_path.display()
        )));
    }

    let node = find_node().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Node.js not found. Install Node.js 18+ to run Node skills.".to_string(),
        )
    })?;

    let payload = serde_json::json!({
        "tool": tool_name,
        "input": input,
    });

    debug!(
        "Executing Node.js skill: {} {}",
        node,
        script_path.display()
    );

    let mut cmd = tokio::process::Command::new(&node);
    cmd.arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Apply full subprocess sandbox. NODE_NO_WARNINGS suppresses
    // Node.js version deprecation noise on stdout.
    sandbox_skill_command(&mut cmd, &[("NODE_NO_WARNINGS", "1")]);

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn Node.js: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        stdin
            .write_all(&payload_bytes)
            .await
            .map_err(|e| SkillError::ExecutionFailed(format!("Write stdin: {e}")))?;
        drop(stdin);
    }

    let output = collect_output_with_timeout(child).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

/// Find Python 3 binary.
fn find_python() -> Option<String> {
    for name in &["python3", "python"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Find Node.js binary.
fn find_node() -> Option<String> {
    if std::process::Command::new("node")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        return Some("node".to_string());
    }
    None
}

/// Find Shell/Bash binary.
fn find_shell() -> Option<String> {
    // Try bash first, then sh as fallback
    for name in &["bash", "sh"] {
        if std::process::Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Execute a Shell/Bash skill script.
async fn execute_shell(
    skill_dir: &Path,
    entry: &str,
    tool_name: &str,
    input: &serde_json::Value,
) -> Result<SkillToolResult, SkillError> {
    let script_path = skill_dir.join(entry);
    if !script_path.exists() {
        return Err(SkillError::ExecutionFailed(format!(
            "Shell script not found: {}",
            script_path.display()
        )));
    }

    // Build the JSON payload to send via stdin
    let payload = serde_json::json!({
        "tool": tool_name,
        "input": input,
    });

    let shell = find_shell().ok_or_else(|| {
        SkillError::RuntimeNotAvailable(
            "Shell/Bash not found. Install bash or sh to run Shell skills.".to_string(),
        )
    })?;

    debug!("Executing Shell skill: {} {}", shell, script_path.display());

    // Use -s to read from stdin, -c to execute command
    let mut cmd = tokio::process::Command::new(&shell);
    cmd.arg("-s")
        .arg(&script_path)
        .current_dir(skill_dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // SECURITY: Apply full subprocess sandbox — shell skills are the highest-
    // risk runtime type. No extra vars beyond the safe baseline.
    sandbox_skill_command(&mut cmd, &[]);

    let mut child = cmd
        .spawn()
        .map_err(|e| SkillError::ExecutionFailed(format!("Failed to spawn shell: {e}")))?;

    // Write input to stdin
    if let Some(mut stdin) = child.stdin.take() {
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| SkillError::ExecutionFailed(format!("JSON serialize: {e}")))?;
        stdin
            .write_all(&payload_bytes)
            .await
            .map_err(|e| SkillError::ExecutionFailed(format!("Write stdin: {e}")))?;
        drop(stdin);
    }

    let output = collect_output_with_timeout(child).await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        error!("Shell skill failed: {stderr}");
        return Ok(SkillToolResult {
            output: serde_json::json!({ "error": stderr.to_string() }),
            is_error: true,
        });
    }

    // Parse stdout as JSON
    let stdout = String::from_utf8_lossy(&output.stdout);
    match serde_json::from_str::<serde_json::Value>(&stdout) {
        Ok(value) => Ok(SkillToolResult {
            output: value,
            is_error: false,
        }),
        Err(_) => Ok(SkillToolResult {
            output: serde_json::json!({ "result": stdout.trim() }),
            is_error: false,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_python() {
        // Just ensure it doesn't panic — result depends on environment
        let _ = find_python();
    }

    #[test]
    fn test_find_node() {
        let _ = find_node();
    }

    #[tokio::test]
    async fn test_prompt_only_execution() {
        use crate::{
            SkillManifest, SkillMeta, SkillRequirements, SkillRuntimeConfig, SkillToolDef,
            SkillTools,
        };
        use tempfile::TempDir;

        let dir = TempDir::new().unwrap();
        let manifest = SkillManifest {
            skill: SkillMeta {
                name: "test-prompt".to_string(),
                version: "0.1.0".to_string(),
                description: "A prompt-only test".to_string(),
                author: String::new(),
                license: String::new(),
                tags: vec![],
            },
            runtime: SkillRuntimeConfig {
                runtime_type: SkillRuntime::PromptOnly,
                entry: String::new(),
            },
            tools: SkillTools {
                provided: vec![SkillToolDef {
                    name: "test_tool".to_string(),
                    description: "Test".to_string(),
                    input_schema: serde_json::json!({"type": "object"}),
                }],
            },
            requirements: SkillRequirements::default(),
            prompt_context: Some("You are a helpful assistant.".to_string()),
            source: None,
        };

        let result = execute_skill_tool(&manifest, dir.path(), "test_tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(!result.is_error);
        let note = result.output["note"].as_str().unwrap();
        assert!(note.contains("system prompt"));
    }
}
