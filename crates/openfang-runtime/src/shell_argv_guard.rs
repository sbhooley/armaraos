//! Preflight checks for `shell_exec`: argv path containment and PID ownership.

use crate::kernel_handle::KernelHandle;
use openfang_types::config::{ShellPathGuardMode, ShellPidGuardMode};
use regex::Regex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::LazyLock;
use tracing::warn;

static RE_SHELL_KILL_PIDS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:^|[\s;&|])(?:/[\w./-]+/)?(?:kill)\s+(?:(?:-[a-zA-Z0-9]+\s+)+)?(\d+)\b")
        .expect("RE_SHELL_KILL_PIDS")
});

static RE_SHELL_PKILL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s;&|])pkill(?:\s|$)").expect("RE_SHELL_PKILL"));

static RE_SHELL_KILLALL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|[\s;&|])killall(?:\s|$)").expect("RE_SHELL_KILLALL"));

static RE_SHELL_TASKKILL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\btaskkill\b").expect("RE_SHELL_TASKKILL"));

static RE_TASKKILL_PID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)/pid\s+(\d+)\b").expect("RE_TASKKILL_PID"));

/// `--name=/abs/path` style flags (Unix absolute after `=`).
static RE_LONG_OPT_EQ_UNIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"--[\w.-]+=(/[^\s";|,)]+)"#).expect("RE_LONG_OPT_EQ_UNIX"));

/// `--name=<drive>:\` style flags on Windows.
#[cfg(windows)]
static RE_LONG_OPT_EQ_WIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"--[\w.-]+=([A-Za-z]:[/\\][^\s";|,)]+)"#).expect("RE_LONG_OPT_EQ_WIN")
});

/// Single argv token `-I/usr/include`.
static RE_SHORT_I_UNIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^-I(/.*)$").expect("RE_SHORT_I_UNIX"));

/// Raw shell string: `-I/path` (with leading space or start).
static RE_RAW_I_FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?:^|\s)-I(/[^\s";|,)]+)"#).expect("RE_RAW_I_FLAG"));

/// Raw shell string: `--opt=/path`.
static RE_RAW_LONG_EQ_UNIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"--[\w.-]+=(/[^\s";|,)]+)"#).expect("RE_RAW_LONG_EQ_UNIX"));

#[cfg(windows)]
static RE_RAW_LONG_EQ_WIN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"--[\w.-]+=([A-Za-z]:[/\\][^\s";|,)]+)"#).expect("RE_RAW_LONG_EQ_WIN")
});

/// Optional kernel + agent for audit / metrics on guard events.
pub(crate) struct ShellGuardNotifyCtx<'a> {
    pub kernel: Option<&'a Arc<dyn KernelHandle>>,
    pub agent_id: Option<&'a str>,
}

/// Workspace / library / home + [`ExecPolicy::extra_allowed_path_prefixes`] resolution
/// for argv path checks (with cached canonical roots).
pub(crate) struct PathCheckContext {
    workspace_root: Option<PathBuf>,
    library_root: Option<PathBuf>,
    workspace_canon: Option<PathBuf>,
    library_canon: Option<PathBuf>,
    home_canon: Option<PathBuf>,
    extra_roots: Vec<PathBuf>,
    extra_canon: Vec<PathBuf>,
}

impl PathCheckContext {
    pub(crate) fn new(
        workspace_root: Option<&Path>,
        ainl_library_root: Option<&Path>,
        extra_allowed_path_prefixes: &[String],
    ) -> Self {
        let workspace_root = workspace_root.map(Path::to_path_buf);
        let library_root = ainl_library_root.map(Path::to_path_buf);
        let workspace_canon = workspace_root
            .as_deref()
            .and_then(crate::path_canon_cache::cached_canonicalize);
        let library_canon = library_root
            .as_deref()
            .and_then(crate::path_canon_cache::cached_canonicalize);
        let home_canon = armaraos_home_hint()
            .as_ref()
            .and_then(|p| crate::path_canon_cache::cached_canonicalize(p));
        let mut extra_roots = Vec::new();
        let mut extra_canon = Vec::new();
        for s in extra_allowed_path_prefixes {
            if s.trim().is_empty() {
                continue;
            }
            let pb = PathBuf::from(s.trim());
            extra_roots.push(pb.clone());
            if let Some(c) = crate::path_canon_cache::cached_canonicalize(&pb) {
                extra_canon.push(c);
            }
        }
        Self {
            workspace_root,
            library_root,
            workspace_canon,
            library_canon,
            home_canon,
            extra_roots,
            extra_canon,
        }
    }

