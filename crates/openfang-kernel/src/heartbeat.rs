//! Heartbeat monitor — detects unresponsive agents for 24/7 autonomous operation.
//!
//! The heartbeat monitor runs as a background tokio task, periodically checking
//! each running agent's `last_active` timestamp. If an agent hasn't been active
//! for longer than 2x its heartbeat interval, a `HealthCheckFailed` event is
//! published to the event bus.
//!
//! **Reactive** agents (default schedule: wake on message) are **excluded** from
//! inactivity-based unresponsiveness unless `[heartbeat] reactive_idle_timeout_secs`
//! is set. Idle between turns is otherwise expected.
//!
//! **Turn watchdog** (`[turn_watchdog]`): while a turn is in progress, wall time in
//! `Thinking`, `ToolUse`, or `Streaming` is capped; exceeding it marks the agent
//! unresponsive (covers hung LLM / stuck tools) for **all** schedules including reactive.
//! WASM agents report `ToolUse` (`wasm_sandbox`) and Python module agents `python_agent` for
//! the duration of sandbox/subprocess execution (`tool_use_secs`).
//!
//! Crashed agents are tracked for auto-recovery: the heartbeat will attempt to
//! reset crashed agents back to Running up to `max_recovery_attempts` times.
//! After exhausting attempts, agents are marked as Terminated (dead).

use crate::registry::AgentRegistry;
use chrono::Utc;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use openfang_types::agent::{AgentId, AgentState, ScheduleMode};
use openfang_types::config::TurnWatchdogSettings;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::debug;

/// Default heartbeat check interval (seconds).
const DEFAULT_CHECK_INTERVAL_SECS: u64 = 30;

/// Multiplier: agent is considered unresponsive if inactive for this many
/// multiples of its heartbeat interval.
const UNRESPONSIVE_MULTIPLIER: u64 = 2;

/// Default maximum recovery attempts before giving up.
const DEFAULT_MAX_RECOVERY_ATTEMPTS: u32 = 3;

/// Default cooldown between recovery attempts (seconds).
const DEFAULT_RECOVERY_COOLDOWN_SECS: u64 = 60;

/// Result of a heartbeat check.
#[derive(Debug, Clone)]
pub struct HeartbeatStatus {
    /// Agent ID.
    pub agent_id: AgentId,
    /// Agent name.
    pub name: String,
    /// Seconds since last activity.
    pub inactive_secs: i64,
    /// Whether the agent is considered unresponsive.
    pub unresponsive: bool,
    /// Current agent state.
    pub state: AgentState,
}

/// Phase of the agent loop subject to [`TurnWatchdogSettings`] wall limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MonitoredLoopPhase {
    Thinking,
    ToolUse,
    Streaming,
}

/// Last reported busy phase for an agent turn (updated from `LoopPhase` callbacks).
#[derive(Debug, Clone)]
pub struct AgentLoopPhaseState {
    pub kind: MonitoredLoopPhase,
    pub since: chrono::DateTime<Utc>,
}

/// Heartbeat monitor configuration.
#[derive(Debug, Clone)]
pub struct HeartbeatConfig {
    /// How often to run the heartbeat check (seconds).
    pub check_interval_secs: u64,
    /// Default threshold for unresponsiveness (seconds).
    /// Overridden per-agent by AutonomousConfig.heartbeat_interval_secs.
    pub default_timeout_secs: u64,
    /// Maximum recovery attempts before marking agent as Terminated.
    pub max_recovery_attempts: u32,
    /// Minimum seconds between recovery attempts for the same agent.
    pub recovery_cooldown_secs: u64,
    /// Opt-in idle limit for reactive agents (seconds between turns). `None` = off.
    pub reactive_idle_timeout_secs: Option<u64>,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            check_interval_secs: DEFAULT_CHECK_INTERVAL_SECS,
            // 180s default: browser tasks and complex LLM calls can take 1-3 minutes
            default_timeout_secs: 180,
            max_recovery_attempts: DEFAULT_MAX_RECOVERY_ATTEMPTS,
            recovery_cooldown_secs: DEFAULT_RECOVERY_COOLDOWN_SECS,
            reactive_idle_timeout_secs: None,
        }
    }
}

