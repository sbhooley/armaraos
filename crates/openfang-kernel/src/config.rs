//! Configuration loading from `~/.armaraos/config.toml` (or legacy `~/.openfang/config.toml`) with defaults.
//!
//! Supports config includes: the `include` field specifies additional TOML files
//! to load and deep-merge before the root config (root overrides includes).

use openfang_types::config::{
    DefaultModelConfig, FallbackProviderConfig, KernelConfig, DEFAULT_OPENROUTER_MODEL_ID,
    OPENROUTER_FREE_FALLBACK_MODELS,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tracing::info;

/// Re-export for CLI and diagnostics (increment in `openfang-types` when migrations change).
pub use openfang_types::config::CONFIG_SCHEMA_VERSION;

/// Fix invalid TOML from older AINL MCP merges (orphan lines after `env = [ ... ]`).
pub fn repair_config_toml_stale_mcp_env(raw: &str) -> String {
    crate::config_toml_repair::repair_stale_mcp_env_continuations(raw)
}

/// Read `config.toml`, repair known stale MCP `env` fragments (same as boot [`load_config`]),
/// persist the repair if the file changed, then parse as [`toml::Value`].
///
/// Use this before **partial** rewrites (provider URLs, channels, etc.) so corrupted on-disk
/// TOML from older AINL bootstrap merges does not block saves or yield opaque parse errors.
pub fn parse_config_toml_file(path: &Path) -> Result<toml::Value, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    let raw = std::fs::read_to_string(path)?;
    let repaired = crate::config_toml_repair::repair_stale_mcp_env_continuations(&raw);
    let to_parse = if repaired != raw {
        match atomic_write(path, &repaired) {
            Ok(()) => {
                info!(
                    path = %path.display(),
                    "Repaired stale MCP env array fragments before partial config.toml rewrite"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %path.display(),
                    "Could not persist repaired config.toml; parsing in-memory repair only"
                );
            }
        }
        repaired
    } else {
        raw
    };
    if to_parse.trim().is_empty() {
        return Ok(toml::Value::Table(toml::map::Map::new()));
    }
    Ok(toml::from_str(&to_parse)?)
}

/// Maximum include nesting depth.
const MAX_INCLUDE_DEPTH: u32 = 10;

/// Load kernel configuration from a TOML file, with defaults.
///
/// If the config contains an `include` field, included files are loaded
/// and deep-merged first, then the root config overrides them.
pub fn load_config(path: Option<&Path>) -> KernelConfig {
    let config_path = path
        .map(|p| p.to_path_buf())
        .unwrap_or_else(default_config_path);

    if config_path.exists() {
        match std::fs::read_to_string(&config_path) {
            Ok(contents) => {
                let repaired =
                    crate::config_toml_repair::repair_stale_mcp_env_continuations(&contents);
                let to_parse = if repaired != contents {
                    match atomic_write(&config_path, &repaired) {
                        Ok(()) => {
                            info!(
                                path = %config_path.display(),
                                "Repaired stale MCP env array fragments in config.toml (invalid TOML from older AINL bootstrap)"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                path = %config_path.display(),
                                "Could not persist repaired config.toml; continuing with in-memory repair"
                            );
                        }
                    }
                    repaired
                } else {
                    contents
                };
                match toml::from_str::<toml::Value>(&to_parse) {
                    Ok(mut root_value) => {
                        // Process includes before deserializing
                        let config_dir = config_path
                            .parent()
                            .unwrap_or_else(|| Path::new("."))
                            .to_path_buf();
                        let mut visited = HashSet::new();
                        if let Ok(canonical) = std::fs::canonicalize(&config_path) {
                            visited.insert(canonical);
                        } else {
                            visited.insert(config_path.clone());
                        }

                        if let Err(e) =
                            resolve_config_includes(&mut root_value, &config_dir, &mut visited, 0)
                        {
                            tracing::warn!(
                                error = %e,
                                "Config include resolution failed, using root config only"
                            );
                        }

                        // Remove the `include` field before deserializing to avoid confusion
                        if let toml::Value::Table(ref mut tbl) = root_value {
                            tbl.remove("include");
                        }

                        // Migrate misplaced api_key/api_listen from [api] section to root level.
                        // The old config schema incorrectly grouped these under [api], so many
                        // users have them in the wrong place. Move them up if not already at root.
                        if let toml::Value::Table(ref mut tbl) = root_value {
                            if let Some(toml::Value::Table(api_section)) = tbl.get("api").cloned() {
                                for key in &["api_key", "api_listen", "log_level"] {
                                    if !tbl.contains_key(*key) {
                                        if let Some(val) = api_section.get(*key) {
                                            tracing::info!(
                                            key,
                                            "Migrating misplaced config field from [api] to root level"
                                        );
                                            tbl.insert(key.to_string(), val.clone());
                                        }
                                    }
                                }
                            }
                        }

                        match root_value.try_into::<KernelConfig>() {
                            Ok(mut config) => {
                                apply_config_schema_migrations(&mut config, &config_path);
                                config.apply_security_shell_guard_overrides();
                                info!(path = %config_path.display(), "Loaded configuration");
                                return config;
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    path = %config_path.display(),
                                    "Failed to deserialize merged config, using defaults"
                                );
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            path = %config_path.display(),
                            "Failed to parse config, using defaults"
                        );
                    }
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %config_path.display(),
                    "Failed to read config file, using defaults"
                );
            }
        }
    } else {
        info!(
            path = %config_path.display(),
            "Config file not found, using defaults"
        );
    }

    KernelConfig::default()
}

