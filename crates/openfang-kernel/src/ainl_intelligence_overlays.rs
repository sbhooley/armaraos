//! Overlay known-good `intelligence/*.lang` files into `~/.armaraos/ainl-library/intelligence/`.
//!
//! GitHub sync mirrors upstream by tag or `main`; tokenizer fixes may land in core before the next
//! release tag. Materializing these bytes after sync (desktop) and on kernel boot keeps the mirror
//! consistent with ArmaraOS without manual copies.
//!
//! Disable with `ARMARAOS_SKIP_INTELLIGENCE_OVERLAYS=1`.

use std::fs;
use std::path::Path;

fn skip_overlays() -> bool {
    std::env::var("ARMARAOS_SKIP_INTELLIGENCE_OVERLAYS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

static OVERLAYS: &[(&str, &[u8])] = &[(
    "auto_tune_ainl_caps.lang",
    include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/ainl-intelligence-overlays/auto_tune_ainl_caps.lang"
    )),
)];

/// Bump when overlay bytes change meaningfully (shown beside `.embedded-revision` for intelligence).
pub const INTELLIGENCE_OVERLAY_REVISION: &str = "1";

/// Write embedded intelligence files under `ainl-library/intelligence/`. Idempotent.
pub fn materialize_intelligence_overlays(home_dir: &Path) -> Result<usize, String> {
    if skip_overlays() {
        tracing::debug!("Skipping intelligence overlays (ARMARAOS_SKIP_INTELLIGENCE_OVERLAYS)");
        return Ok(0);
    }

    let dest_base = home_dir.join("ainl-library").join("intelligence");
    fs::create_dir_all(&dest_base).map_err(|e| format!("create {}: {e}", dest_base.display()))?;

    let mut n = 0usize;
    for (name, bytes) in OVERLAYS {
        let out_path = dest_base.join(name);
        let write = match fs::read(&out_path) {
            Ok(existing) => existing != *bytes,
            Err(_) => true,
        };
        if write {
            fs::write(&out_path, bytes)
                .map_err(|e| format!("write {}: {e}", out_path.display()))?;
            n += 1;
        }
    }

    let manifest_path = dest_base.join(".armaraos-intelligence-overlays-revision.txt");
    let body = format!(
        "{INTELLIGENCE_OVERLAY_REVISION}\nEmbedded overlays (openfang-kernel ainl_intelligence_overlays).\n"
    );
    let _ = fs::write(&manifest_path, &body);

    if n > 0 {
        tracing::info!(
            count = n,
            dir = %dest_base.display(),
            "Materialized AINL intelligence overlays"
        );
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_list_non_empty() {
        assert!(!OVERLAYS.is_empty());
        assert!(OVERLAYS.iter().all(|(_, b)| !b.is_empty()));
    }
}
