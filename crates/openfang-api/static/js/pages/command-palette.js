// ArmaraOS Command Palette — Cmd/Ctrl+K launcher
// Searches pages, agents, actions, and recent sessions.
'use strict';

var CMD_PALETTE_PAGES = [
  { label: 'Get started', sublabel: 'Overview, setup checklist, quick actions', hash: 'overview',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m3 9 9-7 9 7v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/><path d="M9 22V12h6v10"/></svg>' },
  { label: 'All Agents', sublabel: 'Chat with and manage agents', hash: 'agents',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>' },
  { label: 'Bookmarks', sublabel: 'Saved chat snippets', hash: 'bookmarks',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m19 21-7-4-7 4V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2v16z"/></svg>' },
  { label: 'Sessions', sublabel: 'Conversation history and memory', hash: 'sessions',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m12 2-10 5 10 5 10-5z"/><path d="m2 17 10 5 10-5"/><path d="m2 12 10 5 10-5"/></svg>' },
  { label: 'Approvals', sublabel: 'Pending tool approval requests', hash: 'approvals',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/></svg>' },
  { label: 'Comms', sublabel: 'Agent communication topology', hash: 'comms',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="18" cy="5" r="3"/><circle cx="6" cy="12" r="3"/><circle cx="18" cy="19" r="3"/><line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/><line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/></svg>' },
  { label: 'Network', sublabel: 'OFP peer network status', hash: 'network',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="2" width="6" height="6" rx="1"/><rect x="16" y="2" width="6" height="6" rx="1"/><rect x="9" y="16" width="6" height="6" rx="1"/><path d="M5 8v3a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8"/><path d="M12 12v4"/></svg>' },
  { label: 'Workflows', sublabel: 'Automate multi-step agent tasks', hash: 'workflows',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3v12M18 9a9 9 0 0 1-9 9"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/></svg>' },
  { label: 'Scheduler', sublabel: 'Cron jobs and scheduled runs', hash: 'scheduler',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/><polyline points="8 14 10 16 16 11"/></svg>' },
  { label: 'Channels', sublabel: 'Discord, Slack, email integrations', hash: 'channels',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 9h16M4 15h16M10 3l-2 18M16 3l-2 18"/></svg>' },
  { label: 'Skills', sublabel: 'Installed ClawHub skills', hash: 'skills',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/></svg>' },
  { label: 'Hands', sublabel: 'Browser automation and computer use', hash: 'hands',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 11V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v0"/><path d="M14 10V4a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v2"/><path d="M10 10.5V6a2 2 0 0 0-2-2v0a2 2 0 0 0-2 2v8"/><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15"/></svg>' },
  { label: 'App Store', sublabel: 'Browse AINL programs', hash: 'ainl-library',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2 3 6v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6l-3-4z"/><line x1="3" y1="6" x2="21" y2="6"/><path d="M16 10a4 4 0 0 1-8 0"/></svg>' },
  { label: 'Home Files', sublabel: 'Browse ArmaraOS home folder', hash: 'home-files',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/></svg>' },
  { label: 'Analytics', sublabel: 'Token usage and cost breakdown', hash: 'analytics',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="20" x2="18" y2="10"/><line x1="12" y1="20" x2="12" y2="4"/><line x1="6" y1="20" x2="6" y2="14"/></svg>' },
  { label: 'Logs', sublabel: 'Live log stream, daemon logs, audit trail', hash: 'logs',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><polyline points="14 2 14 8 20 8"/><line x1="16" y1="13" x2="8" y2="13"/><line x1="16" y1="17" x2="8" y2="17"/><polyline points="10 9 9 9 8 9"/></svg>' },
  { label: 'Timeline', sublabel: 'Agent event history', hash: 'timeline',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="8" y1="6" x2="21" y2="6"/><line x1="8" y1="12" x2="21" y2="12"/><line x1="8" y1="18" x2="21" y2="18"/><line x1="3" y1="6" x2="3.01" y2="6"/><line x1="3" y1="12" x2="3.01" y2="12"/><line x1="3" y1="18" x2="3.01" y2="18"/></svg>' },
  { label: 'Daemon & Runtime', sublabel: 'Reload config, restart, shutdown', hash: 'runtime',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 2v4"/><path d="M12 18v4"/><path d="m4.93 4.93 2.83 2.83"/><path d="m16.24 16.24 2.83 2.83"/><path d="M2 12h4"/><path d="M18 12h4"/><path d="m4.93 19.07 2.83-2.83"/><path d="m16.24 7.76 2.83-2.83"/></svg>' },
  { label: 'Settings', sublabel: 'Providers, models, security, budget', hash: 'settings',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3"/><path d="M1 14h6M9 8h6M17 16h6"/></svg>' },
];

var CMD_PALETTE_ACTIONS = [
  { label: 'New Agent', sublabel: 'Create a new agent from a template',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="16"/><line x1="8" y1="12" x2="16" y2="12"/></svg>',
    action: function() { window.location.hash = 'agents'; } },
  { label: 'Pending Approvals', sublabel: 'Review tool calls waiting for your approval',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/></svg>',
    action: function() { window.location.hash = 'approvals'; } },
  { label: 'Notifications', sublabel: 'Open the notification center (bell)',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"/><path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"/></svg>',
    action: function() {
      try {
        Alpine.store('notifyCenter').open();
      } catch (e) { /* ignore */ }
    } },
  { label: 'Toggle Focus Mode', sublabel: 'Hide sidebar for distraction-free chat',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M8 3H5a2 2 0 0 0-2 2v3m18 0V5a2 2 0 0 0-2-2h-3m0 18h3a2 2 0 0 0 2-2v-3M3 16v3a2 2 0 0 0 2 2h3"/></svg>',
    action: function() { try { Alpine.store('app').toggleFocusMode(); } catch(e) { /* ignore */ } } },
  { label: 'View Analytics', sublabel: 'Token usage and cost by agent and model',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="20" x2="18" y2="10"/><line x1="12" y1="20" x2="12" y2="4"/><line x1="6" y1="20" x2="6" y2="14"/></svg>',
    action: function() { window.location.hash = 'analytics'; } },
  { label: 'Browse App Store', sublabel: 'Find and install AINL programs',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 2 3 6v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6l-3-4z"/><line x1="3" y1="6" x2="21" y2="6"/><path d="M16 10a4 4 0 0 1-8 0"/></svg>',
    action: function() { window.location.hash = 'ainl-library'; } },
  { label: 'Schedule a Job', sublabel: 'Set up a cron job for an agent',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="4" width="18" height="18" rx="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/></svg>',
    action: function() { window.location.hash = 'scheduler'; } },
  { label: 'Open Settings', sublabel: 'Configure providers, models, security',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.07 4.93a10 10 0 0 1 0 14.14M4.93 4.93a10 10 0 0 0 0 14.14"/></svg>',
    action: function() { window.location.hash = 'settings'; } },
  { label: 'Reload Config', sublabel: 'Apply config.toml changes without restarting',
    icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/></svg>',
    action: function() {
      try {
        if (typeof OpenFangAPI !== 'undefined') {
          OpenFangAPI.post('/api/config/reload', {}).then(function() {
            if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Config reloaded');
          }).catch(function(e) {
            if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Reload failed: ' + (e && e.message ? e.message : String(e)));
          });
        }
      } catch(e) { /* ignore */ }
    } },
];

function commandPalette() {
  return {
    open: false,
    query: '',
    selectedIdx: 0,
    /** Cached recent agents from localStorage, refreshed each time the palette opens. */
    _recentAgents: [],

    init() {
      var self = this;
      document.addEventListener('open-command-palette', function() {
        self.openPalette();
      });
    },

    openPalette() {
      this._loadRecents();
      this.open = true;
      this.query = '';
      this.selectedIdx = 0;
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('cmd-palette-input');
        if (el) { el.focus(); el.select(); }
      });
    },

    closePalette() {
      this.open = false;
      this.query = '';
    },

    _loadRecents() {
      try {
        var raw = localStorage.getItem('armaraos-recent-agents');
        var list = raw ? JSON.parse(raw) : [];
        var agents = (typeof Alpine !== 'undefined' && Alpine.store('app'))
          ? Alpine.store('app').agents || []
          : [];
        var agentMap = {};
        agents.forEach(function(a) { agentMap[String(a.id)] = a; });
        this._recentAgents = list.slice(0, 5).map(function(r) {
          var live = agentMap[r.id];
          return live || { id: r.id, name: r.name || r.id, identity: { emoji: r.emoji || '' }, model_name: '' };
        });
      } catch(e) { this._recentAgents = []; }
    },

    /** Match query against label and sublabel. */
    _matches(label, sublabel) {
      if (!this.query) return true;
      var q = this.query.toLowerCase();
      return (label || '').toLowerCase().indexOf(q) !== -1 ||
             (sublabel || '').toLowerCase().indexOf(q) !== -1;
    },

    /** Grouped result sections. Each section: { id, label, items: [{label, sublabel, icon, action, emoji?}] } */
    get results() {
      var self = this;
      var sections = [];

      // Recents
      var recentItems = this._recentAgents.filter(function(a) {
        return self._matches(a.name, a.model_name || '');
      }).map(function(agent) {
        return {
          id: 'recent-' + agent.id,
          label: agent.name || agent.id,
          sublabel: agent.model_name ? 'Recent · ' + agent.model_name : 'Recent',
          emoji: agent.identity && agent.identity.emoji ? agent.identity.emoji : '',
          icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
          action: function() {
            try { Alpine.store('app').openAgentChat(agent); } catch(e) { window.location.hash = 'agents'; }
          },
        };
      });
      if (recentItems.length > 0) {
        sections.push({ id: 'recent', label: 'Recent', items: recentItems });
      }

      // Agents
      var allAgents = [];
      try {
        allAgents = (Alpine.store('app').agents || []).filter(function(a) {
          return !a.name || (!a.name.startsWith('allowlist-probe') && !a.name.startsWith('offline-cron') && !a.name.startsWith('allow-ir-off'));
        });
      } catch(e) { /* ignore */ }
      var agentLimit = self.query ? Infinity : 5;
      var agentItems = allAgents.filter(function(a) {
        return self._matches(a.name, a.model_name || '');
      }).slice(0, agentLimit).map(function(agent) {
        return {
          id: 'agent-' + agent.id,
          label: agent.name || agent.id,
          sublabel: agent.model_name || 'Agent',
          emoji: agent.identity && agent.identity.emoji ? agent.identity.emoji : '',
          icon: '<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></svg>',
          action: function() {
            try { Alpine.store('app').openAgentChat(agent); } catch(e) { window.location.hash = 'agents'; }
          },
        };
      });
      if (agentItems.length > 0) {
        sections.push({ id: 'agents', label: 'Agents', items: agentItems });
      }

      // Pages
      var pageItems = CMD_PALETTE_PAGES.filter(function(p) {
        return self._matches(p.label, p.sublabel);
      }).map(function(p) {
        return {
          id: 'page-' + p.hash,
          label: p.label,
          sublabel: p.sublabel,
          emoji: '',
          icon: p.icon,
          action: function() { window.location.hash = p.hash; },
        };
      });
      if (pageItems.length > 0) {
        sections.push({ id: 'pages', label: 'Pages', items: pageItems });
      }

      // Actions
      var actionItems = CMD_PALETTE_ACTIONS.filter(function(a) {
        return self._matches(a.label, a.sublabel);
      }).map(function(a) {
        return {
          id: 'action-' + a.label.toLowerCase().replace(/\s+/g, '-'),
          label: a.label,
          sublabel: a.sublabel,
          emoji: '',
          icon: a.icon,
          action: a.action,
        };
      });
      if (actionItems.length > 0) {
        sections.push({ id: 'actions', label: 'Actions', items: actionItems });
      }

      return sections;
    },

    /** Flat list of all visible items (for keyboard index mapping). */
    get flatResults() {
      var flat = [];
      this.results.forEach(function(sec) { sec.items.forEach(function(item) { flat.push(item); }); });
      return flat;
    },

    get hasResults() {
      return this.flatResults.length > 0;
    },

    /** Global flat index for a given section+item. */
    flatIndexFor(sectionIdx, itemIdx) {
      var idx = 0;
      for (var s = 0; s < this.results.length; s++) {
        if (s === sectionIdx) { return idx + itemIdx; }
        idx += this.results[s].items.length;
      }
      return idx;
    },

    selectItem(item) {
      if (!item || typeof item.action !== 'function') return;
      this.closePalette();
      item.action();
    },

    confirmSelected() {
      var item = this.flatResults[this.selectedIdx];
      if (item) this.selectItem(item);
    },

    moveUp() {
      if (this.selectedIdx > 0) this.selectedIdx--;
    },

    moveDown() {
      var max = this.flatResults.length - 1;
      if (this.selectedIdx < max) this.selectedIdx++;
    },

    resetSelection() {
      this.selectedIdx = 0;
    },
  };
}
