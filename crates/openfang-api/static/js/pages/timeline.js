// ArmaraOS Event Timeline — filtered audit trail (GET /api/audit/recent)
'use strict';

function timelinePage() {
  return {
    entries: [],
    loading: false,
    loadError: '',
    filterQ: '',
    limitN: 200,
    tipHash: '',
    totalEntries: 0,
    /** Client-side slice: all | tools | messages | cron | agents | access | security | system */
    viewFilter: 'all',

    async loadTimeline() {
      this.loading = true;
      this.loadError = '';
      try {
        var q = (this.filterQ || '').trim();
        var url = '/api/audit/recent?n=' + encodeURIComponent(String(this.limitN || 200));
        if (q) url += '&q=' + encodeURIComponent(q);
        var data = await OpenFangAPI.get(url);
        this.entries = data.entries || [];
        this.tipHash = data.tip_hash || '';
        this.totalEntries = typeof data.total === 'number' ? data.total : 0;
      } catch (e) {
        this.entries = [];
        this.loadError = e.message || 'Could not load audit timeline.';
      }
      this.loading = false;
    },

    async loadData() {
      return this.loadTimeline();
    },

    setViewFilter(f) {
      this.viewFilter = f || 'all';
    },

    get displayEntries() {
      var self = this;
      return this.entries.filter(function(e) {
        return self.entryMatchesFilter(e, self.viewFilter);
      });
    },

    /** Group by calendar day (ISO date prefix) for sticky headers. */
    get timelineDayGroups() {
      var list = this.displayEntries;
      var groups = [];
      var cur = null;
      for (var i = 0; i < list.length; i++) {
        var e = list[i];
        var dayKey = timelineDayKey(e.timestamp);
        if (dayKey !== cur) {
          cur = dayKey;
          groups.push({ dayKey: dayKey, label: this.formatDayHeading(e.timestamp), items: [] });
        }
        groups[groups.length - 1].items.push(e);
      }
      return groups;
    },

    matchesViewFilter(e) {
      return this.entryMatchesFilter(e, this.viewFilter);
    },

    entryMatchesFilter(e, f) {
      var a = (e.action || '').trim();
      if (f === 'all') return true;
      if (f === 'tools') return a === 'ToolInvoke';
      if (f === 'messages') return a === 'AgentMessage';
      if (f === 'cron') return a.indexOf('CronJob') === 0;
      if (f === 'agents') return a === 'AgentSpawn' || a === 'AgentKill';
      if (f === 'access') {
        return a === 'FileAccess' || a === 'MemoryAccess' || a === 'NetworkAccess';
      }
      if (f === 'security') {
        return a === 'AuthAttempt' || a === 'CapabilityCheck' || a === 'ShellExec';
      }
      if (f === 'system') {
        return a === 'ConfigChange' || a === 'WireConnect' || a === 'UpdateCheck' || a === 'UpdateInstall';
      }
      return true;
    },

    countForFilter(f) {
      var self = this;
      return this.entries.filter(function(e) {
        return self.entryMatchesFilter(e, f);
      }).length;
    },

    agentLabel(agentId) {
      if (!agentId) return '—';
      try {
        var agents = Alpine.store('app').agents || [];
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === agentId) return agents[i].name || agentId;
        }
      } catch (err) { /* ignore */ }
      var s = String(agentId);
      return s.length > 18 ? s.slice(0, 8) + '…' + s.slice(-6) : s;
    },

    formatTime(ts) {
      if (!ts) return '—';
      try {
        var d = new Date(ts);
        return isNaN(d.getTime()) ? '—' : d.toLocaleString();
      } catch (e) {
        return '—';
      }
    },

    formatTimeClock(ts) {
      if (!ts) return '';
      try {
        var d = new Date(ts);
        if (isNaN(d.getTime())) return '';
        return d.toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit', second: '2-digit' });
      } catch (e) {
        return '';
      }
    },

    formatRelative(ts) {
      if (!ts) return '';
      try {
        var d = new Date(ts);
        var t = d.getTime();
        if (isNaN(t)) return '';
        var sec = Math.floor((Date.now() - t) / 1000);
        if (sec < 45) return 'just now';
        if (sec < 3600) return Math.floor(sec / 60) + 'm ago';
        if (sec < 86400) return Math.floor(sec / 3600) + 'h ago';
        if (sec < 172800) return 'yesterday';
        return Math.floor(sec / 86400) + 'd ago';
      } catch (e) {
        return '';
      }
    },

    formatDayHeading(ts) {
      if (!ts) return '';
      try {
        var d = new Date(ts);
        if (isNaN(d.getTime())) return '';
        var start = new Date();
        start.setHours(0, 0, 0, 0);
        var day0 = new Date(d);
        day0.setHours(0, 0, 0, 0);
        var diff = Math.round((start - day0) / 86400000);
        if (diff === 0) return 'Today';
        if (diff === 1) return 'Yesterday';
        return d.toLocaleDateString(undefined, { weekday: 'short', month: 'short', day: 'numeric', year: 'numeric' });
      } catch (e) {
        return '';
      }
    },

    actionLabel(action) {
      var a = (action || '').trim();
      var map = {
        ToolInvoke: 'Tool',
        AgentMessage: 'Message',
        AgentSpawn: 'Spawn',
        AgentKill: 'Stop',
        CronJobRun: 'Cron run',
        CronJobOutput: 'Cron output',
        CronJobFailure: 'Cron failed',
        FileAccess: 'File',
        MemoryAccess: 'Memory',
        NetworkAccess: 'Network',
        ShellExec: 'Shell',
        AuthAttempt: 'Auth',
        CapabilityCheck: 'Capability',
        ConfigChange: 'Config',
        WireConnect: 'Wire',
        UpdateCheck: 'Update check',
        UpdateInstall: 'Update install'
      };
      return map[a] || a || 'Event';
    },

    actionAccentVar(action) {
      var a = (action || '').trim();
      if (a === 'ToolInvoke') return 'var(--warning)';
      if (a === 'AgentMessage') return 'var(--accent)';
      if (a.indexOf('CronJob') === 0) return 'var(--info)';
      if (a === 'AgentSpawn' || a === 'AgentKill') return 'var(--info)';
      if (a === 'AuthAttempt' || a === 'ShellExec') return 'var(--error)';
      if (a === 'FileAccess' || a === 'MemoryAccess') return 'var(--text-dim)';
      if (a === 'NetworkAccess') return 'var(--success)';
      if (a === 'CapabilityCheck') return 'var(--warning)';
      return 'var(--border-strong)';
    },

    outcomeBadgeClass(outcome) {
      var o = (outcome || '').toLowerCase();
      if (!o || o === 'ok' || o === 'success' || o.indexOf('allow') === 0) return 'badge-success';
      if (o.indexOf('denied') >= 0 || o.indexOf('deny') >= 0 || o.indexOf('block') >= 0) return 'badge-error';
      if (o.indexOf('err') >= 0 || o.indexOf('fail') >= 0) return 'badge-error';
      if (o.indexOf('warn') >= 0) return 'badge-warn';
      return 'badge-dim';
    },

    truncate(s, maxLen) {
      if (s == null || s === '') return '—';
      var t = String(s);
      if (t.length <= maxLen) return t;
      return t.slice(0, maxLen) + '…';
    },

    async copyTipSnippet() {
      var h = this.tipHash || '';
      if (!h) return;
      await this._copyText(h, 'Chain tip copied');
    },

    async copyHash(hash) {
      if (!hash) return;
      await this._copyText(hash, 'Entry hash copied');
    },

    async _copyText(text, msg) {
      try {
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(text);
        } else {
          var ta = document.createElement('textarea');
          ta.value = text;
          ta.style.position = 'fixed';
          ta.style.left = '-9999px';
          document.body.appendChild(ta);
          ta.select();
          document.execCommand('copy');
          document.body.removeChild(ta);
        }
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.success(msg);
      } catch (e) {
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.warn('Could not copy');
      }
    }
  };
}

function timelineDayKey(ts) {
  if (!ts) return '';
  var m = String(ts).match(/^(\d{4}-\d{2}-\d{2})/);
  return m ? m[1] : String(ts).slice(0, 10);
}
