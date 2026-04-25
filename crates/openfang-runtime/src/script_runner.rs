//! Deterministic **script runner resolver** for the `script_run` builtin tool.
//!
//! The agent shouldn't have to compose `source venv/bin/activate && python script.py …`
//! or `nohup node server.js &` — those compositions are the exact place LLMs hallucinate
//! and where fragile, OS-specific shell quoting fails. Instead this module takes a
//! **script path** (relative or absolute) plus an optional `language` hint and returns
//! a fully resolved interpreter + argv:
//!
//! - `.py`  → `<workspace>/.venv/bin/python3` if present, else `venv/bin/python3`,
//!   else `python3` on `PATH`. Honors a project-pinned **shebang** when present.
//! - `.sh`  → `bash` (POSIX) or Git Bash on Windows; falls back to `sh` if `bash` is missing.
//! - `.js`  / `.mjs` / `.cjs` → `node`.
//! - `.ts`  / `.tsx` → `<workspace>/node_modules/.bin/tsx` if present (preferred,
//!   zero-config), else `npx --yes tsx`. Bun / Deno taken if a project lockfile indicates them.
//! - `.bash` → `bash` (or `sh` fallback). `.zsh` → `zsh`.
//!
//! All filesystem checks are done with a small canonical-path cache (see
//! [`crate::path_canon_cache`]) so repeated `script_run` calls in the same loop are cheap.
//!
//! ## Why a separate module
//!
//! Keeping this in its own module makes it trivially unit-testable without spinning up the
//! full agent loop: every interesting decision (extension → runner, shebang override,
//! venv preference) is a pure function over `(script_path, workspace_root, language_hint,
//! lookup_fn)`. The actual `tool_runner.rs` glue stays small and obvious.

use std::path::{Path, PathBuf};

/// What language family a script belongs to. Used for human-readable hints in tool output
/// and for picking the right interpreter family when no extension is present.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptLanguage {
    Python,
    Shell,
    Bash,
    Zsh,
    Node,
    TypeScript,
    Bun,
    Deno,
    /// Detected via shebang or fully-explicit `language` hint we don't have a builtin for.
    Other(String),
}

impl ScriptLanguage {
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Python => "python",
            Self::Shell => "shell",
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Node => "node",
            Self::TypeScript => "typescript",
            Self::Bun => "bun",
            Self::Deno => "deno",
            Self::Other(s) => s.as_str(),
        }
    }
}

/// Result of [`resolve_runner`] — everything `tool_runner` needs to spawn the script
/// without further guessing.
#[derive(Debug, Clone)]
pub struct ResolvedRunner {
    /// Absolute path (or bare program name like `python3`) of the interpreter we will spawn.
    pub interpreter: String,
    /// Args that go *before* the script path (e.g. `["--yes", "tsx"]` for `npx --yes tsx`).
    pub interpreter_args: Vec<String>,
    /// Canonical absolute path to the script itself.
    pub script_path: PathBuf,
    /// Detected (or hinted) language family for telemetry / hints.
    pub language: ScriptLanguage,
    /// Default working directory (caller may override). Either the script's parent dir
    /// or the workspace root when the script lives at workspace root.
    pub default_cwd: PathBuf,
    /// Source of the runner decision: `"extension"`, `"shebang"`, `"language_hint"`, or
    /// `"project_marker"` (e.g. `bun.lock`, `deno.json`).
    pub decision_source: &'static str,
}

impl ResolvedRunner {
    /// Build the full argv (interpreter args + script + caller args) for spawning.
    #[must_use]
    pub fn full_argv(&self, caller_args: &[String]) -> Vec<String> {
        let mut argv = Vec::with_capacity(self.interpreter_args.len() + 1 + caller_args.len());
        argv.extend(self.interpreter_args.iter().cloned());
        argv.push(self.script_path.to_string_lossy().to_string());
        argv.extend(caller_args.iter().cloned());
        argv
    }
}

