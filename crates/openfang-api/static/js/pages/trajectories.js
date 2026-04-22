// Trajectories operator view — GET /api/trajectories + kernel SSE (TrajectoryRecorded, GraphMemoryWrite trajectory).
'use strict';

function trajectoriesNormalizeAgentsPayload(raw) {
  if (Array.isArray(raw)) {
    return raw;
  }
  if (raw && Array.isArray(raw.agents)) {
    return raw.agents;
  }
  return [];
}

function trajectoriesPanel() {
  return {
    agents: [],
    agentId: '',
    trajectories: [],
    /** Compact headlines from `GET /api/graph-memory/failures/recent` (cross-surface, Phase 8). */
    failureHeadlines: [],
    liveFeed: [],
    loading: false,
    filterText: '',
    selected: null,
    compressionLoading: false,
    compressionJson: null,
    compressionError: null,
    _kernelHandler: null,

    eventAgentId(data) {
      if (!data || data.agent_id == null) {
        return '';
      }
      var a = data.agent_id;
      if (typeof a === 'string') {
        return a;
      }
      if (typeof a === 'object' && a && a.value != null) {
        return String(a.value);
      }
      return String(a);
    },

    filteredTrajectories() {
      var rows = this.trajectories || [];
      var q = (this.filterText || '').trim().toLowerCase();
      if (!q) {
        return rows;
      }
      return rows.filter(function (row) {
        try {
          var id = String(row && row.id != null ? row.id : '');
          var ep = String(row && row.episode_id != null ? row.episode_id : '');
          var sess = String(row && row.session_id != null ? row.session_id : '');
          var oc = String(row && row.outcome != null ? row.outcome : '');
          var sum = JSON.stringify(row && row.steps != null ? row.steps : '');
          var blob = (id + ' ' + ep + ' ' + sess + ' ' + oc + ' ' + sum).toLowerCase();
          return blob.indexOf(q) >= 0;
        } catch (e) {
          return true;
        }
      });
    },

    selectRow(row) {
      this.selected = row;
    },

    formatOutcome(o) {
      if (o == null) {
        return '';
      }
      if (typeof o === 'string') {
        return o;
      }
      try {
        return JSON.stringify(o);
      } catch (e) {
        return String(o);
      }
    },

    rowSummary(row) {
      if (!row) {
        return '';
      }
      var n = Array.isArray(row.steps) ? row.steps.length : 0;
      return (
        'Episode ' +
        String(row.episode_id || '').slice(0, 8) +
        '… · ' +
        n +
        ' steps · ' +
        (row.outcome != null ? String(row.outcome) : '')
      );
    },

    async copyText(label, text) {
      try {
        if (navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(String(text));
        } else {
          var ta = document.createElement('textarea');
          ta.value = String(text);
          document.body.appendChild(ta);
          ta.select();
          document.execCommand('copy');
          document.body.removeChild(ta);
        }
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.success(label || 'Copied');
        }
      } catch (e) {
        console.warn('trajectories: copy', e);
      }
    },

    async loadCompressionProfiles() {
      if (!this.agentId) {
        this.compressionJson = null;
        this.compressionError = null;
        return;
      }
      this.compressionLoading = true;
      this.compressionError = null;
      try {
        var url =
          '/api/compression/project-profiles?agent_id=' + encodeURIComponent(this.agentId);
        var d = await OpenFangAPI.get(url);
        this.compressionJson = d;
        if (d && d.ok === false && d.error) {
          this.compressionError = String(d.error);
        }
      } catch (e) {
        this.compressionJson = null;
        this.compressionError = e && e.message ? String(e.message) : 'Request failed';
      }
      this.compressionLoading = false;
    },

    pushFeed(line) {
      this.liveFeed.unshift(line);
      if (this.liveFeed.length > 120) {
        this.liveFeed.length = 120;
      }
    },

    ingestKernelEvent(ev) {
      if (!ev || !ev.payload) {
        return;
      }
      var p = ev.payload;
      if (p.type !== 'System' || !p.data) {
        return;
      }
      var sub = p.data.event;
      var aid = this.eventAgentId(p.data);
      if (this.agentId && aid && aid !== String(this.agentId)) {
        return;
      }
      var ts = ev.timestamp ? new Date(ev.timestamp).toLocaleTimeString() : '';
      if (sub === 'TrajectoryRecorded') {
        this.pushFeed(
          '[' +
            ts +
            '] TrajectoryRecorded traj=' +
            String(p.data.trajectory_node_id || '').slice(0, 8) +
            '… ep=' +
            String(p.data.episode_node_id || '').slice(0, 8) +
            '… ' +
            (p.data.summary || '')
        );
        this.loadTrajectories();
      } else if (sub === 'GraphMemoryWrite' && p.data.kind === 'trajectory') {
        this.pushFeed(
          '[' + ts + '] GraphMemoryWrite trajectory — ' + (p.data.provenance && p.data.provenance.summary ? p.data.provenance.summary : '')
        );
        this.loadTrajectories();
      } else if (sub === 'FailureLearned') {
        this.pushFeed(
          '[' +
            ts +
            '] FailureLearned id=' +
            String(p.data.failure_node_id || '').slice(0, 8) +
            '… — ' +
            (p.data.message_preview || '')
        );
        this.loadFailureHeadlines();
      } else if (sub === 'GraphMemoryWrite' && p.data.kind === 'failure') {
        this.pushFeed(
          '[' + ts + '] GraphMemoryWrite failure — ' + (p.data.provenance && p.data.provenance.summary ? p.data.provenance.summary : '')
        );
        this.loadFailureHeadlines();
      } else if (sub === 'ImprovementProposalAdopted') {
        this.pushFeed(
          '[' +
            ts +
            '] ImprovementProposalAdopted proposal=' +
            String(p.data.proposal_id || '') +
            ' (see Proposals page)'
        );
      }
    },

    async loadAgents() {
      var list = [];
      var st = null;
      try {
        st = typeof Alpine !== 'undefined' && Alpine.store && Alpine.store('app');
        if (st && Array.isArray(st.agents) && st.agents.length) {
          list = st.agents;
        }
      } catch (e1) {
        /* ignore */
      }
      try {
        if (!list.length && st && typeof st.refreshAgents === 'function') {
          await st.refreshAgents();
          if (Array.isArray(st.agents) && st.agents.length) {
            list = st.agents;
          }
        }
        if (!list.length) {
          list = trajectoriesNormalizeAgentsPayload(await OpenFangAPI.get('/api/agents'));
        }
        this.agents = list
          .map(function (a) {
            var id = String(a.id != null ? a.id : a.agent_id || '').trim();
            if (!id) {
              return null;
            }
            var nm = (a.name && String(a.name).trim()) || 'Agent';
            return { id: id, name: nm };
          })
          .filter(Boolean);
        if (this.agents.length && !this.agentId) {
          this.agentId = this.agents[0].id;
        }
      } catch (e0) {
        console.warn('trajectories: loadAgents failed', e0);
        this.agents = [];
      }
    },

    async loadFailureHeadlines() {
      if (!this.agentId) {
        this.failureHeadlines = [];
        return;
      }
      try {
        var url =
          '/api/graph-memory/failures/recent?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&limit=5';
        var data = await OpenFangAPI.get(url);
        var rows = (data && data.failures) || [];
        this.failureHeadlines = rows.slice(0, 5);
      } catch (e) {
        console.warn('trajectories: failure headlines load failed', e);
        this.failureHeadlines = [];
      }
    },

    failureHeadlineText(row) {
      try {
        if (!row || !row.node_type || row.node_type.type !== 'failure') {
          return '';
        }
        var f = row.node_type.failure || {};
        return String(f.message || '')
          .trim()
          .slice(0, 200);
      } catch (e) {
        return '';
      }
    },

    async loadTrajectories() {
      if (!this.agentId) {
        this.trajectories = [];
        return;
      }
      this.loading = true;
      try {
        var url =
          '/api/trajectories?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&limit=80';
        var data = await OpenFangAPI.get(url);
        this.trajectories = (data && data.trajectories) || [];
        if (this.selected) {
          var selId = this.selected && this.selected.id;
          if (selId) {
            var found = this.trajectories.find(function (r) {
              return r && r.id === selId;
            });
            this.selected = found || this.selected;
          }
        }
      } catch (e) {
        console.warn('trajectories: load failed', e);
        this.trajectories = [];
      }
      this.loading = false;
    },

    async refreshPanels() {
      await Promise.all([this.loadTrajectories(), this.loadFailureHeadlines()]);
    },

    async init() {
      var self = this;
      await this.loadAgents();
      await this.refreshPanels();
      this._kernelHandler = function (e) {
        try {
          self.ingestKernelEvent(e.detail);
        } catch (err) {
          console.warn('trajectories: ingest failed', err);
        }
      };
      window.addEventListener('armaraos-kernel-event', this._kernelHandler);
    },

    cleanup() {
      if (this._kernelHandler) {
        window.removeEventListener('armaraos-kernel-event', this._kernelHandler);
        this._kernelHandler = null;
      }
    },
  };
}
