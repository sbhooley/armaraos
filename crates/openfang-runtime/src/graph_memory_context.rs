use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Mutex as StdMutex, OnceLock};

use ainl_contracts::{ContextFreshness, ProcedureArtifact};
use ainl_failure_learning::should_emit_failure_suggestion;
use ainl_memory::{recall_task_scoped_episodes, AinlMemoryNode, AinlNodeKind, AinlNodeType};
use openfang_types::agent::AgentManifest;
use tracing::debug;

use crate::graph_memory_writer::GraphMemoryWriter;

static INJECTED_EPISODIC_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_SEMANTIC_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_CONFLICT_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_PROCEDURAL_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_PATTERN_CANDIDATE_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_FAILURE_RECALL_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_TRAJECTORY_RECAP_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_SUGGESTED_NEXT_TOTAL: AtomicU64 = AtomicU64::new(0);
static TRUNCATION_HITS_TOTAL: AtomicU64 = AtomicU64::new(0);
static SKIPPED_LOW_QUALITY_TOTAL: AtomicU64 = AtomicU64::new(0);
static TEMP_MODE_SUPPRESSED_READS_TOTAL: AtomicU64 = AtomicU64::new(0);
static TEMP_MODE_SUPPRESSED_WRITES_TOTAL: AtomicU64 = AtomicU64::new(0);
static AB_CONTROL_TURNS_TOTAL: AtomicU64 = AtomicU64::new(0);
static ROLLOUT_SUPPRESSED_READS_TOTAL: AtomicU64 = AtomicU64::new(0);
static ROLLOUT_SUPPRESSED_WRITES_TOTAL: AtomicU64 = AtomicU64::new(0);
static INJECTED_LINES_TOTAL: AtomicU64 = AtomicU64::new(0);
static PROVENANCE_LINES_TOTAL: AtomicU64 = AtomicU64::new(0);
/// `KernelHandle::notify_graph_memory_write` succeeded (dashboard / SSE `GraphMemoryWrite`).
static GRAPH_MEMORY_KERNEL_NOTIFY_OK_TOTAL: AtomicU64 = AtomicU64::new(0);
/// `KernelHandle::notify_graph_memory_write` failed (timeline may miss writes; see daemon logs).
static GRAPH_MEMORY_KERNEL_NOTIFY_ERR_TOTAL: AtomicU64 = AtomicU64::new(0);

/// Increment when the kernel publishes `GraphMemoryWrite` to the event bus (SSE path).
pub fn record_graph_memory_kernel_notify_ok() {
    GRAPH_MEMORY_KERNEL_NOTIFY_OK_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
}

/// Increment when notify failed (e.g. agent id resolution); SSE will not show this write.
pub fn record_graph_memory_kernel_notify_err() {
    GRAPH_MEMORY_KERNEL_NOTIFY_ERR_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
}

fn selection_debug_snapshot() -> &'static StdMutex<Vec<serde_json::Value>> {
    static SNAPSHOT: OnceLock<StdMutex<Vec<serde_json::Value>>> = OnceLock::new();
    SNAPSHOT.get_or_init(|| StdMutex::new(Vec::new()))
}

#[derive(Debug, Clone)]
pub struct MemoryContextPolicy {
    pub enabled: bool,
    pub temporary_mode: bool,
    pub include_provenance: bool,
    pub include_episodic_hints: bool,
    pub include_semantic_facts: bool,
    pub include_conflicts: bool,
    pub include_procedural_hints: bool,
    /// Non-promoted tool-sequence procedurals (`!prompt_eligible`) as `## SuggestedPatternCandidates`.
    pub include_suggested_pattern_candidates: bool,
    /// When true (and learning policy allows failure stack), recent matching `failure` nodes
    /// are injected into the graph-memory prompt block from FTS over the user message.
    pub include_failure_recall: bool,
    pub max_episodic_lines: usize,
    pub max_semantic_lines: usize,
    pub max_conflict_lines: usize,
    pub max_procedural_lines: usize,
    pub max_pattern_candidate_lines: usize,
    pub max_episodic_chars: usize,
    pub max_semantic_chars: usize,
    pub max_conflict_chars: usize,
    pub max_procedural_chars: usize,
    pub max_pattern_candidate_chars: usize,
    pub max_failure_recall_lines: usize,
    pub max_failure_recall_chars: usize,
    pub recall_window_secs: i64,
    pub semantic_confidence_floor: f32,
    pub contradiction_confidence_floor: f32,
    pub semantic_ttl_secs: i64,
    pub ab_variant: String,
    pub rollout_mode: String,
    pub internal_agent: bool,
    pub opt_in_agent: bool,
}

impl Default for MemoryContextPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            temporary_mode: false,
            include_provenance: true,
            include_episodic_hints: true,
            include_semantic_facts: true,
            include_conflicts: true,
            include_procedural_hints: true,
            include_suggested_pattern_candidates: true,
            include_failure_recall: true,
            max_episodic_lines: 4,
            max_semantic_lines: 5,
            max_conflict_lines: 3,
            max_procedural_lines: 3,
            max_pattern_candidate_lines: 2,
            max_episodic_chars: 700,
            max_semantic_chars: 800,
            max_conflict_chars: 420,
            max_procedural_chars: 420,
            max_pattern_candidate_chars: 320,
            max_failure_recall_lines: 5,
            max_failure_recall_chars: 600,
            recall_window_secs: 60 * 60 * 24 * 30,
            semantic_confidence_floor: 0.55,
            contradiction_confidence_floor: 0.70,
            semantic_ttl_secs: 60 * 60 * 24 * 90,
            ab_variant: "default".to_string(),
            rollout_mode: "default".to_string(),
            internal_agent: false,
            opt_in_agent: false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct PromptMemoryContext {
    pub episodic_lines: Vec<String>,
    pub semantic_lines: Vec<String>,
    pub conflict_lines: Vec<String>,
    pub procedural_lines: Vec<String>,
    pub failure_recall_lines: Vec<String>,
    /// Recent `ainl_trajectories` rows (opt-in: `AINL_MEMORY_INCLUDE_TRAJECTORY_RECAP`).
    pub trajectory_recap_lines: Vec<String>,
    /// Single-line “do this next” hints (opt-in: `AINL_MEMORY_INCLUDE_SUGGESTED_NEXT`).
    pub suggested_next_lines: Vec<String>,
    /// Not-yet-promoted tool-sequence patterns (Phase 3 candidate pool).
    pub pattern_candidate_lines: Vec<String>,
    pub skipped_low_quality: usize,
    pub truncation_hits: usize,
    pub provenance_lines: usize,
    pub selection_debug: Vec<serde_json::Value>,
    pub selected_procedure_ids: Vec<String>,
}

impl PromptMemoryContext {
    pub fn is_empty(&self) -> bool {
        self.episodic_lines.is_empty()
            && self.semantic_lines.is_empty()
            && self.conflict_lines.is_empty()
            && self.procedural_lines.is_empty()
            && self.pattern_candidate_lines.is_empty()
            && self.failure_recall_lines.is_empty()
            && self.trajectory_recap_lines.is_empty()
            && self.suggested_next_lines.is_empty()
    }

    pub fn to_prompt_block(&self) -> String {
        crate::prompt_builder::build_graph_memory_sections(
            &self.episodic_lines,
            &self.failure_recall_lines,
            &self.trajectory_recap_lines,
            &self.semantic_lines,
            &self.conflict_lines,
            &self.pattern_candidate_lines,
            &self.procedural_lines,
            &self.suggested_next_lines,
        )
    }

