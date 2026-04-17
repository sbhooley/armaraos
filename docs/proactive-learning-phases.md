# Proactive Learning & Assistance - Phased Implementation Plan

**Version:** 1.0  
**Date:** 2026-04-11  
**Parent Document:** `docs/proactive-learning-design.md`

## Overview

This document breaks down the proactive learning and assistance system into **non-breaking, independently deployable phases**. Each phase delivers value on its own while respecting user privacy and maintaining backward compatibility.

**Guiding Principles:**
- Privacy-first: User controls come early
- Passive before active: Observe before suggesting
- Simple before complex: Basic patterns before ML
- Opt-out friendly: Easy to disable at any level
- Transparent: Always explain why/how

---

## Phase 1: Event Collection & Privacy Foundation (Week 1-2)

**Goal:** Establish data collection infrastructure with privacy controls, without any user-facing changes.

### Deliverables

#### 1.1 Extended Event Types

**File:** `crates/openfang-types/src/event.rs`

Add new `SystemEvent` variants:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    // ...existing variants...
    
    /// User performed an activity in the system
    UserActivity {
        user_id: String,
        activity_type: UserActivityType,
        context: HashMap<String, serde_json::Value>,
        duration_ms: Option<u64>,
    },
    
    /// User completed a goal or task
    GoalCompleted {
        user_id: String,
        goal_type: String,
        success: bool,
        metadata: HashMap<String, serde_json::Value>,
    },
    
    /// User requested help or showed confusion
    HelpRequested {
        user_id: String,
        context: String,
        help_type: HelpType,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum UserActivityType {
    PageView { page: String },
    Click { element: String },
    CommandRun { command: String, args: Vec<String> },
    AgentMessage { agent_id: AgentId, message_length: usize },
    ResourceEdit { resource_type: String, action: EditAction },
    Search { query: String, scope: SearchScope },
    DocView { doc_path: String, duration_secs: u64 },
    FeatureUse { feature: String },
    SettingChange { setting: String, old_value: Option<String>, new_value: String },
}

// Supporting enums: EditAction, SearchScope, HelpType
```

**Backward Compatibility:**
- New enum variants added (existing code unaffected)
- `#[non_exhaustive]` on enums ensures future additions are safe

#### 1.2 Privacy Configuration

**File:** `crates/openfang-types/src/config.rs`

Add privacy settings to `KernelConfig`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KernelConfig {
    // ...existing fields...
    
    /// Privacy and learning settings
    #[serde(default)]
    pub privacy: PrivacyConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Enable activity tracking for learning
    pub enable_tracking: bool,
    
    /// Anonymize user identifiers in learning data
    pub anonymize_user_id: bool,
    
    /// Redact sensitive data (file paths, message content)
    pub redact_sensitive_data: bool,
    
    /// Data retention period in days (0 = forever)
    pub retention_days: u32,
    
    /// Enable proactive suggestions
    pub enable_suggestions: bool,
    
    /// Suggestion frequency
    pub suggestion_frequency: SuggestionFrequency,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            enable_tracking: true,        // On by default, user can opt-out
            anonymize_user_id: false,     // Keep real IDs by default
            redact_sensitive_data: true,  // Safe by default
            retention_days: 90,            // 3 months default
            enable_suggestions: false,     // Start conservative (off)
            suggestion_frequency: SuggestionFrequency::Medium,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionFrequency {
    Never,
    Low,      // Once per day max
    Medium,   // Few times per day
    High,     // Many times per day
}
```

**Backward Compatibility:**
- `#[serde(default)]` ensures missing fields use defaults
- Old config files load without errors
- Privacy defaults are conservative

#### 1.3 Event Emission Points

**File:** `crates/openfang-api/src/routes.rs`

Emit `UserActivity` events at key interaction points:

```rust
// In message handler
async fn handle_agent_message(...) -> Result<...> {
    // Emit UserActivity event
    if kernel.config().privacy.enable_tracking {
        let user_id = extract_user_id(&req).unwrap_or("anonymous".to_string());
        kernel.event_bus.publish(Event::new(
            AgentId::new(), // System source
            EventTarget::System,
            EventPayload::System(SystemEvent::UserActivity {
                user_id,
                activity_type: UserActivityType::AgentMessage {
                    agent_id,
                    message_length: message.len(),
                },
                context: serde_json::json!({
                    "endpoint": "/api/agents/:id/message",
                    "timestamp": Utc::now(),
                }),
                duration_ms: None,
            }),
        ));
    }
    
    // Existing message handling code...
}

// Similar emissions in:
// - Workflow creation/execution
// - Agent spawn/kill
// - Settings changes
// - Dashboard page views (via frontend JS)
```

**File:** `crates/openfang-api/static/js/activity-tracker.js` (new file)

Client-side activity tracking:

```javascript
// Activity tracker for dashboard interactions
class ActivityTracker {
    constructor() {
        this.enabled = false;
        this.sessionId = null;
        this.eventBuffer = [];
        this.flushInterval = 10000; // 10 seconds
    }
    
    async init() {
        // Check if tracking enabled via API
        const privacy = await fetch('/api/privacy').then(r => r.json());
        this.enabled = privacy.enable_tracking;
        
        if (!this.enabled) return;
        
        this.sessionId = this.generateSessionId();
        this.setupListeners();
        this.startPeriodicFlush();
    }
    
    setupListeners() {
        // Track page views
        window.addEventListener('hashchange', () => {
            this.track('PageView', { page: window.location.hash });
        });
        
        // Track button clicks (with data-track attribute)
        document.addEventListener('click', (e) => {
            if (e.target.dataset.track) {
                this.track('Click', { element: e.target.dataset.track });
            }
        });
        
        // Track search
        document.getElementById('global-search')?.addEventListener('input', debounce((e) => {
            if (e.target.value.length > 2) {
                this.track('Search', { query: this.redact(e.target.value), scope: 'global' });
            }
        }, 1000));
    }
    
    track(activityType, data) {
        this.eventBuffer.push({
            activity_type: activityType,
            data,
            timestamp: new Date().toISOString(),
        });
    }
    
    async flush() {
        if (this.eventBuffer.length === 0) return;
        
        await fetch('/api/learning/events', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                session_id: this.sessionId,
                events: this.eventBuffer,
            }),
        });
        
        this.eventBuffer = [];
    }
    
    redact(text) {
        // Redact sensitive data from search queries, etc.
        return text.replace(/[a-f0-9]{32,}/gi, '<REDACTED>');
    }
}

// Initialize on page load (only if privacy settings allow)
const tracker = new ActivityTracker();
tracker.init();
```

#### 1.4 Privacy API Endpoints

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/privacy - Get privacy settings
async fn get_privacy_settings(State(state): State<AppState>) -> Result<Json<PrivacyConfig>, StatusCode> {
    Ok(Json(state.kernel.config().privacy.clone()))
}

// PUT /api/privacy - Update privacy settings
async fn update_privacy_settings(
    State(state): State<AppState>,
    Json(updated): Json<PrivacyConfig>,
) -> Result<Json<PrivacyConfig>, StatusCode> {
    // Validate settings
    if updated.retention_days > 3650 {
        return Err(StatusCode::BAD_REQUEST); // Max 10 years
    }
    
    // Update config (via kernel)
    state.kernel.update_privacy_config(updated.clone())?;
    
    Ok(Json(updated))
}

// POST /api/privacy/export - Export all learning data
async fn export_learning_data(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Export all patterns, profiles, insights as JSON
    let data = state.kernel.export_learning_data().await?;
    Ok(Json(data))
}

// POST /api/privacy/delete - Delete all learning data
async fn delete_learning_data(
    State(state): State<AppState>,
) -> Result<StatusCode, StatusCode> {
    // Delete all patterns, profiles, insights
    state.kernel.delete_learning_data().await?;
    Ok(StatusCode::OK)
}
```

### Testing

```bash
# New event types compile
cargo test -p openfang-types event

# Privacy config loads and validates
cargo test -p openfang-types privacy_config

# Events emitted at key points
cargo test -p openfang-api test_activity_events

# Privacy API endpoints
cargo test -p openfang-api test_privacy_api

# All existing tests pass
cargo test --workspace
```

### Success Criteria

- [ ] New event types defined and documented
- [ ] Privacy config loads from TOML with safe defaults
- [ ] Events emitted at 10+ key interaction points
- [ ] Client-side tracker respects privacy settings
- [ ] Privacy API endpoints functional
- [ ] All 2,793+ existing tests pass
- [ ] Can deploy (tracking disabled by default for new features)

**Deployment Note:** This phase is pure infrastructure. Users see no changes unless they explicitly enable tracking in settings.

---

## Phase 2: Event Storage & Anonymization (Week 3)

**Goal:** Store activity events efficiently with privacy-preserving anonymization.

### Deliverables

#### 2.1 Activity Event Storage

**Database Schema:**

```sql
-- New table: activity_events
CREATE TABLE IF NOT EXISTS activity_events (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL, -- May be anonymized hash
    activity_type TEXT NOT NULL,
    activity_data TEXT NOT NULL, -- JSON
    context TEXT, -- JSON
    duration_ms INTEGER,
    timestamp TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_activity_user ON activity_events(user_id);
CREATE INDEX idx_activity_type ON activity_events(activity_type);
CREATE INDEX idx_activity_timestamp ON activity_events(timestamp DESC);
CREATE INDEX idx_activity_user_time ON activity_events(user_id, timestamp DESC);
```

**File:** `crates/openfang-memory/src/activity.rs` (new file)

```rust
use crate::MemorySqlitePool;
use chrono::Utc;
use openfang_types::event::UserActivityType;
use serde_json::Value;

#[derive(Clone)]
pub struct ActivityStore {
    pool: MemorySqlitePool,
}

impl ActivityStore {
    pub fn new(pool: MemorySqlitePool) -> Self {
        Self { pool }
    }
    
    /// Store an activity event
    pub fn store_event(
        &self,
        user_id: &str,
        activity_type: UserActivityType,
        context: Value,
        duration_ms: Option<u64>,
    ) -> OpenFangResult<String> {
        let conn = self.pool.get()?;
        let id = Uuid::new_v4().to_string();
        let activity_type_str = serde_json::to_string(&activity_type)?;
        let context_str = serde_json::to_string(&context)?;
        let now = Utc::now().to_rfc3339();
        
        conn.execute(
            "INSERT INTO activity_events (id, user_id, activity_type, activity_data, context, duration_ms, timestamp, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![
                id,
                user_id,
                activity_type_str,
                activity_type_str, // activity_data same as type for now
                context_str,
                duration_ms.map(|d| d as i64),
                now,
            ],
        )?;
        
        Ok(id)
    }
    
    /// Get recent events for a user
    pub fn get_recent_events(
        &self,
        user_id: &str,
        limit: usize,
    ) -> OpenFangResult<Vec<ActivityEvent>> {
        // Query and deserialize events
        todo!()
    }
    
    /// Delete events older than retention period
    pub fn cleanup_old_events(&self, retention_days: u32) -> OpenFangResult<u64> {
        let conn = self.pool.get()?;
        let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        
        let deleted = conn.execute(
            "DELETE FROM activity_events WHERE timestamp < ?1",
            params![cutoff_str],
        )?;
        
        Ok(deleted as u64)
    }
}

#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub id: String,
    pub user_id: String,
    pub activity_type: UserActivityType,
    pub context: Value,
    pub duration_ms: Option<u64>,
    pub timestamp: DateTime<Utc>,
}
```

#### 2.2 Anonymization Service

**File:** `crates/openfang-learning/src/anonymization.rs` (new crate)

```rust
use sha2::{Sha256, Digest};

pub struct AnonymizationService {
    /// Salt for hashing (loaded from config)
    salt: String,
}

impl AnonymizationService {
    pub fn new(salt: String) -> Self {
        Self { salt }
    }
    
    /// Anonymize a user ID to a consistent hash
    pub fn anonymize_user_id(&self, user_id: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{}", self.salt, user_id).as_bytes());
        let result = hasher.finalize();
        format!("anon_{}", hex::encode(&result[..16]))
    }
    
    /// Redact sensitive data from activity data
    pub fn redact_activity(&self, activity: &mut UserActivityType) {
        match activity {
            UserActivityType::Search { query, .. } => {
                // Redact long hex strings (likely UUIDs)
                *query = self.redact_patterns(query);
            }
            UserActivityType::CommandRun { args, .. } => {
                // Redact file paths, secrets
                *args = args.iter().map(|a| self.redact_patterns(a)).collect();
            }
            UserActivityType::ResourceEdit { .. } => {
                // Redact resource names if they look sensitive
            }
            _ => {}
        }
    }
    
    fn redact_patterns(&self, text: &str) -> String {
        let mut redacted = text.to_string();
        
        // Redact UUIDs
        redacted = regex::Regex::new(r"[a-f0-9]{8}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{4}-[a-f0-9]{12}")
            .unwrap()
            .replace_all(&redacted, "<UUID>")
            .to_string();
        
        // Redact file paths
        redacted = regex::Regex::new(r"/[a-zA-Z0-9_/.-]+")
            .unwrap()
            .replace_all(&redacted, "<PATH>")
            .to_string();
        
        // Redact API keys/tokens
        redacted = regex::Regex::new(r"[a-zA-Z0-9_-]{32,}")
            .unwrap()
            .replace_all(&redacted, "<TOKEN>")
            .to_string();
        
        redacted
    }
}
```

#### 2.3 Event Collector Service

**File:** `crates/openfang-learning/src/event_collector.rs`

```rust
pub struct EventCollector {
    activity_store: Arc<ActivityStore>,
    anonymization: Arc<AnonymizationService>,
    privacy_config: Arc<RwLock<PrivacyConfig>>,
}

impl EventCollector {
    /// Process and store an activity event
    pub async fn collect_event(
        &self,
        user_id: &str,
        mut activity_type: UserActivityType,
        context: Value,
        duration_ms: Option<u64>,
    ) -> OpenFangResult<()> {
        let config = self.privacy_config.read().await;
        
        // Check if tracking enabled
        if !config.enable_tracking {
            return Ok(()); // Silent no-op
        }
        
        // Anonymize user ID if configured
        let user_id = if config.anonymize_user_id {
            self.anonymization.anonymize_user_id(user_id)
        } else {
            user_id.to_string()
        };
        
        // Redact sensitive data if configured
        if config.redact_sensitive_data {
            self.anonymization.redact_activity(&mut activity_type);
        }
        
        // Store event
        self.activity_store.store_event(&user_id, activity_type, context, duration_ms)?;
        
        Ok(())
    }
}
```

#### 2.4 Retention Enforcement (Background Job)

**File:** `crates/openfang-kernel/src/learning_maintenance.rs` (new file)

```rust
pub struct LearningMaintenance {
    activity_store: Arc<ActivityStore>,
    privacy_config: Arc<RwLock<PrivacyConfig>>,
}

impl LearningMaintenance {
    /// Background job: Clean up old events
    pub async fn run_cleanup_job(&self) {
        loop {
            // Run every 24 hours
            tokio::time::sleep(Duration::from_secs(86400)).await;
            
            let config = self.privacy_config.read().await;
            
            if config.retention_days > 0 {
                match self.activity_store.cleanup_old_events(config.retention_days) {
                    Ok(deleted) => {
                        info!("Cleaned up {} old activity events", deleted);
                    }
                    Err(e) => {
                        warn!("Failed to clean up activity events: {}", e);
                    }
                }
            }
        }
    }
}
```

### Testing

```bash
# Activity storage tests
cargo test -p openfang-memory activity_store

# Anonymization tests
cargo test -p openfang-learning anonymization

# Retention cleanup tests
cargo test -p openfang-kernel learning_maintenance

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Activity events stored in SQLite
- [ ] Anonymization works correctly (consistent hashes)
- [ ] Sensitive data redaction functional
- [ ] Retention enforcement runs daily
- [ ] All existing tests pass
- [ ] Can deploy (data stored but not yet analyzed)

---

## Phase 3: Basic Pattern Recognition (Week 4-5)

**Goal:** Detect simple behavioral patterns (frequency, temporal, sequences) without ML.

### Deliverables

#### 3.1 Pattern Types & Storage

**File:** `crates/openfang-types/src/learning.rs` (new file)

```rust
// Complete BehaviorPattern, PatternType definitions from design doc
// Storage schema for behavior_patterns table
```

#### 3.2 Simple Pattern Detectors

**File:** `crates/openfang-learning/src/detectors/frequency.rs`

```rust
/// Detect actions that happen at regular intervals
pub struct FrequencyDetector {
    min_observations: u32,
}

impl FrequencyDetector {
    pub fn detect(&self, events: &[ActivityEvent]) -> Vec<BehaviorPattern> {
        // Group events by activity type
        // Calculate time intervals between occurrences
        // Find patterns with consistent frequency
        // Return patterns with confidence scores
        todo!()
    }
}
```

**File:** `crates/openfang-learning/src/detectors/temporal.rs`

```rust
/// Detect time-of-day and day-of-week patterns
pub struct TemporalDetector {
    bucket_duration: Duration,
}

impl TemporalDetector {
    pub fn detect(&self, events: &[ActivityEvent]) -> Vec<BehaviorPattern> {
        // Group events by hour-of-day and day-of-week
        // Find peak times (high frequency buckets)
        // Return temporal patterns
        todo!()
    }
}
```

**File:** `crates/openfang-learning/src/detectors/sequence.rs`

```rust
/// Detect action sequences (A often followed by B)
pub struct SequenceDetector {
    window_size: usize,
    min_occurrences: u32,
}

impl SequenceDetector {
    pub fn detect(&self, events: &[ActivityEvent]) -> Vec<BehaviorPattern> {
        // Build sliding windows of events
        // Count sequence occurrences
        // Calculate transition probabilities
        // Return high-probability sequences
        todo!()
    }
}
```

#### 3.3 Pattern Recognition Engine

**File:** `crates/openfang-learning/src/recognition_engine.rs`

```rust
pub struct PatternRecognitionEngine {
    frequency_detector: FrequencyDetector,
    temporal_detector: TemporalDetector,
    sequence_detector: SequenceDetector,
    
    activity_store: Arc<ActivityStore>,
    pattern_store: Arc<PatternStore>,
}

impl PatternRecognitionEngine {
    /// Run pattern detection for a user
    pub async fn analyze_user_patterns(&self, user_id: &str) -> OpenFangResult<Vec<BehaviorPattern>> {
        // Fetch recent events (last 30 days)
        let events = self.activity_store.get_user_events(user_id, 30)?;
        
        if events.len() < 10 {
            // Not enough data yet
            return Ok(vec![]);
        }
        
        // Run all detectors
        let freq_patterns = self.frequency_detector.detect(&events);
        let temporal_patterns = self.temporal_detector.detect(&events);
        let sequence_patterns = self.sequence_detector.detect(&events);
        
        // Combine and deduplicate
        let mut all_patterns = Vec::new();
        all_patterns.extend(freq_patterns);
        all_patterns.extend(temporal_patterns);
        all_patterns.extend(sequence_patterns);
        
        // Persist new patterns
        for pattern in &all_patterns {
            self.pattern_store.save_pattern(user_id, pattern)?;
        }
        
        Ok(all_patterns)
    }
    
    /// Background job: Analyze patterns for all active users
    pub async fn run_periodic_analysis(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await; // Every hour
            
            // Get active users (those with recent activity)
            let active_users = self.activity_store.get_active_users(7)?;
            
            for user_id in active_users {
                if let Err(e) = self.analyze_user_patterns(&user_id).await {
                    warn!("Failed to analyze patterns for {}: {}", user_id, e);
                }
            }
        }
    }
}
```

#### 3.4 Pattern API Endpoints

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/learning/patterns - List user's detected patterns
async fn list_patterns(
    State(state): State<AppState>,
) -> Result<Json<Vec<BehaviorPattern>>, StatusCode> {
    let user_id = extract_user_id().unwrap_or("anonymous".to_string());
    let patterns = state.kernel.get_user_patterns(&user_id).await?;
    Ok(Json(patterns))
}

// GET /api/learning/patterns/:id - Get pattern detail
async fn get_pattern(
    State(state): State<AppState>,
    Path(pattern_id): Path<String>,
) -> Result<Json<BehaviorPattern>, StatusCode> {
    let pattern = state.kernel.get_pattern(&pattern_id).await?;
    Ok(Json(pattern))
}
```

### Testing

```bash
# Detector unit tests
cargo test -p openfang-learning detectors

# Pattern recognition integration tests
cargo test -p openfang-learning recognition_engine

# API endpoint tests
cargo test -p openfang-api test_patterns_api

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Frequency patterns detected correctly (tested with synthetic data)
- [ ] Temporal patterns detect time-of-day peaks
- [ ] Sequence patterns find common action chains
- [ ] Patterns persisted to database
- [ ] API returns detected patterns
- [ ] All tests pass
- [ ] Can deploy (patterns detected but not yet surfaced to users)

---

## Phase 4: User Profile & Context Tracking (Week 6)

**Goal:** Build user profiles and track working context for contextual relevance.

### Deliverables

#### 4.1 User Profile Types

**File:** `crates/openfang-types/src/learning.rs`

```rust
// Complete UserProfile, SkillLevel, LearningStyle types from design doc
```

#### 4.2 Profile Builder Service

**File:** `crates/openfang-learning/src/profile_builder.rs`

```rust
pub struct ProfileBuilder {
    pattern_store: Arc<PatternStore>,
    activity_store: Arc<ActivityStore>,
    profile_store: Arc<ProfileStore>,
}

impl ProfileBuilder {
    /// Build or update user profile
    pub async fn update_profile(&self, user_id: &str) -> OpenFangResult<UserProfile> {
        let patterns = self.pattern_store.get_user_patterns(user_id)?;
        let events = self.activity_store.get_user_events(user_id, 90)?;
        
        // Infer skill level from patterns
        let skill_level = self.infer_skill_level(&patterns, &events);
        
        // Identify expertise areas (frequent successful actions)
        let expertise_areas = self.identify_expertise(&events);
        
        // Identify learning interests (struggled tasks, help requests)
        let learning_interests = self.identify_interests(&events);
        
        // Build profile
        let profile = UserProfile {
            user_id: user_id.to_string(),
            skill_level,
            expertise_areas,
            learning_interests,
            // ... other fields
        };
        
        // Persist
        self.profile_store.save_profile(&profile)?;
        
        Ok(profile)
    }
    
    fn infer_skill_level(&self, patterns: &[BehaviorPattern], events: &[ActivityEvent]) -> SkillLevel {
        // Count unique features used
        let feature_breadth = self.count_unique_features(events);
        
        // Count advanced feature usage
        let advanced_usage = self.count_advanced_features(events);
        
        // Calculate success rate
        let success_rate = self.calculate_success_rate(events);
        
        // Simple heuristic
        if feature_breadth > 50 && advanced_usage > 10 && success_rate > 0.9 {
            SkillLevel::Expert
        } else if feature_breadth > 30 && advanced_usage > 5 {
            SkillLevel::Advanced
        } else if feature_breadth > 15 {
            SkillLevel::Intermediate
        } else {
            SkillLevel::Beginner
        }
    }
}
```

#### 4.3 Working Context Tracker

**File:** `crates/openfang-learning/src/context_tracker.rs`

```rust
pub struct ContextTracker {
    contexts: Arc<RwLock<HashMap<String, WorkingContext>>>,
}

impl ContextTracker {
    /// Update working context based on recent activity
    pub async fn update_context(&self, user_id: &str, events: &[ActivityEvent]) {
        let mut contexts = self.contexts.write().await;
        
        let context = contexts.entry(user_id.to_string()).or_insert_with(|| {
            WorkingContext {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                current_task: None,
                related_entities: vec![],
                active_agents: vec![],
                recent_tools: vec![],
                started_at: Utc::now(),
                last_activity: Utc::now(),
                metadata: HashMap::new(),
            }
        });
        
        // Update based on recent events
        context.last_activity = Utc::now();
        
        // Infer current task from event sequences
        if let Some(task) = self.infer_current_task(events) {
            context.current_task = Some(task);
        }
        
        // Extract related entities (agents, projects)
        context.active_agents = self.extract_active_agents(events);
        context.recent_tools = self.extract_recent_tools(events);
    }
    
    /// Get current working context for a user
    pub async fn get_context(&self, user_id: &str) -> Option<WorkingContext> {
        self.contexts.read().await.get(user_id).cloned()
    }
}
```

#### 4.4 Profile & Context API

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/learning/profile - Get user profile
async fn get_profile(State(state): State<AppState>) -> Result<Json<UserProfile>, StatusCode> {
    let user_id = extract_user_id().unwrap_or("anonymous".to_string());
    let profile = state.kernel.get_user_profile(&user_id).await?;
    Ok(Json(profile))
}

// PUT /api/learning/profile - Update profile
async fn update_profile(
    State(state): State<AppState>,
    Json(updates): Json<UserProfileUpdate>,
) -> Result<Json<UserProfile>, StatusCode> {
    let user_id = extract_user_id().unwrap_or("anonymous".to_string());
    let profile = state.kernel.update_user_profile(&user_id, updates).await?;
    Ok(Json(profile))
}

// GET /api/learning/context - Get working context
async fn get_context(State(state): State<AppState>) -> Result<Json<WorkingContext>, StatusCode> {
    let user_id = extract_user_id().unwrap_or("anonymous".to_string());
    let context = state.kernel.get_working_context(&user_id).await?;
    Ok(Json(context))
}
```

### Testing

```bash
# Profile builder tests
cargo test -p openfang-learning profile_builder

# Context tracker tests
cargo test -p openfang-learning context_tracker

# API tests
cargo test -p openfang-api test_profile_api

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] User profiles generated from patterns
- [ ] Skill level inference works (tested with mock data)
- [ ] Working context tracked correctly
- [ ] Profile API functional
- [ ] All tests pass
- [ ] Can deploy (profiles built but not yet used for suggestions)