    fn extra_rule_count(&self) -> usize {
        self.extra_roots.len()
    }
}

/// Run path + PID preflights for `shell_exec` before spawn.
#[allow(clippy::too_many_arguments)]
pub(crate) fn preflight_shell_exec(
    command: &str,
    use_direct_exec: bool,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
    extra_allowed_path_prefixes: &[String],
    path_mode: ShellPathGuardMode,
    pid_mode: ShellPidGuardMode,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<(), String> {
    let ctx = ShellGuardNotifyCtx {
        kernel,
        agent_id: caller_agent_id,
    };

    let path_ctx = PathCheckContext::new(
        workspace_root,
        ainl_library_root,
        extra_allowed_path_prefixes,
    );

    if path_mode != ShellPathGuardMode::Off {
        if use_direct_exec {
            let argv = shlex::split(command).ok_or_else(|| {
                "Command contains unmatched quotes or invalid shell syntax".to_string()
            })?;
            if argv.is_empty() {
                return Err("Empty command after parsing".to_string());
            }
            check_paths_for_argv(&argv, &path_ctx, path_mode, Some(&ctx))?;
        } else {
            // Full (`sh -c`) mode: still enforce path guard when the command is splittable,
            // and apply a conservative raw-string scan when shlex fails (embedded `--x=/path`, `-I/...`).
            if let Some(argv) = shlex::split(command) {
                if !argv.is_empty() {
                    check_paths_for_argv(&argv, &path_ctx, path_mode, Some(&ctx))?;
                }
            } else {
                check_paths_in_raw_command(command, &path_ctx, path_mode, Some(&ctx))?;
            }
        }
    }

    if use_direct_exec {
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        if argv.is_empty() {
            return Err("Empty command after parsing".to_string());
        }
        if pid_mode != ShellPidGuardMode::Off {
            preflight_pid_argv(
                &argv,
                caller_agent_id,
                process_manager,
                pid_mode,
                Some(&ctx),
            )?;
        }
    } else if pid_mode != ShellPidGuardMode::Off {
        preflight_pid_full_shell(
            command,
            caller_agent_id,
            process_manager,
            pid_mode,
            Some(&ctx),
        )?;
    }
    Ok(())
}

fn armaraos_home_hint() -> Option<PathBuf> {
    std::env::var("ARMARAOS_HOME")
        .or_else(|_| std::env::var("OPENFANG_HOME"))
        .ok()
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".armaraos")))
}

fn path_allowed(candidate: &Path, pctx: &PathCheckContext) -> bool {
    let lex_ok = |root: &Path| -> bool {
        let a = candidate.to_string_lossy().replace('\\', "/");
        let b = root.to_string_lossy().replace('\\', "/");
        let b = b.trim_end_matches('/');
        a == b || a.starts_with(&format!("{b}/"))
    };

    if let Some(c) = crate::path_canon_cache::cached_canonicalize(candidate) {
        if let Some(ref ws) = pctx.workspace_canon {
            if c.starts_with(ws) {
                return true;
            }
        }
        if let Some(ref lib) = pctx.library_canon {
            if c.starts_with(lib) {
                return true;
            }
        }
        for ex in &pctx.extra_canon {
            if c.starts_with(ex) {
                return true;
            }
        }
        if let Some(h) = pctx.home_canon.as_ref() {
            if c.starts_with(h) {
                return true;
            }
        }
        return system_prefix_ok(&c);
    }

    if let Some(ref ws) = pctx.workspace_root {
        if lex_ok(ws) {
            return true;
        }
    }
    if let Some(ref lib) = pctx.library_root {
        if lex_ok(lib) {
            return true;
        }
    }
    for ex in &pctx.extra_roots {
        if lex_ok(ex) {
            return true;
        }
    }
    if let Some(h) = pctx.home_canon.as_ref() {
        let lossy = candidate.to_string_lossy();
        if lossy.starts_with(h.to_string_lossy().as_ref()) {
            return true;
        }
    }
    system_prefix_ok(candidate)
}

