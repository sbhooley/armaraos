// ArmaraOS Sessions Page — Session listing + Memory tab
'use strict';

function sessionsPage() {
  return {
    tab: 'sessions',
    // -- Sessions state --
    sessions: [],
    searchFilter: '',
    loading: true,
    loadError: '',
    // -- Session rename state --
    renameSessionId: null,
    renameSessionValue: '',
    sessionLabels: {},

    // -- Memory state --
    memAgentId: '',
    kvPairs: [],
    showAdd: false,
    newKey: '',
    newValue: '""',
    editingKey: null,
    editingValue: '',
    memLoading: false,
    memLoadError: '',

    // -- Session rename methods --
    loadSessionLabels() {
      try {
        var raw = localStorage.getItem('armaraos-session-labels');
        this.sessionLabels = raw ? JSON.parse(raw) : {};
      } catch (e) { this.sessionLabels = {}; }
    },

    sessionDisplayName(s) {
      if (!s) return '—';
      var label = this.sessionLabels[s.session_id];
      if (label) return label;
      return s.agent_name || s.agent_id || '—';
    },

    startRename(s) {
      this.renameSessionId = s.session_id;
      this.renameSessionValue = this.sessionLabels[s.session_id] || s.agent_name || '';
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('session-rename-' + s.session_id);
        if (el) { el.focus(); el.select(); }
      });
    },

    commitRename(sid) {
      if (!sid) return;
      var val = (this.renameSessionValue || '').trim();
      var next = Object.assign({}, this.sessionLabels);
      if (val) {
        next[sid] = val;
      } else {
        delete next[sid];
      }
      this.sessionLabels = next;
      try { localStorage.setItem('armaraos-session-labels', JSON.stringify(next)); } catch (e) { /* ignore */ }
      this.renameSessionId = null;
      this.renameSessionValue = '';
    },

    cancelRename() {
      this.renameSessionId = null;
      this.renameSessionValue = '';
    },

    // -- Sessions methods --
    async loadSessions() {
      this.loading = true;
      this.loadError = '';
      this.loadSessionLabels();
      try {
        var data = await OpenFangAPI.get('/api/sessions');
        var sessions = data.sessions || [];
        var agents = Alpine.store('app').agents;
        var agentMap = {};
        agents.forEach(function(a) { agentMap[a.id] = a.name; });
        sessions.forEach(function(s) {
          s.agent_name = agentMap[s.agent_id] || '';
        });
        this.sessions = sessions;
      } catch(e) {
        this.sessions = [];
        this.loadError = e.message || 'Could not load sessions.';
      }
      this.loading = false;
    },

    async loadData() { return this.loadSessions(); },

    get filteredSessions() {
      var f = this.searchFilter.toLowerCase();
      if (!f) return this.sessions;
      return this.sessions.filter(function(s) {
        return (s.agent_name || '').toLowerCase().indexOf(f) !== -1 ||
               (s.agent_id || '').toLowerCase().indexOf(f) !== -1;
      });
    },

    totalMessageCount() {
      return this.sessions.reduce(function(acc, s) {
        return acc + (Number(s.message_count) || 0);
      }, 0);
    },

    filteredMessageCount() {
      return this.filteredSessions.reduce(function(acc, s) {
        return acc + (Number(s.message_count) || 0);
      }, 0);
    },

    shortSessionId(sid) {
      if (!sid) return '—';
      return sid.length > 10 ? sid.substring(0, 8) + '…' : sid;
    },

    formatSessionDate(iso) {
      if (!iso) return '—';
      try {
        return new Date(iso).toLocaleString(undefined, {
          dateStyle: 'medium',
          timeStyle: 'short',
        });
      } catch (e) {
        return String(iso);
      }
    },

    openInChat(session) {
      var agents = Alpine.store('app').agents;
      var agent = agents.find(function(a) { return a.id === session.agent_id; });
      if (agent) {
        Alpine.store('app').pendingAgent = agent;
      }
      location.hash = 'agents';
    },

    deleteSession(sessionId) {
      var self = this;
      OpenFangToast.confirm('Delete Session', 'This will permanently remove the session and its messages.', async function() {
        try {
          await OpenFangAPI.del('/api/sessions/' + sessionId);
          self.sessions = self.sessions.filter(function(s) { return s.session_id !== sessionId; });
          OpenFangToast.success('Session deleted');
        } catch(e) {
          OpenFangToast.error('Failed to delete session: ' + openFangErrText(e));
        }
      });
    },

    // -- Memory methods --
    async loadKv() {
      if (!this.memAgentId) { this.kvPairs = []; return; }
      this.memLoading = true;
      this.memLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/memory/agents/' + this.memAgentId + '/kv');
        this.kvPairs = data.kv_pairs || [];
      } catch(e) {
        this.kvPairs = [];
        this.memLoadError = e.message || 'Could not load memory data.';
      }
      this.memLoading = false;
    },

    async addKey() {
      if (!this.memAgentId || !this.newKey.trim()) return;
      var value;
      try { value = JSON.parse(this.newValue); } catch(e) { value = this.newValue; }
      try {
        await OpenFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.newKey), { value: value });
        this.showAdd = false;
        OpenFangToast.success('Key "' + this.newKey + '" saved');
        this.newKey = '';
        this.newValue = '""';
        await this.loadKv();
      } catch(e) {
        OpenFangToast.error('Failed to save key: ' + openFangErrText(e));
      }
    },

    deleteKey(key) {
      var self = this;
      OpenFangToast.confirm('Delete Key', 'Delete key "' + key + '"? This cannot be undone.', async function() {
        try {
          await OpenFangAPI.del('/api/memory/agents/' + self.memAgentId + '/kv/' + encodeURIComponent(key));
          OpenFangToast.success('Key "' + key + '" deleted');
          await self.loadKv();
        } catch(e) {
          OpenFangToast.error('Failed to delete key: ' + openFangErrText(e));
        }
      });
    },

    startEdit(kv) {
      this.editingKey = kv.key;
      this.editingValue = typeof kv.value === 'object' ? JSON.stringify(kv.value, null, 2) : String(kv.value);
    },

    cancelEdit() {
      this.editingKey = null;
      this.editingValue = '';
    },

    async saveEdit() {
      if (!this.editingKey || !this.memAgentId) return;
      var value;
      try { value = JSON.parse(this.editingValue); } catch(e) { value = this.editingValue; }
      try {
        await OpenFangAPI.put('/api/memory/agents/' + this.memAgentId + '/kv/' + encodeURIComponent(this.editingKey), { value: value });
        OpenFangToast.success('Key "' + this.editingKey + '" updated');
        this.editingKey = null;
        this.editingValue = '';
        await this.loadKv();
      } catch(e) {
        OpenFangToast.error('Failed to save: ' + openFangErrText(e));
      }
    }
  };
}
