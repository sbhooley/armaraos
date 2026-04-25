//! Reject shell job-control in `shell_exec` that cannot run under argv-only execution
//! or should use `process_start` for long-lived / background processes.

use std::sync::OnceLock;

use regex::Regex;

/// Substring present in [`preflight_shell_job_control`] errors — used to skip duplicate
/// corrective hints in `agent_loop`.
pub(crate) const SHELL_JOB_GUARD_TAG: &str = "shell job-control or background syntax";

const USE_PROCESS_START: &str = "Use `process_start` with `command` + `args` and optional `cwd`, then `process_poll` / `process_kill`.";

/// Strip single- and double-quoted regions to spaces so metacharacter scans ignore literals.
fn strip_quoted_regions(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut in_single = false;
    let mut in_double = false;
    for c in cmd.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(' ');
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(' ');
            }
            _ if in_single || in_double => out.push(' '),
            _ => out.push(c),
        }
    }
    out
}

fn remove_io_redirect_ampersand_pairs(s: &str) -> String {
    let mut t = s.to_string();
    for pat in [
        "2>&1", "1>&2", "0>&1", "3>&1", "2>&2", "1>&1", "2>>&1", ">&1", ">&2", "<&2", "<&1",
    ] {
        t = t.replace(pat, "");
    }
    t
}

fn re_nohup_disown() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)(?:^|[;|])\s*nohup\b|(?:^|[;|])\s*disown\b|/\bnohup\b|/\bdisown\b|&&\s*nohup\b|&&\s*disown\b|&&nohup\b|&&disown\b|\|\|\s*nohup\b|\|\|\s*disown\b",
        )
        .expect("nohup/disown regex")
    })
}

fn job_control_word_outside_quotes(cmd: &str) -> bool {
    let cleaned = strip_quoted_regions(cmd);
    re_nohup_disown().is_match(cleaned.trim_start())
}

/// `&` used for shell backgrounding under argv-only mode — not `&&`, not common redirect pairs,
/// not URL-like `?a=1&b=2` (no space before `&`).
fn bare_shell_background_ampersand(cmd: &str) -> bool {
    let cleaned = remove_io_redirect_ampersand_pairs(&strip_quoted_regions(cmd));
    let u = cleaned.replace("&&", "");
    if u.contains(" &") || u.contains("\t&") {
        return true;
    }
    // Trailing `… &` (job background), but not `…2>&1` (already stripped).
    let t = u.trim_end();
    if let Some(pos) = t.rfind('&') {
        if pos > 0 && t.ends_with('&') {
            let prev = t[..pos].trim_end().chars().last();
            if prev != Some('>') && prev != Some('&') {
                return true;
            }
        }
    }
    false
}

/// Return `Err` when the command should use `process_start` instead of `shell_exec`.
///
/// - **All modes:** `nohup` / `disown` outside quotes (best-effort).
/// - **Allowlist / argv-only (`use_direct_exec`):** also rejects shell-background `&` heuristics
///   and standalone `&` / `|` / `;` / `&&` / `||` argv tokens after `shlex` split.
pub(crate) fn preflight_shell_job_control(
    command: &str,
    use_direct_exec: bool,
) -> Result<(), String> {
    fn message() -> String {
        format!(
            "shell_exec cannot run {SHELL_JOB_GUARD_TAG} (nohup/disown, or `&` for backgrounding). {USE_PROCESS_START}"
        )
    }

    if job_control_word_outside_quotes(command) {
        return Err(message());
    }

    if use_direct_exec {
        if bare_shell_background_ampersand(command) {
            return Err(message());
        }
        if let Some(argv) = shlex::split(command) {
            if argv.iter().any(|a| matches!(a.as_str(), "&" | "|" | ";")) {
                return Err(message());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_nohup_outside_quotes() {
        assert!(preflight_shell_job_control("nohup python3 server.py", true).is_err());
        assert!(preflight_shell_job_control("cd x && nohup python3 server.py", false).is_err());
    }

    #[test]
    fn allows_nohup_inside_single_quotes() {
        assert!(preflight_shell_job_control("echo 'nohup is a word'", true).is_ok());
    }

    #[test]
    fn allows_echo_nohup_token() {
        assert!(preflight_shell_job_control("echo nohup", true).is_ok());
    }

    #[test]
    fn rejects_space_ampersand_in_allowlist() {
        assert!(preflight_shell_job_control("sleep 1 &", true).is_err());
    }

    #[test]
    fn allows_curl_query_string_unquoted() {
        // URL & without shell list spacing — must not false-positive.
        assert!(preflight_shell_job_control("curl -s http://example.com?q=1&r=2", true).is_ok());
    }

    #[test]
    fn allows_double_ampersand() {
        assert!(preflight_shell_job_control("cd /tmp && ls", true).is_ok());
    }

    #[test]
    fn full_mode_still_blocks_nohup() {
        assert!(preflight_shell_job_control("nohup python3 x.py", false).is_err());
    }

    #[test]
    fn full_mode_allows_unquoted_url_ampersand() {
        assert!(preflight_shell_job_control("curl http://a.com?x=1&y=2", false).is_ok());
    }
}
