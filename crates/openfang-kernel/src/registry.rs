//! Agent registry — tracks all agents, their state, and indexes.

use dashmap::DashMap;
use openfang_types::agent::{AgentEntry, AgentId, AgentManifest, AgentMode, AgentState};
use openfang_types::error::{OpenFangError, OpenFangResult};
use std::path::PathBuf;

/// Registry of all agents in the kernel.
pub struct AgentRegistry {
    /// Primary index: agent ID → entry.
    agents: DashMap<AgentId, AgentEntry>,
    /// Name index: human-readable name → agent ID.
    name_index: DashMap<String, AgentId>,
    /// Tag index: tag → list of agent IDs.
    tag_index: DashMap<String, Vec<AgentId>>,
    /// ArmaraOS home root (same as `KernelConfig::home_dir`). Agent templates live under
    /// `agent_home/agents/<name>/`. Defaults to [`openfang_types::config::openfang_home_dir`].
    agent_home: PathBuf,
}

impl AgentRegistry {
    /// Create a new empty registry (uses [`openfang_types::config::openfang_home_dir`] for disk paths).
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
            agent_home: openfang_types::config::openfang_home_dir(),
        }
    }

    /// Registry with an explicit ArmaraOS home root (must match the kernel's `config.home_dir`).
    pub fn with_agent_home(agent_home: PathBuf) -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
            agent_home,
        }
    }

    /// Register a new agent.
    pub fn register(&self, entry: AgentEntry) -> OpenFangResult<()> {
        if self.name_index.contains_key(&entry.name) {
            return Err(OpenFangError::AgentAlreadyExists(entry.name.clone()));
        }
        let id = entry.id;
        self.name_index.insert(entry.name.clone(), id);
        for tag in &entry.tags {
            self.tag_index.entry(tag.clone()).or_default().push(id);
        }
        self.agents.insert(id, entry);
        Ok(())
    }

    /// Get an agent entry by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentEntry> {
        self.agents.get(&id).map(|e| e.value().clone())
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        self.name_index
            .get(name)
            .and_then(|id| self.agents.get(id.value()).map(|e| e.value().clone()))
    }

    /// Update agent state.
    pub fn set_state(&self, id: AgentId, state: AgentState) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.state = state;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update agent operational mode.
    pub fn set_mode(&self, id: AgentId, mode: AgentMode) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.mode = mode;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Remove an agent from the registry.
    pub fn remove(&self, id: AgentId) -> OpenFangResult<AgentEntry> {
        let (_, entry) = self
            .agents
            .remove(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        self.name_index.remove(&entry.name);
        for tag in &entry.tags {
            if let Some(mut ids) = self.tag_index.get_mut(tag) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        Ok(entry)
    }

    /// List all agents.
    pub fn list(&self) -> Vec<AgentEntry> {
        self.agents.iter().map(|e| e.value().clone()).collect()
    }

    /// Add a child agent ID to a parent's children list.
    pub fn add_child(&self, parent_id: AgentId, child_id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&parent_id) {
            entry.children.push(child_id);
        }
    }

    /// Longest downward path from `id` to any descendant (0 if no children).
    pub fn spawn_subtree_height(&self, id: AgentId) -> u32 {
        let Some(entry) = self.get(id) else {
            return 0;
        };
        if entry.children.is_empty() {
            return 0;
        }
        entry
            .children
            .iter()
            .map(|c| 1 + self.spawn_subtree_height(*c))
            .max()
            .unwrap_or(0)
    }

    /// Max height from `parent` after adding one new direct child as a leaf (used for `max_spawn_depth` checks).
    pub fn spawn_height_if_add_leaf(&self, parent: AgentId) -> Option<u32> {
        let entry = self.get(parent)?;
        let mut m = 0u32;
        for c in &entry.children {
            m = m.max(1 + self.spawn_subtree_height(*c));
        }
        Some(m.max(1))
    }

    /// Count of registered agents.
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Update an agent's session ID (for session reset).
    pub fn update_session_id(
        &self,
        id: AgentId,
        new_session_id: openfang_types::agent::SessionId,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.session_id = new_session_id;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's workspace path.
    pub fn update_workspace(
        &self,
        id: AgentId,
        workspace: Option<std::path::PathBuf>,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.workspace = workspace;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's visual identity (emoji, avatar, color).
    pub fn update_identity(
        &self,
        id: AgentId,
        identity: openfang_types::agent::AgentIdentity,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.identity = identity;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model configuration.
    pub fn update_model(&self, id: AgentId, new_model: String) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model AND provider together.
    pub fn update_model_and_provider(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.manifest.model.provider = new_provider;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model, provider, and connection hints together.
    pub fn update_model_provider_config(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
        api_key_env: Option<String>,
        base_url: Option<String>,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.manifest.model.provider = new_provider;
        entry.manifest.model.api_key_env = api_key_env;
        entry.manifest.model.base_url = base_url;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's fallback model chain.
    pub fn update_fallback_models(
        &self,
        id: AgentId,
        fallback_models: Vec<openfang_types::agent::FallbackModel>,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.fallback_models = fallback_models;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update the autonomous loop step limit for an agent.
    pub fn update_max_iterations(&self, id: AgentId, max_iterations: u32) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        match entry.manifest.autonomous.as_mut() {
            Some(cfg) => cfg.max_iterations = max_iterations,
            None => {
                entry.manifest.autonomous = Some(openfang_types::agent::AutonomousConfig {
                    max_iterations,
                    ..Default::default()
                });
            }
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Toggle the optional ainl-runtime-engine shim for a single agent.
    pub fn update_ainl_runtime_engine(&self, id: AgentId, enabled: bool) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.ainl_runtime_engine = enabled;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Toggle per-agent native planner mode (manifest metadata `planner_mode`).
    ///
    /// - `true`  => `metadata.planner_mode = "on"`
    /// - `false` => `metadata.planner_mode = "off"`
    pub fn update_native_planner_mode(&self, id: AgentId, enabled: bool) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.metadata.insert(
            "planner_mode".to_string(),
            serde_json::Value::String(if enabled { "on" } else { "off" }.to_string()),
        );
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's skill allowlist.
    pub fn update_skills(&self, id: AgentId, skills: Vec<String>) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.skills = skills;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's MCP server allowlist.
    pub fn update_mcp_servers(&self, id: AgentId, servers: Vec<String>) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.mcp_servers = servers;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's tool allowlist and blocklist.
    pub fn update_tool_filters(
        &self,
        id: AgentId,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        if let Some(al) = allowlist {
            entry.manifest.tool_allowlist = al;
        }
        if let Some(bl) = blocklist {
            entry.manifest.tool_blocklist = bl;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Touch an agent — refresh last_active without changing any other state.
    /// Used by the agent loop to prevent heartbeat false-positives during long LLM calls.
    pub fn touch(&self, id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&id) {
            entry.last_active = chrono::Utc::now();
        }
    }

    /// Update an agent's system prompt (hot-swap, takes effect on next message).
    pub fn update_system_prompt(&self, id: AgentId, new_prompt: String) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.system_prompt = new_prompt;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's name (also updates the name index).
    ///
    /// When `~/.armaraos/agents/<old_name>/` exists on disk, it is renamed to
    /// `agents/<new_name>/` so `agent.toml` and workspace files stay with the agent.
    pub fn update_name(&self, id: AgentId, new_name: String) -> OpenFangResult<()> {
        if let Some(existing_id) = self.name_index.get(&new_name).as_deref().copied() {
            if existing_id != id {
                return Err(OpenFangError::AgentAlreadyExists(new_name));
            }
            // Same agent owns this name — no-op
            return Ok(());
        }
        let old_name = {
            let entry = self
                .agents
                .get(&id)
                .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
            entry.name.clone()
        };
        if old_name == new_name {
            return Ok(());
        }

        let old_path = self.agent_home.join("agents").join(&old_name);
        let new_path = self.agent_home.join("agents").join(&new_name);
        if old_path.is_dir() {
            if new_path.exists() {
                return Err(OpenFangError::AgentAlreadyExists(format!(
                    "agents/{new_name} already exists on disk"
                )));
            }
            std::fs::rename(&old_path, &new_path).map_err(|e| {
                OpenFangError::Config(format!(
                    "rename agent directory {} → {}: {e}",
                    old_name, new_name
                ))
            })?;
        }

        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.name = new_name.clone();
        entry.manifest.name = new_name.clone();
        entry.last_active = chrono::Utc::now();
        // Update name index
        drop(entry);
        self.name_index.remove(&old_name);
        self.name_index.insert(new_name, id);
        Ok(())
    }

    /// Replace the in-memory manifest and keep `entry.tags` / [`AgentManifest::tags`] / tag index aligned.
    pub fn replace_manifest(&self, id: AgentId, manifest: AgentManifest) -> OpenFangResult<()> {
        let old_tags = self
            .agents
            .get(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?
            .tags
            .clone();

        let new_tags = {
            let mut entry = self
                .agents
                .get_mut(&id)
                .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
            entry.manifest = manifest;
            let tags = entry.manifest.tags.clone();
            entry.tags = tags.clone();
            entry.last_active = chrono::Utc::now();
            tags
        };

        for t in &old_tags {
            if let Some(mut ids) = self.tag_index.get_mut(t) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        for t in &new_tags {
            self.tag_index.entry(t.clone()).or_default().push(id);
        }
        Ok(())
    }

    /// Update an agent's description.
    pub fn update_description(&self, id: AgentId, new_desc: String) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.manifest.description = new_desc;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's resource quota (budget limits).
    pub fn update_resources(
        &self,
        id: AgentId,
        hourly: Option<f64>,
        daily: Option<f64>,
        monthly: Option<f64>,
        tokens_per_hour: Option<u64>,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        if let Some(v) = hourly {
            entry.manifest.resources.max_cost_per_hour_usd = v;
        }
        if let Some(v) = daily {
            entry.manifest.resources.max_cost_per_day_usd = v;
        }
        if let Some(v) = monthly {
            entry.manifest.resources.max_cost_per_month_usd = v;
        }
        if let Some(v) = tokens_per_hour {
            entry.manifest.resources.max_llm_tokens_per_hour = v;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Mark an agent's onboarding as complete.
    pub fn mark_onboarding_complete(&self, id: AgentId) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        entry.onboarding_completed = true;
        entry.onboarding_completed_at = Some(chrono::Utc::now());
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Record a successful LLM / agent loop completion (dashboard telemetry).
    pub fn record_turn_success(
        &self,
        id: AgentId,
        latency_ms: Option<u64>,
        fallback_note: Option<String>,
        input_tokens: u64,
        output_tokens: u64,
    ) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        let now = chrono::Utc::now();
        entry.turn_stats.last_turn_at = Some(now);
        entry.turn_stats.last_success_at = Some(now);
        entry.turn_stats.last_latency_ms = latency_ms;
        entry.turn_stats.last_fallback_note = fallback_note;
        entry.turn_stats.last_input_tokens = input_tokens;
        entry.turn_stats.last_output_tokens = output_tokens;
        entry.turn_stats.turns_ok = entry.turn_stats.turns_ok.saturating_add(1);
        entry.last_active = now;
        Ok(())
    }

    /// Record a failed agent loop (dashboard error rate / last error).
    pub fn record_turn_failure(&self, id: AgentId, summary: String) -> OpenFangResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| OpenFangError::AgentNotFound(id.to_string()))?;
        let now = chrono::Utc::now();
        entry.turn_stats.last_turn_at = Some(now);
        entry.turn_stats.last_error_at = Some(now);
        entry.turn_stats.last_error_summary = Some(summary);
        entry.turn_stats.turns_err = entry.turn_stats.turns_err.saturating_add(1);
        entry.last_active = now;
        Ok(())
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use openfang_types::agent::*;
    use std::collections::HashMap;

    fn test_entry(name: &str) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            name: name.to_string(),
            manifest: AgentManifest {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                description: "test".to_string(),
                author: "test".to_string(),
                module: "test".to_string(),
                schedule: ScheduleMode::default(),
                model: ModelConfig::default(),
                fallback_models: vec![],
                resources: ResourceQuota::default(),
                priority: Priority::default(),
                capabilities: ManifestCapabilities::default(),
                profile: None,
                tools: HashMap::new(),
                skills: vec![],
                mcp_servers: vec![],
                metadata: HashMap::new(),
                tags: vec![],
                routing: None,
                autonomous: None,
                pinned_model: None,
                workspace: None,
                generate_identity_files: true,
                exec_policy: None,
                tool_allowlist: vec![],
                tool_blocklist: vec![],
                ainl_runtime_engine: false,
            },
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            turn_stats: Default::default(),
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = AgentRegistry::new();
        let entry = test_entry("test-agent");
        let id = entry.id;
        registry.register(entry).unwrap();
        assert!(registry.get(id).is_some());
    }

    #[test]
    fn test_find_by_name() {
        let registry = AgentRegistry::new();
        let entry = test_entry("my-agent");
        registry.register(entry).unwrap();
        assert!(registry.find_by_name("my-agent").is_some());
    }

    #[test]
    fn test_duplicate_name() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("dup")).unwrap();
        assert!(registry.register(test_entry("dup")).is_err());
    }

    #[test]
    fn test_remove() {
        let registry = AgentRegistry::new();
        let entry = test_entry("removable");
        let id = entry.id;
        registry.register(entry).unwrap();
        registry.remove(id).unwrap();
        assert!(registry.get(id).is_none());
    }

    #[test]
    fn test_spawn_subtree_height_chain() {
        let registry = AgentRegistry::new();
        let mut p = test_entry("p");
        let pid = p.id;
        let mut a = test_entry("a");
        let aid = a.id;
        let mut b = test_entry("b");
        let bid = b.id;
        p.children = vec![aid];
        a.parent = Some(pid);
        a.children = vec![bid];
        b.parent = Some(aid);
        registry.register(p).unwrap();
        registry.register(a).unwrap();
        registry.register(b).unwrap();
        assert_eq!(registry.spawn_subtree_height(pid), 2);
        assert_eq!(registry.spawn_height_if_add_leaf(pid).unwrap(), 2);
        assert_eq!(registry.spawn_subtree_height(aid), 1);
        assert_eq!(registry.spawn_height_if_add_leaf(aid).unwrap(), 1);
    }

    #[test]
    fn test_replace_manifest_updates_tags_and_index() {
        let registry = AgentRegistry::new();
        let mut e = test_entry("tagged");
        e.manifest.tags = vec!["a".into(), "b".into()];
        e.tags = e.manifest.tags.clone();
        let id = e.id;
        registry.register(e).unwrap();

        let mut m = registry.get(id).unwrap().manifest;
        m.tags = vec!["b".into(), "c".into()];
        registry.replace_manifest(id, m).unwrap();

        let updated = registry.get(id).unwrap();
        assert_eq!(updated.tags, vec!["b".to_string(), "c".to_string()]);
        assert_eq!(updated.manifest.tags, updated.tags);
    }

    /// `update_name` renames `agents/<old>` → `agents/<new>` under `ARMARAOS_HOME` when present.
    #[serial_test::serial]
    #[test]
    fn test_update_name_renames_agent_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ARMARAOS_HOME", dir.path().as_os_str());
        let agents = dir.path().join("agents");
        std::fs::create_dir_all(agents.join("alpha")).unwrap();
        std::fs::write(agents.join("alpha").join("marker.txt"), b"x").unwrap();

        let registry = AgentRegistry::new();
        let mut e = test_entry("alpha");
        e.manifest.name = "alpha".to_string();
        let id = e.id;
        registry.register(e).unwrap();

        registry.update_name(id, "beta".to_string()).unwrap();

        assert!(agents.join("beta").is_dir());
        assert!(agents.join("beta").join("marker.txt").exists());
        assert!(!agents.join("alpha").exists());
        assert_eq!(registry.get(id).unwrap().name, "beta");

        std::env::remove_var("ARMARAOS_HOME");
    }
}