    /// One [`ainl_context_compiler::Segment::memory_block`] per non-empty section (same content as
    /// [`Self::to_prompt_block`], split for Phase 6 “compiler as root” / budget-aware scoring).
    /// Labels match `ainl_contracts::context_compiler::segment_kind::MEMORY_BLOCK` usage in hosts.
    pub fn to_memory_block_segments(&self) -> Vec<ainl_context_compiler::Segment> {
        use ainl_context_compiler::Segment;
        let mut out = Vec::new();
        if !self.episodic_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_recent_attempts",
                memory_section_str("## RecentAttempts", &self.episodic_lines),
            ));
        }
        if !self.failure_recall_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_failure_recall",
                memory_section_str("## FailureRecall", &self.failure_recall_lines),
            ));
        }
        if !self.trajectory_recap_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_trajectory_recap",
                memory_section_str("## TrajectoryRecap", &self.trajectory_recap_lines),
            ));
        }
        if !self.semantic_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_known_facts",
                memory_section_str("## KnownFacts", &self.semantic_lines),
            ));
        }
        if !self.conflict_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_known_conflicts",
                memory_section_str("## KnownConflicts", &self.conflict_lines),
            ));
        }
        if !self.pattern_candidate_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_suggested_pattern_candidates",
                memory_section_str(
                    "## SuggestedPatternCandidates",
                    &self.pattern_candidate_lines,
                ),
            ));
        }
        if !self.procedural_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_suggested_procedure",
                memory_section_str("## SuggestedProcedure", &self.procedural_lines),
            ));
        }
        if !self.suggested_next_lines.is_empty() {
            out.push(Segment::memory_block(
                "graph_suggested_next",
                memory_section_str("## SuggestedNext", &self.suggested_next_lines),
            ));
        }
        out
    }
}

fn memory_section_str(heading: &str, lines: &[String]) -> String {
    let mut s = String::new();
    s.push_str(heading);
    s.push('\n');
    for line in lines {
        s.push_str("- ");
        s.push_str(line);
        s.push('\n');
    }
    s
}

fn memory_include_trajectory_recap_env() -> bool {
    match std::env::var("AINL_MEMORY_INCLUDE_TRAJECTORY_RECAP") {
        Ok(s) => {
            let t = s.trim().to_ascii_lowercase();
            !matches!(t.as_str(), "" | "0" | "false" | "no" | "off")
        }
        Err(_) => false,
    }
}

fn memory_include_suggested_next_env() -> bool {
    match std::env::var("AINL_MEMORY_INCLUDE_SUGGESTED_NEXT") {
        Ok(s) => {
            let t = s.trim().to_ascii_lowercase();
            !matches!(t.as_str(), "" | "0" | "false" | "no" | "off")
        }
        Err(_) => false,
    }
}