---

## Phase 5: Basic Insight Generation (Week 7-8)

**Goal:** Generate simple insights (automation opportunities, feature discovery) without advanced ML.

### Deliverables

#### 5.1 Insight Types

**File:** `crates/openfang-types/src/learning.rs`

```rust
// Complete Insight, InsightType definitions from design doc
```

#### 5.2 Simple Insight Generators

**File:** `crates/openfang-learning/src/generators/automation.rs`

```rust
pub struct AutomationGenerator;

impl AutomationGenerator {
    /// Find repetitive sequences that could be automated
    pub fn generate(&self, patterns: &[BehaviorPattern]) -> Vec<Insight> {
        let mut insights = vec![];
        
        for pattern in patterns {
            if let PatternType::Sequence { actions, typical_delay_secs } = &pattern.pattern_type {
                // Repetitive sequence with 3+ steps, seen 10+ times
                if actions.len() >= 3 && pattern.observation_count >= 10 {
                    let time_saved_per_week = self.estimate_time_saved(
                        pattern.observation_count,
                        actions.len(),
                        *typical_delay_secs,
                    );
                    
                    if time_saved_per_week >= 30 { // At least 30 min/week
                        insights.push(Insight {
                            id: Uuid::new_v4().to_string(),
                            user_id: pattern.user_id.clone(),
                            insight_type: InsightType::AutomationOpportunity {
                                task_description: format!("Repeated: {}", actions.join(" → ")),
                                estimated_time_saved_mins_per_week: time_saved_per_week,
                                suggested_approach: self.suggest_automation(actions),
                            },
                            confidence: pattern.confidence,
                            priority: self.calculate_priority(time_saved_per_week),
                            generated_at: Utc::now(),
                            expires_at: None,
                            evidence: vec![pattern.id.clone()],
                        });
                    }
                }
            }
        }
        
        insights
    }
    
    fn estimate_time_saved(&self, observations: u32, steps: usize, delay_secs: u64) -> u32 {
        // Rough estimate: (time per manual execution) * (frequency per week)
        let time_per_execution_mins = (steps as u64 * 30 + delay_secs) / 60; // 30s per step
        let executions_per_week = (observations as f32 / 30.0) * 7.0; // Scale to weekly
        (time_per_execution_mins as f32 * executions_per_week) as u32
    }
}
```

