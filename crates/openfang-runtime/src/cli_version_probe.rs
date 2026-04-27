//! Bounded synchronous `PATH` CLI probes used during kernel boot (`model_catalog::detect_auth`).
//!
//! A broken or interactive `claude` / `qwen` shim must not wedge `OpenFangKernel::boot_with_config`.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const DEFAULT_CLI_PROBE_DEADLINE: Duration = Duration::from_millis(1500);

/// Run `program` with `args`, capture stdout, and return a trimmed string on **success** only.
///
/// On spawn failure, non-zero exit, empty stdout, **timeout**, or hung child, returns `None`.
pub fn probe_stdout(program: &str, args: &[&str]) -> Option<String> {
    probe_stdout_with_deadline(program, args, DEFAULT_CLI_PROBE_DEADLINE)
}

pub(crate) fn probe_stdout_with_deadline(
    program: &str,
    args: &[&str],
    deadline: Duration,
) -> Option<String> {
    let mut cmd = Command::new(program);
    for a in args {
        cmd.arg(a);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(cmd.output());
    });

    let output = match rx.recv_timeout(deadline) {
        Ok(Ok(out)) => out,
        Ok(Err(_)) | Err(_) => return None,
    };

    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn probe_sh_echo_returns_trimmed_stdout() {
        let out = probe_stdout_with_deadline("/bin/sh", &["-c", "echo hi"], Duration::from_secs(2));
        assert_eq!(out.as_deref(), Some("hi"));
    }
}