pub fn memory_context_metrics() -> serde_json::Value {
    let injected_lines_total = INJECTED_LINES_TOTAL.load(AtomicOrdering::Relaxed);
    let provenance_lines_total = PROVENANCE_LINES_TOTAL.load(AtomicOrdering::Relaxed);
    let provenance_coverage_ratio = if injected_lines_total == 0 {
        1.0
    } else {
        provenance_lines_total as f64 / injected_lines_total as f64
    };
    let provenance_coverage_floor = std::env::var("AINL_MEMORY_PROVENANCE_COVERAGE_FLOOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| (0.0..=1.0).contains(v))
        .unwrap_or(0.95);
    let provenance_coverage_min_lines = std::env::var("AINL_MEMORY_PROVENANCE_MIN_LINES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);
    let provenance_gate_pass = injected_lines_total < provenance_coverage_min_lines
        || provenance_coverage_ratio >= provenance_coverage_floor;
    let semantic_total = INJECTED_SEMANTIC_TOTAL.load(AtomicOrdering::Relaxed);
    let conflict_total = INJECTED_CONFLICT_TOTAL.load(AtomicOrdering::Relaxed);
    let conflict_ratio = if semantic_total == 0 {
        0.0
    } else {
        conflict_total as f64 / semantic_total as f64
    };
    let conflict_ratio_max = std::env::var("AINL_MEMORY_CONFLICT_RATIO_MAX")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v >= 0.0)
        .unwrap_or(0.75);
    let conflict_ratio_min_semantic = std::env::var("AINL_MEMORY_CONFLICT_RATIO_MIN_SEMANTIC")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(20);
    let contradiction_gate_pass =
        semantic_total < conflict_ratio_min_semantic || conflict_ratio <= conflict_ratio_max;
    serde_json::json!({
        "injected_episodic_total": INJECTED_EPISODIC_TOTAL.load(AtomicOrdering::Relaxed),
        "injected_semantic_total": semantic_total,
        "injected_conflict_total": conflict_total,
        "injected_procedural_total": INJECTED_PROCEDURAL_TOTAL.load(AtomicOrdering::Relaxed),
        "injected_pattern_candidate_total": INJECTED_PATTERN_CANDIDATE_TOTAL
            .load(AtomicOrdering::Relaxed),
        "injected_failure_recall_total": INJECTED_FAILURE_RECALL_TOTAL.load(AtomicOrdering::Relaxed),
        "injected_trajectory_recap_total": INJECTED_TRAJECTORY_RECAP_TOTAL
            .load(AtomicOrdering::Relaxed),
        "injected_suggested_next_total": INJECTED_SUGGESTED_NEXT_TOTAL.load(AtomicOrdering::Relaxed),
        "truncation_hits_total": TRUNCATION_HITS_TOTAL.load(AtomicOrdering::Relaxed),
        "skipped_low_quality_total": SKIPPED_LOW_QUALITY_TOTAL.load(AtomicOrdering::Relaxed),
        "temp_mode_suppressed_reads_total": TEMP_MODE_SUPPRESSED_READS_TOTAL.load(AtomicOrdering::Relaxed),
        "temp_mode_suppressed_writes_total": TEMP_MODE_SUPPRESSED_WRITES_TOTAL.load(AtomicOrdering::Relaxed),
        "ab_control_turns_total": AB_CONTROL_TURNS_TOTAL.load(AtomicOrdering::Relaxed),
        "rollout_suppressed_reads_total": ROLLOUT_SUPPRESSED_READS_TOTAL.load(AtomicOrdering::Relaxed),
        "rollout_suppressed_writes_total": ROLLOUT_SUPPRESSED_WRITES_TOTAL.load(AtomicOrdering::Relaxed),
        "graph_memory_kernel_notify_ok_total": GRAPH_MEMORY_KERNEL_NOTIFY_OK_TOTAL.load(AtomicOrdering::Relaxed),
        "graph_memory_kernel_notify_err_total": GRAPH_MEMORY_KERNEL_NOTIFY_ERR_TOTAL.load(AtomicOrdering::Relaxed),
        "injected_lines_total": injected_lines_total,
        "provenance_lines_total": provenance_lines_total,
        "provenance_coverage_ratio": provenance_coverage_ratio,
        "provenance_coverage_floor": provenance_coverage_floor,
        "provenance_coverage_min_lines": provenance_coverage_min_lines,
        "provenance_gate_pass": provenance_gate_pass,
        "conflict_ratio": conflict_ratio,
        "conflict_ratio_max": conflict_ratio_max,
        "conflict_ratio_min_semantic": conflict_ratio_min_semantic,
        "contradiction_gate_pass": contradiction_gate_pass,
    })
}

pub fn latest_selection_debug(limit: usize) -> Vec<serde_json::Value> {
    let Ok(guard) = selection_debug_snapshot().lock() else {
        return Vec::new();
    };
    guard.iter().take(limit.max(1)).cloned().collect()
}

impl MemoryContextPolicy {
    pub fn from_manifest(manifest: &AgentManifest) -> Self {
        Self::from_manifest_for_agent(manifest, None)
    }

    pub fn from_manifest_for_agent(manifest: &AgentManifest, agent_id: Option<&str>) -> Self {
        let mut policy = MemoryContextPolicy {
            enabled: metadata_bool(&manifest.metadata, "memory_enabled", true),
            temporary_mode: metadata_bool(&manifest.metadata, "memory_temporary_mode", false),
            include_provenance: metadata_bool(
                &manifest.metadata,
                "memory_include_provenance",
                true,
            ),
            include_episodic_hints: metadata_bool(
                &manifest.metadata,
                "memory_include_episodic_hints",
                true,
            ),
            include_semantic_facts: metadata_bool(
                &manifest.metadata,
                "memory_include_semantic_facts",
                true,
            ),
            include_conflicts: metadata_bool(&manifest.metadata, "memory_include_conflicts", true),
            include_procedural_hints: metadata_bool(
                &manifest.metadata,
                "memory_include_procedural_hints",
                true,
            ),
            include_suggested_pattern_candidates: metadata_bool(
                &manifest.metadata,
                "memory_include_suggested_pattern_candidates",
                true,
            ),
            include_failure_recall: metadata_bool(
                &manifest.metadata,
                "memory_include_failure_recall",
                true,
            ),
            ..Default::default()
        };

        if let Ok(v) = std::env::var("AINL_MEMORY_ENABLED") {
            policy.enabled = parse_bool_with_default(Some(v.as_str()), policy.enabled);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_TEMPORARY_MODE") {
            policy.temporary_mode =
                parse_bool_with_default(Some(v.as_str()), policy.temporary_mode);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_PROCEDURAL_HINTS") {
            policy.include_procedural_hints =
                parse_bool_with_default(Some(v.as_str()), policy.include_procedural_hints);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_SUGGESTED_PATTERN_CANDIDATES") {
            policy.include_suggested_pattern_candidates = parse_bool_with_default(
                Some(v.as_str()),
                policy.include_suggested_pattern_candidates,
            );
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_EPISODIC_HINTS") {
            policy.include_episodic_hints =
                parse_bool_with_default(Some(v.as_str()), policy.include_episodic_hints);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_SEMANTIC_FACTS") {
            policy.include_semantic_facts =
                parse_bool_with_default(Some(v.as_str()), policy.include_semantic_facts);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_CONFLICTS") {
            policy.include_conflicts =
                parse_bool_with_default(Some(v.as_str()), policy.include_conflicts);
        }
        if let Ok(v) = std::env::var("AINL_MEMORY_INCLUDE_FAILURE_RECALL") {
            policy.include_failure_recall =
                parse_bool_with_default(Some(v.as_str()), policy.include_failure_recall);
        }
        policy.ab_variant = manifest
            .metadata
            .get("memory_ab_variant")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .trim()
            .to_ascii_lowercase();
        if let Ok(v) = std::env::var("AINL_MEMORY_AB_VARIANT") {
            policy.ab_variant = v.trim().to_ascii_lowercase();
        }
        policy.rollout_mode = manifest
            .metadata
            .get("memory_rollout")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .trim()
            .to_ascii_lowercase();
        if let Ok(v) = std::env::var("AINL_MEMORY_ROLLOUT") {
            policy.rollout_mode = v.trim().to_ascii_lowercase();
        }
        policy.internal_agent = metadata_bool(&manifest.metadata, "memory_internal_agent", false);
        policy.opt_in_agent = metadata_bool(&manifest.metadata, "memory_opt_in", false);
        if let Some(agent_id) = agent_id {
            policy.apply_control_plane_overrides(agent_id);
        }

        policy
    }

    pub fn allow_reads(&self) -> bool {
        if self.ab_variant == "control" {
            AB_CONTROL_TURNS_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
            return false;
        }
        if !self.rollout_allows_reads() {
            ROLLOUT_SUPPRESSED_READS_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
            return false;
        }
        self.enabled && !self.temporary_mode
    }

    pub fn allow_writes(&self) -> bool {
        if !self.rollout_allows_reads() {
            ROLLOUT_SUPPRESSED_WRITES_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
            return false;
        }
        self.enabled && !self.temporary_mode
    }

    fn rollout_allows_reads(&self) -> bool {
        match self.rollout_mode.as_str() {
            "off" => false,
            "internal" => self.internal_agent,
            "opt_in" => self.opt_in_agent || self.internal_agent,
            _ => true,
        }
    }

    fn apply_control_plane_overrides(&mut self, agent_id: &str) {
        let home = openfang_types::config::openfang_home_dir();
        let controls = home
            .join("agents")
            .join(agent_id)
            .join(".graph-memory")
            .join("controls.json");
        let Ok(raw) = std::fs::read_to_string(&controls) else {
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return;
        };
        if let Some(enabled) = v.get("memory_enabled").and_then(|x| x.as_bool()) {
            self.enabled = enabled;
        }
        if let Some(temporary) = v.get("temporary_mode").and_then(|x| x.as_bool()) {
            self.temporary_mode = temporary;
        }
        if let Some(include_episodic_hints) =
            v.get("include_episodic_hints").and_then(|x| x.as_bool())
        {
            self.include_episodic_hints = include_episodic_hints;
        }
        if let Some(include_semantic_facts) =
            v.get("include_semantic_facts").and_then(|x| x.as_bool())
        {
            self.include_semantic_facts = include_semantic_facts;
        }
        if let Some(include_conflicts) = v.get("include_conflicts").and_then(|x| x.as_bool()) {
            self.include_conflicts = include_conflicts;
        }
        if let Some(include_procedural_hints) =
            v.get("include_procedural_hints").and_then(|x| x.as_bool())
        {
            self.include_procedural_hints = include_procedural_hints;
        }
        if let Some(b) = v
            .get("include_suggested_pattern_candidates")
            .and_then(|x| x.as_bool())
        {
            self.include_suggested_pattern_candidates = b;
        }
        if let Some(include_failure_recall) =
            v.get("include_failure_recall").and_then(|x| x.as_bool())
        {
            self.include_failure_recall = include_failure_recall;
        }
    }
}

pub fn record_temp_mode_read_suppressed() {
    TEMP_MODE_SUPPRESSED_READS_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
}

pub fn record_temp_mode_write_suppressed() {
    TEMP_MODE_SUPPRESSED_WRITES_TOTAL.fetch_add(1, AtomicOrdering::Relaxed);
}

/// Workspace [`ContextFreshness`] from MCP tool inventory (same source as
/// [`crate::ainl_policy::workspace_policy_view`]) for gating **prompt-time** graph-memory
/// features that should not run when the host considers repo context **stale**.
pub async fn workspace_context_freshness_for_prompt(
    mcp_connections: Option<&tokio::sync::Mutex<Vec<crate::mcp::McpConnection>>>,
) -> Option<ContextFreshness> {
    let m = mcp_connections?;
    let guard = m.lock().await;
    Some(crate::ainl_policy::workspace_policy_view(guard.as_slice()).context_freshness)
}

pub async fn build_prompt_memory_context(
    gm: &GraphMemoryWriter,
    policy: &MemoryContextPolicy,
    user_message: Option<&str>,
    failure_recall_enabled: bool,
    // When `Some(Stale)` from the workspace policy view, `## FailureRecall` is skipped
    // (see `ainl_failure_learning::should_emit_failure_suggestion`).
    context_freshness: Option<ContextFreshness>,
) -> PromptMemoryContext {
    let mut ctx = PromptMemoryContext::default();
    if !policy.allow_reads() {
        return ctx;
    }

    let now_ts = chrono::Utc::now().timestamp();
    let mut semantic_to_touch: Vec<AinlMemoryNode> = Vec::new();

    let (recent_episodes_raw, recent_semantic, recent_procedural) = {
        let inner = gm.inner.lock().await;
        (
            inner
                .recall_recent(policy.recall_window_secs)
                .unwrap_or_default(),
            inner
                .recall_by_type(AinlNodeKind::Semantic, policy.recall_window_secs)
                .unwrap_or_default(),
            inner
                .recall_by_type(AinlNodeKind::Procedural, policy.recall_window_secs)
                .unwrap_or_default(),
        )
    };

    let active_conversation = recent_episodes_raw.iter().find_map(|n| match &n.node_type {
        AinlNodeType::Episode { episodic } if !episodic.conversation_id.is_empty() => {
            Some(episodic.conversation_id.clone())
        }
        _ => None,
    });
    let topic_tags: Vec<String> = recent_episodes_raw
        .iter()
        .find_map(|n| match &n.node_type {
            AinlNodeType::Episode { episodic } => Some(episodic.tags.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let scoped_turn_ids: HashSet<uuid::Uuid> = {
        let inner = gm.inner.lock().await;
        let scoped = recall_task_scoped_episodes(
            inner.store(),
            gm.agent_id(),
            active_conversation.as_deref(),
            &topic_tags,
            policy.max_episodic_lines.saturating_mul(4).max(8),
        )
        .unwrap_or_default();
        scoped.into_iter().map(|ep| ep.turn_id).collect()
    };
    let recent_episodes: Vec<AinlMemoryNode> = recent_episodes_raw
        .into_iter()
        .filter(|n| match &n.node_type {
            AinlNodeType::Episode { episodic } => scoped_turn_ids.contains(&episodic.turn_id),
            _ => false,
        })
        .collect();

    let mut episodic_scored: Vec<(f32, i64, String)> = if policy.include_episodic_hints {
        recent_episodes
            .iter()
            .filter_map(|n| {
                let AinlNodeType::Episode { episodic } = &n.node_type else {
                    return None;
                };
                let mut score = 0.0_f32;
                let age = (now_ts - episodic.timestamp).max(0);
                score += 1.0 / ((age / 60) as f32 + 1.0);
                if episodic.flagged {
                    score -= 0.4;
                }
                if let Some(active) = &active_conversation {
                    if &episodic.conversation_id == active {
                        score += 0.3;
                    }
                }
                let mut detail = format!(
                    "ep:{} tools={}",
                    short_id(n.id),
                    join_tools(episodic.effective_tools())
                );
                if let Some(to) = &episodic.delegation_to {
                    detail.push_str(&format!(" delegated_to={to}"));
                }
                if let Some(v) = &episodic.vitals_gate {
                    detail.push_str(&format!(" trust_gate={v}"));
                }
                if policy.include_provenance {
                    if !episodic.conversation_id.is_empty() {
                        detail.push_str(&format!(" [conv:{}]", episodic.conversation_id));
                    }
                    if let Some(prev) = &episodic.follows_episode_id {
                        detail.push_str(&format!(" [follows:{}]", short_id_str(prev)));
                    }
                }
                Some((score, episodic.timestamp, detail))
            })
            .collect()
    } else {
        Vec::new()
    };
    episodic_scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(Ordering::Equal)
            .then_with(|| b.1.cmp(&a.1))
    });
    append_limited_lines(
        &mut ctx.episodic_lines,
        episodic_scored
            .iter()
            .map(|(_, _, s)| truncate_with_ellipsis(s, 180))
            .collect(),
        policy.max_episodic_lines,
        policy.max_episodic_chars,
        &mut ctx.truncation_hits,
    );
    for (score, ts, detail) in episodic_scored.iter().take(ctx.episodic_lines.len()) {
        ctx.selection_debug.push(serde_json::json!({
            "block": "RecentAttempts",
            "score": score,
            "timestamp": ts,
            "selected_line": detail,
        }));
    }

    let mut seen_fact = HashSet::new();
    let mut semantic_scored: Vec<(f32, String, AinlMemoryNode)> = Vec::new();
    if policy.include_semantic_facts {
        for node in recent_semantic {
            let AinlNodeType::Semantic { semantic } = &node.node_type else {
                continue;
            };
            if semantic.confidence < policy.semantic_confidence_floor {
                ctx.skipped_low_quality += 1;
                continue;
            }
            let normalized = semantic.fact.trim().to_ascii_lowercase();
            if normalized.is_empty() || !seen_fact.insert(normalized) {
                continue;
            }
            let recency_score = recency_score(node_timestamp(&node), now_ts);
            let recurrence = (semantic.recurrence_count.min(10) as f32) / 10.0;
            let referenced = (semantic.reference_count.min(20) as f32) / 20.0;
            let stale_penalty = if now_ts - node_timestamp(&node) > policy.semantic_ttl_secs {
                0.25
            } else {
                0.0
            };
            let score = (semantic.confidence * 0.45)
                + (recurrence * 0.20)
                + (referenced * 0.20)
                + (recency_score * 0.15)
                - stale_penalty;
            let mut line = semantic.fact.clone();
            if policy.include_provenance {
                line.push_str(&format!(
                    " [conf={:.2} recur={} refs={} src={}]",
                    semantic.confidence,
                    semantic.recurrence_count,
                    semantic.reference_count,
                    short_id_str(&semantic.source_episode_id)
                ));
            }
            semantic_scored.push((score, line, node));
        }
    }
    semantic_scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    for (score, line, node) in semantic_scored.iter().take(policy.max_semantic_lines) {
        ctx.semantic_lines.push(truncate_with_ellipsis(line, 220));
        semantic_to_touch.push(node.clone());
        if let AinlNodeType::Semantic { semantic } = &node.node_type {
            ctx.selection_debug.push(serde_json::json!({
                "block": "KnownFacts",
                "score": score,
                "fact": semantic.fact,
                "confidence": semantic.confidence,
                "recurrence_count": semantic.recurrence_count,
                "reference_count": semantic.reference_count,
                "source_episode_id": semantic.source_episode_id,
            }));
        }
    }
    let mut semantic_trimmed = Vec::new();
    append_limited_lines(
        &mut semantic_trimmed,
        ctx.semantic_lines.clone(),
        policy.max_semantic_lines,
        policy.max_semantic_chars,
        &mut ctx.truncation_hits,
    );
    ctx.semantic_lines = semantic_trimmed;

    if policy.include_conflicts {
        let mut conflict_lines = Vec::new();
        for node in &semantic_to_touch {
            let AinlNodeType::Semantic { semantic } = &node.node_type else {
                continue;
            };
            if semantic.confidence < policy.contradiction_confidence_floor {
                continue;
            }
            if semantic.contradiction_ids.is_empty() {
                continue;
            }
            let mut line = format!(
                "fact '{}' has {} conflicting evidence node(s)",
                truncate_with_ellipsis(&semantic.fact, 96),
                semantic.contradiction_ids.len()
            );
            if policy.include_provenance {
                let ids = semantic
                    .contradiction_ids
                    .iter()
                    .take(3)
                    .map(|id| short_id_str(id))
                    .collect::<Vec<_>>()
                    .join(", ");
                line.push_str(&format!(" [contradictions={ids}]"));
            }
            conflict_lines.push(line);
            ctx.selection_debug.push(serde_json::json!({
                "block": "KnownConflicts",
                "fact": semantic.fact,
                "confidence": semantic.confidence,
                "contradiction_ids": semantic.contradiction_ids,
            }));
        }
        append_limited_lines(
            &mut ctx.conflict_lines,
            conflict_lines,
            policy.max_conflict_lines,
            policy.max_conflict_chars,
            &mut ctx.truncation_hits,
        );
    }

    if policy.include_suggested_pattern_candidates {
        let mut candidate_scored: Vec<(f32, String)> = recent_procedural
            .iter()
            .filter_map(|n| {
                let AinlNodeType::Procedural { procedural } = &n.node_type else {
                    return None;
                };
                if procedural.retired {
                    return None;
                }
                if procedural.prompt_eligible {
                    return None;
                }
                if procedural.tool_sequence.is_empty() {
                    return None;
                }
                let base = procedural
                    .fitness
                    .unwrap_or(procedural.success_rate)
                    .clamp(0.0, 1.0);
                let freshness = if procedural.last_invoked_at == 0 {
                    0.1
                } else {
                    recency_score(procedural.last_invoked_at as i64, now_ts)
                };
                let score = (base * 0.8) + (freshness * 0.2);
                let mut line = format!(
                    "{} -> {}",
                    if procedural.pattern_name.is_empty() {
                        "procedure".to_string()
                    } else {
                        procedural.pattern_name.clone()
                    },
                    procedural.tool_sequence.join(" -> ")
                );
                if policy.include_provenance {
                    line.push_str(&format!(
                        " [candidate obs={} fitness={}]",
                        procedural.pattern_observation_count,
                        procedural
                            .fitness
                            .map(|f| format!("{f:.2}"))
                            .unwrap_or_else(|| "n/a".to_string())
                    ));
                    if let Some(trace_id) = &procedural.trace_id {
                        line.push_str(&format!(" [trace:{}]", short_id_str(trace_id)));
                    }
                }
                Some((score, line))
            })
            .collect();
        candidate_scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        let selected_pc = ctx.pattern_candidate_lines.len();
        append_limited_lines(
            &mut ctx.pattern_candidate_lines,
            candidate_scored
                .into_iter()
                .map(|(_, line)| truncate_with_ellipsis(&line, 180))
                .collect(),
            policy.max_pattern_candidate_lines,
            policy.max_pattern_candidate_chars,
            &mut ctx.truncation_hits,
        );
        for line in ctx.pattern_candidate_lines.iter().skip(selected_pc) {
            ctx.selection_debug.push(serde_json::json!({
                "block": "SuggestedPatternCandidates",
                "selected_line": line,
            }));
        }
    }

    if policy.include_procedural_hints {
        let mut procedure_scored: Vec<(f32, String, Option<String>)> = recent_procedural
            .iter()
            .filter_map(|n| {
                let AinlNodeType::Procedural { procedural } = &n.node_type else {
                    return None;
                };
                if procedural.retired {
                    return None;
                }
                if !procedural.prompt_eligible {
                    return None;
                }
                let base = procedural
                    .fitness
                    .unwrap_or(procedural.success_rate)
                    .clamp(0.0, 1.0);
                let freshness = if procedural.last_invoked_at == 0 {
                    0.1
                } else {
                    recency_score(procedural.last_invoked_at as i64, now_ts)
                };
                let mut score = (base * 0.8) + (freshness * 0.2);
                let artifact =
                    serde_json::from_slice::<ProcedureArtifact>(&procedural.compiled_graph).ok();
                let procedure_id = artifact.as_ref().map(|artifact| artifact.id.clone());
                if let Some(artifact) = artifact.as_ref() {
                    let has_failure_risk = !artifact.known_failures.is_empty()
                        || !artifact.source_failure_ids.is_empty();
                    if context_freshness == Some(ContextFreshness::Stale) && has_failure_risk {
                        return None;
                    }
                    let available_tools = if procedural.tool_sequence.is_empty() {
                        artifact.required_tools.clone()
                    } else {
                        procedural.tool_sequence.clone()
                    };
                    let reuse = ainl_procedure_learning::score_reuse(
                        artifact,
                        user_message.unwrap_or_default(),
                        &available_tools,
                    );
                    score = (score * 0.35) + (reuse.score * 0.65);
                    if score < 0.35 {
                        return None;
                    }
                }
                let mut line = if let Some(artifact) = artifact.as_ref() {
                    let tools = if artifact.required_tools.is_empty() {
                        procedural.tool_sequence.join(" -> ")
                    } else {
                        artifact.required_tools.join(" -> ")
                    };
                    if score >= 0.72 {
                        let checks = artifact
                            .verification
                            .checks
                            .iter()
                            .take(2)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ");
                        format!(
                            "{}: {} [{}]{}",
                            artifact.title,
                            artifact.intent,
                            tools,
                            if checks.is_empty() {
                                String::new()
                            } else {
                                format!(" verify: {checks}")
                            }
                        )
                    } else if tools.is_empty() {
                        format!("{}: {}", artifact.title, artifact.summary)
                    } else {
                        format!("{}: {} [{}]", artifact.title, artifact.summary, tools)
                    }
                } else if !procedural.tool_sequence.is_empty() {
                    format!(
                        "{} -> {}",
                        if procedural.pattern_name.is_empty() {
                            "procedure".to_string()
                        } else {
                            procedural.pattern_name.clone()
                        },
                        procedural.tool_sequence.join(" -> ")
                    )
                } else {
                    procedural.pattern_name.clone()
                };
                if policy.include_provenance {
                    line.push_str(&format!(
                        " [score={:.2} success={:.2} fitness={}]",
                        score,
                        procedural.success_rate,
                        procedural
                            .fitness
                            .map(|f| format!("{f:.2}"))
                            .unwrap_or_else(|| "n/a".to_string())
                    ));
                    if let Some(trace_id) = &procedural.trace_id {
                        line.push_str(&format!(" [trace:{}]", short_id_str(trace_id)));
                    }
                }
                Some((score, line, procedure_id))
            })
            .collect();
        procedure_scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
        ctx.selected_procedure_ids = procedure_scored
            .iter()
            .filter_map(|(_, _, id)| id.clone())
            .take(policy.max_procedural_lines)
            .collect();
        let selected_before = ctx.procedural_lines.len();
        append_limited_lines(
            &mut ctx.procedural_lines,
            procedure_scored
                .into_iter()
                .map(|(_, line, _)| truncate_with_ellipsis(&line, 180))
                .collect(),
            policy.max_procedural_lines,
            policy.max_procedural_chars,
            &mut ctx.truncation_hits,
        );
        for line in ctx.procedural_lines.iter().skip(selected_before) {
            ctx.selection_debug.push(serde_json::json!({
                "block": "SuggestedProcedure",
                "selected_line": line,
            }));
        }
    }

    if failure_recall_enabled
        && policy.include_failure_recall
        && should_emit_failure_suggestion(context_freshness)
    {
        if let Some(msg) = user_message.filter(|m| !m.trim().is_empty()) {
            if let Some(fts_q) = crate::graph_memory_learning::failure_recall_fts_query(msg) {
                let fetch_cap = policy
                    .max_failure_recall_lines
                    .saturating_mul(2)
                    .clamp(8, 50);
                let hits = {
                    let inner = gm.inner.lock().await;
                    inner.search_failures_for_agent(gm.agent_id(), fts_q.as_str(), fetch_cap)
                };
                if let Ok(nodes) = hits {
                    let mut seen_keys = HashSet::new();
                    let mut candidates: Vec<String> = Vec::new();
                    for n in nodes {
                        let AinlNodeType::Failure { ref failure } = n.node_type else {
                            continue;
                        };
                        let tool_s = failure.tool_name.as_deref().unwrap_or("_");
                        let msg_preview: String = failure.message.chars().take(120).collect();
                        let key = format!(
                            "{}|{}|{}",
                            failure.source,
                            tool_s,
                            msg_preview.to_ascii_lowercase()
                        );
                        if !seen_keys.insert(key) {
                            continue;
                        }
                        let line = format!(
                            "source={} tool={} msg={}",
                            failure.source,
                            tool_s,
                            truncate_with_ellipsis(&failure.message, 160),
                        );
                        candidates.push(line);
                    }
                    let before = ctx.failure_recall_lines.len();
                    append_limited_lines(
                        &mut ctx.failure_recall_lines,
                        candidates,
                        policy.max_failure_recall_lines,
                        policy.max_failure_recall_chars,
                        &mut ctx.truncation_hits,
                    );
                    for line in ctx.failure_recall_lines.iter().skip(before) {
                        ctx.selection_debug.push(serde_json::json!({
                            "block": "FailureRecall",
                            "fts_query": fts_q,
                            "selected_line": line,
                        }));
                    }
                }
            }
        }
    }

    if memory_include_trajectory_recap_env() {
        let max_rows = std::env::var("AINL_MEMORY_TRAJECTORY_RECAP_MAX")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(5)
            .min(20);
        let max_ops = std::env::var("AINL_MEMORY_TRAJECTORY_RECAP_MAX_OPS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4)
            .min(12);
        let records = {
            let inner = gm.inner.lock().await;
            inner.list_trajectories_for_agent(gm.agent_id(), max_rows, None)
        };
        if let Ok(records) = records {
            if !records.is_empty() {
                let lines = ainl_context_compiler::format_trajectory_recap_lines(
                    &records, max_rows, max_ops,
                );
                if !lines.is_empty() {
                    for line in &lines {
                        ctx.selection_debug.push(serde_json::json!({
                            "block": "TrajectoryRecap",
                            "selected_line": line,
                        }));
                    }
                    ctx.trajectory_recap_lines = lines;
                }
            }
        }
    }

    if memory_include_suggested_next_env() {
        if let Some(line) = ctx.procedural_lines.first() {
            ctx.suggested_next_lines
                .push(format!("Try the promoted tool flow: {line}"));
        } else if let Some(line) = ctx.pattern_candidate_lines.first() {
            ctx.suggested_next_lines.push(format!(
                "Next: practice or promote this tool sequence: {line}"
            ));
        } else if let Some(line) = ctx.semantic_lines.first() {
            ctx.suggested_next_lines
                .push(format!("Anchor on a high-confidence fact: {line}"));
        }
        if ctx.suggested_next_lines.len() > 2 {
            ctx.suggested_next_lines.truncate(2);
        }
    }

    if !semantic_to_touch.is_empty() {
        let now_u = now_ts.max(0) as u64;
        let inner = gm.inner.lock().await;
        for mut node in semantic_to_touch {
            if let AinlNodeType::Semantic { ref mut semantic } = node.node_type {
                semantic.reference_count = semantic.reference_count.saturating_add(1);
                semantic.last_referenced_at = now_u;
                let _ = inner.write_node(&node);
            }
        }
    }

    ctx.provenance_lines = count_provenance_lines(&ctx);
    let injected_lines = (ctx.episodic_lines.len()
        + ctx.semantic_lines.len()
        + ctx.conflict_lines.len()
        + ctx.procedural_lines.len()
        + ctx.pattern_candidate_lines.len()
        + ctx.failure_recall_lines.len()
        + ctx.trajectory_recap_lines.len()
        + ctx.suggested_next_lines.len()) as u64;
    INJECTED_EPISODIC_TOTAL.fetch_add(ctx.episodic_lines.len() as u64, AtomicOrdering::Relaxed);
    INJECTED_SEMANTIC_TOTAL.fetch_add(ctx.semantic_lines.len() as u64, AtomicOrdering::Relaxed);
    INJECTED_CONFLICT_TOTAL.fetch_add(ctx.conflict_lines.len() as u64, AtomicOrdering::Relaxed);
    INJECTED_PROCEDURAL_TOTAL.fetch_add(ctx.procedural_lines.len() as u64, AtomicOrdering::Relaxed);
    INJECTED_PATTERN_CANDIDATE_TOTAL.fetch_add(
        ctx.pattern_candidate_lines.len() as u64,
        AtomicOrdering::Relaxed,
    );
    INJECTED_FAILURE_RECALL_TOTAL.fetch_add(
        ctx.failure_recall_lines.len() as u64,
        AtomicOrdering::Relaxed,
    );
    INJECTED_TRAJECTORY_RECAP_TOTAL.fetch_add(
        ctx.trajectory_recap_lines.len() as u64,
        AtomicOrdering::Relaxed,
    );
    INJECTED_SUGGESTED_NEXT_TOTAL.fetch_add(
        ctx.suggested_next_lines.len() as u64,
        AtomicOrdering::Relaxed,
    );
    TRUNCATION_HITS_TOTAL.fetch_add(ctx.truncation_hits as u64, AtomicOrdering::Relaxed);
    SKIPPED_LOW_QUALITY_TOTAL.fetch_add(ctx.skipped_low_quality as u64, AtomicOrdering::Relaxed);
    INJECTED_LINES_TOTAL.fetch_add(injected_lines, AtomicOrdering::Relaxed);
    PROVENANCE_LINES_TOTAL.fetch_add(ctx.provenance_lines as u64, AtomicOrdering::Relaxed);
    if let Ok(mut snapshot) = selection_debug_snapshot().lock() {
        *snapshot = ctx.selection_debug.iter().take(40).cloned().collect();
    }
    debug!(
        episodic = ctx.episodic_lines.len(),
        semantic = ctx.semantic_lines.len(),
        conflict = ctx.conflict_lines.len(),
        procedural = ctx.procedural_lines.len(),
        pattern_candidates = ctx.pattern_candidate_lines.len(),
        failure_recall = ctx.failure_recall_lines.len(),
        suggested_next = ctx.suggested_next_lines.len(),
        skipped_low_quality = ctx.skipped_low_quality,
        truncation_hits = ctx.truncation_hits,
        provenance_lines = ctx.provenance_lines,
        selection_debug_items = ctx.selection_debug.len(),
        "memory prompt context assembled"
    );

    ctx
}

fn metadata_bool(meta: &HashMap<String, serde_json::Value>, key: &str, default: bool) -> bool {
    let v = meta.get(key).and_then(|v| {
        v.as_bool().or_else(|| {
            v.as_str()
                .map(|s| parse_bool_with_default(Some(s), default))
        })
    });
    v.unwrap_or(default)
}

fn parse_bool_with_default(raw: Option<&str>, default: bool) -> bool {
    let Some(raw) = raw else { return default };
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => true,
        "0" | "false" | "no" | "off" => false,
        _ => default,
    }
}

fn short_id(id: uuid::Uuid) -> String {
    id.to_string().chars().take(8).collect()
}

fn short_id_str(id: &str) -> String {
    if id.is_empty() {
        return "n/a".to_string();
    }
    id.chars().take(8).collect()
}

fn truncate_with_ellipsis(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn append_limited_lines(
    target: &mut Vec<String>,
    candidates: Vec<String>,
    max_lines: usize,
    max_chars: usize,
    truncation_hits: &mut usize,
) {
    let mut used_chars = 0usize;
    for line in candidates.into_iter().take(max_lines) {
        let line_len = line.chars().count();
        if used_chars + line_len > max_chars {
            *truncation_hits += 1;
            break;
        }
        used_chars += line_len;
        target.push(line);
    }
}

fn recency_score(ts: i64, now_ts: i64) -> f32 {
    let age_s = (now_ts - ts).max(0) as f32;
    (1.0 / ((age_s / 3600.0) + 1.0)).clamp(0.0, 1.0)
}

fn node_timestamp(node: &AinlMemoryNode) -> i64 {
    match &node.node_type {
        AinlNodeType::Episode { episodic } => episodic.timestamp,
        AinlNodeType::Trajectory { trajectory } => trajectory.recorded_at,
        AinlNodeType::RuntimeState { runtime_state } => runtime_state.updated_at,
        _ => chrono::Utc::now().timestamp(),
    }
}

fn join_tools(tools: &[String]) -> String {
    if tools.is_empty() {
        "_none_".to_string()
    } else {
        truncate_with_ellipsis(&tools.join(","), 96)
    }
}

fn count_provenance_lines(ctx: &PromptMemoryContext) -> usize {
    let mut total = 0usize;
    for line in ctx
        .episodic_lines
        .iter()
        .chain(ctx.semantic_lines.iter())
        .chain(ctx.conflict_lines.iter())
        .chain(ctx.procedural_lines.iter())
        .chain(ctx.pattern_candidate_lines.iter())
        .chain(ctx.failure_recall_lines.iter())
        .chain(ctx.trajectory_recap_lines.iter())
        .chain(ctx.suggested_next_lines.iter())
    {
        if line.contains('[') && line.contains(']') {
            total += 1;
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use ainl_contracts::ContextFreshness;
    use ainl_memory::{AinlMemoryNode, AinlNodeType, GraphMemory};
    use uuid::Uuid;

    fn test_writer(agent_id: &str) -> crate::graph_memory_writer::GraphMemoryWriter {
        let temp = tempfile::tempdir().expect("tempdir");
        let db = temp.path().join("ainl_memory.db");
        let memory = GraphMemory::new(&db).expect("GraphMemory::new");
        let writer = crate::graph_memory_writer::GraphMemoryWriter::from_memory_for_tests(
            memory,
            agent_id.to_string(),
            None,
        );
        // leak tempdir for test lifetime to keep db alive
        std::mem::forget(temp);
        writer
    }

    async fn write_semantic(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
        fact: &str,
        confidence: f32,
        recurrence: u32,
        references: u32,
        contradictions: Vec<String>,
    ) {
        let mut node = AinlMemoryNode::new_fact(fact.to_string(), confidence, Uuid::new_v4());
        if let AinlNodeType::Semantic { ref mut semantic } = node.node_type {
            semantic.recurrence_count = recurrence;
            semantic.reference_count = references;
            semantic.contradiction_ids = contradictions;
            semantic.source_episode_id = Uuid::new_v4().to_string();
        }
        node.agent_id = writer.agent_id().to_string();
        let inner = writer.inner.lock().await;
        inner.write_node(&node).expect("write semantic");
    }

    async fn write_procedure(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
        name: &str,
        success: f32,
        fitness: Option<f32>,
        retired: bool,
    ) {
        let mut node = AinlMemoryNode::new_procedural_tools(
            name.to_string(),
            vec!["file_read".to_string(), "shell_exec".to_string()],
            success,
        );
        if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
            procedural.success_rate = success;
            procedural.fitness = fitness;
            procedural.retired = retired;
            procedural.last_invoked_at = chrono::Utc::now().timestamp() as u64;
            procedural.prompt_eligible = true;
            procedural.pattern_observation_count = procedural
                .pattern_observation_count
                .max(ainl_memory::pattern_promotion::DEFAULT_MIN_OBSERVATIONS);
        }
        node.agent_id = writer.agent_id().to_string();
        let inner = writer.inner.lock().await;
        inner.write_node(&node).expect("write procedural");
    }

    /// Ineligible (pre-promotion) tool-sequence row for SuggestedPatternCandidates.
    async fn write_pattern_candidate(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
        name: &str,
    ) {
        let mut node = AinlMemoryNode::new_procedural_tools(
            name.to_string(),
            vec!["file_read".to_string(), "shell_exec".to_string()],
            0.8,
        );
        if let AinlNodeType::Procedural { ref mut procedural } = node.node_type {
            procedural.last_invoked_at = chrono::Utc::now().timestamp() as u64;
        }
        node.agent_id = writer.agent_id().to_string();
        let inner = writer.inner.lock().await;
        inner.write_node(&node).expect("write pattern candidate");
    }

    async fn write_procedure_artifact(writer: &crate::graph_memory_writer::GraphMemoryWriter) {
        let artifact = test_procedure_artifact(vec![], vec![]);
        let inner = writer.inner.lock().await;
        inner
            .write_procedure_artifact_for_agent(writer.agent_id(), &artifact)
            .expect("write procedure artifact");
    }

    async fn write_risky_procedure_artifact(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
    ) {
        let artifact = test_procedure_artifact(vec!["prior timeout".into()], vec!["f1".into()]);
        let inner = writer.inner.lock().await;
        inner
            .write_procedure_artifact_for_agent(writer.agent_id(), &artifact)
            .expect("write risky procedure artifact");
    }

    fn test_procedure_artifact(
        known_failures: Vec<String>,
        source_failure_ids: Vec<String>,
    ) -> ProcedureArtifact {
        ProcedureArtifact {
            schema_version: ainl_contracts::LEARNER_SCHEMA_VERSION,
            id: "proc:artifact".into(),
            title: "Artifact Procedure".into(),
            intent: "review pull requests safely".into(),
            summary: "Use the proven PR review workflow.".into(),
            required_tools: vec!["file_read".into(), "shell_exec".into()],
            required_adapters: vec![],
            inputs: vec![],
            outputs: vec![],
            preconditions: vec![],
            steps: vec![],
            verification: Default::default(),
            known_failures,
            recovery: vec![],
            source_trajectory_ids: vec![],
            source_failure_ids,
            fitness: 0.95,
            observation_count: 3,
            lifecycle: ainl_contracts::ProcedureLifecycle::Validated,
            render_targets: vec![ainl_contracts::ProcedureArtifactFormat::PromptOnly],
        }
    }

    async fn write_tool_failure(
        writer: &crate::graph_memory_writer::GraphMemoryWriter,
        tool_name: &str,
        message: &str,
    ) {
        let mut node = AinlMemoryNode::new_tool_execution_failure(tool_name, message, None::<&str>);
        node.agent_id = writer.agent_id().to_string();
        let inner = writer.inner.lock().await;
        inner.write_node(&node).expect("write failure");
    }

    #[tokio::test]
    async fn failure_recall_injects_matching_rows_when_enabled() {
        let writer = test_writer("ctx-failure-recall");
        write_tool_failure(
            &writer,
            "shell_exec",
            "ENOENT opening quantumretirement config",
        )
        .await;

        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            Some("quantumretirement shell_exec tool_runner"),
            true,
            None,
        )
        .await;
        assert!(
            !ctx.failure_recall_lines.is_empty(),
            "expected failure recall, got {:?}",
            ctx.failure_recall_lines
        );
        let block = ctx.to_prompt_block();
        assert!(block.contains("## FailureRecall"));
        assert!(
            block.contains("quantumretirement")
                || ctx.failure_recall_lines[0].contains("quantumretirement")
        );
    }

    #[tokio::test]
    async fn failure_recall_skipped_when_learning_gate_off() {
        let writer = test_writer("ctx-failure-recall-off");
        write_tool_failure(&writer, "shell_exec", "bad quantumretirement").await;
        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            Some("quantumretirement shell_exec tool_runner"),
            false,
            None,
        )
        .await;
        assert!(ctx.failure_recall_lines.is_empty());
    }

    #[tokio::test]
    async fn failure_recall_skipped_when_context_freshness_stale() {
        let writer = test_writer("ctx-failure-stale");
        write_tool_failure(
            &writer,
            "shell_exec",
            "ENOENT opening quantumretirement config",
        )
        .await;
        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            Some("quantumretirement shell_exec tool_runner"),
            true,
            Some(ContextFreshness::Stale),
        )
        .await;
        assert!(
            ctx.failure_recall_lines.is_empty(),
            "stale context must suppress failure recall, got {:?}",
            ctx.failure_recall_lines
        );
    }

    #[tokio::test]
    async fn semantic_ranking_prefers_reference_count_and_dedupes() {
        let writer = test_writer("ctx-semantic-test");
        write_semantic(&writer, "alpha fact", 0.8, 1, 20, vec![]).await;
        write_semantic(&writer, "beta fact", 0.8, 1, 1, vec![]).await;
        write_semantic(&writer, "ALPHA FACT", 0.9, 3, 10, vec![]).await; // dedupe variant

        let policy = MemoryContextPolicy {
            max_semantic_lines: 2,
            ..Default::default()
        };
        let ctx = build_prompt_memory_context(&writer, &policy, None, false, None).await;
        assert_eq!(ctx.semantic_lines.len(), 2);
        assert!(
            ctx.semantic_lines[0]
                .to_ascii_lowercase()
                .contains("alpha fact"),
            "expected alpha fact first, got {:?}",
            ctx.semantic_lines
        );
    }

    #[tokio::test]
    async fn contradictions_require_confidence_floor() {
        let writer = test_writer("ctx-conflict-test");
        write_semantic(
            &writer,
            "high confidence conflict",
            0.85,
            1,
            1,
            vec![Uuid::new_v4().to_string()],
        )
        .await;
        write_semantic(
            &writer,
            "low confidence conflict",
            0.40,
            1,
            1,
            vec![Uuid::new_v4().to_string()],
        )
        .await;

        let policy = MemoryContextPolicy {
            contradiction_confidence_floor: 0.70,
            ..Default::default()
        };
        let ctx = build_prompt_memory_context(&writer, &policy, None, false, None).await;
        assert_eq!(ctx.conflict_lines.len(), 1);
        assert!(ctx.conflict_lines[0].contains("high confidence conflict"));
    }

    #[tokio::test]
    async fn procedural_hints_exclude_retired() {
        let writer = test_writer("ctx-proc-test");
        write_procedure(&writer, "good_proc", 0.9, Some(0.95), false).await;
        write_procedure(&writer, "retired_proc", 0.99, Some(0.99), true).await;

        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            None,
            false,
            None,
        )
        .await;
        assert!(ctx
            .procedural_lines
            .iter()
            .any(|line| line.contains("good_proc")));
        assert!(ctx
            .procedural_lines
            .iter()
            .all(|line| !line.contains("retired_proc")));
    }

    #[tokio::test]
    async fn procedure_artifacts_render_rich_prompt_hints() {
        let writer = test_writer("ctx-proc-artifact");
        write_procedure_artifact(&writer).await;
        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            Some("please review this pull request"),
            false,
            None,
        )
        .await;
        let line = ctx.procedural_lines.join("\n");
        assert!(line.contains("Artifact Procedure"), "{line}");
        assert!(line.contains("review pull requests safely"), "{line}");
        assert!(line.contains("file_read -> shell_exec"), "{line}");
        assert_eq!(
            ctx.selected_procedure_ids,
            vec!["proc:artifact".to_string()]
        );
    }

    #[tokio::test]
    async fn procedure_artifact_with_failure_risk_is_gated_when_context_stale() {
        let writer = test_writer("ctx-proc-risk");
        write_risky_procedure_artifact(&writer).await;
        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy::default(),
            Some("please review this pull request"),
            false,
            Some(ContextFreshness::Stale),
        )
        .await;
        assert!(
            ctx.procedural_lines
                .iter()
                .all(|line| !line.contains("Artifact Procedure")),
            "{:?}",
            ctx.procedural_lines
        );
    }

    #[tokio::test]
    async fn pattern_candidates_appear_in_prompt_before_suggested_procedure() {
        let writer = test_writer("ctx-pat-cand");
        write_pattern_candidate(&writer, "emerging_pat").await;
        write_procedure(&writer, "ready_proc", 0.9, Some(0.95), false).await;
        let ctx = build_prompt_memory_context(
            &writer,
            &MemoryContextPolicy {
                include_suggested_pattern_candidates: true,
                ..Default::default()
            },
            None,
            false,
            None,
        )
        .await;
        let block = ctx.to_prompt_block();
        let p_c = block
            .find("## SuggestedPatternCandidates")
            .expect("candidates block");
        let p_s = block
            .find("## SuggestedProcedure")
            .expect("suggested block");
        assert!(
            p_c < p_s,
            "candidates should precede suggested procedure, got {block}"
        );
        assert!(block.contains("emerging_pat"), "{block}");
        assert!(block.contains("ready_proc"), "{block}");
    }

    #[tokio::test]
    async fn block_kill_switches_disable_selected_sections() {
        let writer = test_writer("ctx-switch-test");
        write_semantic(&writer, "switch fact", 0.9, 1, 1, vec![]).await;
        write_procedure(&writer, "switch_proc", 0.9, Some(0.9), false).await;

        let policy = MemoryContextPolicy {
            include_episodic_hints: false,
            include_semantic_facts: false,
            include_conflicts: false,
            include_procedural_hints: true,
            ..Default::default()
        };
        let ctx = build_prompt_memory_context(&writer, &policy, None, false, None).await;
        assert!(ctx.episodic_lines.is_empty());
        assert!(ctx.semantic_lines.is_empty());
        assert!(ctx.conflict_lines.is_empty());
        assert!(!ctx.procedural_lines.is_empty());
    }

    #[test]
    fn deterministic_truncation_respects_budget() {
        let mut out = Vec::new();
        let mut trunc = 0usize;
        append_limited_lines(
            &mut out,
            vec![
                "first".to_string(),
                "second_is_long".to_string(),
                "third".to_string(),
            ],
            3,
            10,
            &mut trunc,
        );
        assert_eq!(out, vec!["first".to_string()]);
        assert_eq!(trunc, 1);
    }

    #[test]
    fn rollout_internal_gate_blocks_non_internal_agents() {
        let policy = MemoryContextPolicy {
            rollout_mode: "internal".to_string(),
            internal_agent: false,
            enabled: true,
            temporary_mode: false,
            ..Default::default()
        };
        assert!(!policy.allow_reads());
        assert!(!policy.allow_writes());
    }

    #[test]
    fn temporary_mode_blocks_reads_and_writes() {
        let policy = MemoryContextPolicy {
            temporary_mode: true,
            enabled: true,
            ..Default::default()
        };
        assert!(!policy.allow_reads());
        assert!(!policy.allow_writes());
    }

    #[test]
    fn memory_metrics_include_provenance_gate_fields() {
        let metrics = memory_context_metrics();
        assert!(metrics.get("provenance_coverage_ratio").is_some());
        assert!(metrics.get("provenance_coverage_floor").is_some());
        assert!(metrics.get("provenance_gate_pass").is_some());
        assert!(metrics.get("conflict_ratio").is_some());
        assert!(metrics.get("contradiction_gate_pass").is_some());
        assert!(metrics.get("graph_memory_kernel_notify_ok_total").is_some());
        assert!(metrics
            .get("graph_memory_kernel_notify_err_total")
            .is_some());
        assert!(metrics.get("injected_failure_recall_total").is_some());
        assert!(metrics.get("injected_trajectory_recap_total").is_some());
    }

    #[test]
    fn to_memory_block_segments_matches_prompt_sections() {
        use ainl_context_compiler::SegmentKind;

        let ctx = PromptMemoryContext {
            episodic_lines: vec!["e1".to_string()],
            failure_recall_lines: vec!["f1".to_string()],
            trajectory_recap_lines: vec!["t1".to_string()],
            semantic_lines: vec!["s1".to_string()],
            ..Default::default()
        };

        let segs = ctx.to_memory_block_segments();
        assert_eq!(segs.len(), 4);
        assert!(segs.iter().all(|s| s.kind == SegmentKind::MemoryBlock));
        assert_eq!(segs[0].tool_name.as_deref(), Some("graph_recent_attempts"));
        assert_eq!(segs[1].tool_name.as_deref(), Some("graph_failure_recall"));
        assert_eq!(segs[2].tool_name.as_deref(), Some("graph_trajectory_recap"));
        assert_eq!(segs[3].tool_name.as_deref(), Some("graph_known_facts"));

        let block = ctx.to_prompt_block();
        for s in &segs {
            assert!(block.contains(s.content.trim()), "segment text in block");
        }
    }
}