**File:** `crates/openfang-learning/src/generators/feature_discovery.rs`

```rust
pub struct FeatureDiscoveryGenerator {
    /// Map of features with their descriptions and benefits
    feature_catalog: HashMap<String, FeatureInfo>,
}

impl FeatureDiscoveryGenerator {
    pub fn generate(&self, profile: &UserProfile, context: &WorkingContext) -> Vec<Insight> {
        let mut insights = vec![];
        
        // For each underutilized feature
        for feature in &profile.underutilized_features {
            // Check relevance to current context
            if let Some(feature_info) = self.feature_catalog.get(feature) {
                if self.is_relevant_to_context(feature_info, context) {
                    insights.push(Insight {
                        insight_type: InsightType::FeatureDiscovery {
                            feature_name: feature.clone(),
                            relevance_explanation: self.explain_relevance(feature_info, context),
                            tutorial_link: Some(format!("/docs/features/{}", feature)),
                        },
                        priority: self.calculate_relevance_score(feature_info, context),
                        // ...
                    });
                }
            }
        }
        
        // Sort by relevance
        insights.sort_by(|a, b| b.priority.cmp(&a.priority));
        insights.truncate(5); // Top 5 suggestions
        
        insights
    }
}

struct FeatureInfo {
    name: String,
    description: String,
    category: String,
    keywords: Vec<String>,
    benefits: Vec<String>,
}
```