/// If the agent has exceeded the configured wall time for its current loop phase, return elapsed secs.
pub fn turn_phase_stall_secs(
    turn_phases: &DashMap<AgentId, AgentLoopPhaseState>,
    tw: &TurnWatchdogSettings,
    agent_id: AgentId,
    now: chrono::DateTime<Utc>,
) -> Option<i64> {
    if !tw.enabled {
        return None;
    }
    let st = turn_phases.get(&agent_id)?;
    let limit = match st.kind {
        MonitoredLoopPhase::Thinking => tw.thinking_secs,
        MonitoredLoopPhase::ToolUse => tw.tool_use_secs,
        MonitoredLoopPhase::Streaming => tw.streaming_secs,
    };
    let elapsed = (now - st.since).num_seconds();
    if elapsed > limit as i64 {
        Some(elapsed)
    } else {
        None
    }
}

/// Tracks per-agent recovery state across heartbeat cycles.
#[derive(Debug)]
pub struct RecoveryTracker {
    /// Per-agent recovery state: (consecutive_failures, last_attempt_epoch_secs).
    state: DashMap<AgentId, (u32, u64)>,
}

impl RecoveryTracker {
    /// Create a new recovery tracker.
    pub fn new() -> Self {
        Self {
            state: DashMap::new(),
        }
    }

    /// Record a recovery attempt for an agent.
    /// Returns the current attempt number (1-indexed).
    pub fn record_attempt(&self, agent_id: AgentId) -> u32 {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let mut entry = self.state.entry(agent_id).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = now;
        entry.0
    }

    /// Check if enough time has passed since the last recovery attempt.
    pub fn can_attempt(&self, agent_id: AgentId, cooldown_secs: u64) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        match self.state.get(&agent_id) {
            Some(entry) => now.saturating_sub(entry.1) >= cooldown_secs,
            None => true, // No prior attempts
        }
    }

    /// Get the current failure count for an agent.
    pub fn failure_count(&self, agent_id: AgentId) -> u32 {
        self.state.get(&agent_id).map(|e| e.0).unwrap_or(0)
    }

    /// Reset recovery state for an agent (e.g. after successful recovery).
    pub fn reset(&self, agent_id: AgentId) {
        self.state.remove(&agent_id);
    }
}

impl Default for RecoveryTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Coalesces user-facing heartbeat failure logs and `HealthCheckFailed` events per agent.
///
/// Without this, the 30s heartbeat loop can re-announce the same incident across ticks and
/// auto-recovery cycles. Cleared when an agent returns to a responsive `Running` state.
#[derive(Debug)]
pub struct FailureNotifyGate {
    last_notify_epoch_secs: DashMap<AgentId, u64>,
    pub min_interval_secs: u64,
}

impl FailureNotifyGate {
    /// Default gap between failure notifications for the same agent (5 minutes).
    pub const DEFAULT_MIN_INTERVAL_SECS: u64 = 300;

    pub fn new(min_interval_secs: u64) -> Arc<Self> {
        Arc::new(Self {
            last_notify_epoch_secs: DashMap::new(),
            min_interval_secs,
        })
    }

    fn now_epoch_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Drop suppression so the next failure for this agent can surface immediately.
    pub fn clear(&self, agent_id: AgentId) {
        self.last_notify_epoch_secs.remove(&agent_id);
    }

    /// Returns `true` if the caller should emit a `warn!` / `HealthCheckFailed` for this agent.
    pub fn allow_notify(&self, agent_id: AgentId) -> bool {
        let now = Self::now_epoch_secs();
        match self.last_notify_epoch_secs.entry(agent_id) {
            Entry::Occupied(mut o) => {
                if now.saturating_sub(*o.get()) >= self.min_interval_secs {
                    *o.get_mut() = now;
                    true
                } else {
                    false
                }
            }
            Entry::Vacant(v) => {
                v.insert(now);
                true
            }
        }
    }
}

/// Grace period (seconds): if an agent's `last_active` is within this window
/// of `created_at`, it has never genuinely processed a message and should not
/// be flagged as unresponsive.  This covers the small gap between registration
/// and the initial `set_state(Running)` call.
const IDLE_GRACE_SECS: i64 = 10;