/// Errors from [`resolve_runner`]. All variants carry a human-readable, actionable hint
/// that surfaces directly in the `script_run` tool's error payload — that hint is what
/// the LLM gets to read and what stops it from blindly retrying or falling back to
/// "paste this into Terminal".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    /// The script path doesn't exist (or isn't a regular file).
    NotFound { tried: String, hint: String },
    /// Path resolved successfully but lives outside the workspace and any allowed prefixes.
    OutsideWorkspace {
        path: String,
        workspace: Option<String>,
        hint: String,
    },
    /// Extension isn't one of the supported families and no `language` hint was provided.
    UnknownExtension { ext: String, hint: String },
    /// The `language` hint was provided but isn't recognized.
    UnknownLanguageHint { hint_value: String, hint: String },
    /// Generic "couldn't read the script file" error.
    Io { error: String, path: String },
}

impl RunnerError {
    #[must_use]
    pub fn user_message(&self) -> String {
        match self {
            Self::NotFound { tried, hint } => {
                format!("script_run: script not found: {tried}. {hint}")
            }
            Self::OutsideWorkspace {
                path,
                workspace,
                hint,
            } => {
                let ws = workspace.as_deref().unwrap_or("<no workspace>");
                format!(
                    "script_run: script `{path}` is outside the agent workspace `{ws}`. {hint}"
                )
            }
            Self::UnknownExtension { ext, hint } => {
                format!(
                    "script_run: cannot auto-detect a runner for extension `{ext}`. {hint}"
                )
            }
            Self::UnknownLanguageHint { hint_value, hint } => {
                format!(
                    "script_run: unknown `language` hint `{hint_value}`. {hint}"
                )
            }
            Self::Io { error, path } => {
                format!("script_run: I/O error reading `{path}`: {error}")
            }
        }
    }
}

/// Pluggable predicate: "does this absolute path exist as a regular file?". Lets tests
/// drive resolution without touching the real filesystem.
pub trait PathProbe {
    fn is_file(&self, path: &Path) -> bool;
    fn read_first_line(&self, path: &Path) -> Option<String>;
}

/// Real filesystem implementation backed by the canonical-path cache.
pub struct FsProbe;

impl PathProbe for FsProbe {
    fn is_file(&self, path: &Path) -> bool {
        std::fs::metadata(path)
            .map(|m| m.is_file())
            .unwrap_or(false)
    }

    fn read_first_line(&self, path: &Path) -> Option<String> {
        // Bound the read so a giant binary doesn't accidentally OOM us.
        use std::io::{BufRead, BufReader, Read};
        let f = std::fs::File::open(path).ok()?;
        let mut reader = BufReader::new(f.take(4096));
        let mut buf = String::new();
        reader.read_line(&mut buf).ok()?;
        Some(buf)
    }
}