/// Run config migrations from `config.config_schema_version` up to [`CONFIG_SCHEMA_VERSION`],
/// then best-effort persist the new schema version to the root `config.toml`.
fn apply_config_schema_migrations(config: &mut KernelConfig, config_path: &Path) {
    let start = config.config_schema_version;
    if start > CONFIG_SCHEMA_VERSION {
        tracing::warn!(
            file = %config_path.display(),
            on_disk = start,
            binary = CONFIG_SCHEMA_VERSION,
            "config.toml schema version is newer than this binary; newer keys may be ignored"
        );
        return;
    }

    for step in start..CONFIG_SCHEMA_VERSION {
        if step == 0 {
            migrate_legacy_openrouter_default_model(config);
        }
        // Future: `else if step == 1 { migrate_v1_to_v2(config); }`
    }
    config.config_schema_version = CONFIG_SCHEMA_VERSION;

    if config_path.exists() && start < CONFIG_SCHEMA_VERSION {
        match persist_config_schema_version_line(config_path, CONFIG_SCHEMA_VERSION) {
            Ok(()) => {
                info!(
                    path = %config_path.display(),
                    from = start,
                    to = CONFIG_SCHEMA_VERSION,
                    "Updated config.toml schema version"
                );
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    path = %config_path.display(),
                    "Could not persist config_schema_version to config.toml (in-memory migrations still applied)"
                );
            }
        }
    }
}

/// Set or update root-level `config_schema_version` in `config.toml`.
///
/// Parses the full file as TOML, updates the key, and writes it back. This avoids corrupting
/// multiline strings or other constructs that a line-based rewriter could break.
pub fn persist_config_schema_version_line(path: &Path, version: u32) -> std::io::Result<()> {
    let raw = std::fs::read_to_string(path)?;
    let repaired = crate::config_toml_repair::repair_stale_mcp_env_continuations(&raw);
    let to_parse = if repaired != raw {
        if let Err(e) = atomic_write(path, &repaired) {
            tracing::warn!(
                error = %e,
                path = %path.display(),
                "Could not persist repaired config.toml before schema version bump; using in-memory repair"
            );
        } else {
            info!(
                path = %path.display(),
                "Repaired stale MCP env array fragments before config_schema_version update"
            );
        }
        repaired
    } else {
        raw
    };
    let mut root: toml::Value = toml::from_str(&to_parse).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("config.toml is not valid TOML: {e}"),
        )
    })?;
    let Some(tbl) = root.as_table_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "config root must be a TOML table",
        ));
    };
    tbl.insert(
        "config_schema_version".to_string(),
        toml::Value::Integer(i64::from(version)),
    );
    let out = toml::to_string_pretty(&root).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("config serialize failed: {e}"),
        )
    })?;
    atomic_write(path, &out)
}

