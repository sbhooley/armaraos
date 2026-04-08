//! Compile-time embedded agent templates.
//!
//! All 30 bundled agent templates are embedded into the binary via `include_str!`.
//! This ensures `openfang agent new` works immediately after install — no filesystem
//! discovery needed.
//!
//! Update strategy: `install_bundled_agents` writes a new template if the file does not
//! exist **or** if the on-disk content differs from the bundled version (i.e. the app was
//! upgraded). User-created agents in `~/.armaraos/agents/` that are *not* named after a
//! bundled template are never touched.

/// Returns all bundled agent templates as `(name, toml_content)` pairs.
pub fn bundled_agents() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "analyst",
            include_str!("../../../agents/analyst/agent.toml"),
        ),
        (
            "architect",
            include_str!("../../../agents/architect/agent.toml"),
        ),
        (
            "assistant",
            include_str!("../../../agents/assistant/agent.toml"),
        ),
        ("coder", include_str!("../../../agents/coder/agent.toml")),
        (
            "code-reviewer",
            include_str!("../../../agents/code-reviewer/agent.toml"),
        ),
        (
            "customer-support",
            include_str!("../../../agents/customer-support/agent.toml"),
        ),
        (
            "data-scientist",
            include_str!("../../../agents/data-scientist/agent.toml"),
        ),
        (
            "debugger",
            include_str!("../../../agents/debugger/agent.toml"),
        ),
        (
            "devops-lead",
            include_str!("../../../agents/devops-lead/agent.toml"),
        ),
        (
            "doc-writer",
            include_str!("../../../agents/doc-writer/agent.toml"),
        ),
        (
            "email-assistant",
            include_str!("../../../agents/email-assistant/agent.toml"),
        ),
        (
            "health-tracker",
            include_str!("../../../agents/health-tracker/agent.toml"),
        ),
        (
            "hello-world",
            include_str!("../../../agents/hello-world/agent.toml"),
        ),
        (
            "home-automation",
            include_str!("../../../agents/home-automation/agent.toml"),
        ),
        (
            "legal-assistant",
            include_str!("../../../agents/legal-assistant/agent.toml"),
        ),
        (
            "meeting-assistant",
            include_str!("../../../agents/meeting-assistant/agent.toml"),
        ),
        ("ops", include_str!("../../../agents/ops/agent.toml")),
        (
            "orchestrator",
            include_str!("../../../agents/orchestrator/agent.toml"),
        ),
        (
            "personal-finance",
            include_str!("../../../agents/personal-finance/agent.toml"),
        ),
        (
            "planner",
            include_str!("../../../agents/planner/agent.toml"),
        ),
        (
            "recruiter",
            include_str!("../../../agents/recruiter/agent.toml"),
        ),
        (
            "researcher",
            include_str!("../../../agents/researcher/agent.toml"),
        ),
        (
            "sales-assistant",
            include_str!("../../../agents/sales-assistant/agent.toml"),
        ),
        (
            "security-auditor",
            include_str!("../../../agents/security-auditor/agent.toml"),
        ),
        (
            "social-media",
            include_str!("../../../agents/social-media/agent.toml"),
        ),
        (
            "test-engineer",
            include_str!("../../../agents/test-engineer/agent.toml"),
        ),
        (
            "translator",
            include_str!("../../../agents/translator/agent.toml"),
        ),
        (
            "travel-planner",
            include_str!("../../../agents/travel-planner/agent.toml"),
        ),
        ("tutor", include_str!("../../../agents/tutor/agent.toml")),
        ("writer", include_str!("../../../agents/writer/agent.toml")),
    ]
}

/// Install (or upgrade) bundled agent templates to `~/.armaraos/agents/`.
///
/// Behaviour:
/// - **New install:** writes the template when no file exists yet.
/// - **Upgrade:** overwrites the on-disk file when it differs from the bundled version, so
///   system-prompt improvements and platform knowledge blocks propagate automatically on
///   app rebuild. User agents whose names don't match any bundled template are never touched.
pub fn install_bundled_agents(agents_dir: &std::path::Path) {
    for (name, content) in bundled_agents() {
        let dest_dir = agents_dir.join(name);
        let dest_file = dest_dir.join("agent.toml");

        // Write when absent or when the bundled content differs from what's on disk.
        let needs_write = if dest_file.exists() {
            std::fs::read_to_string(&dest_file)
                .map(|existing| existing != content)
                .unwrap_or(true) // can't read → overwrite to be safe
        } else {
            true
        };

        if needs_write && std::fs::create_dir_all(&dest_dir).is_ok() {
            let _ = std::fs::write(&dest_file, content);
        }
    }
}
