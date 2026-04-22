// Trajectories operator view — GET /api/trajectories + kernel SSE (TrajectoryRecorded / FailureLearned).
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
    failures: [],
    liveFeed: [],
    loading: false,
    failuresLoading: false,
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
      } else if (sub === 'FailureLearned') {
        this.pushFeed(
          '[' +
            ts +
            '] FailureLearned id=' +
            String(p.data.failure_node_id || '').slice(0, 8) +
            '… tool=' +
            (p.data.tool_name || '') +
            ' — ' +
            (p.data.message_preview || '')
        );
        this.loadFailures();
      } else if (sub === 'GraphMemoryWrite' && p.data.kind === 'trajectory') {
        this.pushFeed(
          '[' + ts + '] GraphMemoryWrite trajectory — ' + (p.data.provenance && p.data.provenance.summary ? p.data.provenance.summary : '')
        );
        this.loadTrajectories();
      } else if (sub === 'GraphMemoryWrite' && p.data.kind === 'failure') {
        this.pushFeed(
          '[' + ts + '] GraphMemoryWrite failure — ' + (p.data.provenance && p.data.provenance.summary ? p.data.provenance.summary : '')
        );
        this.loadFailures();
      }
    },

    failRowLine(row) {
      try {
        var nt = row && row.node_type;
        if (!nt || nt.type !== 'failure') {
          return '';
        }
        var f = nt.failure || {};
        var src = f.source || '';
        var tool = f.tool_name ? ' · ' + f.tool_name : '';
        var msg = String(f.message || '').slice(0, 200);
        return src + tool + ' — ' + msg;
      } catch (e) {
        return '';
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
      } catch (e) {
        console.warn('trajectories: load failed', e);
        this.trajectories = [];
      }
      this.loading = false;
    },

    async loadFailures() {
      if (!this.agentId) {
        this.failures = [];
        return;
      }
      this.failuresLoading = true;
      try {
        var url =
          '/api/graph-memory/failures/recent?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&limit=60';
        var data = await OpenFangAPI.get(url);
        this.failures = (data && data.failures) || [];
      } catch (e) {
        console.warn('trajectories: failures load failed', e);
        this.failures = [];
      }
      this.failuresLoading = false;
    },

    async refreshPanels() {
      await this.loadTrajectories();
      await this.loadFailures();
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