#### 5.3 Insight Engine

**File:** `crates/openfang-learning/src/insight_engine.rs`

```rust
pub struct InsightEngine {
    automation_gen: AutomationGenerator,
    feature_discovery: FeatureDiscoveryGenerator,
    
    pattern_store: Arc<PatternStore>,
    profile_store: Arc<ProfileStore>,
    context_tracker: Arc<ContextTracker>,
    insight_store: Arc<InsightStore>,
}

impl InsightEngine {
    /// Generate insights for a user
    pub async fn generate_insights(&self, user_id: &str) -> OpenFangResult<Vec<Insight>> {
        // Fetch inputs
        let patterns = self.pattern_store.get_user_patterns(user_id)?;
        let profile = self.profile_store.get_profile(user_id).await?;
        let context = self.context_tracker.get_context(user_id).await.unwrap_or_default();
        
        // Generate insights
        let mut all_insights = Vec::new();
        all_insights.extend(self.automation_gen.generate(&patterns));
        all_insights.extend(self.feature_discovery.generate(&profile, &context));
        
        // Rank by priority and confidence
        all_insights.sort_by(|a, b| {
            (b.priority, (b.confidence * 100.0) as u8)
                .cmp(&(a.priority, (a.confidence * 100.0) as u8))
        });
        
        // Store insights
        for insight in &all_insights {
            self.insight_store.save_insight(insight)?;
        }
        
        Ok(all_insights)
    }
    
    /// Background job: Generate insights periodically
    pub async fn run_periodic_generation(&self) {
        loop {
            tokio::time::sleep(Duration::from_secs(1800)).await; // Every 30 min
            
            let active_users = self.get_active_users().await;
            
            for user_id in active_users {
                if let Err(e) = self.generate_insights(&user_id).await {
                    warn!("Failed to generate insights for {}: {}", user_id, e);
                }
            }
        }
    }
}
```

