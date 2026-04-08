//! Host-side network hints for VPN / proxy awareness (dashboard copy, telemetry).
//!
//! Uses `if-addrs` to list interfaces and heuristics for common tunnel names.
//! This is advisory only — false negatives and positives are possible.

use serde_json::json;

/// Build JSON for `GET /api/system/network-hints` and provider-test error payloads.
pub fn collect() -> serde_json::Value {
    let mut tunnel_names: Vec<String> = Vec::new();
    let mut all_names: Vec<String> = Vec::new();

    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for a in addrs {
            let n = a.name.clone();
            if !all_names.contains(&n) {
                all_names.push(n.clone());
            }
            if tunnel_like(&n) && !tunnel_names.contains(&n) {
                tunnel_names.push(n);
            }
        }
    }

    all_names.sort();
    tunnel_names.sort();

    let likely_vpn = !tunnel_names.is_empty();
    let confidence = if tunnel_names.iter().any(|n| {
        let l = n.to_lowercase();
        l.starts_with("utun") || l.starts_with("wg") || l.contains("wintun") || l.starts_with("tun")
    }) {
        "medium"
    } else if likely_vpn {
        "low"
    } else {
        "none"
    };

    let proxy = proxy_env_flags();
    let mut notes: Vec<String> = Vec::new();
    if likely_vpn {
        notes.push(
            "Virtual tunnel network interfaces are present (common when a VPN is active). Outbound HTTPS to LLM APIs may be blocked or filtered; the local dashboard connection is usually unaffected."
                .to_string(),
        );
    }
    if proxy.any_set {
        notes.push(
            "HTTP_PROXY / HTTPS_PROXY / ALL_PROXY may affect how the daemon reaches API providers. Ensure the proxy allows your provider endpoints."
                .to_string(),
        );
    }

    json!({
        "likely_vpn": likely_vpn,
        "confidence": confidence,
        "tunnel_interface_names": tunnel_names,
        "interface_names": all_names,
        "proxy_env": proxy.json,
        "notes": notes,
    })
}

fn tunnel_like(name: &str) -> bool {
    let n = name.to_lowercase();
    n.starts_with("utun")
        || n.starts_with("tun")
        || n.starts_with("tap")
        || n.starts_with("wg")
        || n.contains("wintun")
        || n.contains("wireguard")
        || n.contains("ppp")
        || n.contains("ipsec")
        || n.contains("l2tp")
        || n.contains("nordlynx")
        || n.contains("zerotier")
        || n.contains("tailscale")
        || n.contains("openvpn")
        || n.ends_with("vpn")
        || n.contains("warp")
        || n.contains("nordvpn")
        || n.contains("cisco")
}

struct ProxyEnv {
    any_set: bool,
    json: serde_json::Value,
}

fn proxy_env_flags() -> ProxyEnv {
    let keys = ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "NO_PROXY"];
    let mut map = serde_json::Map::new();
    let mut any = false;
    for k in keys {
        let set = std::env::var(k).ok().filter(|s| !s.trim().is_empty()).is_some();
        if set {
            any = true;
        }
        map.insert(k.to_lowercase(), json!(set));
    }
    ProxyEnv {
        any_set: any,
        json: serde_json::Value::Object(map),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_returns_shape() {
        let v = collect();
        assert!(v.get("likely_vpn").is_some());
        assert!(v.get("confidence").is_some());
        assert!(v.get("tunnel_interface_names").is_some());
        assert!(v.get("interface_names").is_some());
        assert!(v.get("proxy_env").is_some());
        assert!(v.get("notes").is_some());
    }
}
