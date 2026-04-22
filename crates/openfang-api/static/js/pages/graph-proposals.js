// Improvement proposal ledger + SSE (ImprovementProposalAdopted).
'use strict';

function graphProposalsNormalizeAgentsPayload(raw) {
  if (Array.isArray(raw)) {
    return raw;
  }
  if (raw && Array.isArray(raw.agents)) {
    return raw.agents;
  }
  return [];
}

function graphProposalsPanel() {
  return {
    agents: [],
    agentId: '',
    proposals: [],
    loading: false,
    disabledMessage: null,
    liveFeed: [],
    selected: null,
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

    rowStatus(p) {
      if (p == null) {
        return '—';
      }
      if (p.adopted_at != null) {
        return 'adopted';
      }
      if (p.accepted) {
        return 'validated';
      }
      if (p.validation_error) {
        return 'rejected';
      }
      return 'submitted';
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
      if (sub === 'ImprovementProposalAdopted') {
        this.pushFeed(
          '[' +
            ts +
            '] ImprovementProposalAdopted proposal=' +
            String(p.data.proposal_id || '') +
            ' → graph ' +
            String(p.data.graph_node_id || '').slice(0, 8) +
            '… kind=' +
            (p.data.kind || '')
        );
        this.loadProposals();
      }
    },

    selectRow(p) {
      this.selected = p;
    },

    async copyText(text) {
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
          OpenFangToast.success('Copied');
        }
      } catch (e) {
        console.warn('graph-proposals: copy', e);
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
          list = graphProposalsNormalizeAgentsPayload(await OpenFangAPI.get('/api/agents'));
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
        console.warn('graph-proposals: loadAgents failed', e0);
        this.agents = [];
      }
    },

    async loadProposals() {
      if (!this.agentId) {
        this.proposals = [];
        this.disabledMessage = null;
        return;
      }
      this.loading = true;
      this.disabledMessage = null;
      try {
        var d = await OpenFangAPI.get(
          '/api/graph-memory/improvement-proposals?agent_id=' +
            encodeURIComponent(this.agentId) +
            '&limit=80'
        );
        if (d && d.ok === false && d.error) {
          this.disabledMessage = String(d.error);
          this.proposals = Array.isArray(d.proposals) ? d.proposals : [];
        } else {
          this.proposals = d && d.ok && Array.isArray(d.proposals) ? d.proposals : [];
        }
      } catch (e) {
        console.warn('graph-proposals: load failed', e);
        this.proposals = [];
        this.disabledMessage = e && e.message ? String(e.message) : 'Request failed';
      }
      this.loading = false;
    },

    async init() {
      var self = this;
      await this.loadAgents();
      await this.loadProposals();
      this._kernelHandler = function (e) {
        try {
          self.ingestKernelEvent(e.detail);
        } catch (err) {
          console.warn('graph-proposals: ingest failed', err);
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