#### 5.4 Insight API

**File:** `crates/openfang-api/src/routes.rs`

```rust
// GET /api/learning/insights - List pending insights
async fn list_insights(
    State(state): State<AppState>,
) -> Result<Json<Vec<Insight>>, StatusCode> {
    let user_id = extract_user_id().unwrap_or("anonymous".to_string());
    let insights = state.kernel.get_user_insights(&user_id, false).await?; // Not dismissed
    Ok(Json(insights))
}

// POST /api/learning/insights/:id/dismiss - Dismiss insight
async fn dismiss_insight(
    State(state): State<AppState>,
    Path(insight_id): Path<String>,
    Json(payload): Json<DismissPayload>,
) -> Result<StatusCode, StatusCode> {
    state.kernel.dismiss_insight(&insight_id, payload.reason).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct DismissPayload {
    reason: String,
}
```

### Testing

```bash
# Insight generators
cargo test -p openfang-learning generators

# Insight engine
cargo test -p openfang-learning insight_engine

# API
cargo test -p openfang-api test_insights_api

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Automation insights generated for repetitive sequences
- [ ] Feature discovery suggests relevant features
- [ ] Insights ranked by priority
- [ ] Insights API functional
- [ ] Dismiss mechanism works (with reason tracking)
- [ ] All tests pass
- [ ] Can deploy (insights generated but delivery still manual via API)

---

## Phase 6: Proactive Suggestion Delivery (Week 9-10)

**Goal:** Automatically deliver insights to users at the right time through dashboard notifications.

### Deliverables

#### 6.1 Proactive Assistant Agent

**File:** `~/.armaraos/agents/proactive-assistant.toml`

```toml
name = "proactive-assistant"
module = "builtin:proactive"
description = "Monitors user behavior and offers helpful suggestions"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = """
You are a proactive assistant. Monitor user patterns and suggest improvements.
Be concise, respectful, and only suggest when highly relevant.
"""

[capabilities]
tools = ["insight_get", "insight_dismiss", "context_get", "notification_send"]

[scheduling]
schedule = "*/10 * * * *"  # Every 10 minutes
only_when_active = true
```

#### 6.2 Proactive Assistant Tools

**File:** `crates/openfang-runtime/src/tool_runner.rs`

```rust
// tool: insight_get
ToolDefinition {
    name: "insight_get".to_string(),
    description: "Get pending insights for the user".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "min_priority": { "type": "integer", "description": "Minimum priority (0-10)" },
            "limit": { "type": "integer", "description": "Max insights to return" }
        }
    }),
}

// tool: notification_send
ToolDefinition {
    name: "notification_send".to_string(),
    description: "Send a notification to the user".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "title": { "type": "string" },
            "message": { "type": "string" },
            "insight_id": { "type": "string", "description": "Related insight ID" },
            "priority": { "type": "integer", "description": "0-10" },
            "actions": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "label": { "type": "string" },
                        "action": { "type": "string" }
                    }
                }
            }
        },
        "required": ["title", "message"]
    }),
}
```

#### 6.3 Notification Center Integration

**File:** `crates/openfang-api/static/js/pages/notifications.js`

Add support for "suggestion" notification type:

```javascript
// Extend notification rendering
function renderNotification(notif) {
    if (notif.type === 'suggestion') {
        return `
            <div class="notification suggestion" data-insight="${notif.insight_id}">
                <div class="notif-icon">💡</div>
                <div class="notif-content">
                    <strong>${escapeHtml(notif.title)}</strong>
                    <p>${escapeHtml(notif.message)}</p>
                    <div class="notif-actions">
                        ${notif.actions.map(a => `
                            <button onclick="handleSuggestionAction('${notif.insight_id}', '${a.action}')">
                                ${escapeHtml(a.label)}
                            </button>
                        `).join('')}
                    </div>
                </div>
                <button class="notif-dismiss" onclick="dismissSuggestion('${notif.insight_id}', 'not_now')">✕</button>
            </div>
        `;
    }
    
    // ...existing notification types
}

async function handleSuggestionAction(insightId, action) {
    if (action === 'accept') {
        // Mark insight as accepted
        await fetch(`/api/learning/insights/${insightId}/accept`, { method: 'POST' });
        // Possibly navigate to tutorial or execute action
    } else if (action === 'dismiss') {
        await dismissSuggestion(insightId, 'not_interested');
    }
    
    // Remove notification from UI
    document.querySelector(`[data-insight="${insightId}"]`).remove();
}

async function dismissSuggestion(insightId, reason) {
    await fetch(`/api/learning/insights/${insightId}/dismiss`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ reason }),
    });
}
```

#### 6.4 Smart Delivery Scheduler

**File:** `crates/openfang-learning/src/delivery_scheduler.rs`

```rust
pub struct DeliveryScheduler {
    context_tracker: Arc<ContextTracker>,
    privacy_config: Arc<RwLock<PrivacyConfig>>,
}

impl DeliveryScheduler {
    /// Determine if now is a good time to deliver an insight
    pub async fn should_deliver_now(&self, insight: &Insight, user_id: &str) -> bool {
        let config = self.privacy_config.read().await;
        
        // Check if suggestions enabled
        if !config.enable_suggestions {
            return false;
        }
        
        // Check quiet hours
        if self.is_quiet_hours(&config).await {
            return false;
        }
        
        // Check recent suggestion frequency
        if self.too_many_recent_suggestions(user_id).await {
            return false;
        }
        
        // Check if user is in focused work mode
        if let Some(context) = self.context_tracker.get_context(user_id).await {
            if self.is_focused_work(&context) {
                return false;
            }
        }
        
        // Check priority threshold based on frequency setting
        let min_priority = match config.suggestion_frequency {
            SuggestionFrequency::Never => return false,
            SuggestionFrequency::Low => 8,    // Only high-priority
            SuggestionFrequency::Medium => 5, // Medium and up
            SuggestionFrequency::High => 3,   // Most suggestions
        };
        
        insight.priority >= min_priority
    }
    
    fn is_focused_work(&self, context: &WorkingContext) -> bool {
        // User has been active continuously for >20 min without context switch
        let session_duration = (Utc::now() - context.started_at).num_minutes();
        let recent_activity = (Utc::now() - context.last_activity).num_minutes();
        
        session_duration > 20 && recent_activity < 2
    }
}
```

#### 6.5 Proactive Agent Loop

**File:** `crates/openfang-kernel/src/proactive_agent.rs` (new file)

```rust
pub struct ProactiveAgent {
    insight_engine: Arc<InsightEngine>,
    delivery_scheduler: Arc<DeliveryScheduler>,
}

