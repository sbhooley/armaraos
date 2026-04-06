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
  return {
    health: {},
    status: {},
    usageSummary: {},
    recentAudit: [],
    channels: [],
    providers: [],
    mcpServers: [],
    skillCount: 0,
    scheduleCount: 0,
    /** GET /api/observability/snapshot */
    observability: {},
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
    /** When false (onboarded users), Setup Wizard is hidden until they click "Get started". */
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

    /** Sidebar "Get started" clicked while already on this page — reveal wizard for onboarded users. */
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

    async loadOverview() {
      this.loading = true;
      clearPageLoadError(this);
      try {
        await Promise.all([
          this.loadHealth(),
          this.loadStatus(),
          this.loadUsage(),
          this.loadAudit(),
          this.loadChannels(),
          this.loadSchedules(),
          this.loadProviders(),
          this.loadMcpServers(),
          this.loadSkills(),
          this.loadObservability()
        ]);
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

    async loadData() { return this.loadOverview(); },

    // Silent background refresh (no loading spinner)
    async silentRefresh() {
      try {
        await Promise.all([
          this.loadHealth(),
          this.loadStatus(),
          this.loadUsage(),
          this.loadAudit(),
          this.loadChannels(),
          this.loadSchedules(),
          this.loadProviders(),
          this.loadMcpServers(),
          this.loadSkills(),
          this.loadObservability()
        ]);
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
        var data = await OpenFangAPI.get('/api/usage');
        var agents = data.agents || [];
        var totalTokens = 0;
        var totalTools = 0;
        var totalCost = 0;
        agents.forEach(function(a) {
          totalTokens += (a.total_tokens || 0);
          totalTools += (a.tool_calls || 0);
          totalCost += (a.cost_usd || 0);
        });
        this.usageSummary = {
          total_tokens: totalTokens,
          total_tools: totalTools,
          total_cost: totalCost,
          agent_count: agents.length
        };
      } catch(e) {
        this.usageSummary = { total_tokens: 0, total_tools: 0, total_cost: 0, agent_count: 0 };
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
        this.mcpServers = data.servers || [];
      } catch(e) { this.mcpServers = []; }
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

    get connectedMcp() {
      return this.mcpServers.filter(function(s) { return s.status === 'connected'; });
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
        { key: 'skill', label: 'Browse or install a skill (optional)', done: false, action: '#skills', perpetual: true }
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
      return 'Core complete — optional channel ' + this.setupOptionalDoneCount + '/' + this.setupOptionalTotal + ' · message & skills stay open below';
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
      if (!secs) return '-';
      var d = Math.floor(secs / 86400);
      var h = Math.floor((secs % 86400) / 3600);
      var m = Math.floor((secs % 3600) / 60);
      if (d > 0) return d + 'd ' + h + 'h';
      if (h > 0) return h + 'h ' + m + 'm';
      return m + 'm';
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
  };
}