/// Resolve a script path + optional language hint into an executable `ResolvedRunner`.
///
/// `script` may be relative (resolved against `workspace_root`) or absolute.
/// `language_hint` is optional and overrides extension detection when set.
/// `extra_allowed_prefixes` are additional absolute path prefixes that the script
/// may live under besides `workspace_root` (e.g. ainl-library, project sandbox).
pub fn resolve_runner(
    script: &str,
    workspace_root: Option<&Path>,
    language_hint: Option<&str>,
    extra_allowed_prefixes: &[PathBuf],
    probe: &dyn PathProbe,
) -> Result<ResolvedRunner, RunnerError> {
    if script.trim().is_empty() {
        return Err(RunnerError::NotFound {
            tried: String::new(),
            hint: "Provide a `script` path (relative to the workspace, or absolute).".to_string(),
        });
    }

    // 1. Resolve to an absolute path.
    let raw = PathBuf::from(script);
    let absolute: PathBuf = if raw.is_absolute() {
        raw
    } else if let Some(ws) = workspace_root {
        ws.join(&raw)
    } else {
        // No workspace and not absolute — treat as cwd-relative (best effort).
        std::env::current_dir()
            .map(|c| c.join(&raw))
            .unwrap_or(raw)
    };

    if !probe.is_file(&absolute) {
        let hint = workspace_root
            .map(|w| {
                format!(
                    "Use a path relative to the agent workspace (`{}`), or pass an absolute path. \
                     Did you create the file yet? Try `file_list` to confirm.",
                    w.display()
                )
            })
            .unwrap_or_else(|| {
                "Confirm the file exists; use `file_list` to enumerate the workspace.".to_string()
            });
        return Err(RunnerError::NotFound {
            tried: absolute.to_string_lossy().to_string(),
            hint,
        });
    }

    // 2. Workspace containment check (defense-in-depth — `shell_exec` is enforced by
    //    `shell_argv_guard`; here we apply the same intent for the script path itself).
    if let Some(ws) = workspace_root {
        let allowed_here = path_starts_with_any(&absolute, ws, extra_allowed_prefixes);
        if !allowed_here {
            return Err(RunnerError::OutsideWorkspace {
                path: absolute.to_string_lossy().to_string(),
                workspace: Some(ws.to_string_lossy().to_string()),
                hint: "Move the script under the workspace, or add its directory to \
                       `security.exec_policy.extra_allowed_path_prefixes` in config.toml."
                    .to_string(),
            });
        }
    }

    // 3. Pick a language: explicit hint first, then extension, then shebang.
    let (language, decision_source) = if let Some(hint) = language_hint {
        let lang = parse_language_hint(hint).ok_or_else(|| RunnerError::UnknownLanguageHint {
            hint_value: hint.to_string(),
            hint: "Supported: python, shell, bash, zsh, node, typescript, bun, deno.".to_string(),
        })?;
        (lang, "language_hint")
    } else if let Some(ext) = absolute.extension().and_then(|e| e.to_str()) {
        if let Some(lang) = language_from_extension(ext, &absolute, workspace_root, probe) {
            (lang, "extension")
        } else {
            // Extension didn't match; try shebang as a last resort.
            if let Some(lang) = language_from_shebang(&absolute, probe) {
                (lang, "shebang")
            } else {
                return Err(RunnerError::UnknownExtension {
                    ext: ext.to_string(),
                    hint: "Pass `language: 'python' | 'shell' | 'node' | 'typescript' | 'bun' | 'deno'`, \
                           or rename the file with the matching extension."
                        .to_string(),
                });
            }
        }
    } else if let Some(lang) = language_from_shebang(&absolute, probe) {
        (lang, "shebang")
    } else {
        return Err(RunnerError::UnknownExtension {
            ext: "(none)".to_string(),
            hint: "Add a recognized extension (e.g. `.py`, `.sh`, `.ts`) or pass `language` \
                   in the tool args."
                .to_string(),
        });
    };

    // 4. Pick the actual interpreter for the chosen language.
    let (interpreter, interpreter_args) =
        pick_interpreter(&language, &absolute, workspace_root, probe);

    let default_cwd = absolute
        .parent()
        .map(Path::to_path_buf)
        .or_else(|| workspace_root.map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));

    Ok(ResolvedRunner {
        interpreter,
        interpreter_args,
        script_path: absolute,
        language,
        default_cwd,
        decision_source,
    })
}

fn path_starts_with_any(path: &Path, ws: &Path, extras: &[PathBuf]) -> bool {
    if path.starts_with(ws) {
        return true;
    }
    for prefix in extras {
        if !prefix.as_os_str().is_empty() && path.starts_with(prefix) {
            return true;
        }
    }
    false
}

