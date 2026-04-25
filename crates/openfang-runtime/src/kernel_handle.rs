//! Trait abstraction for kernel operations needed by the agent runtime.
//!
//! This trait allows `openfang-runtime` to call back into the kernel for
//! inter-agent operations (spawn, send, list, kill) without creating
//! a circular dependency. The kernel implements this trait and passes
//! it into the agent loop.

use async_trait::async_trait;
use openfang_types::agent::AgentId;
use openfang_types::capability::Capability;
use openfang_types::event::GraphMemoryWriteProvenance;
use openfang_types::orchestration::{
    DelegateSelectionOptions, OrchestrationContext, SelectionStrategy,
};
use openfang_types::task_queue::TaskClaimStrategy;

/// Agent info returned by list and discovery operations.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model_provider: String,
    pub model_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
}

/// Handle to kernel operations, passed into the agent loop so agents
/// can interact with each other via tools.
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait KernelHandle: Send + Sync {
    /// Spawn a new agent from a TOML manifest string.
    /// `parent_id` is the UUID string of the spawning agent (for lineage tracking).
    /// Returns (agent_id, agent_name) on success.
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String>;

    /// Send a message to another agent and get the response.
    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, String>;

    /// Publish a short preview of an `agent_send` so dashboards can show who messaged whom.
    async fn notify_inter_agent_message(
        &self,
        from_agent_id: &str,
        to_agent_id: &str,
        message_preview: &str,
    ) -> Result<(), String> {
        let _ = (from_agent_id, to_agent_id, message_preview);
        Ok(())
    }

    /// List all running agents.
    fn list_agents(&self) -> Vec<AgentInfo>;

    /// Kill an agent by ID.
    fn kill_agent(&self, agent_id: &str) -> Result<(), String>;

    /// Store a value in shared memory (cross-agent accessible).
    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String>;

    /// Recall a value from shared memory.
    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String>;

    /// List all keys (and values) stored in shared memory. Optional prefix filter.
    fn memory_list(&self, prefix: Option<&str>)
        -> Result<Vec<(String, serde_json::Value)>, String>;

    /// Find agents by query (matches on name substring, tag, or tool name; case-insensitive).
    fn find_agents(&self, query: &str) -> Vec<AgentInfo>;

    /// Post a task to the shared task queue. Returns the task ID.
    ///
    /// `orchestration_meta` is merged into the task `payload` JSON (e.g. `orchestration.trace_id`
    /// for sticky routing on [`Self::task_claim`]).
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
        orchestration_meta: Option<serde_json::Value>,
        priority: i64,
    ) -> Result<String, String>;

    /// Claim the next available task. With `prefer_orchestration_trace_id`, tries tasks posted
    /// for that orchestration trace first (see task payload `orchestration.trace_id`).
    async fn task_claim(
        &self,
        agent_id: &str,
        prefer_orchestration_trace_id: Option<&str>,
        strategy: TaskClaimStrategy,
    ) -> Result<Option<serde_json::Value>, String>;

    /// Mark a task as completed with a result string.
    async fn task_complete(&self, task_id: &str, result: &str) -> Result<(), String>;

    /// List tasks, optionally filtered by status.
    async fn task_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, String>;

    /// Publish a custom event that can trigger proactive agents.
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String>;

    /// Notify dashboards that AINL graph memory was updated for `agent_id`.
    async fn notify_graph_memory_write(
        &self,
        agent_id: &str,
        kind: &str,
        provenance: Option<GraphMemoryWriteProvenance>,
    ) -> Result<(), String> {
        let _ = (agent_id, kind, provenance);
        Ok(())
    }

    /// Add an entity to the knowledge graph.
    async fn knowledge_add_entity(
        &self,
        entity: openfang_types::memory::Entity,
    ) -> Result<String, String>;

    /// Add a relation to the knowledge graph.
    async fn knowledge_add_relation(
        &self,
        relation: openfang_types::memory::Relation,
    ) -> Result<String, String>;

    /// Query the knowledge graph with a pattern.
    async fn knowledge_query(
        &self,
        pattern: openfang_types::memory::GraphPattern,
    ) -> Result<Vec<openfang_types::memory::GraphMatch>, String>;

    /// Create a cron job for the calling agent.
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let _ = (agent_id, job_json);
        Err("Cron scheduler not available".to_string())
    }

    /// List cron jobs for the calling agent.
    async fn cron_list(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let _ = agent_id;
        Err("Cron scheduler not available".to_string())
    }

    /// Cancel a cron job by ID.
    async fn cron_cancel(&self, job_id: &str) -> Result<(), String> {
        let _ = job_id;
        Err("Cron scheduler not available".to_string())
    }

    /// Human-readable list of registered outbound channel adapters (names match `channel_send` / cron `delivery`).
    fn list_channels_summary(&self) -> String {
        "Channel listing not available.".to_string()
    }

    /// Check if a tool requires approval based on current policy.
    fn requires_approval(&self, tool_name: &str) -> bool {
        let _ = tool_name;
        false
    }

    /// Request approval for a tool execution. Blocks until approved/denied/timed out.
    /// Returns `Ok(true)` if approved, `Ok(false)` if denied or timed out.
    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
    ) -> Result<bool, String> {
        let _ = (agent_id, tool_name, action_summary);
        Ok(true) // Default: auto-approve
    }

    /// List available Hands and their activation status.
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, String> {
        Err("Hands system not available".to_string())
    }

    /// Install a Hand from TOML content.
    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = (toml_content, skill_content);
        Err("Hands system not available".to_string())
    }

    /// Activate a Hand — spawns a specialized autonomous agent.
    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let _ = (hand_id, config);
        Err("Hands system not available".to_string())
    }

    /// Check the status and dashboard metrics of an active Hand.
    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, String> {
        let _ = hand_id;
        Err("Hands system not available".to_string())
    }

    /// Deactivate a running Hand and stop its agent.
    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), String> {
        let _ = instance_id;
        Err("Hands system not available".to_string())
    }

    /// List discovered external A2A agents as (name, url) pairs.
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// Get the URL of a discovered external A2A agent by name.
    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }

    /// Send a message to a user on a named channel adapter (e.g., "email", "telegram").
    /// When `thread_id` is provided, the message is sent as a thread reply.
    /// Returns a confirmation string on success.
    /// Get the default recipient for a channel (e.g. default_chat_id for Telegram).
    async fn get_channel_default_recipient(&self, channel: &str) -> Option<String> {
        let _ = channel;
        None
    }

    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, message, thread_id);
        Err("Channel send not available".to_string())
    }

    /// Send media content (image/file) to a user on a named channel adapter.
    /// `media_type` is "image" or "file", `media_url` is the URL, `caption` is optional text.
    /// When `thread_id` is provided, the media is sent as a thread reply.
    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (
            channel, recipient, media_type, media_url, caption, filename, thread_id,
        );
        Err("Channel media send not available".to_string())
    }

    /// Send a local file (raw bytes) to a user on a named channel adapter.
    /// Used by the `channel_send` tool when `file_path` is provided.
    /// When `thread_id` is provided, the file is sent as a thread reply.
    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, data, filename, mime_type, thread_id);
        Err("Channel file data send not available".to_string())
    }

    /// Refresh an agent's last_active timestamp without changing any other state.
    /// Called by the agent loop before long LLM calls to prevent heartbeat false-positives.
    fn touch_agent(&self, agent_id: &str) {
        let _ = agent_id;
    }

    /// Live `[llm]` HTTP timeouts (and related settings) when the host tracks them.
    /// `None` means use built-in defaults for ad-hoc drivers (e.g. tests without a kernel).
    fn live_llm_config(&self) -> Option<openfang_types::config::LlmConfig> {
        None
    }

    /// Resolve the configured base URL for a given LLM provider.
    ///
    /// Looks up `[provider_urls]` from `config.toml` first, then the runtime model
    /// catalog (updated by dashboard `set_provider_url` etc.). Returns `None` when
    /// the provider is built-in / hardcoded with no override.
    ///
    /// Used by the **planner-mode autowire** in `agent_loop` to detect when an agent
    /// targets the `ainl-inference-server` (so `ARMARA_NATIVE_INFER_URL` doesn't have
    /// to be set by hand). Default impl returns `None` so non-kernel hosts (tests,
    /// `ainl-runtime` embedders) behave identically to today.
    fn lookup_provider_url(&self, provider: &str) -> Option<String> {
        let _ = provider;
        None
    }

    /// Resolve an LLM driver via the host `LlmDriverFactory` (instrumented, LRU when shared).
    ///
    /// Used for rare fallback paths (OpenRouter free-tier, manifest `fallback_models`) so they
    /// contribute to the same `llm_*` metrics as the primary driver. Default: unavailable — callers
    /// use `create_driver` with `[llm]` HTTP timeouts from [`live_llm_config`](Self::live_llm_config).
    fn get_llm_driver(
        &self,
        config: &crate::llm_driver::DriverConfig,
    ) -> Result<std::sync::Arc<dyn crate::llm_driver::LlmDriver>, String> {
        let _ = config;
        Err("LLM factory not available".to_string())
    }

    /// Spawn an agent with capability inheritance enforcement.
    /// `parent_caps` are the parent's granted capabilities. The kernel MUST verify
    /// that every capability in the child manifest is covered by `parent_caps`.
    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[openfang_types::capability::Capability],
    ) -> Result<(String, String), String> {
        // Default: delegate to spawn_agent (no enforcement)
        // The kernel MUST override this with real enforcement
        let _ = parent_caps;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Resolve an agent reference (UUID or registered name) to an [`AgentId`].
    ///
    /// Default implementation accepts UUID strings only; the kernel overrides with registry lookup.
    fn resolve_agent_id(&self, agent_id: &str) -> Result<AgentId, String> {
        agent_id
            .parse()
            .map_err(|_| "Agent selection: use a full agent UUID with this host".to_string())
    }

    /// Send a message with optional orchestration context (backward compatible with [`send_to_agent`](Self::send_to_agent)).
    async fn send_to_agent_with_context(
        &self,
        agent_id: &str,
        message: &str,
        orchestration_ctx: Option<OrchestrationContext>,
    ) -> Result<String, String> {
        let _ = orchestration_ctx;
        self.send_to_agent(agent_id, message).await
    }

    /// Spawn with optional orchestration context for the child agent's first turn.
    async fn spawn_agent_with_context(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        orchestration_ctx: Option<OrchestrationContext>,
    ) -> Result<(String, String), String> {
        let _ = orchestration_ctx;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Agents whose manifests grant **all** of the required [`Capability`] values
    /// (kernel expands `[capabilities]` / tools the same way as runtime enforcement).
    fn find_by_capabilities(
        &self,
        required_caps: &[Capability],
        preferred_tags: &[String],
        exclude_agents: &[AgentId],
    ) -> Vec<AgentInfo> {
        let _ = (required_caps, preferred_tags, exclude_agents);
        vec![]
    }

    /// Pick one agent for a task using `strategy`.
    ///
    /// `task_description` ranks candidates (with `preferred_tags`) for [`SelectionStrategy::BestMatch`]
    /// and as a tie-breaker for other strategies.
    async fn select_agent_for_task(
        &self,
        task_description: &str,
        required_caps: &[Capability],
        preferred_tags: &[String],
        selection_strategy: SelectionStrategy,
        options: DelegateSelectionOptions,
    ) -> Result<AgentId, String> {
        let _ = (
            task_description,
            required_caps,
            preferred_tags,
            selection_strategy,
            options,
        );
        Err("Agent selection not available".to_string())
    }

    /// Configured `[[agent_pools]]` entries with live worker counts.
    fn list_agent_pools(&self) -> Vec<serde_json::Value> {
        vec![]
    }

    /// Spawn another worker from a manifest pool (up to `max_instances`).
    async fn spawn_agent_pool_worker(
        &self,
        pool_name: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        let _ = (pool_name, parent_id);
        Err("Agent pool operations are not available on this host".to_string())
    }

    /// Record an orchestration trace event (bounded ring buffer on the real kernel; default no-op).
    fn record_orchestration_trace(
        &self,
        _event: openfang_types::orchestration_trace::OrchestrationTraceEvent,
    ) {
    }

    /// Queue orchestration context for the agent's next LLM turn (picked up like `spawn_agent_with_context`).
    ///
    /// Used when [`crate::tool_runner::tool_task_claim`] reconstructs context from task payload.
    fn set_pending_orchestration_ctx(
        &self,
        agent_id: &str,
        ctx: OrchestrationContext,
    ) -> Result<(), String> {
        let _ = (agent_id, ctx);
        Err("set_pending_orchestration_ctx not available".to_string())
    }

    /// Best-effort audit trail for `shell_exec` argv guards (`path_guard`, `pid_guard`).
    ///
    /// `guard_kind` is a short stable token (e.g. `path_enforce`, `path_warn`, `pid_enforce`).
    /// `outcome` is a one-line summary (`denied`, `warn_only`, etc.).
    fn record_shell_guard_event(
        &self,
        agent_id: Option<&str>,
        guard_kind: &str,
        detail: &str,
        outcome: &str,
    ) {
        let _ = (agent_id, guard_kind, detail, outcome);
    }
}