fn atomic_write(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, contents)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_first) => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path).inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
        }
    }
}

/// If `[default_model]` still has the old OpenRouter placeholder (first catalog row /
/// auto-detect default), align it with [`DEFAULT_OPENROUTER_MODEL_ID`] in memory.
fn migrate_legacy_openrouter_default_model(config: &mut KernelConfig) {
    if config.default_model.provider != "openrouter" {
        return;
    }
    let m = config.default_model.model.trim();
    const LEGACY: &[&str] = &[
        "openrouter/google/gemini-2.5-flash",
        "google/gemini-2.5-flash",
        "elephant-alpha",
        "openrouter/elephant-alpha",
    ];
    if LEGACY.contains(&m) {
        info!(
            old = %m,
            new = %DEFAULT_OPENROUTER_MODEL_ID,
            "Migrating legacy OpenRouter default model to current bundled default"
        );
        config.default_model.model = DEFAULT_OPENROUTER_MODEL_ID.to_string();
    }
}

/// Resolve config includes by deep-merging included files into the root value.
///
/// Included files are loaded first and the root config overrides them.
/// Security: rejects absolute paths, `..` components, and circular references.
fn resolve_config_includes(
    root_value: &mut toml::Value,
    config_dir: &Path,
    visited: &mut HashSet<PathBuf>,
    depth: u32,
) -> Result<(), String> {
    if depth > MAX_INCLUDE_DEPTH {
        return Err(format!(
            "Config include depth exceeded maximum of {MAX_INCLUDE_DEPTH}"
        ));
    }

    // Extract include list from the current value
    let includes = match root_value {
        toml::Value::Table(tbl) => {
            if let Some(toml::Value::Array(arr)) = tbl.get("include") {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect::<Vec<_>>()
            } else {
                return Ok(());
            }
        }
        _ => return Ok(()),
    };

    if includes.is_empty() {
        return Ok(());
    }

    // Merge each include (earlier includes are overridden by later ones,
    // and the root config overrides everything).
    let mut merged_base = toml::Value::Table(toml::map::Map::new());

    for include_path_str in &includes {
        // SECURITY: reject absolute paths
        let include_path = Path::new(include_path_str);
        if include_path.is_absolute() {
            return Err(format!(
                "Config include rejects absolute path: {include_path_str}"
            ));
        }
        // SECURITY: reject `..` components
        for component in include_path.components() {
            if let std::path::Component::ParentDir = component {
                return Err(format!(
                    "Config include rejects path traversal: {include_path_str}"
                ));
            }
        }

        let resolved = config_dir.join(include_path);
        // SECURITY: verify resolved path stays within config dir
        let canonical = std::fs::canonicalize(&resolved).map_err(|e| {
            format!(
                "Config include '{}' cannot be resolved: {e}",
                include_path_str
            )
        })?;
        let canonical_dir = std::fs::canonicalize(config_dir)
            .map_err(|e| format!("Config dir cannot be canonicalized: {e}"))?;
        if !canonical.starts_with(&canonical_dir) {
            return Err(format!(
                "Config include '{}' escapes config directory",
                include_path_str
            ));
        }

        // SECURITY: circular detection
        if !visited.insert(canonical.clone()) {
            return Err(format!(
                "Circular config include detected: {include_path_str}"
            ));
        }

        info!(include = %include_path_str, "Loading config include");

        let contents = std::fs::read_to_string(&canonical)
            .map_err(|e| format!("Failed to read config include '{}': {e}", include_path_str))?;
        let mut include_value: toml::Value = toml::from_str(&contents)
            .map_err(|e| format!("Failed to parse config include '{}': {e}", include_path_str))?;

        // Recursively resolve includes in the included file
        let include_dir = canonical.parent().unwrap_or(config_dir).to_path_buf();
        resolve_config_includes(&mut include_value, &include_dir, visited, depth + 1)?;

        // Remove include field from the included file
        if let toml::Value::Table(ref mut tbl) = include_value {
            tbl.remove("include");
        }

        // Deep merge: include overrides the base built so far
        deep_merge_toml(&mut merged_base, &include_value);
    }

    // Now deep merge: root overrides the merged includes
    // Save root's current values (minus include), then merge root on top
    let root_without_include = {
        let mut v = root_value.clone();
        if let toml::Value::Table(ref mut tbl) = v {
            tbl.remove("include");
        }
        v
    };
    deep_merge_toml(&mut merged_base, &root_without_include);
    *root_value = merged_base;

    Ok(())
}