fn system_prefix_ok(path: &Path) -> bool {
    let s = path.to_string_lossy();
    #[cfg(unix)]
    {
        let prefixes = [
            "/usr/",
            "/bin/",
            "/sbin/",
            "/lib/",
            "/lib64/",
            "/opt/homebrew/",
            "/opt/local/",
            "/nix/",
            "/etc/",
            "/var/",
            "/tmp/",
            "/dev/",
            "/System/",
            "/Library/",
            "/Applications/",
        ];
        prefixes.iter().any(|p| s.starts_with(p))
    }
    #[cfg(windows)]
    {
        let sl = s.to_lowercase();
        sl.starts_with(r"c:\windows\")
            || sl.starts_with(r"c:\program files\")
            || sl.starts_with(r"c:\program files (x86)\")
            || sl.starts_with(r"c:\programdata\")
    }
}

fn looks_like_absolute_path_token(tok: &str) -> bool {
    if tok.starts_with('/') {
        return tok.len() > 1 || tok == "/";
    }
    #[cfg(windows)]
    {
        let t = tok.trim_matches('"');
        if t.len() >= 3 {
            let b = t.as_bytes();
            if b[1] == b':' && (b[2] == b'\\' || b[2] == b'/') && b[0].is_ascii_alphabetic() {
                return true;
            }
        }
    }
    false
}

fn collect_path_candidates_from_token(tok: &str) -> Vec<PathBuf> {
    let t = tok.trim_matches('"').trim_matches('\'');
    let mut out = Vec::new();
    if looks_like_regex_token(t) {
        return out;
    }
    if looks_like_absolute_path_token(t) {
        out.push(PathBuf::from(t));
    }
    for cap in RE_LONG_OPT_EQ_UNIX.captures_iter(t) {
        if let Some(m) = cap.get(1) {
            out.push(PathBuf::from(m.as_str()));
        }
    }
    #[cfg(windows)]
    {
        for cap in RE_LONG_OPT_EQ_WIN.captures_iter(t) {
            if let Some(m) = cap.get(1) {
                out.push(PathBuf::from(m.as_str()));
            }
        }
    }
    if let Some(cap) = RE_SHORT_I_UNIX.captures(t) {
        if let Some(m) = cap.get(1) {
            out.push(PathBuf::from(m.as_str()));
        }
    }
    out
}

fn looks_like_regex_token(tok: &str) -> bool {
    if tok.is_empty() || !tok.starts_with('/') {
        return false;
    }
    // Common regex fragments from sed/rg that are not filesystem paths.
    // Example: /promoter\.
    if tok.contains("\\.") || tok.contains(".*") || tok.contains("[") || tok.contains("(") {
        return true;
    }
    if tok.len() > 2 && tok.ends_with('/') {
        let inner = &tok[1..tok.len() - 1];
        if inner.contains('\\')
            || inner.contains('.')
            || inner.contains('^')
            || inner.contains('$')
            || inner.contains('[')
            || inner.contains('(')
            || inner.contains('*')
            || inner.contains('+')
            || inner.contains('?')
            || inner.contains('|')
        {
            return true;
        }
    }
    false
}

fn collect_path_candidates_from_argv(argv: &[String]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for tok in argv {
        for p in collect_path_candidates_from_token(tok) {
            let k = p.to_string_lossy().to_string();
            if seen.insert(k) {
                out.push(p);
            }
        }
    }
    out
}

fn collect_path_candidates_from_raw_command(command: &str) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for cap in RE_RAW_LONG_EQ_UNIX.captures_iter(command) {
        if let Some(m) = cap.get(1) {
            let p = PathBuf::from(m.as_str());
            let k = p.to_string_lossy().to_string();
            if seen.insert(k) {
                out.push(p);
            }
        }
    }
    #[cfg(windows)]
    {
        for cap in RE_RAW_LONG_EQ_WIN.captures_iter(command) {
            if let Some(m) = cap.get(1) {
                let p = PathBuf::from(m.as_str());
                let k = p.to_string_lossy().to_string();
                if seen.insert(k) {
                    out.push(p);
                }
            }
        }
    }
    for cap in RE_RAW_I_FLAG.captures_iter(command) {
        if let Some(m) = cap.get(1) {
            let p = PathBuf::from(m.as_str());
            let k = p.to_string_lossy().to_string();
            if seen.insert(k) {
                out.push(p);
            }
        }
    }
    out
}

fn record_path_enforce(ctx: Option<&ShellGuardNotifyCtx<'_>>, detail: &str) {
    crate::shell_guard_metrics::inc_path_enforce_denied();
    if let Some(c) = ctx {
        if let Some(k) = c.kernel {
            k.record_shell_guard_event(c.agent_id, "path_enforce", detail, "denied");
        }
    }
}

fn record_path_warn(ctx: Option<&ShellGuardNotifyCtx<'_>>, detail: &str) {
    crate::shell_guard_metrics::inc_path_warn();
    warn!(message = %detail, "shell_path_guard warn mode");
    if let Some(c) = ctx {
        if let Some(k) = c.kernel {
            k.record_shell_guard_event(c.agent_id, "path_warn", detail, "warn_only");
        }
    }
}

fn path_deny_bullet(path_display: &str, pctx: &PathCheckContext, ctx: &str) -> String {
    format!(
        "shell_path_guard[PATH_OUTSIDE_ALLOWLIST] path={path_display:?} context={ctx} \
         extra_prefix_rules={} \
         | hint: use workspace-relative paths, `ainl-library/...` in file tools, or add a root to [exec_policy].extra_allowed_path_prefixes in config.toml or the agent manifest.",
        pctx.extra_rule_count()
    )
}

fn check_paths_for_argv(
    argv: &[String],
    pctx: &PathCheckContext,
    mode: ShellPathGuardMode,
    ctx: Option<&ShellGuardNotifyCtx<'_>>,
) -> Result<(), String> {
    for p in collect_path_candidates_from_argv(argv) {
        let t = p.to_string_lossy();
        if path_allowed(&p, pctx) {
            continue;
        }
        let msg = path_deny_bullet(t.as_ref(), pctx, "argv_token");
        let detail = format!("{msg} argv_token_context");
        match mode {
            ShellPathGuardMode::Off => {}
            ShellPathGuardMode::Warn => {
                record_path_warn(ctx, &detail);
            }
            ShellPathGuardMode::Enforce => {
                record_path_enforce(ctx, &detail);
                return Err(msg);
            }
        }
    }
    Ok(())
}

fn check_paths_in_raw_command(
    command: &str,
    pctx: &PathCheckContext,
    mode: ShellPathGuardMode,
    ctx: Option<&ShellGuardNotifyCtx<'_>>,
) -> Result<(), String> {
    for p in collect_path_candidates_from_raw_command(command) {
        let t = p.to_string_lossy();
        if path_allowed(&p, pctx) {
            continue;
        }
        let msg = path_deny_bullet(t.as_ref(), pctx, "raw_shell");
        let detail = format!("{msg} raw_shell_fallback");
        match mode {
            ShellPathGuardMode::Off => {}
            ShellPathGuardMode::Warn => {
                record_path_warn(ctx, &detail);
            }
            ShellPathGuardMode::Enforce => {
                record_path_enforce(ctx, &detail);
                return Err(msg);
            }
        }
    }
    Ok(())
}

fn argv0_basename(argv0: &str) -> String {
    Path::new(argv0)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(argv0)
        .to_ascii_lowercase()
}

/// PIDs that `kill` will signal: handles `-9`, `--signal=SIG 123`, `-s 9 123`, and BSD `-p 123`.
fn parse_kill_pids_from_kill_argv(argv: &[String]) -> Vec<u32> {
    if argv.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut i = 1usize;
    while i < argv.len() {
        let t = argv[i].trim();
        if t == "--" {
            i += 1;
            while i < argv.len() {
                if let Ok(n) = argv[i].trim().parse::<u32>() {
                    if n > 0 {
                        out.push(n);
                    }
                }
                i += 1;
            }
            break;
        }
        if t == "-l" || t == "-L" || t == "-V" || t == "--version" || t == "--list" {
            return Vec::new();
        }
        if t == "-s" || t == "--signal" {
            i = i.saturating_add(2);
            continue;
        }
        if t.starts_with("--signal=") {
            i += 1;
            continue;
        }
        if t == "-n" {
            i = i.saturating_add(2);
            continue;
        }
        #[cfg(unix)]
        if t == "-p" {
            if i + 1 < argv.len() {
                if let Ok(n) = argv[i + 1].trim().parse::<u32>() {
                    if n > 0 {
                        out.push(n);
                    }
                }
            }
            i += 2;
            continue;
        }
        if let Some(rest) = t.strip_prefix('-') {
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                i += 1;
                continue;
            }
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }
        if let Ok(n) = t.parse::<u32>() {
            if n > 0 {
                out.push(n);
            }
        }
        i += 1;
    }
    out
}

fn pid_guard_fail(ctx: Option<&ShellGuardNotifyCtx<'_>>, msg: String) -> Result<(), String> {
    crate::shell_guard_metrics::inc_pid_enforce_denied();
    if let Some(c) = ctx {
        if let Some(k) = c.kernel {
            k.record_shell_guard_event(c.agent_id, "pid_enforce", &msg, "denied");
        }
    }
    Err(msg)
}

fn preflight_pid_argv(
    argv: &[String],
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    mode: ShellPidGuardMode,
    ctx: Option<&ShellGuardNotifyCtx<'_>>,
) -> Result<(), String> {
    if mode == ShellPidGuardMode::Off {
        return Ok(());
    }
    let bin = argv0_basename(&argv[0]);
    match bin.as_str() {
        "pkill" | "killall" => {
            return pid_guard_fail(
                ctx,
                "shell_pid_guard: `pkill` / `killall` are blocked in shell_exec — they can terminate unrelated host processes. \
                 Use `process_kill` with a `process_id` from `process_start` / `process_list`."
                    .to_string(),
            );
        }
        "kill" => {
            let pids = parse_kill_pids_from_kill_argv(argv);
            if pids.is_empty() {
                return Ok(());
            }
            let Some(agent) = caller_agent_id else {
                return pid_guard_fail(
                    ctx,
                    "shell_pid_guard: cannot verify `kill` targets without an agent context."
                        .to_string(),
                );
            };
            let Some(pm) = process_manager else {
                return pid_guard_fail(
                    ctx,
                    "shell_pid_guard: process manager unavailable; cannot verify `kill` targets."
                        .to_string(),
                );
            };
            for pid in pids {
                if !pm.agent_owns_os_pid(agent, pid) {
                    return pid_guard_fail(
                        ctx,
                        format!(
                            "shell_pid_guard: PID {pid} is not a `process_start` child owned by this agent. \
                             Use `process_kill` with the `process_id` from `process_list`."
                        ),
                    );
                }
            }
        }
        #[cfg(windows)]
        "taskkill" => {
            let joined = argv.join(" ");
            for cap in RE_TASKKILL_PID.captures_iter(&joined) {
                if let Some(m) = cap.get(1) {
                    if let Ok(pid) = m.as_str().parse::<u32>() {
                        if pid == 0 {
                            continue;
                        }
                        let Some(agent) = caller_agent_id else {
                            return pid_guard_fail(
                                ctx,
                                "shell_pid_guard: cannot verify `taskkill` targets without an agent context."
                                    .to_string(),
                            );
                        };
                        let Some(pm) = process_manager else {
                            return pid_guard_fail(
                                ctx,
                                "shell_pid_guard: process manager unavailable; cannot verify `taskkill` targets."
                                    .to_string(),
                            );
                        };
                        if !pm.agent_owns_os_pid(agent, pid) {
                            return pid_guard_fail(
                                ctx,
                                format!(
                                    "shell_pid_guard: PID {pid} is not a `process_start` child owned by this agent. \
                                     Use `process_kill` with the `process_id` from `process_list`."
                                ),
                            );
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn preflight_pid_full_shell(
    command: &str,
    caller_agent_id: Option<&str>,
    process_manager: Option<&crate::process_manager::ProcessManager>,
    mode: ShellPidGuardMode,
    ctx: Option<&ShellGuardNotifyCtx<'_>>,
) -> Result<(), String> {
    if mode == ShellPidGuardMode::Off {
        return Ok(());
    }
    if RE_SHELL_PKILL.is_match(command) || RE_SHELL_KILLALL.is_match(command) {
        return pid_guard_fail(
            ctx,
            "shell_pid_guard: `pkill` / `killall` are blocked in unrestricted shell_exec. \
             Use `process_kill` with a `process_id` from `process_start` / `process_list`."
                .to_string(),
        );
    }
    if RE_SHELL_TASKKILL.is_match(command) {
        let Some(agent) = caller_agent_id else {
            return pid_guard_fail(
                ctx,
                "shell_pid_guard: cannot verify `taskkill` without an agent context.".to_string(),
            );
        };
        let Some(pm) = process_manager else {
            return pid_guard_fail(
                ctx,
                "shell_pid_guard: process manager unavailable; cannot verify `taskkill`."
                    .to_string(),
            );
        };
        for cap in RE_TASKKILL_PID.captures_iter(command) {
            if let Some(m) = cap.get(1) {
                if let Ok(pid) = m.as_str().parse::<u32>() {
                    if pid > 0 && !pm.agent_owns_os_pid(agent, pid) {
                        return pid_guard_fail(
                            ctx,
                            format!(
                                "shell_pid_guard: PID {pid} is not a `process_start` child owned by this agent."
                            ),
                        );
                    }
                }
            }
        }
    }
    let Some(agent) = caller_agent_id else {
        return Ok(());
    };
    let Some(pm) = process_manager else {
        return Ok(());
    };
    if let Some(argv) = shlex::split(command) {
        if !argv.is_empty() && argv0_basename(&argv[0]) == "kill" {
            for pid in parse_kill_pids_from_kill_argv(&argv) {
                if pid > 0 && !pm.agent_owns_os_pid(agent, pid) {
                    return pid_guard_fail(
                        ctx,
                        format!(
                            "shell_pid_guard: PID {pid} is not a `process_start` child owned by this agent. \
                             Use `process_kill` with the `process_id` from `process_list`."
                        ),
                    );
                }
            }
            return Ok(());
        }
    }
    for cap in RE_SHELL_KILL_PIDS.captures_iter(command) {
        if let Some(m) = cap.get(1) {
            if let Ok(pid) = m.as_str().parse::<u32>() {
                if pid > 0 && !pm.agent_owns_os_pid(agent, pid) {
                    return pid_guard_fail(
                        ctx,
                        format!(
                            "shell_pid_guard: PID {pid} is not a `process_start` child owned by this agent. \
                             Use `process_kill` with the `process_id` from `process_list`."
                        ),
                    );
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_manager::ProcessManager;
    use openfang_types::config::ShellPathGuardMode;
    use tempfile::tempdir;

    #[test]
    fn path_guard_blocks_unknown_absolute_without_workspace() {
        let argv = vec![
            "cat".into(),
            "/definitely_made_up_openfang_shell_argv_guard_path/nope".into(),
        ];
        let p = PathCheckContext::new(None, None, &[]);
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_err());
    }

    #[test]
    fn path_guard_blocks_embedded_long_opt() {
        let argv = vec![
            "gcc".into(),
            "--sysroot=/definitely_made_up_openfang_embedded_bad/nope".into(),
        ];
        let p = PathCheckContext::new(None, None, &[]);
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_err());
    }

    #[test]
    fn path_guard_allows_extra_prefix() {
        let argv = vec![
            "cat".into(),
            "/definitely_made_up_openfang_extra_prefix/nope".into(),
        ];
        let p = PathCheckContext::new(
            None,
            None,
            &["/definitely_made_up_openfang_extra_prefix".into()],
        );
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_ok());
    }

    #[test]
    fn path_guard_allows_usr_bin() {
        let argv = vec!["ls".into(), "/usr/bin".into()];
        let p = PathCheckContext::new(None, None, &[]);
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_ok());
    }

    #[test]
    fn path_guard_ignores_regex_like_tokens() {
        let argv = vec!["rg".into(), "/promoter\\.".into(), "gateway_server.py".into()];
        let p = PathCheckContext::new(None, None, &[]);
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_ok());
    }

    #[test]
    fn path_guard_allows_under_workspace_lexical() {
        let dir = tempdir().unwrap();
        let inside = dir.path().join("nested").join("x.txt");
        let argv = vec!["cat".into(), inside.display().to_string()];
        let p = PathCheckContext::new(Some(dir.path()), None, &[]);
        let r = check_paths_for_argv(&argv, &p, ShellPathGuardMode::Enforce, None);
        assert!(r.is_ok());
    }

    #[test]
    fn full_mode_path_guard_via_preflight() {
        let cmd = "gcc -I/definitely_made_up_openfang_full_mode_bad/include -c foo.c";
        let r = preflight_shell_exec(
            cmd,
            false,
            None,
            None,
            &[],
            ShellPathGuardMode::Enforce,
            ShellPidGuardMode::Off,
            None,
            None,
            None,
        );
        assert!(r.is_err());
    }

    #[test]
    fn full_mode_raw_scan_when_shlex_fails() {
        let cmd = "gcc --sysroot=/definitely_made_up_openfang_raw_scan_bad/sys 'unclosed";
        let r = preflight_shell_exec(
            cmd,
            false,
            None,
            None,
            &[],
            ShellPathGuardMode::Enforce,
            ShellPidGuardMode::Off,
            None,
            None,
            None,
        );
        assert!(r.is_err());
    }

    #[test]
    fn kill_argv_parses_signal_before_pids() {
        let argv = vec!["kill".into(), "-s".into(), "9".into(), "1001".into()];
        let pids = parse_kill_pids_from_kill_argv(&argv);
        assert_eq!(pids, vec![1001]);
    }

    #[test]
    fn kill_argv_signal_short_form() {
        let argv = vec!["kill".into(), "-9".into(), "1002".into()];
        let pids = parse_kill_pids_from_kill_argv(&argv);
        assert_eq!(pids, vec![1002]);
    }

    #[tokio::test]
    async fn pid_guard_kill_rejects_foreign_pid() {
        let pm = ProcessManager::new(5);
        let cmd = if cfg!(windows) { "cmd" } else { "sleep" };
        let args: Vec<String> = if cfg!(windows) {
            vec!["/C".into(), "timeout".into(), "/t".into(), "30".into()]
        } else {
            vec!["60".into()]
        };
        let id = pm.start("a1", cmd, &args, None, None).await.unwrap();
        let foreign: u32 = 999_001;
        let argv = vec!["kill".into(), foreign.to_string()];
        let ctx = ShellGuardNotifyCtx {
            kernel: None,
            agent_id: Some("a1"),
        };
        let err = preflight_pid_argv(
            &argv,
            Some("a1"),
            Some(&pm),
            ShellPidGuardMode::Enforce,
            Some(&ctx),
        )
        .unwrap_err();
        assert!(err.contains("shell_pid_guard"));
        let _ = pm.kill(&id).await;
    }
}