impl ProactiveAgent {
    /// Periodic check: Should we suggest anything now?
    pub async fn run_periodic_check(&self, user_id: &str) {
        // Get pending insights
        let insights = match self.insight_engine.get_pending_insights(user_id).await {
            Ok(i) => i,
            Err(e) => {
                warn!("Failed to get insights: {}", e);
                return;
            }
        };
        
        // For each insight, check if we should deliver now
        for insight in insights {
            if self.delivery_scheduler.should_deliver_now(&insight, user_id).await {
                // Send notification
                self.deliver_insight(&insight, user_id).await;
                
                // Mark as delivered
                self.mark_delivered(&insight.id).await;
                
                // Only deliver one per check (avoid spam)
                break;
            }
        }
    }
    
    async fn deliver_insight(&self, insight: &Insight, user_id: &str) {
        // Create notification
        let notification = self.format_notification(insight);
        
        // Send via notification center
        // (emits event that dashboard picks up via SSE)
        self.emit_notification_event(user_id, notification).await;
    }
}
```

### Testing

```bash
# Delivery scheduler tests
cargo test -p openfang-learning delivery_scheduler

# Proactive agent tests
cargo test -p openfang-kernel proactive_agent

# End-to-end: Generate insight → Deliver → User dismisses
cargo test -p openfang-kernel test_proactive_e2e

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Proactive assistant agent runs on schedule
- [ ] Insights delivered via notification center
- [ ] Smart timing prevents spam (focused work detection)
- [ ] User can dismiss with reasons
- [ ] Privacy settings respected (quiet hours, frequency)
- [ ] All tests pass
- [ ] Can deploy (proactive suggestions now active for opt-in users)

---

## Phase 7: Knowledge Graph Idea Linking (Week 11)

**Goal:** Use knowledge graph to suggest connections between concepts.

### Deliverables

#### 7.1 Graph Traversal Algorithms

**File:** `crates/openfang-learning/src/graph_algorithms.rs`

```rust
pub struct GraphTraverser {
    knowledge_store: Arc<KnowledgeStore>,
}

impl GraphTraverser {
    /// Find related concepts via BFS
    pub async fn find_related_concepts(
        &self,
        entity_id: &str,
        max_hops: usize,
    ) -> OpenFangResult<Vec<(Entity, Vec<Relation>)>> {
        // BFS from entity_id
        // Track paths
        // Return entities within max_hops with relation paths
        todo!()
    }
    
    /// Find common neighbors (concepts connected to multiple entities)
    pub async fn find_common_neighbors(
        &self,
        entity_ids: &[String],
    ) -> OpenFangResult<Vec<Entity>> {
        // Find entities that connect to all input entities
        // Useful for finding themes/patterns
        todo!()
    }
    
    /// Calculate PageRank-style scores
    pub async fn calculate_importance_scores(&self) -> OpenFangResult<HashMap<String, f32>> {
        // Simple PageRank on knowledge graph
        // Identifies central/important concepts
        todo!()
    }
}
```

#### 7.2 Idea Linker Generator

**File:** `crates/openfang-learning/src/generators/idea_linker.rs`

```rust
pub struct IdeaLinker {
    graph_traverser: Arc<GraphTraverser>,
}

impl IdeaLinker {
    /// Generate idea linking insights
    pub async fn generate(&self, context: &WorkingContext) -> Vec<Insight> {
        let mut insights = vec![];
        
        // For each entity in working context
        for entity_id in &context.related_entities {
            // Find concepts 1-2 hops away
            let related = match self.graph_traverser.find_related_concepts(entity_id, 2).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            
            for (related_entity, path) in related {
                // Filter by relevance
                if self.is_relevant_link(&path) {
                    insights.push(Insight {
                        insight_type: InsightType::ConceptLink {
                            entity_a: entity_id.clone(),
                            entity_b: related_entity.id.clone(),
                            relation_type: self.describe_path(&path),
                            explanation: self.explain_connection(&path),
                        },
                        confidence: self.calculate_confidence(&path),
                        priority: 6, // Medium priority
                        // ...
                    });
                }
            }
        }
        
        // Limit to top 3 most interesting links
        insights.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
        insights.truncate(3);
        
        insights
    }
    
    fn is_relevant_link(&self, path: &[Relation]) -> bool {
        // Filter out obvious/trivial connections
        // Prefer surprising but logical connections
        path.len() >= 2 && path.len() <= 3
    }
}
```

#### 7.3 Integration with Insight Engine

**File:** `crates/openfang-learning/src/insight_engine.rs`

```rust
impl InsightEngine {
    pub async fn generate_insights(&self, user_id: &str) -> OpenFangResult<Vec<Insight>> {
        // ...existing generators...
        
        // Add idea linker
        let context = self.context_tracker.get_context(user_id).await.unwrap_or_default();
        let link_insights = self.idea_linker.generate(&context).await?;
        all_insights.extend(link_insights);
        
        // ...rank and store...
    }
}
```

### Testing

```bash
# Graph traversal tests
cargo test -p openfang-learning graph_algorithms

# Idea linker tests
cargo test -p openfang-learning idea_linker

# Integration tests
cargo test -p openfang-learning test_idea_linking

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Graph traversal finds related concepts
- [ ] Idea linking generates relevant connections
- [ ] Insights include explanation of connection
- [ ] All tests pass
- [ ] Can deploy (concept links appear in suggestions)

---

## Phase 8: Learning Dashboard & Visualizations (Week 12-13)

**Goal:** Dedicated dashboard for learning insights, patterns, and progress tracking.

### Deliverables

#### 8.1 Learning Dashboard Page

**File:** `crates/openfang-api/static/index_body.html`

Add navigation:

```html
<!-- In sidebar -->
<li>
    <a href="#learning" class="nav-link" data-page="learning">
        <span class="nav-icon">🎓</span>
        <span class="nav-label">Learning</span>
    </a>
