// ArmaraOS Overview Dashboard — Landing page with system stats + provider status
'use strict';

/** True when `armaraos-kernel-event` should trigger an overview refresh (agent/system signals). */
function overviewShouldRefreshOnKernelEvent(ev) {
  if (!ev || !ev.detail) return false;
  var p = ev.detail.payload;
  if (!p) return false;
  if (p.type === 'Lifecycle') {
    var e = p.data && p.data.event;
    return e === 'Spawned' || e === 'Terminated' || e === 'Crashed' || e === 'Started' ||
      e === 'Suspended' || e === 'Resumed';
  }
  if (p.type === 'System') {
    var se = p.data && p.data.event;
    return se === 'KernelStarted' || se === 'KernelStopping' || se === 'QuotaEnforced' ||
      se === 'HealthCheckFailed' || se === 'QuotaWarning';
  }
  return false;
}

function overviewPage() {
  return Object.assign(armaraosFleetVitalsCore(), {
    /** For fleet vitals + agent cards; always null on Command Center (no inline chat). */
    activeChatAgent: null,
    filterState: 'all',
    /** Guard so fleet periodic refresh starts once per overview mount. */
    _ccFleetStarted: false,

    health: {},
    status: {},
    usageSummary: {},
    budget: {},
    recentAudit: [],
    channels: [],
    providers: [],
    mcpServers: [],
    /** GET /api/mcp/servers `readiness` object (`version` + `checks`). */
    mcpReadiness: null,
    /** @deprecated use `mcpReadiness.checks.calendar` — kept for one release. */
    mcpCalendarReadiness: null,
    skillCount: 0,
    scheduleCount: 0,
    /** GET /api/observability/snapshot */
    observability: {},
    /** Recently opened agents (from localStorage). */
    recentAgents: [],
    loading: true,
    loadError: '',
    loadErrorDetail: '',
    loadErrorHint: '',
    loadErrorRequestId: '',
    loadErrorWhere: '',
    loadErrorServerPath: '',
    refreshTimer: null,
    lastRefresh: null,
    _kernelEventHandler: null,
    _kernelDebounce: null,

    /** `localStorage` openfang-onboarded — finished setup wizard (or dismissed onboarding). */
    onboarded: false,
    /** When false (onboarded users), Setup Wizard is hidden until they click "Command Center". */
    overviewWizardCtaVisible: true,

    initOverviewWizardCta() {
      try {
        this.onboarded = localStorage.getItem('openfang-onboarded') === 'true';
        this.overviewWizardCtaVisible = !this.onboarded;
      } catch (e) {
        this.onboarded = false;
        this.overviewWizardCtaVisible = true;
      }
    },

    revealSetupWizardCta() {
      this.overviewWizardCtaVisible = true;
    },

    /** Sidebar "Command Center" clicked while already on this page — reveal wizard for onboarded users. */
    onOverviewNavSamePage() {
      try {
        if (localStorage.getItem('openfang-onboarded') === 'true') {
          this.revealSetupWizardCta();
        }
      } catch (e) { /* ignore */ }
    },

    refreshOnboardingFlags() {
      try {
        var now = localStorage.getItem('openfang-onboarded') === 'true';
        if (now && !this.onboarded) {
          this.overviewWizardCtaVisible = false;
        }
        this.onboarded = now;
      } catch (e) { /* ignore */ }
    },

    loadRecentAgents() {
      try {
        var raw = localStorage.getItem('armaraos-recent-agents');
        var list = raw ? JSON.parse(raw) : [];
        var agents = Alpine.store('app').agents || [];
        var agentMap = {};
        agents.forEach(function(a) { agentMap[String(a.id)] = a; });
        this.recentAgents = list.slice(0, 3).map(function(r) {
          var live = agentMap[r.id];
          return live ? live : { id: r.id, name: r.name || r.id, identity: { emoji: r.emoji || '' } };
        });
      } catch (e) { this.recentAgents = []; }
    },

    async loadOverview() {
      this.loading = true;
      clearPageLoadError(this);
      this.loadRecentAgents();
      try {
        await Promise.all([
          this.loadHealth(),
          this.loadStatus(),
          this.loadUsage(),
          this.loadBudget(),
          this.loadAudit(),
          this.loadChannels(),
          this.loadSchedules(),
          this.loadProviders(),
          this.loadMcpServers(),
          this.loadSkills(),
          this.loadObservability()
        ]);
        try {
          var a = Alpine.store('app');
          if (a && typeof a.refreshAgents === 'function') await a.refreshAgents();
        } catch (e) { /* ignore */ }
        this.lastRefresh = Date.now();
        this.refreshOnboardingFlags();
      } catch(e) {
        applyPageLoadError(this, e, 'Could not load overview data.');
      }
      this.loading = false;
    },

    copyOverviewErrorDebug() {
      copyPageLoadErrorDebug(this, 'ArmaraOS overview load error');
    },

    /** Command Center quick action: Settings → Providers (via session key read on settings load). */
    goConfigureProviders() {
      try { sessionStorage.setItem('armaraos-settings-tab', 'providers'); } catch (e) { /* ignore */ }
      if (typeof location !== 'undefined') location.hash = 'settings';
    },

    async loadData() { return this.loadOverview(); },

    // Silent background refresh (no loading spinner)
    async silentRefresh() {
      try {
        await Promise.all([
          this.loadHealth(),
          this.loadStatus(),
          this.loadUsage(),
          this.loadBudget(),
          this.loadAudit(),
          this.loadChannels(),
          this.loadSchedules(),
          this.loadProviders(),
          this.loadMcpServers(),
          this.loadSkills(),
          this.loadObservability()
        ]);
        try {
          var a2 = Alpine.store('app');
          if (a2 && typeof a2.refreshAgents === 'function') await a2.refreshAgents();
        } catch (e) { /* ignore */ }
        this.lastRefresh = Date.now();
        this.refreshOnboardingFlags();
      } catch(e) { /* silent */ }
    },

    startAutoRefresh() {
      this.stopAutoRefresh();
      this.refreshTimer = setInterval(() => this.silentRefresh(), 30000);
      this.bindKernelEventRefresh();
    },

    stopAutoRefresh() {
      if (this.refreshTimer) {
        clearInterval(this.refreshTimer);
        this.refreshTimer = null;
      }
      this.unbindKernelEventRefresh();
    },

    /** Tears down Command Center fleet timers and refresh state when leaving #overview. */
    onOverviewPageLeave() {
      this.stopAutoRefresh();
      this._ccFleetStarted = false;
      try { if (typeof this.fleetTeardown === 'function') this.fleetTeardown(); } catch (e) { /* ignore */ }
    },

    startCommandCenterFleet: function() {
      if (this._ccFleetStarted) return;
      this._ccFleetStarted = true;
      var self = this;
      this.demoProfile = this.readFleetDemoProfileFromEnv();
      this.demoMode = this.readFleetDemoFromEnv();
      this.applyDemoThemeIfNeeded();
      this.loadPersistedAgentHourly();
      this.pruneAgentHourlyMaps();
      this.loadPersistedHourlyUiPrefs();
      this.$nextTick(function() {
        self.syncAgentActivityFeeds();
        self.fleetStartPeriodic();
        if (self._activitySyncInt) { try { clearInterval(self._activitySyncInt); } catch (eA) { /* ignore */ } self._activitySyncInt = null; }
        self._activitySyncInt = setInterval(function() { self.syncAgentActivityFeeds(); }, 650);
        if (self._idlePulseInt) { try { clearInterval(self._idlePulseInt); } catch (eB) { /* ignore */ } self._idlePulseInt = null; }
        self._idlePulseInt = setInterval(function() {
          self._idlePulseTick = (self._idlePulseTick + 1) % 10000;
        }, 1200);
        if (self._activePulseInt) { try { clearInterval(self._activePulseInt); } catch (eC) { /* ignore */ } self._activePulseInt = null; }
        self._activePulseInt = setInterval(function() {
          self.tickActivePhaseMicroActivity();
        }, 1100);
        if (self.demoMode) self.fleetStartDemo();
      });
    },

    /** Open agent settings on All Agents; used by vitals card gear. */
    showDetail: function(agent) {
      if (!agent) return;
      try { Alpine.store('app').openAgentSettings(agent); } catch (e) { try { if (location && location.hash !== undefined) { location.hash = 'agents'; } } catch (e2) { /* ignore */ } }
    },

    /** When opened from a template that expects the Agents inline chat. */
    chatWithAgent: function(agent) {
      if (!agent) return;
      try { Alpine.store('app').openAgentChat(agent); } catch (e) { try { if (location) { location.hash = 'agents'; } } catch (e2) { /* ignore */ } }
    },

    /** Debounced refresh when kernel SSE emits lifecycle/system events (same tab). */
    bindKernelEventRefresh() {
      var self = this;
      if (this._kernelEventHandler) return;
      this._kernelEventHandler = function(ev) {
        if (!overviewShouldRefreshOnKernelEvent(ev)) return;
        if (self._kernelDebounce) clearTimeout(self._kernelDebounce);
        self._kernelDebounce = setTimeout(function() {
          self._kernelDebounce = null;
          self.silentRefresh();
          try {
            var app = Alpine.store('app');
            if (app && typeof app.refreshAgents === 'function') app.refreshAgents();
          } catch (e) { /* ignore */ }
        }, 400);
      };
      window.addEventListener('armaraos-kernel-event', this._kernelEventHandler);
    },

    unbindKernelEventRefresh() {
      if (this._kernelEventHandler) {
        window.removeEventListener('armaraos-kernel-event', this._kernelEventHandler);
        this._kernelEventHandler = null;
      }
      if (this._kernelDebounce) {
        clearTimeout(this._kernelDebounce);
        this._kernelDebounce = null;
      }
    },

    async loadHealth() {
      try {
        this.health = await OpenFangAPI.get('/api/health');
      } catch(e) { this.health = { status: 'unreachable' }; }
    },

    async loadStatus() {
      try {
        this.status = await OpenFangAPI.get('/api/status');
      } catch(e) { this.status = {}; throw e; }
    },

    async loadUsage() {
      try {
        // Use persistent SQLite-backed totals so Get started survives daemon
        // restarts and desktop upgrades/reinstalls.
        var summary = await OpenFangAPI.get('/api/usage/summary');
        var totalTokens = (summary.total_input_tokens || 0) + (summary.total_output_tokens || 0);
        var totalTools = summary.total_tool_calls || 0;
        var totalCost = summary.total_cost_usd || 0;
        var appAgents = (Alpine.store('app').agents || []);
        this.usageSummary = {
          total_tokens: totalTokens,
          total_tools: totalTools,
          total_cost: totalCost,
          agent_count: appAgents.length,
          quota_enforcement: summary.quota_enforcement || {},
          compression_savings: summary.compression_savings || null
        };
      } catch(e) {
        this.usageSummary = {
          total_tokens: 0,
          total_tools: 0,
          total_cost: 0,
          agent_count: 0,
          quota_enforcement: {},
          compression_savings: null
        };
      }
    },

    async loadBudget() {
      try {
        this.budget = await OpenFangAPI.get('/api/budget');
      } catch(e) {
        this.budget = { total_spent_usd: 0, budget_limit_usd: 0, period: 'monthly' };
      }
    },

    async loadAudit() {
      try {
        var data = await OpenFangAPI.get('/api/audit/recent?n=8');
        this.recentAudit = data.entries || [];
      } catch(e) { this.recentAudit = []; }
    },

    async loadChannels() {
      try {
        var data = await OpenFangAPI.get('/api/channels');
        this.channels = (data.channels || []).filter(function(ch) { return ch.has_token; });
      } catch(e) { this.channels = []; }
    },

    async loadSchedules() {
      try {
        var data = await OpenFangAPI.get('/api/schedules');
        this.scheduleCount = (data.schedules || []).filter(function(s) { return s && s.enabled !== false; }).length;
      } catch(e) { this.scheduleCount = 0; }
    },

    async loadProviders() {
      try {
        var data = await OpenFangAPI.get('/api/providers');
        this.providers = data.providers || [];
      } catch(e) { this.providers = []; }
    },

    async loadMcpServers() {
      try {
        var data = await OpenFangAPI.get('/api/mcp/servers');
        var configured = Array.isArray(data.configured) ? data.configured : [];
        var connected = Array.isArray(data.connected) ? data.connected : [];
        var mergedByName = {};
        configured.forEach(function(s) {
          var n = String((s && s.name) || '').trim();
          if (!n) return;
          if (!mergedByName[n]) mergedByName[n] = { name: n, status: 'configured' };
        });
        connected.forEach(function(s) {
          var n = String((s && s.name) || '').trim();
          if (!n) return;
          var row = mergedByName[n] || { name: n, status: 'configured' };
          row.status = 'connected';
          if (s && s.tool_count != null) row.tool_count = s.tool_count;
          mergedByName[n] = row;
        });
        this.mcpServers = Object.keys(mergedByName).sort().map(function(name) {
          return mergedByName[name];
        });
        this.mcpReadiness = data.readiness || null;
        this.mcpCalendarReadiness = data.calendar_readiness || null;
      } catch(e) { this.mcpServers = []; this.mcpReadiness = null; }
    },

    async loadSkills() {
      try {
        var data = await OpenFangAPI.get('/api/skills');
        this.skillCount = (data.skills || []).length;
      } catch(e) { this.skillCount = 0; }
    },

    async loadObservability() {
      try {
        this.observability = await OpenFangAPI.get('/api/observability/snapshot');
      } catch(e) {
        this.observability = {};
      }
    },

    get configuredProviders() {
      return this.providers.filter(function(p) { return p.auth_status === 'configured'; });
    },

    get unconfiguredProviders() {
      return this.providers.filter(function(p) { return p.auth_status === 'not_set' || p.auth_status === 'missing'; });
    },

    get availableMcp() {
      return this.mcpServers.filter(function(s) { return s && s.name; });
    },

    get connectedMcp() {
      return this.availableMcp.filter(function(s) { return s.status === 'connected'; });
    },

    get graphMemoryMetrics() {
      var m = (this.status && this.status.graph_memory_context_metrics) || {};
      var c = (this.status && this.status.graph_memory_contract_metrics) || {};
      return {
        episodic: Number(m.injected_episodic_total || 0),
        semantic: Number(m.injected_semantic_total || 0),
        conflict: Number(m.injected_conflict_total || 0),
        procedural: Number(m.injected_procedural_total || 0),
        truncations: Number(m.truncation_hits_total || 0),
        skippedLowQuality: Number(m.skipped_low_quality_total || 0),
        tempReadsSuppressed: Number(m.temp_mode_suppressed_reads_total || 0),
        tempWritesSuppressed: Number(m.temp_mode_suppressed_writes_total || 0),
        rolloutReadsSuppressed: Number(m.rollout_suppressed_reads_total || 0),
        rolloutWritesSuppressed: Number(m.rollout_suppressed_writes_total || 0),
        provenanceGatePass: m.provenance_gate_pass !== false,
        contradictionGatePass: m.contradiction_gate_pass !== false,
        inboxImported: Number(c.inbox_imported_total || 0),
        inboxQuarantined: Number(c.inbox_quarantined_total || 0),
        inboxInvalidScopeSkipped: Number(c.inbox_invalid_scope_skipped_total || 0),
      };
    },

    get graphMemorySignalsActive() {
      var m = this.graphMemoryMetrics;
      return (m.episodic + m.semantic + m.conflict + m.procedural) > 0 ||
        m.tempReadsSuppressed > 0 || m.tempWritesSuppressed > 0 ||
        m.rolloutReadsSuppressed > 0 || m.rolloutWritesSuppressed > 0;
    },

    /** Entries for readiness chips (`readiness.checks` from API). */
    get mcpReadinessChecks() {
      var r = this.mcpReadiness;
      if (!r || !r.checks || typeof r.checks !== 'object') return [];
      var out = [];
      try {
        Object.keys(r.checks).forEach(function(id) {
          var c = r.checks[id];
          if (!c) return;
          out.push({
            id: id,
            label: c.label || id,
            ready: !!c.ready,
            severity: c.severity || (c.ready ? 'ok' : 'warn')
          });
        });
      } catch (e) { return []; }
      return out;
    },

    // Provider health badge color
    providerBadgeClass(p) {
      if (p.auth_status === 'configured') {
        if (p.health === 'cooldown' || p.health === 'open') return 'badge-warn';
        return 'badge-success';
      }
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') return 'badge-muted';
      return 'badge-dim';
    },

    // Provider health tooltip
    providerTooltip(p) {
      if (p.health === 'cooldown') return p.display_name + ' \u2014 cooling down (rate limited)';
      if (p.health === 'open') return p.display_name + ' \u2014 circuit breaker open';
      if (p.auth_status === 'configured') return p.display_name + ' \u2014 ready';
      return p.display_name + ' \u2014 not configured';
    },

    // Audit action badge color
    actionBadgeClass(action) {
      if (!action) return 'badge-dim';
      if (action === 'AgentSpawn' || action === 'AuthSuccess') return 'badge-success';
      if (action === 'AgentKill' || action === 'AgentTerminated' || action === 'AuthFailure' || action === 'CapabilityDenied') return 'badge-error';
      if (action === 'RateLimited' || action === 'ToolInvoke') return 'badge-warn';
      return 'badge-created';
    },

    // ── Setup Checklist ──
    checklistDismissed: localStorage.getItem('of-checklist-dismissed') === 'true',

    get setupChecklist() {
      return [
        { key: 'provider', label: 'Configure an LLM provider', done: this.configuredProviders.length > 0, action: '#settings' },
        { key: 'agent', label: 'Create your first agent', done: (Alpine.store('app').agents || []).length > 0, action: '#agents' },
        { key: 'schedule', label: 'Create a scheduled job', done: this.scheduleCount > 0, action: '#scheduler' },
        { key: 'channel', label: 'Connect a messaging channel (optional)', done: this.channels.length > 0, action: '#channels' },
        // Shortcuts only — never marked complete (always show ○ + Go)
        { key: 'chat', label: 'Send your first message (optional)', done: false, action: '#agents', perpetual: true },
        { key: 'skill', label: 'Browse skills or MCP servers (optional)', done: false, action: '#skills', perpetual: true }
      ];
    },

    get setupChecklistCore() {
      return this.setupChecklist.filter(function(i) {
        return i.key === 'provider' || i.key === 'agent' || i.key === 'schedule';
      });
    },

    get setupChecklistOptional() {
      return this.setupChecklist.filter(function(i) {
        return i.key === 'channel' || i.key === 'chat' || i.key === 'skill';
      });
    },

    /** Card title: phase shifts once core (provider, agent, schedule) is complete. */
    get setupChecklistCardTitle() {
      if (this.setupCoreDoneCount < this.setupCoreTotal) return 'Getting Started';
      return 'Optional setup';
    },

    get setupProgress() {
      var required = this.setupChecklist.filter(function(i) {
        return i.key !== 'chat' && i.key !== 'skill' && i.key !== 'channel';
      });
      var doneReq = required.filter(function(item) { return item.done; }).length;
      return (doneReq / required.length) * 100;
    },

    /** Completable rows only (excludes perpetual chat/skill shortcuts). */
    get setupDoneCount() {
      return this.setupChecklist.filter(function(item) {
        return !item.perpetual && item.done;
      }).length;
    },

    get setupTrackableTotal() {
      return this.setupChecklist.filter(function(item) { return !item.perpetual; }).length;
    },

    get setupCoreDoneCount() {
      return this.setupChecklist.filter(function(item) {
        return item.key !== 'chat' && item.key !== 'skill' && item.key !== 'channel' && item.done;
      }).length;
    },

    get setupCoreTotal() {
      return 3;
    },

    get setupOptionalDoneCount() {
      var ch = this.setupChecklist.find(function(item) { return item.key === 'channel'; });
      return ch && ch.done ? 1 : 0;
    },

    /** Only the channel row counts toward optional progress (chat/skill stay perpetual). */
    get setupOptionalTotal() {
      return 1;
    },

    /** Show checklist until core + optional tasks are done, or user dismisses. */
    get showSetupChecklist() {
      if (this.loading || this.loadError || this.checklistDismissed) return false;
      if (this.setupCoreDoneCount < this.setupCoreTotal) return true;
      return this.setupOptionalDoneCount < this.setupOptionalTotal;
    },

    get setupChecklistSubtitle() {
      if (this.setupCoreDoneCount < this.setupCoreTotal) {
        return this.setupCoreDoneCount + '/' + this.setupCoreTotal + ' core steps · ' + this.setupDoneCount + '/' + this.setupTrackableTotal + ' completed';
      }
      return 'Core complete — optional channel ' + this.setupOptionalDoneCount + '/' + this.setupOptionalTotal + ' · chat & skills/MCP shortcuts stay open below';
    },

    /** After core steps, drive the bar from optional tasks (0–100%). */
    get setupProgressForBar() {
      if (this.setupCoreDoneCount < this.setupCoreTotal) return this.setupProgress;
      return (this.setupOptionalDoneCount / this.setupOptionalTotal) * 100;
    },

    dismissChecklist() {
      this.checklistDismissed = true;
      localStorage.setItem('of-checklist-dismissed', 'true');
    },

    formatUptime(secs) {
      var n = secs == null || secs === '' ? NaN : Number(secs);
      if (!Number.isFinite(n) || n < 0) return '—';
      n = Math.floor(n);
      if (n < 60) return n + 's';
      var d = Math.floor(n / 86400);
      var h = Math.floor((n % 86400) / 3600);
      var m = Math.floor((n % 3600) / 60);
      if (d > 0) return d + 'd ' + h + 'h';
      if (h > 0) return h + 'h ' + m + 'm';
      return m + 'm';
    },

    /** Input tokens avoided by caps/budget (all-time from usage summary; else 7d from status). */
    get overviewQuotaInputAvoided() {
      var sum = (this.usageSummary && this.usageSummary.quota_enforcement) || {};
      var st = (this.status && this.status.quota_enforcement) || {};
      var v = sum.total_est_input_tokens_avoided;
      if (v == null || v === undefined) v = st.total_est_input_tokens_avoided;
      var n = v == null ? 0 : Number(v);
      return Number.isFinite(n) ? Math.max(0, Math.floor(n)) : 0;
    },

    get overviewQuotaCostAvoidedUsd() {
      var sum = (this.usageSummary && this.usageSummary.quota_enforcement) || {};
      var st = (this.status && this.status.quota_enforcement) || {};
      var v = sum.total_est_cost_avoided_usd;
      if (v == null || v === undefined) v = st.total_est_cost_avoided_usd;
      var n = v == null ? 0 : Number(v);
      return Number.isFinite(n) ? Math.max(0, n) : 0;
    },

    /**
     * Compression + prompt-cache counterfactuals (SQLite all-time) plus quota-avoided input.
     * Falls back to `/api/status` 7d eco when summary has no `compression_savings` yet.
     */
    get overviewTokensSaved() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      if (cs && cs.estimated_total_input_tokens_saved != null) {
        return (Number(cs.estimated_total_input_tokens_saved) || 0) + this.overviewQuotaInputAvoided;
      }
      var ec = (this.status && this.status.eco_compression) || {};
      var v = ec.estimated_total_input_tokens_saved;
      var n = v == null ? 0 : Number(v);
      var comp = Number.isFinite(n) ? Math.max(0, Math.floor(n)) : 0;
      return comp + this.overviewQuotaInputAvoided;
    },

    get overviewCostSavedUsd() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      if (cs && cs.estimated_total_cost_saved_usd != null) {
        return (Number(cs.estimated_total_cost_saved_usd) || 0) + this.overviewQuotaCostAvoidedUsd;
      }
      var ec = (this.status && this.status.eco_compression) || {};
      var v = ec.estimated_total_cost_saved_usd;
      var n = v == null ? 0 : Number(v);
      var comp = Number.isFinite(n) ? Math.max(0, n) : 0;
      return comp + this.overviewQuotaCostAvoidedUsd;
    },

    /**
     * Sum of pre-compression input tokens (audit baseline) across persisted compression turns.
     * Persisted in `eco_compression_events.original_tokens_est` per turn since v15. `0` when no rows.
     */
    get overviewOriginalInputTokensTotal() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      var v = cs && cs.original_input_tokens_total;
      var n = v == null ? 0 : Number(v);
      return Number.isFinite(n) ? Math.max(0, Math.floor(n)) : 0;
    },

    /**
     * Sum of provider-reported (billed) input tokens across persisted compression turns.
     * `0` for rows persisted before v15 (provider usage was not yet captured per row).
     */
    get overviewBilledInputTokensTotal() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      var v = cs && cs.billed_input_tokens_total;
      var n = v == null ? 0 : Number(v);
      return Number.isFinite(n) ? Math.max(0, Math.floor(n)) : 0;
    },

    /** Catalog-priced cost actually billed for the input tokens above (USD). */
    get overviewBilledInputCostUsdTotal() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      var v = cs && cs.billed_input_cost_usd_total;
      var n = v == null ? 0 : Number(v);
      return Number.isFinite(n) ? Math.max(0, n) : 0;
    },

    /**
     * Per-(provider, model) rollup: turns, original vs billed input tokens, USD saved/billed.
     * Empty array when API doesn't return `compression_savings.by_provider_model`.
     */
    get overviewCompressionByProviderModel() {
      var cs = (this.usageSummary && this.usageSummary.compression_savings) || null;
      var arr = cs && cs.by_provider_model;
      return Array.isArray(arr) ? arr : [];
    },

    formatNumber(n) {
      if (!n) return '0';
      if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
      if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
      return String(n);
    },

    /** Humanize CamelCase event names from SSE payload. */
    friendlyKernelEventName(name) {
      if (!name) return '';
      var s = String(name);
      var map = {
        AgentActivity: 'Agent activity',
        AgentSpawn: 'Agent started',
        AgentKill: 'Agent stopped',
        KernelReady: 'Kernel ready',
        Shutdown: 'Shutting down',
      };
      if (map[s]) return map[s];
      return s.replace(/([A-Z])/g, ' $1').replace(/^\s+/, '').trim();
    },

    /** One-line summary of the latest SSE kernel event (from Alpine.store('kernelEvents').last). */
    formatLastKernelEvent(j) {
      if (!j || !j.payload) return '';
      var p = j.payload;
      var ts = '';
      if (j.timestamp) ts = this.timeAgo(j.timestamp) + ' · ';
      if (p.type === 'Lifecycle' && p.data && p.data.event) {
        return ts + 'Lifecycle · ' + this.friendlyKernelEventName(p.data.event);
      }
      if (p.type === 'System' && p.data && p.data.event) {
        return ts + 'System · ' + this.friendlyKernelEventName(p.data.event);
      }
      if (p.type) return ts + this.friendlyKernelEventName(p.type);
      return ts + 'Kernel update';
    },

    formatCost(n) {
      if (!n || n === 0) return '$0.00';
      if (n < 0.01) return '<$0.01';
      return '$' + n.toFixed(2);
    },

    // Relative time formatting ("2m ago", "1h ago", "just now")
    timeAgo(timestamp) {
      if (!timestamp) return '';
      var now = Date.now();
      var ts = new Date(timestamp).getTime();
      var diff = Math.floor((now - ts) / 1000);
      if (diff < 10) return 'just now';
      if (diff < 60) return diff + 's ago';
      if (diff < 3600) return Math.floor(diff / 60) + 'm ago';
      if (diff < 86400) return Math.floor(diff / 3600) + 'h ago';
      return Math.floor(diff / 86400) + 'd ago';
    },

    // Map raw audit action names to user-friendly labels
    friendlyAction(action) {
      if (!action) return 'Unknown';
      var map = {
        'AgentSpawn': 'Agent Created',
        'AgentKill': 'Agent Stopped',
        'AgentTerminated': 'Agent Stopped',
        'ToolInvoke': 'Tool Used',
        'ToolResult': 'Tool Completed',
        'MessageReceived': 'Message In',
        'MessageSent': 'Response Sent',
        'SessionReset': 'Session Reset',
        'SessionCompact': 'Compacted',
        'ModelSwitch': 'Model Changed',
        'AuthAttempt': 'Login Attempt',
        'AuthSuccess': 'Login OK',
        'AuthFailure': 'Login Failed',
        'CapabilityDenied': 'Denied',
        'RateLimited': 'Rate Limited',
        'WorkflowRun': 'Workflow Run',
        'TriggerFired': 'Trigger Fired',
        'SkillInstalled': 'Skill Installed',
        'McpConnected': 'MCP Connected'
      };
      return map[action] || action.replace(/([A-Z])/g, ' $1').trim();
    },

    // Audit action icon (small inline SVG)
    actionIcon(action) {
      if (!action) return '';
      var icons = {
        'AgentSpawn': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M12 8v8M8 12h8"/></svg>',
        'AgentKill': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M15 9l-6 6M9 9l6 6"/></svg>',
        'AgentTerminated': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><path d="M15 9l-6 6M9 9l6 6"/></svg>',
        'ToolInvoke': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>',
        'MessageReceived': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
        'MessageSent': '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z"/></svg>'
      };
      return icons[action] || '<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/></svg>';
    },

    // Resolve agent UUID to name if possible
    agentName(agentId) {
      if (!agentId) return '-';
      var agents = Alpine.store('app').agents || [];
      var agent = agents.find(function(a) { return a.id === agentId; });
      return agent ? agent.name : agentId.substring(0, 8) + '\u2026';
    }
  });
}