/// Check all running and crashed agents and return their heartbeat status.
///
/// This is a pure function — it doesn't start a background task.
/// The caller (kernel) can run this periodically or in a background task.
pub fn check_agents(
    registry: &AgentRegistry,
    config: &HeartbeatConfig,
    turn_phases: &DashMap<AgentId, AgentLoopPhaseState>,
    turn_watchdog: &TurnWatchdogSettings,
) -> Vec<HeartbeatStatus> {
    let now = Utc::now();
    let mut statuses = Vec::new();

    // Drop phase tracking for agents that are not actively Running.
    turn_phases.retain(|id, _| {
        registry
            .get(*id)
            .is_some_and(|e| e.state == AgentState::Running)
    });

    for entry_ref in registry.list() {
        // Check Running agents (for unresponsiveness) and Crashed agents (for recovery)
        match entry_ref.state {
            AgentState::Running | AgentState::Crashed => {}
            _ => continue,
        }

        let inactive_secs = (now - entry_ref.last_active).num_seconds();

        // --- Stuck mid-turn (applies to Running + reactive and non-reactive) ---
        if entry_ref.state == AgentState::Running {
            if let Some(stall_secs) =
                turn_phase_stall_secs(turn_phases, turn_watchdog, entry_ref.id, now)
            {
                debug!(
                    agent = %entry_ref.name,
                    stall_secs,
                    "Agent stuck in loop phase — exceeds turn_watchdog limit"
                );
                statuses.push(HeartbeatStatus {
                    agent_id: entry_ref.id,
                    name: entry_ref.name.clone(),
                    inactive_secs: stall_secs,
                    unresponsive: true,
                    state: entry_ref.state,
                });
                continue;
            }
        }

        // --- Reactive: optional idle-between-turns monitoring ---
        if entry_ref.state == AgentState::Running
            && matches!(entry_ref.manifest.schedule, ScheduleMode::Reactive)
        {
            match config.reactive_idle_timeout_secs {
                None => {
                    debug!(
                        agent = %entry_ref.name,
                        "Skipping heartbeat inactivity check — reactive schedule (no reactive_idle_timeout_secs)"
                    );
                    continue;
                }
                Some(timeout_secs) => {
                    let never_active = (entry_ref.last_active - entry_ref.created_at).num_seconds()
                        <= IDLE_GRACE_SECS;
                    if never_active {
                        debug!(
                            agent = %entry_ref.name,
                            inactive_secs,
                            "Skipping idle reactive agent — never received a message"
                        );
                        continue;
                    }
                    let timeout_i = timeout_secs as i64;
                    let unresponsive = inactive_secs > timeout_i;
                    if unresponsive {
                        debug!(
                            agent = %entry_ref.name,
                            inactive_secs,
                            timeout_secs,
                            "Reactive agent idle beyond reactive_idle_timeout_secs"
                        );
                    } else {
                        debug!(
                            agent = %entry_ref.name,
                            inactive_secs,
                            "Reactive agent heartbeat OK (opt-in idle limit)"
                        );
                    }
                    statuses.push(HeartbeatStatus {
                        agent_id: entry_ref.id,
                        name: entry_ref.name.clone(),
                        inactive_secs,
                        unresponsive,
                        state: entry_ref.state,
                    });
                    continue;
                }
            }
        }

        // Determine timeout: autonomous heartbeat interval is a *polling* hint, not max LLM latency.
        // Never go below the global default (reactive agents routinely block for minutes on tools/LLM).
        let mut timeout_secs = entry_ref
            .manifest
            .autonomous
            .as_ref()
            .map(|a| a.heartbeat_interval_secs * UNRESPONSIVE_MULTIPLIER)
            .unwrap_or(config.default_timeout_secs) as i64;
        timeout_secs = timeout_secs.max(config.default_timeout_secs as i64);

        // Scheduled / proactive agents may be idle a long time between ticks; raise the floor.
        let schedule_floor: i64 = match &entry_ref.manifest.schedule {
            ScheduleMode::Reactive => 0,
            ScheduleMode::Continuous {
                check_interval_secs,
            } => (*check_interval_secs as i64)
                .saturating_mul(2)
                .saturating_add(120),
            ScheduleMode::Periodic { .. } | ScheduleMode::Proactive { .. } => {
                (config.default_timeout_secs as i64)
                    .saturating_mul(10)
                    .max(900)
            }
        };
        timeout_secs = timeout_secs.max(schedule_floor);

        // --- Skip idle agents that have never genuinely processed a message ---
        //
        // When an agent is spawned, both `created_at` and `last_active` are set
        // to now.  Administrative operations (set_state, etc.) bump `last_active`
        // by a tiny amount.  If `last_active` is still within IDLE_GRACE_SECS of
        // `created_at`, the agent was never active beyond its initial startup and
        // should NOT be flagged as unresponsive.  This prevents disabled/unused
        // agents from entering an infinite crash-recover loop (GitHub #844).
        //
        // Periodic / Hand agents with long schedule intervals (e.g. 3600s) are
        // also covered: they sit idle between ticks and their `last_active` stays
        // near `created_at` until the first tick fires.
        let never_active =
            (entry_ref.last_active - entry_ref.created_at).num_seconds() <= IDLE_GRACE_SECS;

        if never_active && entry_ref.state == AgentState::Running {
            debug!(
                agent = %entry_ref.name,
                inactive_secs,
                "Skipping idle agent — never received a message"
            );
            continue;
        }

        // Crashed agents are always considered unresponsive
        let mut unresponsive =
            entry_ref.state == AgentState::Crashed || inactive_secs > timeout_secs;

        // Mid-turn: `last_active` is stamped once per loop iteration (before the LLM call).
        // Long single turns can exceed the schedule-based inactivity floor while still healthy.
        // Stall detection for those turns is handled by `turn_phase_stall_secs` above.
        if unresponsive
            && entry_ref.state == AgentState::Running
            && turn_watchdog.enabled
            && turn_phases.contains_key(&entry_ref.id)
        {
            unresponsive = false;
        }

        if unresponsive && entry_ref.state == AgentState::Running {
            debug!(
                agent = %entry_ref.name,
                inactive_secs,
                timeout_secs,
                "Agent is unresponsive (last_active heuristic)"
            );
        } else if entry_ref.state == AgentState::Crashed {
            debug!(
                agent = %entry_ref.name,
                inactive_secs,
                "Agent is crashed — eligible for recovery"
            );
        } else {
            debug!(
                agent = %entry_ref.name,
                inactive_secs,
                "Agent heartbeat OK"
            );
        }

        statuses.push(HeartbeatStatus {
            agent_id: entry_ref.id,
            name: entry_ref.name.clone(),
            inactive_secs,
            unresponsive,
            state: entry_ref.state,
        });
    }

    statuses
}