fn language_from_extension(
    ext: &str,
    absolute: &Path,
    workspace_root: Option<&Path>,
    probe: &dyn PathProbe,
) -> Option<ScriptLanguage> {
    match ext.to_ascii_lowercase().as_str() {
        "py" => Some(ScriptLanguage::Python),
        "sh" => Some(ScriptLanguage::Shell),
        "bash" => Some(ScriptLanguage::Bash),
        "zsh" => Some(ScriptLanguage::Zsh),
        "js" | "mjs" | "cjs" => Some(detect_js_runtime(absolute, workspace_root, probe)),
        "ts" | "tsx" | "mts" | "cts" => {
            Some(detect_ts_runtime(absolute, workspace_root, probe))
        }
        _ => None,
    }
}

fn detect_js_runtime(
    _absolute: &Path,
    workspace_root: Option<&Path>,
    probe: &dyn PathProbe,
) -> ScriptLanguage {
    if let Some(ws) = workspace_root {
        if probe.is_file(&ws.join("bun.lockb")) || probe.is_file(&ws.join("bun.lock")) {
            return ScriptLanguage::Bun;
        }
        if probe.is_file(&ws.join("deno.json")) || probe.is_file(&ws.join("deno.jsonc")) {
            return ScriptLanguage::Deno;
        }
    }
    ScriptLanguage::Node
}

fn detect_ts_runtime(
    _absolute: &Path,
    workspace_root: Option<&Path>,
    probe: &dyn PathProbe,
) -> ScriptLanguage {
    if let Some(ws) = workspace_root {
        if probe.is_file(&ws.join("bun.lockb")) || probe.is_file(&ws.join("bun.lock")) {
            return ScriptLanguage::Bun;
        }
        if probe.is_file(&ws.join("deno.json")) || probe.is_file(&ws.join("deno.jsonc")) {
            return ScriptLanguage::Deno;
        }
    }
    ScriptLanguage::TypeScript
}