</li>
```

**File:** `crates/openfang-api/static/js/pages/learning.js` (new file)

```javascript
// Learning dashboard page component
Alpine.data('learningPage', () => ({
    activeTab: 'insights',
    insights: [],
    patterns: [],
    profile: null,
    stats: null,
    loading: true,
    
    async init() {
        await this.loadData();
    },
    
    async loadData() {
        this.loading = true;
        
        const [insights, patterns, profile, stats] = await Promise.all([
            fetch('/api/learning/insights').then(r => r.json()),
            fetch('/api/learning/patterns').then(r => r.json()),
            fetch('/api/learning/profile').then(r => r.json()),
            fetch('/api/learning/stats').then(r => r.json()),
        ]);
        
        this.insights = insights;
        this.patterns = patterns;
        this.profile = profile;
        this.stats = stats;
        this.loading = false;
    },
    
    async dismissInsight(insightId, reason) {
        await fetch(`/api/learning/insights/${insightId}/dismiss`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ reason }),
        });
        
        this.insights = this.insights.filter(i => i.id !== insightId);
        OpenFangToast.success('Insight dismissed');
    },
    
    async acceptInsight(insightId) {
        await fetch(`/api/learning/insights/${insightId}/accept`, {
            method: 'POST',
        });
        
        this.insights = this.insights.filter(i => i.id !== insightId);
        OpenFangToast.success('Thanks for the feedback!');
    },
}));
```

**File:** `crates/openfang-api/static/html/pages/learning.html` (new file)

```html
<div x-data="learningPage" class="page-learning">
    <div class="page-header">
        <h1>🎓 Learning & Insights</h1>
        <p class="subtitle">Personalized suggestions based on your usage patterns</p>
    </div>
    
    <!-- Tabs -->
    <div class="tabs">
        <button @click="activeTab = 'insights'" :class="activeTab === 'insights' && 'active'">
            Insights
        </button>
        <button @click="activeTab = 'patterns'" :class="activeTab === 'patterns' && 'active'">
            Patterns
        </button>
        <button @click="activeTab = 'profile'" :class="activeTab === 'profile' && 'active'">
            Profile
        </button>
        <button @click="activeTab = 'stats'" :class="activeTab === 'stats' && 'active'">
            Stats
        </button>
    </div>
    
    <!-- Insights Tab -->
    <div x-show="activeTab === 'insights'" class="tab-content">
        <template x-if="insights.length === 0">
            <div class="empty-state">
                <p>No pending insights. Check back later!</p>
            </div>
        </template>
        
        <template x-for="insight in insights" :key="insight.id">
            <div class="insight-card" :class="`priority-${insight.priority}`">
                <div class="insight-header">
                    <span class="insight-type" x-text="formatInsightType(insight.insight_type.type)"></span>
                    <span class="insight-confidence">
                        Confidence: <span x-text="Math.round(insight.confidence * 100)">90</span>%
                    </span>
                </div>
                
                <div class="insight-body">
                    <template x-if="insight.insight_type.type === 'AutomationOpportunity'">
                        <div>
                            <h3>💡 Automation Opportunity</h3>
                            <p><strong x-text="insight.insight_type.data.task_description"></strong></p>
                            <p>Estimated time saved: <strong x-text="insight.insight_type.data.estimated_time_saved_mins_per_week"></strong> min/week</p>
                            <p class="suggestion">Try: <em x-text="insight.insight_type.data.suggested_approach"></em></p>
                        </div>
                    </template>
                    
                    <template x-if="insight.insight_type.type === 'FeatureDiscovery'">
                        <div>
                            <h3>🔍 Feature Discovery</h3>
                            <p>Have you tried <strong x-text="insight.insight_type.data.feature_name"></strong>?</p>
                            <p x-text="insight.insight_type.data.relevance_explanation"></p>
                            <template x-if="insight.insight_type.data.tutorial_link">
                                <a :href="insight.insight_type.data.tutorial_link" target="_blank">Learn more →</a>
                            </template>
                        </div>
                    </template>
                    
                    <template x-if="insight.insight_type.type === 'ConceptLink'">
                        <div>
                            <h3>🔗 Idea Link</h3>
                            <p>
                                <strong x-text="insight.insight_type.data.entity_a"></strong>
                                <span class="relation-arrow">→</span>
                                <strong x-text="insight.insight_type.data.entity_b"></strong>
                            </p>
                            <p class="relation-type">via <em x-text="insight.insight_type.data.relation_type"></em></p>
                            <p x-text="insight.insight_type.data.explanation"></p>
                        </div>
                    </template>
                </div>
                
                <div class="insight-actions">
                    <button @click="acceptInsight(insight.id)" class="btn-primary">👍 Helpful</button>
                    <button @click="dismissInsight(insight.id, 'not_now')" class="btn-secondary">Later</button>
                    <button @click="dismissInsight(insight.id, 'not_interested')" class="btn-text">Not interested</button>
                </div>
            </div>
        </template>
    </div>
    
    <!-- Patterns Tab -->
    <div x-show="activeTab === 'patterns'" class="tab-content">
        <p class="info">Your detected behavioral patterns (last 30 days)</p>
        
        <div class="patterns-grid">
            <template x-for="pattern in patterns" :key="pattern.id">
                <div class="pattern-card">
                    <div class="pattern-icon" x-text="getPatternIcon(pattern.pattern_type.type)"></div>
                    <h4 x-text="formatPatternType(pattern.pattern_type.type)"></h4>
                    <p class="pattern-description" x-text="describePattern(pattern)"></p>
                    <div class="pattern-meta">
                        <span>Observed: <strong x-text="pattern.observation_count"></strong> times</span>
                        <span>Confidence: <strong x-text="Math.round(pattern.confidence * 100)"></strong>%</span>
                    </div>
                </div>
            </template>
        </div>
    </div>
    
    <!-- Profile Tab -->
    <div x-show="activeTab === 'profile'" class="tab-content">
        <template x-if="profile">
            <div class="profile-view">
                <div class="profile-section">
                    <h3>Skill Level</h3>
                    <div class="skill-badge" :class="`skill-${profile.skill_level.toLowerCase()}`">
                        <span x-text="profile.skill_level"></span>
                    </div>
                </div>
                
                <div class="profile-section">
                    <h3>Expertise Areas</h3>
                    <div class="tag-list">
                        <template x-for="area in profile.expertise_areas" :key="area">
                            <span class="tag" x-text="area"></span>
                        </template>
                    </div>
                </div>
                
                <div class="profile-section">
                    <h3>Learning Interests</h3>
                    <div class="tag-list">
                        <template x-for="interest in profile.learning_interests" :key="interest">
                            <span class="tag tag-interest" x-text="interest"></span>
                        </template>
                    </div>
                </div>
                
                <div class="profile-section">
                    <h3>Active Goals</h3>
                    <template x-for="goal in profile.active_goals" :key="goal.id">
                        <div class="goal-card">
                            <h4 x-text="goal.description"></h4>
                            <div class="progress-bar">
                                <div class="progress-fill" :style="`width: ${goal.progress * 100}%`"></div>
                            </div>
                            <span class="progress-text" x-text="`${Math.round(goal.progress * 100)}% complete`"></span>
                        </div>
                    </template>
                </div>
            </div>
        </template>
    </div>
    
    <!-- Stats Tab -->
    <div x-show="activeTab === 'stats'" class="tab-content">
        <template x-if="stats">
            <div class="stats-grid">
                <div class="stat-card">
                    <div class="stat-value" x-text="stats.total_time_saved_mins"></div>
                    <div class="stat-label">Minutes saved (all time)</div>
                </div>
                
                <div class="stat-card">
                    <div class="stat-value" x-text="`$${stats.total_cost_saved.toFixed(2)}`"></div>
                    <div class="stat-label">Cost saved (all time)</div>
                </div>
                
                <div class="stat-card">
                    <div class="stat-value" x-text="stats.features_discovered"></div>
                    <div class="stat-label">New features discovered</div>
                </div>
                
                <div class="stat-card">
                    <div class="stat-value" x-text="stats.workflows_optimized"></div>
                    <div class="stat-label">Workflows optimized</div>
                </div>
            </div>
        </template>
    </div>
</div>
```

#### 8.2 Privacy Dashboard Integration

**File:** `crates/openfang-api/static/html/pages/privacy.html` (new file)

```html
<div x-data="privacyPage" class="page-privacy">
    <div class="page-header">
        <h1>🔒 Privacy & Learning Controls</h1>
        <p class="subtitle">Control what data is collected and how it's used</p>
    </div>
    
    <!-- Data Collection Toggle -->
    <div class="privacy-section">
        <h2>Data Collection</h2>
        
        <label class="toggle-switch">
            <input type="checkbox" x-model="privacy.enable_tracking" @change="saveSettings">
            <span class="toggle-slider"></span>
            <span class="toggle-label">Enable activity tracking</span>
        </label>
        
        <label class="toggle-switch">
            <input type="checkbox" x-model="privacy.anonymize_user_id" @change="saveSettings">
            <span class="toggle-slider"></span>
            <span class="toggle-label">Anonymize my user ID</span>
        </label>
        
        <label class="toggle-switch">
            <input type="checkbox" x-model="privacy.redact_sensitive_data" @change="saveSettings">
            <span class="toggle-slider"></span>
            <span class="toggle-label">Redact sensitive data (file paths, etc.)</span>
        </label>
    </div>
    
    <!-- Suggestion Controls -->
    <div class="privacy-section">
        <h2>Proactive Suggestions</h2>
        
        <label class="toggle-switch">
            <input type="checkbox" x-model="privacy.enable_suggestions" @change="saveSettings">
            <span class="toggle-slider"></span>
            <span class="toggle-label">Enable proactive suggestions</span>
        </label>
        
        <label>
            Suggestion frequency:
            <select x-model="privacy.suggestion_frequency" @change="saveSettings">
                <option value="never">Never</option>
                <option value="low">Low (once per day)</option>
                <option value="medium">Medium (few times per day)</option>
                <option value="high">High (many times per day)</option>
            </select>
        </label>
    </div>
    
    <!-- Data Retention -->
    <div class="privacy-section">
        <h2>Data Retention</h2>
        
        <label>
            Keep learning data for:
            <input type="number" x-model.number="privacy.retention_days" @change="saveSettings" min="7" max="3650">
            days
        </label>
        
        <p class="help-text">Set to 0 to keep forever. Minimum 7 days, maximum 10 years.</p>
    </div>
    
    <!-- Data Export & Deletion -->
    <div class="privacy-section">
        <h2>Your Data</h2>
        
        <button @click="exportData" class="btn-secondary">📥 Export All Learning Data</button>
        <button @click="deleteData" class="btn-danger">🗑️ Delete All Learning Data</button>
    </div>
