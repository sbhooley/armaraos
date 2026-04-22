// Typed failures view — GET /api/graph-memory/failures/* + kernel SSE (FailureLearned, GraphMemoryWrite failure).
'use strict';

function graphFailuresNormalizeAgentsPayload(raw) {
  if (Array.isArray(raw)) {
    return raw;
  }
  if (raw && Array.isArray(raw.agents)) {
    return raw.agents;
  }
  return [];
}

function graphFailuresPanel() {
  return {
    agents: [],
    agentId: '',
    failures: [],
    loading: false,
    searchQ: '',
    searchLoading: false,
    searchRows: [],
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
      if (sub === 'FailureLearned') {
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
        this.loadRecent();
      } else if (sub === 'GraphMemoryWrite' && p.data.kind === 'failure') {
        this.pushFeed(
          '[' + ts + '] GraphMemoryWrite failure — ' + (p.data.provenance && p.data.provenance.summary ? p.data.provenance.summary : '')
        );
        this.loadRecent();
      }
    },

    selectRow(row) {
      this.selected = row;
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
        console.warn('graph-failures: copy', e);
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
          list = graphFailuresNormalizeAgentsPayload(await OpenFangAPI.get('/api/agents'));
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
        console.warn('graph-failures: loadAgents failed', e0);
        this.agents = [];
      }
    },

    async loadRecent() {
      if (!this.agentId) {
        this.failures = [];
        return;
      }
      this.loading = true;
      try {
        var url =
          '/api/graph-memory/failures/recent?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&limit=80';
        var data = await OpenFangAPI.get(url);
        this.failures = (data && data.failures) || [];
      } catch (e) {
        console.warn('graph-failures: recent load failed', e);
        this.failures = [];
      }
      this.loading = false;
    },

    async runSearch() {
      var q = (this.searchQ || '').trim();
      if (!this.agentId || !q) {
        this.searchRows = [];
        return;
      }
      this.searchLoading = true;
      try {
        var url =
          '/api/graph-memory/failures/search?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&q=' +
          encodeURIComponent(q) +
          '&limit=40';
        var data = await OpenFangAPI.get(url);
        this.searchRows = (data && data.failures) || [];
      } catch (e) {
        console.warn('graph-failures: search failed', e);
        this.searchRows = [];
      }
      this.searchLoading = false;
    },

    async refreshAll() {
      await this.loadRecent();
      if ((this.searchQ || '').trim()) {
        await this.runSearch();
      }
    },

    async init() {
      var self = this;
      await this.loadAgents();
      await this.refreshAll();
      this._kernelHandler = function (e) {
        try {
          self.ingestKernelEvent(e.detail);
        } catch (err) {
          console.warn('graph-failures: ingest failed', err);
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