/// Check if an agent is currently within its quiet hours.
///
/// Quiet hours format: "HH:MM-HH:MM" (24-hour format, UTC).
/// Returns true if the current time falls within the quiet period.
pub fn is_quiet_hours(quiet_hours: &str) -> bool {
    let parts: Vec<&str> = quiet_hours.split('-').collect();
    if parts.len() != 2 {
        return false;
    }

    let now = Utc::now();
    let current_minutes = now.format("%H").to_string().parse::<u32>().unwrap_or(0) * 60
        + now.format("%M").to_string().parse::<u32>().unwrap_or(0);

    let parse_time = |s: &str| -> Option<u32> {
        let hm: Vec<&str> = s.trim().split(':').collect();
        if hm.len() != 2 {
            return None;
        }
        let h = hm[0].parse::<u32>().ok()?;
        let m = hm[1].parse::<u32>().ok()?;
        if h > 23 || m > 59 {
            return None;
        }
        Some(h * 60 + m)
    };

    let start = match parse_time(parts[0]) {
        Some(v) => v,
        None => return false,
    };
    let end = match parse_time(parts[1]) {
        Some(v) => v,
        None => return false,
    };

    if start <= end {
        // Same-day range: e.g., 22:00-06:00 would be cross-midnight
        // This is start <= current < end
        current_minutes >= start && current_minutes < end
    } else {
        // Cross-midnight: e.g., 22:00-06:00
        current_minutes >= start || current_minutes < end
    }
}

/// Aggregate heartbeat summary.
#[derive(Debug, Clone, Default)]
pub struct HeartbeatSummary {
    /// Total agents checked.
    pub total_checked: usize,
    /// Number of responsive agents.
    pub responsive: usize,
    /// Number of unresponsive agents.
    pub unresponsive: usize,
    /// Details of unresponsive agents.
    pub unresponsive_agents: Vec<HeartbeatStatus>,
}