</div>
```

### Testing

```bash
# Frontend components load
npm test learning-dashboard

# API endpoints return correct data
cargo test -p openfang-api test_learning_dashboard

# Privacy controls work
cargo test -p openfang-api test_privacy_controls

# All tests
cargo test --workspace
```

### Success Criteria

- [ ] Learning dashboard accessible and functional
- [ ] All tabs render correctly (insights, patterns, profile, stats)
- [ ] Privacy controls work (toggles, export, delete)
- [ ] Insights actionable from dashboard
- [ ] All tests pass
- [ ] Can deploy (full learning system now user-facing)

---

## Phase 9: Advanced Generators & Polish (Week 14)

**Goal:** Add workflow optimization, cost optimization, and polish existing features.

### Deliverables

#### 9.1 Workflow Optimization Generator

**File:** `crates/openfang-learning/src/generators/workflow_optimizer.rs`

```rust
pub struct WorkflowOptimizer;

impl WorkflowOptimizer {
    pub fn generate(&self, patterns: &[BehaviorPattern], profile: &UserProfile) -> Vec<Insight> {
        // Analyze workflow patterns
        // Identify inefficiencies (long delays, failed steps)
        // Suggest optimizations
        todo!()
    }
}
```

#### 9.2 Cost Optimization Analyzer

**File:** `crates/openfang-learning/src/generators/cost_optimizer.rs`

```rust
pub struct CostOptimizationAnalyzer;

impl CostOptimizationAnalyzer {
    pub fn generate(&self, patterns: &[BehaviorPattern]) -> Vec<Insight> {
        // Analyze tool usage patterns
        // Identify expensive operations done frequently
        // Suggest cheaper alternatives
        todo!()
    }
}
```

#### 9.3 Feedback Learning System

**File:** `crates/openfang-learning/src/feedback_processor.rs`

```rust
pub struct FeedbackProcessor {
    insight_store: Arc<InsightStore>,
    pattern_store: Arc<PatternStore>,
}

impl FeedbackProcessor {
    pub async fn process_feedback(&self, insight_id: &str, action: InsightAction, reason: Option<String>) {
        match action {
            InsightAction::Accepted => {
                // Positive signal: boost similar insights
                self.reinforce_insight_type(insight_id).await;
            }
            InsightAction::Dismissed { reason } => {
                match reason.as_deref() {
                    Some("not_interested") => {
                        // Suppress similar insights
                        self.suppress_similar_insights(insight_id).await;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    
    async fn reinforce_insight_type(&self, insight_id: &str) {
        // Increase confidence weights for this insight type
        // Reduce priority threshold for similar insights
    }
    
    async fn suppress_similar_insights(&self, insight_id: &str) {
        // Decrease confidence weights
        // Increase priority threshold
        // Mark similar patterns as "user not interested"
    }
}
```

#### 9.4 Documentation

**Files:**
- `docs/learning-system.md` - User guide for learning features
- `docs/privacy-guide.md` - Privacy controls and data handling
- `docs/api-reference.md` - Update with learning API endpoints

### Testing

```bash
# All generators work
cargo test -p openfang-learning generators

# Feedback learning works
cargo test -p openfang-learning feedback_processor

# Documentation tests
cargo test --doc

# Full integration test suite
cargo test --workspace
```

### Success Criteria

- [ ] Workflow optimization insights generated
- [ ] Cost optimization insights generated
- [ ] Feedback learning improves suggestions over time
- [ ] Documentation complete and accurate
- [ ] All tests pass (2,793+ workspace tests expected)
- [ ] Can deploy (complete proactive learning system)

---

## Summary Timeline

| Phase | Weeks | Deliverable | User Impact |
|-------|-------|-------------|-------------|
| 1 | 1-2 | Event collection + privacy foundation | None (infrastructure) |
| 2 | 3 | Event storage + anonymization | None (background) |
| 3 | 4-5 | Basic pattern recognition | None (analysis only) |
| 4 | 6 | User profile + context tracking | None (preparation) |
| 5 | 7-8 | Basic insight generation | None (API only) |
| 6 | 9-10 | Proactive suggestion delivery | **High** (notifications appear) |
| 7 | 11 | Knowledge graph idea linking | Medium (better suggestions) |
| 8 | 12-13 | Learning dashboard | **High** (full UI) |
| 9 | 14 | Advanced generators + polish | Medium (more insights) |

**Total:** 14 weeks for complete implementation

**MVP (Minimum Viable Product):** Phases 1-6 (10 weeks)
- Provides event collection, pattern recognition, insights, and delivery
- Missing: dashboard UI and advanced generators
- Fully functional proactive system via notifications

---

## Deployment Strategy

### Per-Phase Rollout

1. **Phases 1-5:** Deploy to production but keep `enable_suggestions: false` by default
   - Data collection and analysis running
   - No user-facing changes
   - Build up pattern database

2. **Phase 6:** Enable for beta users only
   - Set `enable_suggestions: true` for opted-in users
   - Monitor feedback closely
   - Iterate on delivery timing

3. **Phase 8:** General availability
   - Enable suggestions by default (users can opt-out)
   - Full dashboard access
   - Announcement/tutorial for users

### Feature Flags

```toml
# In config.toml
[experimental_features]
proactive_learning = true    # Master switch
pattern_recognition = true
proactive_suggestions = false  # Start disabled
learning_dashboard = false     # Deploy UI separately
```

---

## Privacy & Ethical Compliance

### GDPR Compliance

- ✅ Right to access (export endpoint)
- ✅ Right to deletion (delete endpoint)
- ✅ Data minimization (retention periods)
- ✅ Consent (opt-in/opt-out controls)
- ✅ Transparency (privacy dashboard)

### Ethical AI Principles

- Avoid filter bubbles (diverse suggestions)
- Prevent manipulation (explainability required)
- Respect autonomy (easy opt-out)
- Fairness (no user discrimination)

---

## Success Metrics (Per Phase)

### Phase 6 Metrics (Delivery)

- Suggestion delivery rate: >50% eligible suggestions delivered
- User dismissal rate: <50% (target: 30-40%)
- Acceptance rate: >20%

### Phase 8 Metrics (Dashboard)

- Dashboard visit rate: >40% of users per week
- Time saved reported: >100 min/user/month
- Feature adoption increase: >15%

### Overall System Health

- Pattern detection accuracy: >70%
- Insight relevance: >60% (user survey)
- Privacy opt-out rate: <5%
- Data storage growth: <50MB/user/month

---

**Next Steps:**
1. Review phased plan with stakeholders
2. Set up feature flags and privacy config
3. Begin Phase 1 implementation
4. Create beta user program for Phase 6 testing
