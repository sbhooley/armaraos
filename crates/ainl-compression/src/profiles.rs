//! Built-in compression profiles (policy vocabulary for hosts).
//!
//! Persistence and per-project overrides live in embedding hosts (`openfang-runtime`, etc.);
//! this module defines **portable defaults** and **project → profile** heuristics.

use crate::EfficientMode;

/// One named profile: stable id, human label, and default eco mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressionProfile {
    pub id: &'static str,
    pub display_name: &'static str,
    pub default_mode: EfficientMode,
    pub description: &'static str,
}

/// Registry of profiles shipped with `ainl-compression` (no disk I/O).
pub const BUILTIN_PROFILES: &[CompressionProfile] = &[
    CompressionProfile {
        id: "default",
        display_name: "Default",
        default_mode: EfficientMode::Balanced,
        description: "General prompts; balanced retention (~55%).",
    },
    CompressionProfile {
        id: "cost_sensitive",
        display_name: "Cost sensitive",
        default_mode: EfficientMode::Aggressive,
        description: "High-volume / batch paths; prefer stronger reduction when safe.",
    },
    CompressionProfile {
        id: "quality_preserve",
        display_name: "Quality preserve",
        default_mode: EfficientMode::Balanced,
        description: "Customer- or prod-adjacent contexts; conservative defaults (future: stricter preserve gates).",
    },
];

#[must_use]
pub fn list_builtin_profiles() -> &'static [CompressionProfile] {
    BUILTIN_PROFILES
}

#[must_use]
pub fn resolve_builtin_profile(id: &str) -> Option<&'static CompressionProfile> {
    let k = id.trim();
    BUILTIN_PROFILES
        .iter()
        .find(|p| p.id.eq_ignore_ascii_case(k))
}

/// Heuristic mapping from a **project identifier** (repo slug, cwd basename, MCP `project_id`) to a built-in profile id.
///
/// Hosts may override with on-disk config later; this stays dependency-free.
#[must_use]
pub fn suggest_profile_id_for_project(project_id: &str) -> &'static str {
    let p = project_id.to_ascii_lowercase();
    if p.contains("prod")
        || p.contains("customer")
        || p.contains("staging")
        || p.contains("release")
    {
        "quality_preserve"
    } else if p.contains("batch") || p.contains("ci") || p.contains("cron") || p.contains("scaled")
    {
        "cost_sensitive"
    } else {
        "default"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_roundtrip() {
        let p = resolve_builtin_profile("DEFAULT").expect("default");
        assert_eq!(p.id, "default");
        assert_eq!(p.default_mode, EfficientMode::Balanced);
    }

    #[test]
    fn suggest_prod_leans_quality() {
        assert_eq!(
            suggest_profile_id_for_project("acme-customer-prod"),
            "quality_preserve"
        );
    }

    #[test]
    fn suggest_ci_leans_cost() {
        assert_eq!(
            suggest_profile_id_for_project("nightly-ci-batch"),
            "cost_sensitive"
        );
    }
}