/// Deep-merge two TOML values. `overlay` values override `base` values.
/// For tables, recursively merge. For everything else, overlay wins.
pub fn deep_merge_toml(base: &mut toml::Value, overlay: &toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base_tbl), toml::Value::Table(overlay_tbl)) => {
            for (key, overlay_val) in overlay_tbl {
                if let Some(base_val) = base_tbl.get_mut(key) {
                    deep_merge_toml(base_val, overlay_val);
                } else {
                    base_tbl.insert(key.clone(), overlay_val.clone());
                }
            }
        }
        (base, overlay) => {
            *base = overlay.clone();
        }
    }
}

/// Get the default config file path.
///
/// Respects `ARMARAOS_HOME` (preferred) and `OPENFANG_HOME` (legacy) env vars.
pub fn default_config_path() -> PathBuf {
    openfang_home().join("config.toml")
}

/// Get the ArmaraOS home directory (legacy name preserved for compatibility).
///
/// Delegates to [`openfang_types::config::openfang_home_dir`] (creates `~/.armaraos` or migrates
/// from `~/.openfang` when appropriate).
pub fn openfang_home() -> PathBuf {
    openfang_types::config::openfang_home_dir()
}

/// Primary OpenRouter model id for fresh ArmaraOS **desktop** installs (no `config.toml` yet).
pub const DESKTOP_DEFAULT_OPENROUTER_MODEL: &str = DEFAULT_OPENROUTER_MODEL_ID;

/// Apply bundled OpenRouter defaults for the ArmaraOS desktop app when the user has no
/// `config.toml` yet. The desktop shell calls this before [`crate::OpenFangKernel::boot_with_config`].
///
/// Sets `[default_model]` to OpenRouter + [`DESKTOP_DEFAULT_OPENROUTER_MODEL`] and adds one
/// `[[fallback_providers]]` entry from [`OPENROUTER_FREE_FALLBACK_MODELS`] (skips duplicate of primary).
pub fn apply_desktop_bundled_llm_defaults(config: &mut KernelConfig) {
    config.default_model = DefaultModelConfig {
        provider: "openrouter".to_string(),
        model: DEFAULT_OPENROUTER_MODEL_ID.to_string(),
        api_key_env: "OPENROUTER_API_KEY".to_string(),
        base_url: None,
    };
    let fb_model = OPENROUTER_FREE_FALLBACK_MODELS
        .iter()
        .copied()
        .find(|m| *m != DEFAULT_OPENROUTER_MODEL_ID)
        .unwrap_or_else(|| {
            OPENROUTER_FREE_FALLBACK_MODELS
                .first()
                .copied()
                .unwrap_or("meta-llama/llama-3.1-8b-instruct:free")
        });
    config.fallback_providers = vec![FallbackProviderConfig {
        provider: "openrouter".to_string(),
        model: fb_model.to_string(),
        api_key_env: String::new(),
        base_url: None,
    }];
}

#[cfg(test)]
mod tests {
    use super::*;
    use openfang_types::config::{ShellPathGuardMode, ShellPidGuardMode};
    use std::io::Write;