fn language_from_shebang(absolute: &Path, probe: &dyn PathProbe) -> Option<ScriptLanguage> {
    let line = probe.read_first_line(absolute)?;
    let trimmed = line.trim_start();
    if !trimmed.starts_with("#!") {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("python") {
        Some(ScriptLanguage::Python)
    } else if lower.contains("/bash") || lower.contains(" bash") {
        Some(ScriptLanguage::Bash)
    } else if lower.contains("/zsh") || lower.contains(" zsh") {
        Some(ScriptLanguage::Zsh)
    } else if lower.contains("/sh") || lower.ends_with("/sh") || lower.contains("env sh") {
        Some(ScriptLanguage::Shell)
    } else if lower.contains("node") {
        Some(ScriptLanguage::Node)
    } else if lower.contains("bun") {
        Some(ScriptLanguage::Bun)
    } else if lower.contains("deno") {
        Some(ScriptLanguage::Deno)
    } else {
        None
    }
}

fn parse_language_hint(hint: &str) -> Option<ScriptLanguage> {
    match hint.trim().to_ascii_lowercase().as_str() {
        "python" | "py" | "python3" => Some(ScriptLanguage::Python),
        "shell" | "sh" => Some(ScriptLanguage::Shell),
        "bash" => Some(ScriptLanguage::Bash),
        "zsh" => Some(ScriptLanguage::Zsh),
        "node" | "nodejs" | "javascript" | "js" => Some(ScriptLanguage::Node),
        "typescript" | "ts" => Some(ScriptLanguage::TypeScript),
        "bun" => Some(ScriptLanguage::Bun),
        "deno" => Some(ScriptLanguage::Deno),
        _ => None,
    }
}

/// Pick the actual interpreter binary + leading args for a given language. Prefers
/// project-local binaries (`venv/bin/python3`, `node_modules/.bin/tsx`) when present
/// so the agent doesn't have to know how to "activate" them.
fn pick_interpreter(
    lang: &ScriptLanguage,
    _script: &Path,
    workspace_root: Option<&Path>,
    probe: &dyn PathProbe,
) -> (String, Vec<String>) {
    match lang {
        ScriptLanguage::Python => pick_python(workspace_root, probe),
        ScriptLanguage::Shell => (pick_first_existing(&["/bin/sh", "/usr/bin/sh"], probe, "sh"), vec![]),
        ScriptLanguage::Bash => (
            pick_first_existing(
                &[
                    "/bin/bash",
                    "/usr/bin/bash",
                    "/usr/local/bin/bash",
                    "/opt/homebrew/bin/bash",
                ],
                probe,
                "bash",
            ),
            vec![],
        ),
        ScriptLanguage::Zsh => (
            pick_first_existing(
                &["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh"],
                probe,
                "zsh",
            ),
            vec![],
        ),
        ScriptLanguage::Node => (pick_node(workspace_root, probe), vec![]),
        ScriptLanguage::TypeScript => pick_typescript(workspace_root, probe),
        ScriptLanguage::Bun => (
            pick_first_existing(
                &[
                    "/usr/local/bin/bun",
                    "/opt/homebrew/bin/bun",
                    "/usr/bin/bun",
                ],
                probe,
                "bun",
            ),
            vec!["run".to_string()],
        ),
        ScriptLanguage::Deno => (
            pick_first_existing(
                &[
                    "/usr/local/bin/deno",
                    "/opt/homebrew/bin/deno",
                    "/usr/bin/deno",
                ],
                probe,
                "deno",
            ),
            vec!["run".to_string(), "-A".to_string()],
        ),
        ScriptLanguage::Other(name) => (name.clone(), vec![]),
    }
}

fn pick_python(workspace_root: Option<&Path>, probe: &dyn PathProbe) -> (String, Vec<String>) {
    if let Some(ws) = workspace_root {
        for rel in [
            ".venv/bin/python3",
            ".venv/bin/python",
            "venv/bin/python3",
            "venv/bin/python",
            ".venv/Scripts/python.exe",
            "venv/Scripts/python.exe",
        ] {
            let candidate = ws.join(rel);
            if probe.is_file(&candidate) {
                return (candidate.to_string_lossy().to_string(), vec![]);
            }
        }
    }
    (
        pick_first_existing(
            &[
                "/usr/local/bin/python3",
                "/opt/homebrew/bin/python3",
                "/usr/bin/python3",
            ],
            probe,
            "python3",
        ),
        vec![],
    )
}

fn pick_node(workspace_root: Option<&Path>, probe: &dyn PathProbe) -> String {
    if let Some(ws) = workspace_root {
        for rel in ["node_modules/.bin/node", ".volta/bin/node"] {
            let candidate = ws.join(rel);
            if probe.is_file(&candidate) {
                return candidate.to_string_lossy().to_string();
            }
        }
    }
    pick_first_existing(
        &[
            "/usr/local/bin/node",
            "/opt/homebrew/bin/node",
            "/usr/bin/node",
        ],
        probe,
        "node",
    )
}

fn pick_typescript(
    workspace_root: Option<&Path>,
    probe: &dyn PathProbe,
) -> (String, Vec<String>) {
    if let Some(ws) = workspace_root {
        // Project-pinned tsx is the most reliable — no network, no "npm install -g".
        for rel in ["node_modules/.bin/tsx", "node_modules/.bin/ts-node"] {
            let candidate = ws.join(rel);
            if probe.is_file(&candidate) {
                return (candidate.to_string_lossy().to_string(), vec![]);
            }
        }
    }
    // Fall back to `npx --yes tsx` (zero-config, but may fetch on first run).
    (
        pick_first_existing(
            &["/usr/local/bin/npx", "/opt/homebrew/bin/npx", "/usr/bin/npx"],
            probe,
            "npx",
        ),
        vec!["--yes".to_string(), "tsx".to_string()],
    )
}

fn pick_first_existing(candidates: &[&str], probe: &dyn PathProbe, fallback: &str) -> String {
    for c in candidates {
        if probe.is_file(Path::new(c)) {
            return (*c).to_string();
        }
    }
    fallback.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Test probe: every path you put in `existing_files` is a file, anything else isn't.
    /// `shebangs` overrides `read_first_line` for specific paths.
    struct MockProbe {
        existing_files: HashSet<PathBuf>,
        shebangs: std::collections::HashMap<PathBuf, String>,
    }

    impl MockProbe {
        fn new(files: &[&str]) -> Self {
            Self {
                existing_files: files.iter().map(PathBuf::from).collect(),
                shebangs: std::collections::HashMap::new(),
            }
        }
        fn with_shebang(mut self, path: &str, line: &str) -> Self {
            self.shebangs
                .insert(PathBuf::from(path), line.to_string());
            self.existing_files.insert(PathBuf::from(path));
            self
        }
    }

    impl PathProbe for MockProbe {
        fn is_file(&self, path: &Path) -> bool {
            self.existing_files.contains(path)
        }
        fn read_first_line(&self, path: &Path) -> Option<String> {
            self.shebangs.get(path).cloned()
        }
    }

    #[test]
    fn resolves_python_with_workspace_venv() {
        let probe = MockProbe::new(&[
            "/ws/start.py",
            "/ws/.venv/bin/python3",
        ]);
        let r = resolve_runner("start.py", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::Python);
        assert_eq!(r.interpreter, "/ws/.venv/bin/python3");
        assert!(r.interpreter_args.is_empty());
        assert_eq!(r.script_path, PathBuf::from("/ws/start.py"));
        assert_eq!(r.decision_source, "extension");
        let argv = r.full_argv(&["--port".to_string(), "8080".to_string()]);
        assert_eq!(
            argv,
            vec!["/ws/start.py", "--port", "8080"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn resolves_python_falls_back_to_system() {
        let probe = MockProbe::new(&["/ws/start.py", "/usr/bin/python3"]);
        let r = resolve_runner("start.py", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.interpreter, "/usr/bin/python3");
    }

    #[test]
    fn resolves_typescript_prefers_local_tsx() {
        let probe = MockProbe::new(&[
            "/ws/server.ts",
            "/ws/node_modules/.bin/tsx",
        ]);
        let r = resolve_runner("server.ts", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::TypeScript);
        assert_eq!(r.interpreter, "/ws/node_modules/.bin/tsx");
        assert!(r.interpreter_args.is_empty());
    }

    #[test]
    fn resolves_typescript_falls_back_to_npx_tsx() {
        let probe = MockProbe::new(&["/ws/server.ts", "/usr/local/bin/npx"]);
        let r = resolve_runner("server.ts", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.interpreter, "/usr/local/bin/npx");
        assert_eq!(
            r.interpreter_args,
            vec!["--yes".to_string(), "tsx".to_string()]
        );
    }

    #[test]
    fn detects_bun_project_for_typescript() {
        let probe = MockProbe::new(&["/ws/server.ts", "/ws/bun.lock", "/opt/homebrew/bin/bun"]);
        let r = resolve_runner("server.ts", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::Bun);
        assert_eq!(r.interpreter, "/opt/homebrew/bin/bun");
        assert_eq!(r.interpreter_args, vec!["run".to_string()]);
    }

    #[test]
    fn detects_deno_project_for_typescript() {
        let probe = MockProbe::new(&["/ws/server.ts", "/ws/deno.json", "/usr/local/bin/deno"]);
        let r = resolve_runner("server.ts", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::Deno);
        assert_eq!(
            r.interpreter_args,
            vec!["run".to_string(), "-A".to_string()]
        );
    }

    #[test]
    fn resolves_shell_script() {
        let probe = MockProbe::new(&["/ws/run.sh", "/bin/bash"]);
        let r = resolve_runner("run.sh", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::Shell);
        // .sh defaults to /bin/sh, not bash, even if bash is present.
        assert!(
            r.interpreter == "sh"
                || r.interpreter == "/bin/sh"
                || r.interpreter == "/usr/bin/sh"
        );
    }

    #[test]
    fn shebang_overrides_unknown_extension() {
        let probe = MockProbe::new(&["/ws/runner"])
            .with_shebang("/ws/runner", "#!/usr/bin/env python3\n");
        let r = resolve_runner("runner", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        assert_eq!(r.language, ScriptLanguage::Python);
        assert_eq!(r.decision_source, "shebang");
    }

    #[test]
    fn language_hint_overrides_extension() {
        let probe = MockProbe::new(&["/ws/server.js"]);
        let r = resolve_runner(
            "server.js",
            Some(Path::new("/ws")),
            Some("typescript"),
            &[],
            &probe,
        )
        .unwrap();
        assert_eq!(r.language, ScriptLanguage::TypeScript);
        assert_eq!(r.decision_source, "language_hint");
    }

    #[test]
    fn errors_when_script_missing() {
        let probe = MockProbe::new(&[]);
        let err =
            resolve_runner("missing.py", Some(Path::new("/ws")), None, &[], &probe).unwrap_err();
        match err {
            RunnerError::NotFound { tried, hint } => {
                assert_eq!(tried, "/ws/missing.py");
                assert!(hint.contains("workspace"));
            }
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[test]
    fn errors_when_outside_workspace() {
        let probe = MockProbe::new(&["/elsewhere/start.py"]);
        let err = resolve_runner(
            "/elsewhere/start.py",
            Some(Path::new("/ws")),
            None,
            &[],
            &probe,
        )
        .unwrap_err();
        match err {
            RunnerError::OutsideWorkspace { path, .. } => {
                assert_eq!(path, "/elsewhere/start.py");
            }
            other => panic!("expected OutsideWorkspace, got {other:?}"),
        }
    }

    #[test]
    fn extra_allowed_prefix_admits_outside_path() {
        let probe = MockProbe::new(&["/lib/share/run.py", "/usr/bin/python3"]);
        let r = resolve_runner(
            "/lib/share/run.py",
            Some(Path::new("/ws")),
            None,
            &[PathBuf::from("/lib")],
            &probe,
        )
        .unwrap();
        assert_eq!(r.script_path, PathBuf::from("/lib/share/run.py"));
    }

    #[test]
    fn errors_on_unknown_extension_without_shebang() {
        let probe = MockProbe::new(&["/ws/script.weird"]);
        let err = resolve_runner(
            "script.weird",
            Some(Path::new("/ws")),
            None,
            &[],
            &probe,
        )
        .unwrap_err();
        match err {
            RunnerError::UnknownExtension { ext, .. } => assert_eq!(ext, "weird"),
            other => panic!("expected UnknownExtension, got {other:?}"),
        }
    }

    #[test]
    fn errors_on_unknown_language_hint() {
        let probe = MockProbe::new(&["/ws/server.ts"]);
        let err = resolve_runner(
            "server.ts",
            Some(Path::new("/ws")),
            Some("brainfuck"),
            &[],
            &probe,
        )
        .unwrap_err();
        assert!(matches!(err, RunnerError::UnknownLanguageHint { .. }));
    }

    #[test]
    fn full_argv_appends_caller_args_after_script() {
        let probe = MockProbe::new(&["/ws/run.sh"]);
        let r = resolve_runner("run.sh", Some(Path::new("/ws")), None, &[], &probe).unwrap();
        let argv = r.full_argv(&["arg1".to_string(), "arg2".to_string()]);
        assert_eq!(argv.last(), Some(&"arg2".to_string()));
        assert_eq!(argv[argv.len() - 2], "arg1");
        // The script must come before any caller args.
        let script_pos = argv
            .iter()
            .position(|s| s.ends_with("run.sh"))
            .unwrap();
        let arg1_pos = argv.iter().position(|s| s == "arg1").unwrap();
        assert!(script_pos < arg1_pos);
    }
}