/// Produce a summary from heartbeat statuses.
pub fn summarize(statuses: &[HeartbeatStatus]) -> HeartbeatSummary {
    let unresponsive_agents: Vec<HeartbeatStatus> = statuses
        .iter()
        .filter(|s| s.unresponsive)
        .cloned()
        .collect();

    HeartbeatSummary {
        total_checked: statuses.len(),
        responsive: statuses.len() - unresponsive_agents.len(),
        unresponsive: unresponsive_agents.len(),
        unresponsive_agents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use openfang_types::agent::*;
    use openfang_types::config::TurnWatchdogSettings;
    use std::collections::HashMap;

    fn check_agents_no_phases(registry: &AgentRegistry, config: &HeartbeatConfig) -> Vec<HeartbeatStatus> {
        let phases = DashMap::new();
        let tw = TurnWatchdogSettings {
            enabled: false,
            ..Default::default()
        };
        check_agents(registry, config, &phases, &tw)
    }

    /// Helper: build a minimal AgentEntry for heartbeat tests.
    fn make_entry(
        name: &str,
        state: AgentState,
        created_at: chrono::DateTime<Utc>,
        last_active: chrono::DateTime<Utc>,
    ) -> AgentEntry {
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
            },
            state,
            mode: AgentMode::default(),
            created_at,
            last_active,
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
    fn test_idle_agent_skipped_by_heartbeat() {
        // An agent spawned 5 minutes ago that has never processed a message
        // (last_active == created_at). It should NOT appear in heartbeat
        // statuses because it was never genuinely active.
        let registry = crate::registry::AgentRegistry::new();
        let five_min_ago = Utc::now() - Duration::seconds(300);
        let idle_agent = make_entry(
            "idle-agent",
            AgentState::Running,
            five_min_ago,
            five_min_ago,
        );
        registry.register(idle_agent).unwrap();

        let config = HeartbeatConfig::default(); // timeout = 180s
        let statuses = check_agents_no_phases(&registry, &config);

        // The idle agent should be skipped entirely
        assert!(
            statuses.is_empty(),
            "idle agent should be skipped by heartbeat"
        );
    }

    #[test]
    fn test_autonomous_short_heartbeat_uses_global_idle_floor() {
        // heartbeat_interval_secs * 2 would be 60s — must not beat the global default (180s).
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let two_min_ago = Utc::now() - Duration::seconds(120);
        let mut agent = make_entry("auto-floor", AgentState::Running, ten_min_ago, two_min_ago);
        agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 60,
        };
        agent.manifest.autonomous = Some(AutonomousConfig {
            heartbeat_interval_secs: 30,
            ..Default::default()
        });
        registry.register(agent).unwrap();

        let config = HeartbeatConfig::default();
        let statuses = check_agents_no_phases(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            !statuses[0].unresponsive,
            "120s idle should stay within the 180s global floor"
        );
    }

    #[test]
    fn test_reactive_running_idle_not_flagged() {
        // Default schedule is Reactive: after real activity, long idle is still OK.
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let five_min_ago = Utc::now() - Duration::seconds(300);
        let agent = make_entry(
            "reactive-chat",
            AgentState::Running,
            ten_min_ago,
            five_min_ago,
        );
        registry.register(agent).unwrap();

        let config = HeartbeatConfig::default();
        let statuses = check_agents_no_phases(&registry, &config);

        assert!(
            statuses.is_empty(),
            "reactive agent must not be checked on idle time"
        );
    }

    #[test]
    fn test_periodic_schedule_allows_long_idle() {
        let registry = crate::registry::AgentRegistry::new();
        let hour_ago = Utc::now() - Duration::seconds(3600);
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let mut agent = make_entry("cronny", AgentState::Running, hour_ago, ten_min_ago);
        agent.manifest.schedule = ScheduleMode::Periodic {
            cron: "0 * * * *".to_string(),
        };
        registry.register(agent).unwrap();

        let config = HeartbeatConfig::default();
        let statuses = check_agents_no_phases(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            !statuses[0].unresponsive,
            "periodic agents should tolerate long idle between ticks"
        );
    }

    #[test]
    fn test_active_agent_detected_unresponsive() {
        // An agent that WAS active (last_active >> created_at) but has gone
        // silent for longer than the timeout — should be flagged unresponsive.
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let five_min_ago = Utc::now() - Duration::seconds(300);
        let mut active_agent = make_entry(
            "active-agent",
            AgentState::Running,
            ten_min_ago,
            five_min_ago,
        );
        active_agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 60,
        };
        registry.register(active_agent).unwrap();

        let config = HeartbeatConfig::default(); // timeout = 180s, inactive = ~300s
        let statuses = check_agents_no_phases(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0].unresponsive,
            "active agent past timeout should be unresponsive"
        );
    }

    #[test]
    fn test_active_agent_within_timeout_is_ok() {
        // An agent that has been active recently (within timeout).
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let just_now = Utc::now() - Duration::seconds(10);
        let mut healthy_agent =
            make_entry("healthy-agent", AgentState::Running, ten_min_ago, just_now);
        healthy_agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 60,
        };
        registry.register(healthy_agent).unwrap();

        let config = HeartbeatConfig::default(); // timeout = 180s
        let statuses = check_agents_no_phases(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            !statuses[0].unresponsive,
            "recently active agent should not be unresponsive"
        );
    }

    #[test]
    fn test_crashed_agent_not_skipped_even_if_idle() {
        // A crashed agent should still appear in statuses for recovery,
        // even if it was never genuinely active.
        let registry = crate::registry::AgentRegistry::new();
        let five_min_ago = Utc::now() - Duration::seconds(300);
        let crashed_agent = make_entry(
            "crashed-idle",
            AgentState::Crashed,
            five_min_ago,
            five_min_ago,
        );
        registry.register(crashed_agent).unwrap();

        let config = HeartbeatConfig::default();
        let statuses = check_agents_no_phases(&registry, &config);

        assert_eq!(statuses.len(), 1);
        assert!(
            statuses[0].unresponsive,
            "crashed agent should be marked unresponsive"
        );
    }

    #[test]
    fn test_turn_watchdog_stall_triggers() {
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let mut agent = make_entry("stally", AgentState::Running, ten_min_ago, ten_min_ago);
        agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 60,
        };
        let id = agent.id;
        registry.register(agent).unwrap();

        let phases = DashMap::new();
        phases.insert(
            id,
            AgentLoopPhaseState {
                kind: MonitoredLoopPhase::Thinking,
                since: Utc::now() - Duration::seconds(500),
            },
        );

        let config = HeartbeatConfig::default();
        let tw = TurnWatchdogSettings {
            enabled: true,
            thinking_secs: 60,
            tool_use_secs: 7200,
            streaming_secs: 1800,
        };
        let statuses = check_agents(&registry, &config, &phases, &tw);
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].unresponsive);
    }

    #[test]
    fn test_turn_watchdog_disabled_ignores_stall() {
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let just_now = Utc::now() - Duration::seconds(5);
        let mut agent = make_entry("no-tw", AgentState::Running, ten_min_ago, just_now);
        agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 60,
        };
        let id = agent.id;
        registry.register(agent).unwrap();

        let phases = DashMap::new();
        phases.insert(
            id,
            AgentLoopPhaseState {
                kind: MonitoredLoopPhase::Thinking,
                since: Utc::now() - Duration::seconds(500),
            },
        );

        let config = HeartbeatConfig::default();
        let tw = TurnWatchdogSettings {
            enabled: false,
            thinking_secs: 60,
            tool_use_secs: 7200,
            streaming_secs: 1800,
        };
        let statuses = check_agents(&registry, &config, &phases, &tw);
        assert_eq!(statuses.len(), 1);
        assert!(
            !statuses[0].unresponsive,
            "disabled turn_watchdog must not flag stall"
        );
    }

    #[test]
    fn test_reactive_idle_opt_in_unresponsive() {
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let five_min_ago = Utc::now() - Duration::seconds(300);
        let agent = make_entry(
            "react-opt",
            AgentState::Running,
            ten_min_ago,
            five_min_ago,
        );
        registry.register(agent).unwrap();

        let config = HeartbeatConfig {
            reactive_idle_timeout_secs: Some(120),
            ..Default::default()
        };

        let phases = DashMap::new();
        let tw = TurnWatchdogSettings {
            enabled: false,
            ..Default::default()
        };
        let statuses = check_agents(&registry, &config, &phases, &tw);
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].unresponsive);
    }

    #[test]
    fn test_quiet_hours_parsing() {
        // We can't easily test time-dependent logic, but we can test format parsing
        assert!(!is_quiet_hours("invalid"));
        assert!(!is_quiet_hours(""));
        assert!(!is_quiet_hours("25:00-06:00")); // Invalid hours handled gracefully
    }

    #[test]
    fn test_quiet_hours_format_valid() {
        // The function returns true/false based on current time
        // We just verify it doesn't panic on valid input
        let _ = is_quiet_hours("22:00-06:00");
        let _ = is_quiet_hours("00:00-23:59");
        let _ = is_quiet_hours("09:00-17:00");
    }

    #[test]
    fn test_heartbeat_config_default() {
        let config = HeartbeatConfig::default();
        assert_eq!(config.check_interval_secs, 30);
        assert_eq!(config.default_timeout_secs, 180);
    }

    #[test]
    fn test_summarize_empty() {
        let summary = summarize(&[]);
        assert_eq!(summary.total_checked, 0);
        assert_eq!(summary.responsive, 0);
        assert_eq!(summary.unresponsive, 0);
    }

    #[test]
    fn test_summarize_mixed() {
        let statuses = vec![
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-1".to_string(),
                inactive_secs: 10,
                unresponsive: false,
                state: AgentState::Running,
            },
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-2".to_string(),
                inactive_secs: 120,
                unresponsive: true,
                state: AgentState::Running,
            },
            HeartbeatStatus {
                agent_id: AgentId::new(),
                name: "agent-3".to_string(),
                inactive_secs: 5,
                unresponsive: false,
                state: AgentState::Running,
            },
        ];

        let summary = summarize(&statuses);
        assert_eq!(summary.total_checked, 3);
        assert_eq!(summary.responsive, 2);
        assert_eq!(summary.unresponsive, 1);
        assert_eq!(summary.unresponsive_agents.len(), 1);
        assert_eq!(summary.unresponsive_agents[0].name, "agent-2");
    }

    #[test]
    fn test_heartbeat_config_custom_timeout() {
        let config = HeartbeatConfig {
            default_timeout_secs: 600,
            ..HeartbeatConfig::default()
        };
        assert_eq!(config.default_timeout_secs, 600);
        assert_eq!(config.check_interval_secs, DEFAULT_CHECK_INTERVAL_SECS);
        assert_eq!(config.max_recovery_attempts, DEFAULT_MAX_RECOVERY_ATTEMPTS);
    }

    #[test]
    fn test_mid_turn_inactivity_ignored_when_watchdog_enabled() {
        let registry = crate::registry::AgentRegistry::new();
        let ten_min_ago = Utc::now() - Duration::seconds(600);
        let last_active = Utc::now() - Duration::seconds(400);
        let mut agent = make_entry("mid-turn", AgentState::Running, ten_min_ago, last_active);
        agent.manifest.schedule = ScheduleMode::Continuous {
            check_interval_secs: 120,
        };
        let id = agent.id;
        registry.register(agent).unwrap();

        let phases = DashMap::new();
        phases.insert(
            id,
            AgentLoopPhaseState {
                kind: MonitoredLoopPhase::Thinking,
                since: Utc::now() - Duration::seconds(400),
            },
        );

        let config = HeartbeatConfig::default();
        let tw = TurnWatchdogSettings {
            enabled: true,
            thinking_secs: 900,
            ..Default::default()
        };
        let statuses = check_agents(&registry, &config, &phases, &tw);
        assert_eq!(statuses.len(), 1);
        assert!(
            !statuses[0].unresponsive,
            "long in-LLM wall time must not use last_active while Thinking and under turn_watchdog"
        );
    }

    #[test]
    fn test_failure_notify_gate_coalesces() {
        let gate = FailureNotifyGate::new(60);
        let id = AgentId::new();
        assert!(gate.allow_notify(id));
        assert!(!gate.allow_notify(id));
        gate.clear(id);
        assert!(gate.allow_notify(id));
    }

    #[test]
    fn test_recovery_tracker() {
        let tracker = RecoveryTracker::new();
        let agent_id = AgentId::new();

        assert_eq!(tracker.failure_count(agent_id), 0);
        assert!(tracker.can_attempt(agent_id, 60));

        let attempt = tracker.record_attempt(agent_id);
        assert_eq!(attempt, 1);
        assert_eq!(tracker.failure_count(agent_id), 1);

        // Just recorded — cooldown should block (unless cooldown is 0)
        assert!(!tracker.can_attempt(agent_id, 60));
        assert!(tracker.can_attempt(agent_id, 0));

        let attempt = tracker.record_attempt(agent_id);
        assert_eq!(attempt, 2);

        tracker.reset(agent_id);
        assert_eq!(tracker.failure_count(agent_id), 0);
    }
}
