//! Best-effort TCP port checks before `shell_exec` runs commands that look like local servers.
//!
//! Goal: fail fast with an actionable error when a dev server is about to bind a port that is
//! already taken — instead of opaque `Address already in use` after spawn.

use regex::Regex;
use std::io;
use std::net::TcpListener;
use std::sync::OnceLock;

fn re_host_port() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r#"(?:^|[\s/;'"`])(?:127\.0\.0\.1|0\.0\.0\.0|localhost):(\d{2,5})(?:[\s/;'"`]|$)"#,
        )
        .expect("host:port regex")
    })
}

fn re_long_flag_port() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"--port(?:=|\s+)(\d{2,5})\b").expect("--port regex"))
}

fn re_short_p_port() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"(?:^|\s)-p(?:=|\s+)(\d{2,5})\b").expect("-p port regex"))
}

fn re_env_port() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| Regex::new(r"\bPORT(?:=|\s+)(\d{2,5})\b").expect("PORT= regex"))
}

/// Heuristic: command likely starts a process that listens on localhost.
pub(crate) fn command_may_bind_tcp(cmd: &str) -> bool {
    let c = cmd.to_ascii_lowercase();
    // Dev / app servers and static file servers
    c.contains("http.server")
        || c.contains("simplehttpserver")
        || c.contains("uvicorn")
        || c.contains("gunicorn")
        || c.contains("hypercorn")
        || c.contains("daphne")
        || c.contains("granian")
        || c.contains("flask run")
        || c.contains("quart run")
        || c.contains("manage.py runserver")
        || c.contains("django-admin runserver")
        || c.contains("webpack-dev-server")
        || c.contains("webpack serve")
        || c.contains(" vite")
        || c.contains("vite ")
        || c.starts_with("vite")
        || c.contains("next dev")
        || c.contains("parcel ")
        || c.contains("php -s")
        || c.contains("php --server")
        || c.contains("ruby -run")
        || c.contains("npx serve")
        || c.contains("serve -s")
        || c.contains("live-server")
        || (c.contains("node ")
            && (c.contains("listen") || c.contains("server.js") || c.contains("index.js")))
        || (c.contains("npm ") && (c.contains("start") || c.contains("run dev")))
        || (c.contains("pnpm ") && (c.contains("start") || c.contains("dev")))
        || (c.contains("yarn ") && (c.contains("start") || c.contains("dev")))
        || (c.contains("bun ") && (c.contains("run") || c.contains("dev")))
        || (c.contains("cargo run") && (c.contains("server") || c.contains("listen")))
}

fn push_port(ports: &mut Vec<u16>, p: u16) {
    if !(1..=65535).contains(&p) {
        return;
    }
    if !ports.contains(&p) {
        ports.push(p);
    }
}

/// Parse likely listen ports from flags / host literals (best-effort).
pub(crate) fn extract_candidate_tcp_ports(cmd: &str) -> Vec<u16> {
    let mut ports = Vec::new();
    for cap in re_host_port().captures_iter(cmd) {
        if let Ok(n) = cap[1].parse::<u16>() {
            push_port(&mut ports, n);
        }
    }
    for cap in re_long_flag_port().captures_iter(cmd) {
        if let Ok(n) = cap[1].parse::<u16>() {
            push_port(&mut ports, n);
        }
    }
    for cap in re_short_p_port().captures_iter(cmd) {
        if let Ok(n) = cap[1].parse::<u16>() {
            push_port(&mut ports, n);
        }
    }
    for cap in re_env_port().captures_iter(cmd) {
        if let Ok(n) = cap[1].parse::<u16>() {
            push_port(&mut ports, n);
        }
    }
    // `python -m http.server 9000` — positional port
    if cmd.to_ascii_lowercase().contains("http.server") {
        if let Some(rest) = cmd.to_ascii_lowercase().split("http.server").nth(1) {
            let tail = rest.trim_start();
            if let Some(first) = tail.split_whitespace().next() {
                if let Ok(n) = first.parse::<u16>() {
                    push_port(&mut ports, n);
                }
            }
        }
    }
    ports
}

fn apply_default_listen_ports(cmd: &str, ports: &mut Vec<u16>) {
    if !ports.is_empty() {
        return;
    }
    let c = cmd.to_ascii_lowercase();
    if c.contains("http.server") || c.contains("simplehttpserver") {
        push_port(ports, 8000);
    } else if c.contains("flask run") || c.contains("quart run") {
        push_port(ports, 5000);
    } else if c.contains("manage.py runserver") || c.contains("django-admin runserver") {
        push_port(ports, 8000);
    }
}

fn port_busy_on_loopback(port: u16) -> bool {
    match TcpListener::bind(std::net::SocketAddr::from(([127, 0, 0, 1], port))) {
        Ok(listener) => {
            drop(listener);
            false
        }
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => true,
        Err(_) => false,
    }
}

/// If any candidate port is busy on 127.0.0.1, return an error string for `shell_exec`.
pub(crate) fn preflight_shell_listen_ports(command: &str) -> Result<(), String> {
    if !command_may_bind_tcp(command) {
        return Ok(());
    }
    let mut ports = extract_candidate_tcp_ports(command);
    apply_default_listen_ports(command, &mut ports);
    if ports.is_empty() {
        return Ok(());
    }
    ports.sort_unstable();
    ports.dedup();
    for p in ports {
        if port_busy_on_loopback(p) {
            let next = p.saturating_add(1);
            return Err(format!(
                "Port {p} appears to already be in use on 127.0.0.1 (runtime preflight bind check). \
Another process is likely listening. Choose a free port (e.g. {next}), stop the other service, \
or probe first: `lsof -nP -iTCP:{p} -sTCP:LISTEN` (macOS/Linux) / \
`netstat -ano | findstr :{p}` (Windows cmd). \
If you did not intend to start a server, avoid auto-binding dev servers unless the user asked."
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_http_server_default() {
        let cmd = "python3 -m http.server";
        assert!(command_may_bind_tcp(cmd));
        let mut p = extract_candidate_tcp_ports(cmd);
        apply_default_listen_ports(cmd, &mut p);
        assert_eq!(p, vec![8000]);
    }

    #[test]
    fn extracts_host_port_and_flags() {
        let cmd = "uvicorn app:app --host 0.0.0.0 --port 8765 -p 9999 PORT=4000";
        let mut p = extract_candidate_tcp_ports(cmd);
        apply_default_listen_ports(cmd, &mut p);
        p.sort_unstable();
        p.dedup();
        assert!(p.contains(&8765));
        assert!(p.contains(&9999));
        assert!(p.contains(&4000));
    }
}
