# Proactive Learning & Assistance System Design

**Version:** 1.0  
**Date:** 2026-04-11  
**Status:** Design Review

## Executive Summary

This document outlines a comprehensive design for enabling ArmaraOS agents to learn user actions, recognize patterns, and proactively provide assistance, suggestions, and insights. The system builds on existing infrastructure (memory substrate, knowledge graph, event bus, learning frames) to create an intelligent assistant that becomes more helpful over time.

---

## Vision

**"ArmaraOS learns how you work and helps you work better."**

The system should:
- **Learn silently** - Pattern recognition happens in the background
- **Suggest contextually** - Recommendations appear at the right moment
- **Respect privacy** - User controls what's tracked and stored
- **Improve over time** - More data = better suggestions
- **Link ideas** - Surface non-obvious connections
- **Teach proactively** - Help users discover new capabilities

---

## Current State Analysis

### Existing Infrastructure

**1. Memory Substrate** (`crates/openfang-memory/`)
- **Semantic store** - Text search and embeddings
- **Knowledge graph** - Entity-relation storage (SQLite)
- **Structured store** - Key-value pairs, sessions, agent state
- **Session management** - Conversation history tracking

**2. Event System** (`crates/openfang-types/src/event.rs`)
- Comprehensive event bus with:
  - `SystemEvent::UserAction` - User actions tracked
  - `SystemEvent::AgentActivity` - Agent behavior phases
  - Tool execution events
  - Lifecycle events
- Event streaming to dashboard (SSE)

**3. Learning Frame v1** (`docs/learning-frame-v1.md`)
- Skill capture from interactions
- `[learn]` prefix for opt-in chat capture
- Episode recording with intent/outcome
- AINL integration for skill minting

**4. Knowledge Graph** (`crates/openfang-memory/src/knowledge.rs`)
- Entity types: Person, Organization, Project, Concept, Event, Location, Document, Tool
- Relation types: WorksAt, KnowsAbout, RelatedTo, DependsOn, Uses, etc.
- Graph pattern queries

### Gaps & Opportunities

1. **No Pattern Recognition Engine**
   - Events are logged but not analyzed for patterns
   - No temporal pattern detection (time, frequency, sequences)
   - No user behavior clustering or profiling

2. **No Proactive Agent**
   - System is reactive (waits for user input)
   - No monitoring agent that watches for opportunities
   - No suggestion delivery mechanism

3. **Limited Context Awareness**
   - Agents don't know user's current task or goal
   - No multi-session context tracking
   - No "working on X" state management

4. **Knowledge Graph Underutilized**
   - Exists but not actively used for suggestions
   - No automatic concept linking
   - No insight generation from graph structure

5. **No Personalization System**
   - No user preference tracking
   - No adaptation to user's skill level
   - No customization of suggestion frequency/style

6. **Privacy Controls Missing**
   - No granular opt-in/opt-out
   - No data deletion mechanisms
   - No anonymization options

---

## Design Proposal

### Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│                        User Interaction Layer               │
│  (Dashboard, CLI, Channels, Desktop App)                   │
└─────────────────┬───────────────────────────────────────────┘
                  │ Events
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                     Event Collection Layer                  │
│  • UserAction events         • Tool executions             │
│  • AgentActivity events      • Session events              │
└─────────────────┬───────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                  Pattern Recognition Engine                 │
│  • Temporal patterns        • Sequence detection           │
│  • Frequency analysis       • Anomaly detection            │
│  • Cluster analysis         • Trend detection              │
└─────────────────┬───────────────────────────────────────────┘
                  │ Patterns
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                      Learning Substrate                     │
│  • User Profile            • Behavior Patterns              │
│  • Preferences             • Habit Models                   │
│  • Context State           • Skill Level Tracking           │
└─────────────────┬───────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                   Insight Generation Engine                 │
│  • Idea linking           • Opportunity detection           │
│  • Knowledge suggestions  • Learning recommendations        │
│  • Workflow optimization  • Cost reduction tips             │
└─────────────────┬───────────────────────────────────────────┘
                  │ Insights
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                    Suggestion Delivery Agent                │
│  • Context-aware timing   • Priority ranking                │
│  • User preference aware  • Multi-channel delivery          │
└─────────────────┬───────────────────────────────────────────┘
                  │
                  ▼