    #[test]
    fn test_security_table_overrides_shell_path_guard() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"
[exec_policy]
shell_path_guard = "off"
shell_pid_guard = "off"

[security]
shell_path_guard = "enforce"
"#
        )
        .unwrap();
        let cfg = load_config(Some(&path));
        assert_eq!(
            cfg.exec_policy.shell_path_guard,
            ShellPathGuardMode::Enforce
        );
        assert_eq!(cfg.exec_policy.shell_pid_guard, ShellPidGuardMode::Off);
    }

    #[test]
    fn test_load_config_defaults() {
        // Use an explicit nonexistent path so the user's real ~/.armaraos/config.toml
        // (which may have log_level = "debug" set) doesn't influence the test result.
        let config = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_load_config_missing_file() {
        let config = load_config(Some(Path::new("/nonexistent/config.toml")));
        assert_eq!(config.log_level, "info");
    }

    #[test]
    fn test_deep_merge_simple() {
        let mut base: toml::Value = toml::from_str(
            r#"
            log_level = "debug"
            api_listen = "0.0.0.0:4200"
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            log_level = "info"
            network_enabled = true
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        assert_eq!(base["log_level"].as_str(), Some("info"));
        assert_eq!(base["api_listen"].as_str(), Some("0.0.0.0:4200"));
        assert_eq!(base["network_enabled"].as_bool(), Some(true));
    }

    #[test]
    fn test_deep_merge_nested_tables() {
        let mut base: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.1
            consolidation_threshold = 10000
        "#,
        )
        .unwrap();
        let overlay: toml::Value = toml::from_str(
            r#"
            [memory]
            decay_rate = 0.5
        "#,
        )
        .unwrap();
        deep_merge_toml(&mut base, &overlay);
        let mem = base["memory"].as_table().unwrap();
        assert_eq!(mem["decay_rate"].as_float(), Some(0.5));
        assert_eq!(mem["consolidation_threshold"].as_integer(), Some(10000));
    }

    #[test]
    fn test_basic_include() {
        let dir = tempfile::tempdir().unwrap();
        let base_path = dir.path().join("base.toml");
        let root_path = dir.path().join("config.toml");

        // Base config
        let mut f = std::fs::File::create(&base_path).unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        writeln!(f, "api_listen = \"0.0.0.0:9999\"").unwrap();
        drop(f);

        // Root config (includes base, overrides log_level)
        let mut f = std::fs::File::create(&root_path).unwrap();
        writeln!(f, "include = [\"base.toml\"]").unwrap();
        writeln!(f, "log_level = \"warn\"").unwrap();
        drop(f);

        let config = load_config(Some(&root_path));
        assert_eq!(config.log_level, "warn"); // root overrides
        assert_eq!(config.api_listen, "0.0.0.0:9999"); // from base
    }

    #[test]
    fn test_nested_include() {
        let dir = tempfile::tempdir().unwrap();
        let grandchild = dir.path().join("grandchild.toml");
        let child = dir.path().join("child.toml");
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&grandchild).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&child).unwrap();
        writeln!(f, "include = [\"grandchild.toml\"]").unwrap();
        writeln!(f, "log_level = \"debug\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"child.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // root wins
    }

    #[test]
    fn test_circular_include_detected() {
        let dir = tempfile::tempdir().unwrap();
        let a_path = dir.path().join("a.toml");
        let b_path = dir.path().join("b.toml");

        let mut f = std::fs::File::create(&a_path).unwrap();
        writeln!(f, "include = [\"b.toml\"]").unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let mut f = std::fs::File::create(&b_path).unwrap();
        writeln!(f, "include = [\"a.toml\"]").unwrap();
        drop(f);

        // Should not panic — circular detection triggers, falls back gracefully
        let config = load_config(Some(&a_path));
        // Falls back to defaults due to the circular error
        assert!(!config.log_level.is_empty());
    }

    #[test]
    fn test_path_traversal_blocked() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"../etc/passwd\"]").unwrap();
        drop(f);

        // Should not panic — path traversal triggers error, falls back
        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // defaults
    }

    #[test]
    fn test_max_depth_exceeded() {
        let dir = tempfile::tempdir().unwrap();

        // Create a chain of 12 files (exceeds MAX_INCLUDE_DEPTH=10)
        for i in (0..12).rev() {
            let name = format!("level{i}.toml");
            let path = dir.path().join(&name);
            let mut f = std::fs::File::create(&path).unwrap();
            if i < 11 {
                let next = format!("level{}.toml", i + 1);
                writeln!(f, "include = [\"{next}\"]").unwrap();
            }
            writeln!(f, "log_level = \"level{i}\"").unwrap();
            drop(f);
        }

        let root = dir.path().join("level0.toml");
        let config = load_config(Some(&root));
        // Falls back due to depth limit — but should not panic
        assert!(!config.log_level.is_empty());
    }

    #[test]
    fn test_absolute_path_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "include = [\"/etc/shadow\"]").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "info"); // defaults
    }

    #[test]
    fn test_no_includes_works() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");

        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"trace\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.log_level, "trace");
    }

    #[test]
    fn test_legacy_config_gets_schema_version_persisted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&root).unwrap();
        writeln!(f, "log_level = \"info\"").unwrap();
        drop(f);

        let config = load_config(Some(&root));
        assert_eq!(config.config_schema_version, super::CONFIG_SCHEMA_VERSION);

        let disk = std::fs::read_to_string(&root).unwrap();
        assert!(disk.contains("config_schema_version"));
        assert!(disk.contains(&format!(
            "config_schema_version = {}",
            super::CONFIG_SCHEMA_VERSION
        )));
    }

    #[test]
    fn test_persist_config_schema_version_replaces_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "config_schema_version = 0\nlog_level = \"info\"\n").unwrap();
        super::persist_config_schema_version_line(&path, super::CONFIG_SCHEMA_VERSION).unwrap();
        let disk = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            disk.matches(&format!(
                "config_schema_version = {}",
                super::CONFIG_SCHEMA_VERSION
            ))
            .count(),
            1
        );
    }

    #[test]
    fn test_persist_config_schema_version_repairs_stale_mcp_env_before_parse() {
        let broken = r#"config_schema_version = 0
log_level = "info"

[[mcp_servers]]
env = ["AINL_MCP_EXPOSURE_PROFILE", "AINL_MCP_TOOLS"]
    "AINL_MCP_EXPOSURE_PROFILE",
    "AINL_MCP_TOOLS",
]
name = "ainl"
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, broken).unwrap();
        super::persist_config_schema_version_line(&path, super::CONFIG_SCHEMA_VERSION).unwrap();
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            toml::from_str::<toml::Value>(&disk).is_ok(),
            "disk should be valid TOML after repair + persist: {disk:?}"
        );
        assert!(
            disk.contains(&format!(
                "config_schema_version = {}",
                super::CONFIG_SCHEMA_VERSION
            )),
            "schema version should be updated: {disk:?}"
        );
    }

    /// Regression: a line-based rewriter must not touch `config_schema_version` text inside multiline strings.
    #[test]
    fn test_persist_config_schema_version_preserves_multiline_values() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let tricky = r#"note = """
line with config_schema_version = 0 inside
"""
log_level = "info"
"#;
        std::fs::write(&path, tricky).unwrap();
        super::persist_config_schema_version_line(&path, super::CONFIG_SCHEMA_VERSION).unwrap();
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            disk.contains("line with config_schema_version = 0 inside"),
            "substring inside multiline string must survive: {disk:?}"
        );
        assert!(
            disk.contains(&format!(
                "config_schema_version = {}",
                super::CONFIG_SCHEMA_VERSION
            )),
            "root schema key must be set: {disk:?}"
        );
        toml::from_str::<toml::Value>(&disk).expect("result must be valid TOML");
    }
}