┌─────────────────────────────────────────────────────────────┐
│                        User Interface                       │
│  • Notification center    • Inline suggestions              │
│  • Learning dashboard     • Privacy controls                │
└─────────────────────────────────────────────────────────────┘
```

---

## Component 1: Enhanced Event Collection

**Goal:** Capture rich user behavior data without performance impact.

### 1.1 User Activity Tracking

**New Event Type Extensions:**

```rust
// Location: crates/openfang-types/src/event.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SystemEvent {
    // ...existing variants...
    
    /// User performed an action in the UI or CLI
    UserActivity {
        /// User identifier (anonymized hash or actual ID based on privacy settings)
        user_id: String,
        
        /// Activity type (page_view, button_click, command_run, etc.)
        activity_type: UserActivityType,
        
        /// Context metadata
        context: HashMap<String, serde_json::Value>,
        
        /// Duration of activity (for tasks with start/end)
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
#[serde(rename_all = "snake_case")]
pub enum UserActivityType {
    /// Navigated to a page
    PageView { page: String },
    
    /// Clicked a button or link
    Click { element: String },
    
    /// Executed a CLI command
    CommandRun { command: String, args: Vec<String> },
    
    /// Sent a message to an agent
    AgentMessage { agent_id: AgentId, message_length: usize },
    
    /// Created or edited a resource
    ResourceEdit { resource_type: String, action: EditAction },
    
    /// Searched for something
    Search { query: String, scope: SearchScope },
    
    /// Viewed documentation
    DocView { doc_path: String, duration_secs: u64 },
    
    /// Used a tool or feature
    FeatureUse { feature: String },
    
    /// Changed a setting
    SettingChange { setting: String, old_value: Option<String>, new_value: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditAction {
    Create,
    Update,
    Delete,
    Rename,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchScope {
    Global,
    Agents,
    Sessions,
    Memory,
    Files,
    Documentation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HelpType {
    /// User explicitly asked for help
    Explicit,
    
    /// System detected user struggling (e.g., many failed attempts)
    Implicit,
    
    /// User hovered over help icon
    HoverHelp,
}
```

### 1.2 Context Tracking

**New Type: WorkingContext**

```rust
// Location: crates/openfang-types/src/learning.rs (new file)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Tracks what the user is currently working on
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingContext {
    /// Unique context ID
    pub id: String,
    
    /// User ID
    pub user_id: String,
    
    /// Current task or goal
    pub current_task: Option<String>,
    
    /// Related entities (agents, projects, documents)
    pub related_entities: Vec<String>,
    
    /// Active agents user is interacting with
    pub active_agents: Vec<AgentId>,
    
    /// Recently used tools/features
    pub recent_tools: Vec<String>,
    
    /// When this context started
    pub started_at: DateTime<Utc>,
    
    /// Last activity timestamp
    pub last_activity: DateTime<Utc>,
    
    /// Arbitrary context metadata
    pub metadata: HashMap<String, serde_json::Value>,
}
```

### 1.3 Privacy-Aware Collection

**Privacy Settings:**

```rust
// Location: crates/openfang-types/src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PrivacyConfig {
    /// Enable activity tracking
    pub enable_tracking: bool,
    
    /// Anonymize user identifiers
    pub anonymize_user_id: bool,
    
    /// Redact sensitive data from events (file paths, etc.)
    pub redact_sensitive_data: bool,
    
    /// Retention period for activity data (days, 0 = forever)
    pub retention_days: u32,
    
    /// Activities to exclude from tracking
    pub excluded_activities: Vec<UserActivityType>,
    
    /// Enable proactive suggestions
    pub enable_suggestions: bool,
    
    /// Suggestion frequency (never, low, medium, high)
    pub suggestion_frequency: SuggestionFrequency,
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            enable_tracking: true,
            anonymize_user_id: false,
            redact_sensitive_data: true,
            retention_days: 90,
            excluded_activities: vec![],
            enable_suggestions: true,
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
- Tracking is opt-out (on by default, user can disable)
- All privacy settings have safe defaults
- `#[serde(default)]` ensures old configs work

---

## Component 2: Pattern Recognition Engine

**Goal:** Automatically detect meaningful patterns in user behavior.

### 2.1 Pattern Types

```rust
// Location: crates/openfang-learning/src/patterns.rs (new crate)

use chrono::{DateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};

/// A detected behavioral pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorPattern {
    /// Unique pattern ID
    pub id: String,
    
    /// Pattern type
    pub pattern_type: PatternType,
    
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    
    /// How many times observed
    pub observation_count: u32,
    
    /// When first detected
    pub first_seen: DateTime<Utc>,
    
    /// When last observed
    pub last_seen: DateTime<Utc>,
    
    /// Pattern metadata
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum PatternType {
    /// User performs action A, then often performs action B
    Sequence {
        actions: Vec<String>,
        typical_delay_secs: u64,
    },
    
    /// User performs action at consistent times
    Temporal {
        action: String,
        time_of_day: Vec<NaiveTime>,
        days_of_week: Vec<u8>, // 0-6, Sunday=0
    },
    
    /// User performs action with consistent frequency
    Frequency {
        action: String,
        avg_interval_secs: u64,
        variance_secs: u64,
    },
    
    /// User always performs action when condition is met
    Conditional {
        trigger: String,
        action: String,
        trigger_probability: f32,
    },
    
    /// User prefers option A over option B
    Preference {
        choice_point: String,
        preferred_option: String,
        preference_strength: f32, // 0.5 = no preference, 1.0 = always
    },
    
    /// User struggles with feature/task
    Difficulty {
        task: String,
        failure_rate: f32,
        avg_attempts: f32,
    },
    
    /// User session duration pattern
    SessionDuration {
        avg_duration_mins: f32,
        typical_start_time: NaiveTime,
    },
    
    /// User tool usage pattern
    ToolUsage {
        tool: String,
        usage_frequency: ToolFrequency,
        common_parameters: Vec<String>,
    },
    
    /// Workflow pattern (multi-step task)
    Workflow {
        workflow_name: String,
        steps: Vec<WorkflowStep>,
        success_rate: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolFrequency {
    Hourly,
    Daily,
    Weekly,
    Monthly,
    Rare,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub step_name: String,
    pub typical_duration_secs: u64,
    pub success_rate: f32,
}
```

### 2.2 Pattern Detection Algorithms

**Sequence Detection (Markov Chains):**

```rust
// Location: crates/openfang-learning/src/detectors/sequence.rs

/// Detects action sequences using sliding window + Markov chain
pub struct SequenceDetector {
    /// Window size for sequence analysis
    window_size: usize,
    
    /// Minimum occurrences to establish pattern
    min_occurrences: u32,
    
    /// Transition probability threshold
    min_probability: f32,
}

impl SequenceDetector {
    /// Analyze recent events and detect sequences
    pub fn detect(&self, events: &[UserActivityEvent]) -> Vec<BehaviorPattern> {
        // Build transition matrix
        // Calculate probabilities
        // Filter by threshold
        // Return patterns with confidence scores
        todo!()
    }
}
```

**Temporal Pattern Detection (Time Series Analysis):**

```rust
// Location: crates/openfang-learning/src/detectors/temporal.rs

/// Detects time-based patterns (hourly, daily, weekly cycles)
pub struct TemporalDetector {
    /// Minimum observations to establish pattern
    min_observations: u32,
    
    /// Time bucket size (e.g., 1 hour)
    bucket_duration: std::time::Duration,
}

impl TemporalDetector {
    /// Analyze event timestamps and detect temporal patterns
    pub fn detect(&self, events: &[UserActivityEvent]) -> Vec<BehaviorPattern> {
        // Group events by time buckets
        // Calculate frequency distribution
        // Identify peaks (time-of-day, day-of-week)
        // Return temporal patterns
        todo!()
    }
}
```

**Preference Learning (Multi-Armed Bandit):**

```rust
// Location: crates/openfang-learning/src/detectors/preference.rs

/// Learns user preferences from choices
pub struct PreferenceDetector {
    /// Epsilon for exploration vs exploitation
    epsilon: f32,
}

impl PreferenceDetector {
    /// Analyze user choices and learn preferences
    pub fn detect(&self, choices: &[UserChoice]) -> Vec<BehaviorPattern> {
        // Track choice frequencies
        // Calculate preference probabilities
        // Return preference patterns
        todo!()
    }
}

#[derive(Debug, Clone)]
pub struct UserChoice {
    pub choice_point: String,
    pub option_selected: String,
    pub alternatives: Vec<String>,
    pub timestamp: DateTime<Utc>,
}
```

### 2.3 Pattern Recognition Service

```rust
// Location: crates/openfang-learning/src/recognition_engine.rs

pub struct PatternRecognitionEngine {
    sequence_detector: SequenceDetector,
    temporal_detector: TemporalDetector,
    preference_detector: PreferenceDetector,
    
    /// Detected patterns cache
    patterns: Arc<RwLock<HashMap<String, BehaviorPattern>>>,
    
    /// Event buffer for analysis
    event_buffer: Arc<RwLock<VecDeque<UserActivityEvent>>>,
}

impl PatternRecognitionEngine {
    /// Process new event and update patterns
    pub async fn process_event(&self, event: UserActivityEvent) {
        // Add to buffer
        // Run detectors periodically (not on every event)
        // Update pattern cache
        // Emit PatternDetected events
    }
    
    /// Get all detected patterns for a user
    pub fn get_patterns(&self, user_id: &str) -> Vec<BehaviorPattern> {
        // Filter patterns by user_id
        // Sort by confidence
        // Return
        todo!()
    }
    
    /// Background job: Run full pattern analysis
    pub async fn analyze_patterns(&self) {
        // Run all detectors on buffered events
        // Persist new patterns to database
        // Clean up old patterns
    }
}
```

**Storage:**

```sql
-- New table: behavior_patterns
CREATE TABLE IF NOT EXISTS behavior_patterns (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    pattern_type TEXT NOT NULL,
    pattern_data TEXT NOT NULL, -- JSON
    confidence REAL NOT NULL,
    observation_count INTEGER NOT NULL,
    first_seen TEXT NOT NULL,
    last_seen TEXT NOT NULL,
    metadata TEXT, -- JSON
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_patterns_user ON behavior_patterns(user_id);
CREATE INDEX idx_patterns_type ON behavior_patterns(pattern_type);
CREATE INDEX idx_patterns_confidence ON behavior_patterns(confidence DESC);
```

---

## Component 3: Learning Substrate & User Profile

**Goal:** Build comprehensive user models that improve over time.

### 3.1 User Profile Type

```rust
// Location: crates/openfang-types/src/learning.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    /// User identifier
    pub user_id: String,
    
    /// Skill level assessment
    pub skill_level: SkillLevel,
    
    /// Areas of expertise
    pub expertise_areas: Vec<String>,
    
    /// Areas of learning interest
    pub learning_interests: Vec<String>,
    
    /// Preferred learning style
    pub learning_style: LearningStyle,
    
    /// Communication preferences
    pub communication_prefs: CommunicationPreferences,
    
    /// Frequently used features
    pub frequent_features: HashMap<String, u32>, // feature -> usage_count
    
    /// Rarely used features (candidates for teaching)
    pub underutilized_features: Vec<String>,
    
    /// Goals the user is working towards
    pub active_goals: Vec<UserGoal>,
    
    /// Profile metadata
    pub metadata: HashMap<String, serde_json::Value>,
    
    /// When profile was created
    pub created_at: DateTime<Utc>,
    
    /// When profile was last updated
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLevel {
    Beginner,
    Intermediate,
    Advanced,
    Expert,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningStyle {
    /// Prefers step-by-step tutorials
    Guided,
    
    /// Prefers documentation and self-discovery
    SelfDirected,
    
    /// Prefers examples and experimentation
    ExampleDriven,
    
    /// Prefers video/visual content
    Visual,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunicationPreferences {
    /// Verbosity level (terse, normal, verbose)
    pub verbosity: Verbosity,
    
    /// Include explanations vs just do it
    pub explain_actions: bool,
    
    /// Preferred notification channels
    pub notification_channels: Vec<String>,
    
    /// Quiet hours (no suggestions)
    pub quiet_hours: Option<(NaiveTime, NaiveTime)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    Terse,
    Normal,
    Verbose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserGoal {
    pub id: String,
    pub description: String,
    pub goal_type: GoalType,
    pub target_date: Option<DateTime<Utc>>,
    pub progress: f32, // 0.0 - 1.0
    pub milestones: Vec<Milestone>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalType {
    LearnFeature(String),
    CompleteProject(String),
    OptimizeWorkflow(String),
    ReduceCost(f64), // target reduction
    ImproveEfficiency(f32), // target improvement %
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub description: String,
    pub completed: bool,
    pub completed_at: Option<DateTime<Utc>>,
}
```

### 3.2 Profile Building Service

```rust
// Location: crates/openfang-learning/src/profile_builder.rs

pub struct ProfileBuilder {
    /// User profiles cache
    profiles: Arc<RwLock<HashMap<String, UserProfile>>>,
    
    /// Pattern recognition engine (for input)
    pattern_engine: Arc<PatternRecognitionEngine>,
}

impl ProfileBuilder {
    /// Update user profile based on new patterns and events
    pub async fn update_profile(&self, user_id: &str) {
        // Fetch latest patterns
        // Infer skill level from patterns
        // Identify expertise areas (frequent successful actions)
        // Identify learning interests (struggled tasks, help requests)
        // Update profile in memory and persist
    }
    
    /// Infer skill level from behavior patterns
    fn infer_skill_level(&self, patterns: &[BehaviorPattern]) -> SkillLevel {
        // Analyze:
        // - Feature usage breadth (how many features used)
        // - Advanced feature usage (complex features)
        // - Error rate / success rate
        // - Help request frequency
        // - Time to complete tasks
        todo!()
    }
    
    /// Identify underutilized features
    fn find_underutilized_features(&self, user_id: &str) -> Vec<String> {
        // Compare user's feature usage to available features
        // Return features never or rarely used
        todo!()
    }
}
```

**Storage:**

```sql
-- New table: user_profiles
CREATE TABLE IF NOT EXISTS user_profiles (
    user_id TEXT PRIMARY KEY,
    skill_level TEXT NOT NULL,
    expertise_areas TEXT NOT NULL, -- JSON array
    learning_interests TEXT NOT NULL, -- JSON array
    learning_style TEXT NOT NULL,
    communication_prefs TEXT NOT NULL, -- JSON
    frequent_features TEXT NOT NULL, -- JSON object
    underutilized_features TEXT NOT NULL, -- JSON array
    active_goals TEXT NOT NULL, -- JSON array
    metadata TEXT, -- JSON
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
```

---

## Component 4: Insight Generation Engine

**Goal:** Transform patterns and profiles into actionable insights.

### 4.1 Insight Types

```rust
// Location: crates/openfang-learning/src/insights.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    /// Unique insight ID
    pub id: String,
    
    /// User this insight is for
    pub user_id: String,
    
    /// Insight type
    pub insight_type: InsightType,
    
    /// Confidence score (0.0 - 1.0)
    pub confidence: f32,
    
    /// Priority (0-10, 10 = highest)
    pub priority: u8,
    
    /// When this insight was generated
    pub generated_at: DateTime<Utc>,
    
    /// When this insight expires (if time-sensitive)
    pub expires_at: Option<DateTime<Utc>>,
    
    /// Supporting evidence (pattern IDs, event IDs)
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum InsightType {
    /// Suggest automating a repetitive task
    AutomationOpportunity {
        task_description: String,
        estimated_time_saved_mins_per_week: u32,
        suggested_approach: String,
    },
    
    /// Link related concepts or entities
    ConceptLink {
        entity_a: String,
        entity_b: String,
        relation_type: String,
        explanation: String,
    },
    
    /// Suggest a more efficient workflow
    WorkflowOptimization {
        current_workflow: String,
        suggested_workflow: String,
        improvement_description: String,
        estimated_improvement_pct: f32,
    },
    
    /// Teach about an underutilized feature
    FeatureDiscovery {
        feature_name: String,
        relevance_explanation: String,
        tutorial_link: Option<String>,
    },
    
    /// Warn about potential cost inefficiency
    CostOptimization {
        current_practice: String,
        suggested_alternative: String,
        estimated_savings_usd_per_month: f64,
    },
    
    /// Suggest next step in a workflow
    NextAction {
        context: String,
        suggested_action: String,
        reasoning: String,
    },
    
    /// Identify knowledge gap
    LearningOpportunity {
        topic: String,
        difficulty_level: SkillLevel,
        resources: Vec<String>,
        estimated_value: String,
    },
    
    /// Surface interesting pattern
    PatternNotification {
        pattern_description: String,
        significance: String,
    },
    
    /// Proactive error prevention
    ErrorPrevention {
        likely_error: String,
        prevention_tip: String,
        confidence: f32,
    },
    
    /// Context-aware reminder
    Reminder {
        message: String,
        based_on_pattern: String,
    },
}
```

### 4.2 Insight Generators

**Automation Opportunity Detector:**

```rust
// Location: crates/openfang-learning/src/generators/automation.rs

pub struct AutomationGenerator;

impl AutomationGenerator {
    /// Find repetitive sequences that could be automated
    pub fn generate(&self, patterns: &[BehaviorPattern]) -> Vec<Insight> {
        let mut insights = vec![];
        
        for pattern in patterns {
            if let PatternType::Sequence { actions, .. } = &pattern.pattern_type {
                // If sequence repeats frequently and has 3+ steps
                if pattern.observation_count > 10 && actions.len() >= 3 {
                    insights.push(Insight {
                        insight_type: InsightType::AutomationOpportunity {
                            task_description: format!("Repeated sequence: {}", actions.join(" → ")),
                            estimated_time_saved_mins_per_week: self.estimate_time_saved(pattern),
                            suggested_approach: self.suggest_automation(actions),
                        },
                        confidence: pattern.confidence,
                        priority: self.calculate_priority(pattern),
                        // ...
                    });
                }
            }
        }
        
        insights
    }
}
```

**Idea Linker (Knowledge Graph Traversal):**

```rust
// Location: crates/openfang-learning/src/generators/idea_linker.rs

pub struct IdeaLinker {
    knowledge_store: Arc<KnowledgeStore>,
}

impl IdeaLinker {
    /// Find non-obvious connections between concepts
    pub async fn generate(&self, working_context: &WorkingContext) -> Vec<Insight> {
        let mut insights = vec![];
        
        // For each entity in user's working context
        for entity_id in &working_context.related_entities {
            // Traverse graph to find:
            // 1. Related concepts (1-2 hops away)
            // 2. Common neighbors (entities connected to multiple context items)
            // 3. Unexpected connections (low PageRank but high relevance)
            
            let related = self.find_related_concepts(entity_id, 2).await;
            
            for (related_entity, path) in related {
                insights.push(Insight {
                    insight_type: InsightType::ConceptLink {
                        entity_a: entity_id.clone(),
                        entity_b: related_entity.id,
                        relation_type: self.describe_path(&path),
                        explanation: self.explain_connection(&path),
                    },
                    // ...
                });
            }
        }
        
        insights
    }
    
    /// Find related concepts via graph traversal
    async fn find_related_concepts(&self, entity_id: &str, max_hops: usize) -> Vec<(Entity, Vec<Relation>)> {
        // BFS/DFS graph traversal
        // Return entities within max_hops with paths
        todo!()
    }
}
```

**Feature Discovery Guide:**

```rust
// Location: crates/openfang-learning/src/generators/feature_discovery.rs

pub struct FeatureDiscoveryGenerator;

impl FeatureDiscoveryGenerator {
    /// Suggest relevant features user hasn't tried
    pub fn generate(&self, profile: &UserProfile, context: &WorkingContext) -> Vec<Insight> {
        let mut insights = vec![];
        
        // Filter underutilized features by relevance to current context
        for feature in &profile.underutilized_features {
            if self.is_relevant_to_context(feature, context) {
                insights.push(Insight {
                    insight_type: InsightType::FeatureDiscovery {
                        feature_name: feature.clone(),
                        relevance_explanation: self.explain_relevance(feature, context),
                        tutorial_link: self.get_tutorial_link(feature),
                    },
                    priority: self.calculate_relevance_score(feature, context),
                    // ...
                });
            }
        }
        
        insights
    }
}
```

### 4.3 Insight Generation Service

```rust
// Location: crates/openfang-learning/src/insight_engine.rs

pub struct InsightEngine {
    automation_gen: AutomationGenerator,
    idea_linker: IdeaLinker,
    feature_discovery: FeatureDiscoveryGenerator,
    workflow_optimizer: WorkflowOptimizer,
    cost_analyzer: CostOptimizationAnalyzer,
    
    /// Generated insights cache
    insights: Arc<RwLock<HashMap<String, Vec<Insight>>>>,
}

impl InsightEngine {
    /// Generate insights for a user based on patterns and profile
    pub async fn generate_insights(&self, user_id: &str) -> Vec<Insight> {
        let patterns = self.pattern_engine.get_patterns(user_id);
        let profile = self.profile_builder.get_profile(user_id).await;
        let context = self.context_tracker.get_working_context(user_id).await;
        
        let mut all_insights = vec![];
        
        // Run all generators in parallel
        let automation = self.automation_gen.generate(&patterns);
        let links = self.idea_linker.generate(&context).await;
        let features = self.feature_discovery.generate(&profile, &context);
        let workflows = self.workflow_optimizer.generate(&patterns, &profile);
        let costs = self.cost_analyzer.generate(&patterns);
        
        all_insights.extend(automation);
        all_insights.extend(links);
        all_insights.extend(features);
        all_insights.extend(workflows);
        all_insights.extend(costs);
        
        // Rank by priority and confidence
        all_insights.sort_by(|a, b| {
            (b.priority, (b.confidence * 100.0) as u8)
                .cmp(&(a.priority, (a.confidence * 100.0) as u8))
        });
        
        // Store and return
        self.insights.write().await.insert(user_id.to_string(), all_insights.clone());
        all_insights
    }
    
    /// Background job: Generate insights periodically
    pub async fn run_periodic_generation(&self) {
        loop {
            // For each active user
            // Generate insights
            // Emit InsightGenerated events
            tokio::time::sleep(Duration::from_secs(300)).await; // Every 5 minutes
        }
    }
}
```

**Storage:**

```sql
-- New table: insights
CREATE TABLE IF NOT EXISTS insights (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    insight_type TEXT NOT NULL,
    insight_data TEXT NOT NULL, -- JSON
    confidence REAL NOT NULL,
    priority INTEGER NOT NULL,
    generated_at TEXT NOT NULL,
    expires_at TEXT,
    evidence TEXT, -- JSON array
    dismissed BOOLEAN DEFAULT 0,
    dismissed_at TEXT,
    acted_upon BOOLEAN DEFAULT 0,
    acted_upon_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_insights_user ON insights(user_id);
CREATE INDEX idx_insights_priority ON insights(priority DESC, confidence DESC);
CREATE INDEX idx_insights_type ON insights(insight_type);
CREATE INDEX idx_insights_active ON insights(user_id, dismissed, expires_at);
```

---

## Component 5: Proactive Suggestion Delivery

**Goal:** Surface insights at the right time through the right channel.

### 5.1 Suggestion Delivery Agent

**New Agent Type: Proactive Assistant**

```toml
# Location: ~/.armaraos/agents/proactive-assistant.toml

name = "proactive-assistant"
module = "builtin:proactive"
description = "Monitors user behavior and proactively offers helpful suggestions"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"
system_prompt = """
You are a proactive assistant that helps users work more efficiently.

Your role:
- Monitor user behavior patterns
- Suggest relevant features and optimizations
- Link related ideas and concepts
- Teach new capabilities at the right moment
- Never interrupt unless highly relevant

Guidelines:
- Be concise and respectful of user's time
- Provide actionable suggestions
- Explain your reasoning briefly
- Respect user's privacy settings
- Learn from feedback (dismiss = reduce similar suggestions)
"""

[capabilities]
tools = [
    "insight_get",
    "insight_dismiss",
    "pattern_get",
    "profile_get",
    "context_get",
    "channel_send"
]
memory_read = ["insights.*", "patterns.*", "profiles.*"]
memory_write = ["insights.*"]

[scheduling]
# Run every 5 minutes when user is active
schedule = "*/5 * * * *"
only_when_active = true
```

**New Tools for Proactive Agent:**

```rust
// Location: crates/openfang-runtime/src/tool_runner.rs

// tool: insight_get
ToolDefinition {
    name: "insight_get".to_string(),
    description: "Get pending insights for a user, filtered by priority and context".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "user_id": { "type": "string" },
            "min_priority": { "type": "integer", "description": "Minimum priority (0-10)" },
            "context_relevant_only": { "type": "boolean", "description": "Only insights relevant to current context" },
            "limit": { "type": "integer", "description": "Max number of insights to return" }
        },
        "required": ["user_id"]
    }),
}

// tool: insight_dismiss
ToolDefinition {
    name: "insight_dismiss".to_string(),
    description: "Mark an insight as dismissed (user not interested)".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "insight_id": { "type": "string" },
            "reason": { "type": "string", "description": "Why dismissed (for learning)" }
        },
        "required": ["insight_id"]
    }),
}

// tool: context_get
ToolDefinition {
    name: "context_get".to_string(),
    description: "Get user's current working context (active task, agents, tools)".to_string(),
    input_schema: serde_json::json!({
        "type": "object",
        "properties": {
            "user_id": { "type": "string" }
        },
        "required": ["user_id"]
    }),
}
```

### 5.2 Delivery Channels

**Dashboard Notification Center:**

Extend existing notification center with proactive suggestions:

```javascript
// Location: static/js/pages/notifications.js

// New notification type: "suggestion"
{
  type: 'suggestion',
  insight_id: 'insight-123',
  priority: 8,
  title: 'Automation Opportunity',
  message: 'You repeat these 5 steps often. Want to automate?',
  actions: [
    { label: 'Show me how', action: 'accept', insight_id: 'insight-123' },
    { label: 'Not now', action: 'dismiss', reason: 'not_now' },
    { label: 'Never suggest this', action: 'dismiss', reason: 'not_interested' }
  ],
  timestamp: '2026-04-11T10:30:00Z'
}
```

**Inline Suggestions (Contextual):**

When user is performing an action, show relevant suggestion inline:

```html
<!-- Example: User is creating a workflow manually -->
<div class="inline-suggestion">
  <div class="suggestion-icon">💡</div>
  <div class="suggestion-content">
    <strong>Tip:</strong> You can use the <code>agent_coordinate</code> tool to build this workflow dynamically.
    <a href="#" onclick="showTutorial('agent_coordinate')">Learn more</a>
    <button onclick="dismissSuggestion('insight-456')">✕</button>
  </div>
</div>
```

**Learning Dashboard:**

New dashboard page: `#learning`

Features:
- **Pattern Insights** - Your behavioral patterns visualized
- **Skill Progress** - Tracked learning goals and milestones
- **Feature Discovery** - Recommended features to try
- **Efficiency Stats** - Time saved by automations, cost optimizations
- **Knowledge Graph** - Visual concept map with suggested connections

### 5.3 Smart Timing

**Delivery Timing Logic:**

```rust
// Location: crates/openfang-learning/src/delivery_scheduler.rs

pub struct DeliveryScheduler;

impl DeliveryScheduler {
    /// Determine if now is a good time to show a suggestion
    pub fn should_deliver_now(&self, insight: &Insight, context: &WorkingContext, prefs: &CommunicationPreferences) -> bool {
        // Check quiet hours
        if self.is_quiet_hours(prefs) {
            return false;
        }
        
        // Check recent suggestion frequency (don't spam)
        if self.too_many_recent_suggestions(context.user_id) {
            return false;
        }
        
        // Check context relevance
        if !self.is_contextually_relevant(insight, context) {
            return false;
        }
        
        // Check user activity (don't interrupt during focused work)
        if self.is_user_focused(context) {
            return false;
        }
        
        // Check priority threshold
        if insight.priority < self.get_priority_threshold(prefs) {
            return false;
        }
        
        true
    }
    
    /// Detect if user is in focused work mode (few interruptions)
    fn is_user_focused(&self, context: &WorkingContext) -> bool {
        // Check if user has been continuously active for >20 min
        // No page switches, high activity rate
        // Indicates deep work, don't interrupt
        todo!()
    }
    
    /// Get ideal delivery moments (context switch, task completion)
    pub fn find_ideal_moments(&self, context: &WorkingContext) -> Vec<DeliveryMoment> {
        vec![
            DeliveryMoment::TaskCompletion, // User just finished something
            DeliveryMoment::ContextSwitch,  // User switched to different task
            DeliveryMoment::LowActivity,    // User idle for >2 min
            DeliveryMoment::SessionStart,   // Beginning of work session
        ]
    }
}

#[derive(Debug, Clone)]
pub enum DeliveryMoment {
    TaskCompletion,
    ContextSwitch,
    LowActivity,
    SessionStart,
    Scheduled,
}
```

---

## Component 6: Privacy & Control

**Goal:** Give users full transparency and control over learning.

### 6.1 Privacy Dashboard

**New Dashboard Page:** `#privacy`

Features:
1. **Data Collection Toggle**
   - Enable/disable activity tracking
   - Granular control (tracking type by type)
   - Export all collected data (JSON download)
   - Delete all learning data (with confirmation)

2. **What We Know About You**
   - Show detected patterns (transparent)
   - Show user profile (skill level, preferences, etc.)
   - Show recent insights generated
   - Explanation for each data point

3. **Suggestion Controls**
   - Suggestion frequency slider
   - Disable specific insight types
   - Quiet hours configuration
   - Channel preferences

4. **Data Retention**
   - Retention period slider (7 days - forever)
   - Manual cleanup button
   - Auto-cleanup schedule

### 6.2 Feedback Loop

**Learning from User Feedback:**

```rust
// Location: crates/openfang-learning/src/feedback.rs

pub struct FeedbackProcessor;

impl FeedbackProcessor {
    /// Process user feedback on an insight
    pub async fn process_feedback(&self, insight_id: &str, action: InsightAction, reason: Option<String>) {
        match action {
            InsightAction::Accepted => {
                // Positive signal: increase confidence in similar insights
                // Increase priority of related insight types
                self.reinforce_pattern(insight_id).await;
            }
            
            InsightAction::Dismissed { reason } => {
                match reason.as_deref() {
                    Some("not_now") => {
                        // Neutral: reschedule for later
                        self.reschedule_insight(insight_id).await;
                    }
                    Some("not_interested") => {
                        // Negative: reduce similar suggestions
                        self.suppress_similar(insight_id).await;
                    }
                    Some("not_relevant") => {
                        // Improve relevance detection
                        self.adjust_relevance_model(insight_id).await;
                    }
                    _ => {}
                }
            }
            
            InsightAction::Reported => {
                // User reported as incorrect/harmful
                // Strong negative signal
                self.suppress_pattern(insight_id).await;
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum InsightAction {
    Accepted,
    Dismissed { reason: String },
    Reported,
}
```

### 6.3 Anonymization

**Anonymized Learning Mode:**

```rust
// When privacy_config.anonymize_user_id = true:

pub fn anonymize_user_id(user_id: &str) -> String {
    use sha2::{Sha256, Digest};
    
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    let result = hasher.finalize();
    
    // Return hex-encoded hash
    format!("anon_{}", hex::encode(&result[..16]))
}

// All learning data uses anonymized ID
// Original user_id never stored in learning tables
// Allows pattern recognition without PII
```

---

## API Endpoints

### Learning & Insights API

```
# Pattern recognition
GET  /api/learning/patterns                         # List user's patterns
GET  /api/learning/patterns/:id                     # Get pattern detail
POST /api/learning/patterns/:id/feedback            # Provide feedback on pattern

# User profile
GET  /api/learning/profile                          # Get user profile
PUT  /api/learning/profile                          # Update profile
POST /api/learning/profile/goals                    # Add a goal
PUT  /api/learning/profile/goals/:id                # Update goal progress

# Insights
GET  /api/learning/insights                         # List pending insights
GET  /api/learning/insights/:id                     # Get insight detail
POST /api/learning/insights/:id/dismiss             # Dismiss insight
POST /api/learning/insights/:id/accept              # Accept/act on insight
POST /api/learning/insights/:id/report              # Report problematic insight

# Working context
GET  /api/learning/context                          # Get current working context
PUT  /api/learning/context                          # Update working context

# Privacy controls
GET  /api/learning/privacy                          # Get privacy settings
PUT  /api/learning/privacy                          # Update privacy settings
POST /api/learning/privacy/export                   # Export all learning data
POST /api/learning/privacy/delete                   # Delete all learning data

# Analytics (aggregated, privacy-safe)
GET  /api/learning/stats                            # Usage stats
GET  /api/learning/feature-adoption                 # Feature adoption over time
GET  /api/learning/efficiency-gains                 # Time/cost savings

# Proactive assistant controls
GET  /api/learning/assistant/status                 # Is proactive assistant active?
POST /api/learning/assistant/enable                 # Enable assistant
POST /api/learning/assistant/disable                # Disable assistant
PUT  /api/learning/assistant/settings               # Update assistant settings
```

---

## Privacy & Ethical Considerations

### Data Minimization

- Only collect data necessary for improvement
- Aggregation over raw event storage where possible
- Automatic cleanup of old data (respect retention settings)

### Transparency

- Clear documentation of what is tracked
- Visible indicators when learning is active
- Explanation for every insight ("based on...")

### User Control

- Easy opt-out at any level (global, per-feature)
- Granular controls (not just on/off)
- Data portability (export) and right to delete

### Security

- Learning data encrypted at rest
- Access controls (only authorized agents)
- Audit trail for data access

### Bias Prevention

- Avoid reinforcing harmful patterns
- Diversity in insights (not just efficiency)
- Human review of high-priority suggestions

---

## Performance Considerations

### Computational Overhead

- Pattern detection runs async (background jobs)
- Event buffering prevents per-event processing
- Insight generation is lazy (on-demand + periodic)
- Caching heavily used (patterns, profiles, insights)

### Storage Growth

- Automatic data retention enforcement
- Aggregation of old events (hourly → daily → weekly)
- Index optimization for common queries

### Scalability

- Per-user isolation (one user's patterns don't affect others)
- Horizontal scaling: pattern detection can be distributed
- Ring buffers for event streams (bounded memory)

---

## Success Metrics

### Engagement

- % of users with proactive assistant enabled
- Insight acceptance rate (target: >30%)
- Insights acted upon per week

### Value Delivery

- Time saved through automation suggestions
- Cost savings realized
- Features discovered and adopted
- Workflows optimized

### User Satisfaction

- Suggestion relevance rating (survey)
- Dismissal rate by insight type
- Privacy controls usage (opt-out rate <10%)

### System Health

- Pattern detection latency (<5s)
- Insight generation latency (<2s)
- Storage growth rate (GB/user/month)

---

## Future Enhancements (Out of Scope)

1. **Multi-User Collaboration Patterns** - Learn team workflows
2. **Cross-Instance Learning** - Share anonymized patterns
3. **Predictive Error Detection** - Prevent errors before they happen
4. **Natural Language Queries** - "Show me what I'm doing inefficiently"
5. **Gamification** - Badges, streaks, challenges for skill development
6. **Social Learning** - Learn from similar users (opt-in)
7. **Voice-Based Suggestions** - Desktop app integration
8. **Adaptive UI** - Rearrange interface based on usage patterns

---

## Dependencies

### Existing Systems

- Memory substrate (SQLite, knowledge graph)
- Event bus (SystemEvent types)
- Session management
- Learning Frame v1 (skill capture)

### New Dependencies

- Pattern recognition libraries (time series analysis, clustering)
- Graph algorithms (for idea linking)
- Background job scheduler (periodic pattern analysis)
- Privacy compliance tools (data export, anonymization)

---

## References

- Current memory system: `crates/openfang-memory/`
- Event types: `crates/openfang-types/src/event.rs`
- Learning Frame v1: `docs/learning-frame-v1.md`
- Knowledge graph: `crates/openfang-memory/src/knowledge.rs`
- Session management: `crates/openfang-memory/src/session.rs`

---

**Document Status:** Ready for phased implementation planning  
**Next Steps:** Break into non-breaking phases with clear milestones
