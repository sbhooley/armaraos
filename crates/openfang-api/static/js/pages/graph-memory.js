/** Graph Memory dashboard — D3 force-directed view of `ainl_memory.db` graph. */

function graphMemoryNormalizeAgentsPayload(raw) {
  if (Array.isArray(raw)) {
    return raw;
  }
  if (raw && Array.isArray(raw.agents)) {
    return raw.agents;
  }
  return [];
}

/** Deterministic one-line copy when API `explain` is absent (timeline fallback). */
function graphMemoryFormatProceduralFallback(meta) {
  var m = meta || {};
  var seq = Array.isArray(m.tool_sequence) ? m.tool_sequence.join(' → ') : '';
  var conf = m.confidence != null ? Number(m.confidence) : null;
  var name = m.pattern_name ? String(m.pattern_name) : '';
  var parts = [];
  if (name) {
    parts.push('Pattern “' + name + '”');
  } else {
    parts.push('Repeated tool chain');
  }
  if (seq) {
    parts.push(seq);
  }
  if (conf != null && Number.isFinite(conf)) {
    parts.push('confidence ' + conf.toFixed(2));
  }
  return parts.filter(Boolean).join(' · ');
}

/** Deterministic one-line copy for persona nodes (timeline fallback). */
function graphMemoryFormatPersonaFallback(meta) {
  var m = meta || {};
  var trait = m.trait_name ? String(m.trait_name) : '';
  var st = m.strength != null ? Number(m.strength) : null;
  var cycle = m.evolution_cycle != null ? Number(m.evolution_cycle) : 0;
  var line = trait ? 'Trait “' + trait + '”' : 'Persona trait';
  if (st != null && Number.isFinite(st)) {
    line += ' at ' + st.toFixed(2);
  }
  if (cycle > 0) {
    line += ' · evolved (cycle ' + cycle + ')';
  }
  return line;
}

function graphMemoryPanel() {
  return {
    nodes: [],
    edges: [],
    agents: [],
    agentId: '',
    graphEdgeMode: 'augmented',
    graphExpandEgo: true,
    graphSyntheticProvenance: true,
    filters: ['episode', 'semantic', 'procedural', 'persona', 'runtime_state'],
    selected: null,
    loading: false,
    simulation: null,
    liveTimeline: [],
    timelineLimit: 60,
    seenNodeIdsByAgent: {},
    timelineScope: 'all',
    snapshots: [],
    auditEntries: [],
    memoryControls: {
      memory_enabled: true,
      temporary_mode: false,
      shared_memory_enabled: false,
      include_episodic_hints: true,
      include_semantic_facts: true,
      include_conflicts: true,
      include_procedural_hints: true,
      include_suggested_pattern_candidates: true,
    },
    inspectScope: 'agent_private',
    inspectEntries: [],
    rememberFactText: '',
    selectedSnapshotId: '',
    viewMode: 'live',
    improvementProposals: [],
    improvementProposalsLoading: false,
    governanceBusy: false,
    snapshotLabel: '',
    runCaptures: { A: null, B: null },
    runResults: { A: null, B: null },
    memoryDiagnostics: null,
    memorySelectionDebug: [],
    showWhySelectedDrawer: false,
    /** Collapsible "Memory operations (advanced)" card — default closed to reduce noise for casual users. */
    memoryOperationsAdvancedOpen: false,
    /** Bound handler for `armaraos-kernel-event` — removed in {@link cleanupKernelListener}. */
    _graphMemoryKernelHandler: null,
    /** Dedupe `GraphMemoryWrite` by kernel event id (window listener + store watch + replay). */
    _processedGraphMemoryEventIds: {},
    /** Polling fallback when `$watch` on `kernelEvents.received` does not fire. */
    _kernelEventsPollTimer: null,
    _lastSeenKernelReceivedCount: -1,
    /** D3 zoom behavior for graph canvas — reused so handlers do not stack on re-render. */
    _gmZoomBehavior: null,
    /** Last pan/zoom transform — reapplied after `renderGraph` so filters do not reset the view. */
    _gmLastTransform: null,
    /** Min/max scale passed to `d3.zoom().scaleExtent` — wide min so large graphs can fully fit. */
    _gmZoomMin: 0.02,
    _gmZoomMax: 12,

    kindColor: {
      episode: '#7ab0ff',
      semantic: '#4ae8b0',
      procedural: '#ffd073',
      persona: '#d48cff',
      runtime_state: '#a8b0cc',
    },

    isGraphMemoryKernelEvent(d) {
      var p = d && d.payload;
      if (!p || p.type !== 'System' || !p.data) {
        return false;
      }
      var ev = p.data.event;
      return ev === 'GraphMemoryWrite';
    },

    /**
     * Ingest one kernel SSE event (same shape as `GET /api/events/stream` JSON).
     * Also driven by `Alpine.store('kernelEvents')` when `CustomEvent` delivery is unreliable (e.g. WebView).
     */
    ingestKernelDetailForTimeline(detail) {
      if (!this.isGraphMemoryKernelEvent(detail)) {
        return;
      }
      var evid =
        detail && detail.id != null
          ? String(detail.id)
          : '';
      if (!evid) {
        var pd = detail && detail.payload && detail.payload.data;
        var k = pd && pd.kind ? String(pd.kind) : '';
        evid =
          String(detail && detail.timestamp != null ? detail.timestamp : '') +
          ':' +
          k +
          ':' +
          String(
            pd && pd.agent_id != null ? pd.agent_id : ''
          );
      }
      if (this._processedGraphMemoryEventIds[evid]) {
        return;
      }
      this._processedGraphMemoryEventIds[evid] = true;
      var n = Object.keys(this._processedGraphMemoryEventIds).length;
      if (n > 400) {
        var keys = Object.keys(this._processedGraphMemoryEventIds);
        var drop = keys.slice(0, n - 300);
        for (var i = 0; i < drop.length; i++) {
          delete this._processedGraphMemoryEventIds[drop[i]];
        }
      }
      this.handleGraphMemoryKernelEvent(detail);
    },

    /** Replay recent SSE events from `Alpine.store('kernelEvents').items` (subset of bus history). */
    replayRecentKernelGraphMemoryFromStore() {
      var self = this;
      try {
        var ke =
          typeof Alpine !== 'undefined' && Alpine.store
            ? Alpine.store('kernelEvents')
            : null;
        if (!ke || !Array.isArray(ke.items) || !ke.items.length) {
          return;
        }
        var batch = ke.items.slice(-80);
        for (var j = 0; j < batch.length; j++) {
          var entry = batch[j];
          if (!entry || !entry.payload) {
            continue;
          }
          self.ingestKernelDetailForTimeline({
            id: entry.id,
            timestamp: entry.ts,
            payload: entry.payload,
          });
        }
      } catch (e) {
        console.warn('graph-memory: replay kernel items failed', e);
      }
    },

    notifySuccess(msg) {
      if (typeof OpenFangToast !== 'undefined' && OpenFangToast.success) {
        OpenFangToast.success(msg, 2200);
      }
    },

    notifyError(msg) {
      if (typeof OpenFangToast !== 'undefined' && OpenFangToast.error) {
        OpenFangToast.error(msg, 4200);
      }
    },

    errorText(err, fallback) {
      try {
        if (err && err.message) {
          return String(err.message);
        }
      } catch (e) {
        /* ignore */
      }
      return fallback || 'Request failed';
    },

    writeKindLabel(kind) {
      if (kind === 'fact') {
        return 'fact';
      }
      if (kind === 'procedural') {
        return 'procedural pattern';
      }
      if (kind === 'delegation') {
        return 'delegation episode';
      }
      return kind || 'write';
    },

    timelineKindClass(kind) {
      if (kind === 'fact') {
        return 'semantic';
      }
      if (kind === 'delegation') {
        return 'episode';
      }
      if (kind === 'procedural') {
        return 'procedural';
      }
      if (kind === 'persona') {
        return 'persona';
      }
      return kind || 'semantic';
    },

    agentNameById(id) {
      var match = this.agents.find(function (a) {
        return String(a.id) === String(id);
      });
      if (!match) {
        return String(id || '');
      }
      return match.name || match.label || String(id || '');
    },

    parseEventTimeMs(detail) {
      var raw = detail && detail.timestamp;
      if (!raw) {
        return Date.now();
      }
      var t = Date.parse(raw);
      if (!Number.isFinite(t)) {
        return Date.now();
      }
      return t;
    },

    rememberSeenNodes(agentId, nodes) {
      var aid = String(agentId || '').trim();
      if (!aid) {
        return;
      }
      var next = {};
      (nodes || []).slice(0, 300).forEach(function (n) {
        if (n && n.id) {
          next[String(n.id)] = true;
        }
      });
      this.seenNodeIdsByAgent[aid] = next;
    },

    kindMatchesWrite(node, writeKind) {
      if (!node) {
        return false;
      }
      var nk = String(node.kind || '');
      var label = String(node.label || '');
      if (writeKind === 'episode') {
        return nk === 'episode' && label.indexOf('Delegated to ') !== 0;
      }
      if (writeKind === 'delegation') {
        return nk === 'episode' && label.indexOf('Delegated to ') === 0;
      }
      if (writeKind === 'fact') {
        return nk === 'semantic' || nk === 'persona';
      }
      if (writeKind === 'procedural') {
        return nk === 'procedural';
      }
      if (writeKind === 'persona') {
        return nk === 'persona';
      }
      return true;
    },

    summarizeNode(node) {
      var ex = node && node.explain;
      var what =
        ex &&
        typeof ex === 'object' &&
        ex.what_happened != null &&
        String(ex.what_happened).trim();
      var label = what
        ? String(what).trim()
        : String((node && node.label) || '').trim();
      if (!label) {
        var k = String((node && node.kind) || '');
        var meta = (node && node.meta) || {};
        if (k === 'procedural') {
          label = graphMemoryFormatProceduralFallback(meta);
        } else if (k === 'persona') {
          label = graphMemoryFormatPersonaFallback(meta);
        } else {
          label = 'Untitled memory node';
        }
      }
      if (label.length > 96) {
        label = label.slice(0, 96) + '…';
      }
      return {
        nodeId: String((node && node.id) || ''),
        nodeKind: String((node && node.kind) || 'semantic'),
        nodeLabel: label,
      };
    },

    pushTimelineEntry(entry) {
      var cap = this.timelineLimit;
      this.liveTimeline = [entry].concat(this.liveTimeline).slice(0, cap);
    },

    filteredTimeline() {
      if (this.timelineScope === 'current') {
        var aid = String(this.agentId || '');
        return this.liveTimeline.filter(function (e) {
          return String(e.agentId || '') === aid;
        });
      }
      return this.liveTimeline;
    },

    writesPerSecond() {
      var now = Date.now();
      var windowMs = 60000;
      var timeline = this.filteredTimeline();
      var count = timeline.filter(function (e) {
        return now - Number(e.atMs || 0) <= windowMs;
      }).length;
      return count / (windowMs / 1000);
    },

    async copyNodeId(nodeId) {
      var id = String(nodeId || '').trim();
      if (!id) {
        return;
      }
      try {
        if (navigator && navigator.clipboard && navigator.clipboard.writeText) {
          await navigator.clipboard.writeText(id);
        } else {
          var ta = document.createElement('textarea');
          ta.value = id;
          ta.setAttribute('readonly', 'readonly');
          ta.style.position = 'fixed';
          ta.style.left = '-9999px';
          document.body.appendChild(ta);
          ta.select();
          document.execCommand('copy');
          document.body.removeChild(ta);
        }
        if (typeof OpenFangToast !== 'undefined' && OpenFangToast.info) {
          OpenFangToast.info('Copied node id', 1800);
        }
      } catch (e) {
        console.error('graph-memory: copy node id failed', e);
      }
    },

    async resolveWriteDetails(agentId, writeKind) {
      var aid = String(agentId || '').trim();
      if (!aid) {
        return [];
      }
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory?agent_id=' + encodeURIComponent(aid) + '&limit=80'
        );
        var nodes = (data && data.nodes) || [];
        var seen = this.seenNodeIdsByAgent[aid] || {};
        var fresh = nodes.filter(function (n) {
          return n && n.id && !seen[String(n.id)];
        });
        var matchingFresh = fresh.filter(
          function (n) {
            return this.kindMatchesWrite(n, writeKind);
          }.bind(this)
        );
        var fallback = nodes.filter(
          function (n) {
            return this.kindMatchesWrite(n, writeKind);
          }.bind(this)
        );
        this.rememberSeenNodes(aid, nodes);
        var picks = (matchingFresh.length ? matchingFresh : fallback).slice(0, 4);
        return picks.map(this.summarizeNode.bind(this));
      } catch (e) {
        console.error('graph-memory: resolve write details failed', e);
        return [];
      }
    },

    async handleGraphMemoryKernelEvent(detail) {
      var payload = detail && detail.payload && detail.payload.data;
      if (!payload) {
        return;
      }
      var aid = String(payload.agent_id || '').trim();
      var kind = String(payload.kind || '').trim() || 'write';
      var whenMs = this.parseEventTimeMs(detail);
      var prov = payload.provenance || null;
      var items;
      if (prov && prov.summary) {
        var nk = prov.node_kind
          ? String(prov.node_kind)
          : this.timelineKindClass(kind);
        var nid =
          prov.node_ids && prov.node_ids.length ? String(prov.node_ids[0]) : '';
        items = [
          {
            nodeId: nid,
            nodeKind: nk,
            nodeLabel: String(prov.summary),
            reason: prov.reason ? String(prov.reason) : '',
          },
        ];
      } else {
        items = await this.resolveWriteDetails(aid, kind);
      }
      if (!items.length) {
        items = [
          {
            nodeId: '',
            nodeKind: this.timelineKindClass(kind),
            nodeLabel: 'New ' + this.writeKindLabel(kind) + ' stored',
            reason: '',
          },
        ];
      }
      var self = this;
      var evId = detail && detail.id != null ? String(detail.id) : '';
      items.forEach(function (item, idx) {
        self.pushTimelineEntry({
          id:
            (evId || String(whenMs)) +
            ':' +
            kind +
            ':' +
            aid +
            ':' +
            (item.nodeId || 'none') +
            ':' +
            String(idx),
          atMs: whenMs,
          agentId: aid,
          agentName: self.agentNameById(aid),
          writeKind: kind,
          nodeKind: item.nodeKind || self.timelineKindClass(kind),
          nodeLabel: item.nodeLabel,
          nodeId: item.nodeId || '',
          reason: item.reason || '',
        });
      });
      if (this.viewMode === 'live' && String(this.agentId || '') === aid) {
        await this.fetchGraph();
      }
    },

    /** Focus a node on the graph after live refresh (timeline click). */
    selectNodeById(nodeId) {
      var id = String(nodeId || '').trim();
      if (!id) {
        return;
      }
      var n = (this.nodes || []).find(function (x) {
        return x && String(x.id) === id;
      });
      if (!n) {
        return;
      }
      this.selected = n;
      var self = this;
      this.$nextTick(function () {
        self.renderGraph();
      });
    },

    /** Short deterministic line for procedural / persona nodes (timeline fallback). */
    timelineDetailLine(entry) {
      if (entry && entry.reason) {
        return String(entry.reason).replace(/_/g, ' ');
      }
      return '';
    },

    /** Remove kernel SSE bridge listener (see {@link init}). */
    cleanupKernelListener() {
      if (this._graphMemoryKernelHandler) {
        try {
          window.removeEventListener(
            'armaraos-kernel-event',
            this._graphMemoryKernelHandler
          );
        } catch (e) {
          /* ignore */
        }
        this._graphMemoryKernelHandler = null;
      }
      if (this._kernelEventsPollTimer) {
        try {
          clearInterval(this._kernelEventsPollTimer);
        } catch (e2) {
          /* ignore */
        }
        this._kernelEventsPollTimer = null;
      }
      this._lastSeenKernelReceivedCount = -1;
    },

    async init() {
      await this.loadAgents();
      await this.fetchGraph();
      await this.refreshGovernanceData();

      var self = this;
      this.cleanupKernelListener();
      this._graphMemoryKernelHandler = function (e) {
        try {
          self.ingestKernelDetailForTimeline(e.detail);
        } catch (err) {
          console.warn('graph-memory: kernel event handling failed', err);
        }
      };
      window.addEventListener('armaraos-kernel-event', this._graphMemoryKernelHandler);

      try {
        if (typeof this.$watch === 'function') {
          this.$watch(
            function () {
              try {
                return Alpine.store('kernelEvents').received;
              } catch (eR) {
                return 0;
              }
            },
            function () {
              try {
                var ke = Alpine.store('kernelEvents');
                if (ke && ke.last) {
                  self.ingestKernelDetailForTimeline(ke.last);
                }
              } catch (eW) {
                console.warn('graph-memory: kernelEvents watch failed', eW);
              }
            }
          );
        }
      } catch (eWatch) {
        console.warn('graph-memory: could not bind kernelEvents $watch', eWatch);
      }

      this.replayRecentKernelGraphMemoryFromStore();

      try {
        var ke0 = Alpine.store('kernelEvents');
        this._lastSeenKernelReceivedCount =
          ke0 && typeof ke0.received === 'number' ? ke0.received : -1;
      } catch (e0) {
        this._lastSeenKernelReceivedCount = -1;
      }
      this._kernelEventsPollTimer = setInterval(function () {
        try {
          var ke = Alpine.store('kernelEvents');
          var n = ke && typeof ke.received === 'number' ? ke.received : 0;
          if (n === self._lastSeenKernelReceivedCount) {
            return;
          }
          self._lastSeenKernelReceivedCount = n;
          if (ke && ke.items && ke.items.length) {
            var tail = ke.items.slice(-40);
            for (var t = 0; t < tail.length; t++) {
              var entry = tail[t];
              if (!entry || !entry.payload) {
                continue;
              }
              self.ingestKernelDetailForTimeline({
                id: entry.id,
                timestamp: entry.ts,
                payload: entry.payload,
              });
            }
          } else if (ke && ke.last) {
            self.ingestKernelDetailForTimeline(ke.last);
          }
        } catch (ePoll) {
          /* ignore */
        }
      }, 900);

      var canvas = document.getElementById('graph-memory-canvas');
      if (canvas && window.ResizeObserver) {
        new ResizeObserver(function () {
          if (self.nodes.length > 0) {
            self.renderGraph();
          }
        }).observe(canvas);
      }
    },

    /**
     * Build real <option> nodes under the agent <select>.
     * WebKit (Safari / Tauri) often omits options generated via <template x-for> inside <select>.
     */
    syncAgentSelectDom() {
      var sel = document.getElementById('gm-agent-select');
      if (!sel || typeof sel.appendChild !== 'function') {
        return;
      }
      var prev = String(this.agentId || '');
      while (sel.firstChild) {
        sel.removeChild(sel.firstChild);
      }
      if (!this.agents.length) {
        var ph = document.createElement('option');
        ph.value = '';
        ph.disabled = true;
        ph.textContent = 'No agents loaded';
        sel.appendChild(ph);
        this.agentId = '';
        return;
      }
      this.agents.forEach(function (a) {
        var opt = document.createElement('option');
        opt.value = a.id;
        opt.textContent = a.label || a.id;
        sel.appendChild(opt);
      });
      var hasPrev = this.agents.some(function (a) {
        return String(a.id) === prev;
      });
      this.agentId = hasPrev ? prev : String(this.agents[0].id);
      try {
        sel.value = this.agentId;
      } catch (eVal) {
        /* ignore */
      }
    },

    /** Populate agent picker (auth headers via OpenFangAPI; optional cache from Alpine store). */
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
          list = graphMemoryNormalizeAgentsPayload(await OpenFangAPI.get('/api/agents'));
        }
        // Option `value` must be `GET /api/agents` `id` (UUID string) — same path segment as
        // `GraphMemoryWriter::open(session.agent_id)` and `?agent_id=` for `/api/graph-memory`.
        this.agents = list
          .map(function (a) {
            var id = String(a.id != null ? a.id : a.agent_id || '').trim();
            if (!id) {
              return null;
            }
            var nm = (a.name && String(a.name).trim()) || 'Agent';
            var short = id.length > 10 ? id.slice(0, 8) + '…' : id;
            return { id: id, name: nm, label: nm + ' — ' + short };
          })
          .filter(Boolean);
        if (this.agents.length > 0 && !this.agentId) {
          this.agentId = this.agents[0].id;
        }
        await this.$nextTick();
        this.syncAgentSelectDom();
      } catch (e0) {
        console.error('graph-memory: loadAgents failed', e0);
        this.agents = [];
        await this.$nextTick();
        this.syncAgentSelectDom();
      }
    },

    async refreshPanel() {
      await this.loadAgents();
      this.viewMode = 'live';
      await this.fetchGraph();
      await this.refreshGovernanceData();
    },

    async fetchGraph() {
      if (!this.agentId) {
        return;
      }
      this.loading = true;
      try {
        var qs = this.graphQueryString(300);
        var path =
          '/api/graph-memory?agent_id=' +
          encodeURIComponent(this.agentId) +
          qs;
        var data = await OpenFangAPI.get(path);
        this.nodes = data.nodes || [];
        this.edges = data.edges || [];
        this.rememberSeenNodes(this.agentId, this.nodes);
        await this.$nextTick();
        this.renderGraph();
      } catch (e) {
        console.error('graph-memory fetch:', e);
      } finally {
        this.loading = false;
      }
    },

    async onAgentChange() {
      this.viewMode = 'live';
      await this.fetchGraph();
      await this.refreshGovernanceData();
    },

    improvementProposalRowStatus(p) {
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

    async loadImprovementProposals() {
      var aid = String(this.agentId || '').trim();
      if (!aid) {
        this.improvementProposals = [];
        return;
      }
      this.improvementProposalsLoading = true;
      try {
        var d = await OpenFangAPI.get(
          '/api/graph-memory/improvement-proposals?agent_id=' + encodeURIComponent(aid) + '&limit=50'
        );
        this.improvementProposals = d && d.ok && Array.isArray(d.proposals) ? d.proposals : [];
      } catch (e) {
        console.warn('graph-memory: loadImprovementProposals', e);
        this.improvementProposals = [];
      } finally {
        this.improvementProposalsLoading = false;
      }
    },

    async refreshGovernanceData() {
      await Promise.all([
        this.loadSnapshots(),
        this.loadAudit(),
        this.loadMemoryControls(),
        this.loadInspectEntries(),
        this.loadMemoryDiagnostics(),
        this.loadImprovementProposals(),
      ]);
    },

    async loadMemoryDiagnostics() {
      try {
        var status = await OpenFangAPI.get('/api/status');
        this.memoryDiagnostics = (status && status.graph_memory_context_metrics) || null;
        this.memorySelectionDebug = (status && status.graph_memory_selection_debug) || [];
      } catch (e) {
        this.memoryDiagnostics = null;
        this.memorySelectionDebug = [];
      }
    },

    async loadMemoryControls() {
      if (!this.agentId) {
        return;
      }
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory/controls?agent_id=' + encodeURIComponent(this.agentId)
        );
        var c = (data && data.controls) || {};
        this.memoryControls = {
          memory_enabled: c.memory_enabled !== false,
          temporary_mode: !!c.temporary_mode,
          shared_memory_enabled: !!c.shared_memory_enabled,
          include_episodic_hints: c.include_episodic_hints !== false,
          include_semantic_facts: c.include_semantic_facts !== false,
          include_conflicts: c.include_conflicts !== false,
          include_procedural_hints: c.include_procedural_hints !== false,
          include_suggested_pattern_candidates: c.include_suggested_pattern_candidates !== false,
        };
      } catch (e) {
        console.error('graph-memory: load controls failed', e);
      }
    },

    async saveMemoryControls() {
      if (!this.agentId || this.governanceBusy) {
        return;
      }
      this.governanceBusy = true;
      try {
        await OpenFangAPI.put('/api/graph-memory/controls', {
          agent_id: this.agentId,
          memory_enabled: !!this.memoryControls.memory_enabled,
          temporary_mode: !!this.memoryControls.temporary_mode,
          shared_memory_enabled: !!this.memoryControls.shared_memory_enabled,
          include_episodic_hints: !!this.memoryControls.include_episodic_hints,
          include_semantic_facts: !!this.memoryControls.include_semantic_facts,
          include_conflicts: !!this.memoryControls.include_conflicts,
          include_procedural_hints: !!this.memoryControls.include_procedural_hints,
          include_suggested_pattern_candidates:
            !!this.memoryControls.include_suggested_pattern_candidates,
        });
        this.notifySuccess('Memory controls saved');
      } catch (e) {
        console.error('graph-memory: save controls failed', e);
        this.notifyError(this.errorText(e, 'Could not save memory controls'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async loadInspectEntries() {
      if (!this.agentId) {
        this.inspectEntries = [];
        return;
      }
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory/what-do-you-remember?agent_id=' +
            encodeURIComponent(this.agentId) +
            '&scope=' +
            encodeURIComponent(this.inspectScope) +
            '&limit=50'
        );
        this.inspectEntries = (data && data.entries) || [];
      } catch (e) {
        console.error('graph-memory: inspect failed', e);
        this.inspectEntries = [];
      }
    },

    async rememberFact() {
      var fact = String(this.rememberFactText || '').trim();
      if (!this.agentId || !fact || this.governanceBusy) {
        return;
      }
      this.governanceBusy = true;
      try {
        await OpenFangAPI.post('/api/graph-memory/remember', {
          agent_id: this.agentId,
          fact: fact,
          scope: this.inspectScope,
          confidence: 0.9,
        });
        this.rememberFactText = '';
        await this.loadInspectEntries();
        await this.fetchGraph();
        this.notifySuccess('Fact remembered');
      } catch (e) {
        console.error('graph-memory: remember failed', e);
        this.notifyError(this.errorText(e, 'Could not remember fact'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async forgetFact(entry) {
      var fact = entry && entry.fact ? String(entry.fact) : '';
      if (!this.agentId || !fact || this.governanceBusy) {
        return;
      }
      this.governanceBusy = true;
      try {
        await OpenFangAPI.post('/api/graph-memory/forget', {
          agent_id: this.agentId,
          fact: fact,
        });
        await this.loadInspectEntries();
        await this.fetchGraph();
        this.notifySuccess('Fact forgotten');
      } catch (e) {
        console.error('graph-memory: forget failed', e);
        this.notifyError(this.errorText(e, 'Could not forget fact'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async clearInspectScope() {
      if (!this.agentId || this.governanceBusy) {
        return;
      }
      if (!window.confirm('Clear all semantic facts in this scope?')) {
        return;
      }
      this.governanceBusy = true;
      try {
        await OpenFangAPI.post('/api/graph-memory/clear-scope', {
          agent_id: this.agentId,
          scope: this.inspectScope,
          reason: 'dashboard clear scope',
        });
        await this.loadInspectEntries();
        await this.fetchGraph();
        this.notifySuccess('Scope cleared');
      } catch (e) {
        console.error('graph-memory: clear scope failed', e);
        this.notifyError(this.errorText(e, 'Could not clear scope'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async loadSnapshots() {
      if (!this.agentId) {
        this.snapshots = [];
        this.selectedSnapshotId = '';
        return;
      }
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory/snapshots?agent_id=' + encodeURIComponent(this.agentId)
        );
        this.snapshots = (data && data.snapshots) || [];
        if (
          this.snapshots.length > 0 &&
          !this.snapshots.some(
            function (s) {
              return s.id === this.selectedSnapshotId;
            }.bind(this)
          )
        ) {
          this.selectedSnapshotId = this.snapshots[0].id;
        }
        if (!this.snapshots.length) {
          this.selectedSnapshotId = '';
        }
      } catch (e) {
        console.error('graph-memory: load snapshots failed', e);
        this.snapshots = [];
        this.selectedSnapshotId = '';
        this.notifyError(this.errorText(e, 'Could not load snapshots'));
      }
    },

    async loadAudit() {
      if (!this.agentId) {
        this.auditEntries = [];
        return;
      }
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory/audit?agent_id=' +
            encodeURIComponent(this.agentId) +
            '&limit=40'
        );
        this.auditEntries = (data && data.entries) || [];
      } catch (e) {
        console.error('graph-memory: load audit failed', e);
        this.auditEntries = [];
      }
    },

    async previewSelectedSnapshot() {
      if (!this.agentId || !this.selectedSnapshotId || this.governanceBusy) {
        return;
      }
      this.loading = true;
      try {
        var qs = this.graphQueryString(300);
        var path =
          '/api/graph-memory/snapshot-graph?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&snapshot_id=' +
          encodeURIComponent(this.selectedSnapshotId) +
          qs;
        var data = await OpenFangAPI.get(path);
        this.nodes = data.nodes || [];
        this.edges = data.edges || [];
        this.selected = null;
        this.viewMode = 'snapshot';
        await this.$nextTick();
        this.renderGraph();
      } catch (e) {
        console.error('graph-memory: preview snapshot failed', e);
        this.notifyError(this.errorText(e, 'Could not preview snapshot'));
      } finally {
        this.loading = false;
      }
    },

    async returnToLiveGraph() {
      this.viewMode = 'live';
      await this.fetchGraph();
    },

    graphQueryString(limit) {
      var params = new URLSearchParams();
      params.set('limit', String(limit || 300));
      params.set('edge_mode', this.graphEdgeMode === 'augmented' ? 'augmented' : 'strict');
      if (this.graphExpandEgo) {
        params.set('ego_expand_1hop', 'true');
      }
      if (this.graphSyntheticProvenance) {
        params.set('synthetic_provenance', 'true');
      }
      return '&' + params.toString();
    },

    async setGraphEdgeMode(mode) {
      var m = mode === 'augmented' ? 'augmented' : 'strict';
      if (this.graphEdgeMode === m) {
        return;
      }
      this.graphEdgeMode = m;
      if (m === 'augmented') {
        // Augmented mode presets both expansion and provenance edges.
        this.graphExpandEgo = true;
        this.graphSyntheticProvenance = true;
      }
      if (this.viewMode === 'snapshot' && this.selectedSnapshotId) {
        await this.previewSelectedSnapshot();
        return;
      }
      await this.fetchGraph();
    },

    async refreshGraphWithEdgeOptions() {
      if (this.viewMode === 'snapshot' && this.selectedSnapshotId) {
        await this.previewSelectedSnapshot();
        return;
      }
      await this.fetchGraph();
    },

    async createSnapshot() {
      if (!this.agentId || this.governanceBusy) {
        return;
      }
      this.governanceBusy = true;
      try {
        var body = {
          agent_id: this.agentId,
          label: this.snapshotLabel ? String(this.snapshotLabel).trim() : 'manual',
        };
        var res = await OpenFangAPI.post('/api/graph-memory/snapshot', body);
        if (res && res.ok) {
          this.notifySuccess('Snapshot created');
        }
        this.snapshotLabel = '';
        await this.refreshGovernanceData();
      } catch (e) {
        console.error('graph-memory: create snapshot failed', e);
        this.notifyError(this.errorText(e, 'Could not create snapshot'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async rollbackSnapshot() {
      if (!this.agentId || !this.selectedSnapshotId || this.governanceBusy) {
        return;
      }
      if (!window.confirm('Rollback graph memory to selected snapshot?')) {
        return;
      }
      this.governanceBusy = true;
      try {
        var body = {
          agent_id: this.agentId,
          snapshot_id: this.selectedSnapshotId,
          reason: 'dashboard rollback',
        };
        var res = await OpenFangAPI.post('/api/graph-memory/rollback', body);
        if (res && res.ok) {
          this.notifySuccess('Rollback complete');
        }
        this.viewMode = 'live';
        await this.fetchGraph();
        await this.refreshGovernanceData();
      } catch (e) {
        console.error('graph-memory: rollback failed', e);
        this.notifyError(this.errorText(e, 'Could not rollback snapshot'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async resetGraphMemory() {
      if (!this.agentId || this.governanceBusy) {
        return;
      }
      if (
        !window.confirm(
          'Reset graph memory for this agent? A pre-reset snapshot will be created.'
        )
      ) {
        return;
      }
      this.governanceBusy = true;
      try {
        var body = {
          agent_id: this.agentId,
          reason: 'dashboard reset',
          create_snapshot: true,
        };
        var res = await OpenFangAPI.post('/api/graph-memory/reset', body);
        if (res && res.ok) {
          this.notifySuccess('Graph memory reset');
        }
        this.viewMode = 'live';
        this.selected = null;
        await this.fetchGraph();
        await this.refreshGovernanceData();
      } catch (e) {
        console.error('graph-memory: reset failed', e);
        this.notifyError(this.errorText(e, 'Could not reset graph memory'));
      } finally {
        this.governanceBusy = false;
      }
    },

    async deleteSelectedNode() {
      if (!this.agentId || !this.selected || !this.selected.id || this.governanceBusy) {
        return;
      }
      if (!window.confirm('Delete selected node and connected edges?')) {
        return;
      }
      this.governanceBusy = true;
      try {
        var body = {
          agent_id: this.agentId,
          node_id: this.selected.id,
          reason: 'dashboard node delete',
        };
        var res = await OpenFangAPI.post('/api/graph-memory/delete-node', body);
        if (res && res.ok) {
          this.notifySuccess('Node deleted');
        }
        this.viewMode = 'live';
        this.selected = null;
        await this.fetchGraph();
        await this.refreshGovernanceData();
      } catch (e) {
        console.error('graph-memory: delete node failed', e);
        this.notifyError(this.errorText(e, 'Could not delete node'));
      } finally {
        this.governanceBusy = false;
      }
    },

    cloneGraphState() {
      return {
        nodes: JSON.parse(JSON.stringify(this.nodes || [])),
        edges: JSON.parse(JSON.stringify(this.edges || [])),
      };
    },

    proceduralHintStats() {
      var rows = (this.nodes || []).filter(function(n) { return n && n.kind === 'procedural'; });
      if (!rows.length) {
        return { total: 0, active: 0, retired: 0, avgSuccess: 0, avgFitness: 0 };
      }
      var retired = 0;
      var successSum = 0;
      var successCount = 0;
      var fitnessSum = 0;
      var fitnessCount = 0;
      rows.forEach(function(r) {
        var m = r.meta || {};
        if (m.retired) retired += 1;
        if (typeof m.success_rate === 'number' && !Number.isNaN(m.success_rate)) {
          successSum += m.success_rate;
          successCount += 1;
        }
        if (typeof m.fitness === 'number' && !Number.isNaN(m.fitness)) {
          fitnessSum += m.fitness;
          fitnessCount += 1;
        }
      });
      return {
        total: rows.length,
        active: rows.length - retired,
        retired: retired,
        avgSuccess: successCount ? (successSum / successCount) : 0,
        avgFitness: fitnessCount ? (fitnessSum / fitnessCount) : 0
      };
    },

    runEpisodeSignature(node) {
      if (!node || node.kind !== 'episode') {
        return '';
      }
      var meta = node.meta || {};
      var tools = Array.isArray(meta.tool_calls) ? meta.tool_calls : [];
      var del = meta.delegation_to ? String(meta.delegation_to) : '';
      return tools.join(' > ') + (del ? ' | delegate:' + del : '');
    },

    nodeTextKey(node) {
      if (!node) {
        return '';
      }
      if (node.kind === 'procedural') {
        return String((node.meta && node.meta.pattern_name) || node.label || '');
      }
      if (node.kind === 'persona') {
        return String((node.meta && node.meta.trait_name) || node.label || '');
      }
      return String(node.label || '');
    },

    computeRunDelta(startGraph, endGraph) {
      var startNodes = (startGraph && startGraph.nodes) || [];
      var endNodes = (endGraph && endGraph.nodes) || [];
      var startIds = {};
      var endIds = {};
      startNodes.forEach(function (n) {
        if (n && n.id) {
          startIds[String(n.id)] = true;
        }
      });
      endNodes.forEach(function (n) {
        if (n && n.id) {
          endIds[String(n.id)] = true;
        }
      });
      var addedNodes = endNodes.filter(function (n) {
        return n && n.id && !startIds[String(n.id)];
      });
      var removedNodes = startNodes.filter(function (n) {
        return n && n.id && !endIds[String(n.id)];
      });
      var countKinds = function (arr) {
        var c = { episode: 0, semantic: 0, procedural: 0, persona: 0, runtime_state: 0 };
        arr.forEach(function (n) {
          var k = String((n && n.kind) || '');
          if (!c[k] && c[k] !== 0) {
            c[k] = 0;
          }
          c[k] += 1;
        });
        return c;
      };
      var startProcedural = new Set(
        startNodes
          .filter(function (n) {
            return n.kind === 'procedural';
          })
          .map(this.nodeTextKey.bind(this))
          .filter(Boolean)
      );
      var endProcedural = new Set(
        endNodes
          .filter(function (n) {
            return n.kind === 'procedural';
          })
          .map(this.nodeTextKey.bind(this))
          .filter(Boolean)
      );
      var startPersona = new Set(
        startNodes
          .filter(function (n) {
            return n.kind === 'persona';
          })
          .map(this.nodeTextKey.bind(this))
          .filter(Boolean)
      );
      var endPersona = new Set(
        endNodes
          .filter(function (n) {
            return n.kind === 'persona';
          })
          .map(this.nodeTextKey.bind(this))
          .filter(Boolean)
      );
      var overlap = function (a, b) {
        var n = 0;
        a.forEach(function (v) {
          if (b.has(v)) {
            n += 1;
          }
        });
        return n;
      };
      var episodeSignatures = addedNodes
        .filter(function (n) {
          return n.kind === 'episode';
        })
        .map(this.runEpisodeSignature.bind(this))
        .filter(Boolean);
      return {
        addedCounts: countKinds(addedNodes),
        removedCounts: countKinds(removedNodes),
        continuity: {
          procedural_before: startProcedural.size,
          procedural_after: endProcedural.size,
          procedural_overlap: overlap(startProcedural, endProcedural),
          persona_before: startPersona.size,
          persona_after: endPersona.size,
          persona_overlap: overlap(startPersona, endPersona),
        },
        episodeSignatures: Array.from(new Set(episodeSignatures)).sort(),
      };
    },

    async startRunCapture(slot) {
      await this.fetchGraph();
      this.runCaptures[slot] = {
        atMs: Date.now(),
        graph: this.cloneGraphState(),
      };
      this.runResults[slot] = null;
    },

    async endRunCapture(slot) {
      var st = this.runCaptures[slot];
      if (!st || !st.graph) {
        return;
      }
      await this.fetchGraph();
      var endGraph = this.cloneGraphState();
      this.runResults[slot] = this.computeRunDelta(st.graph, endGraph);
    },

    determinismSummary() {
      var a = this.runResults.A;
      var b = this.runResults.B;
      if (!a || !b) {
        return null;
      }
      var aSig = (a.episodeSignatures || []).slice().sort();
      var bSig = (b.episodeSignatures || []).slice().sort();
      var exact = JSON.stringify(aSig) === JSON.stringify(bSig);
      return {
        exactEpisodeMatch: exact,
        aEpisodeCount: aSig.length,
        bEpisodeCount: bSig.length,
      };
    },

    toggleFilter(kind) {
      if (this.filters.includes(kind)) {
        this.filters = this.filters.filter(function (k) {
          return k !== kind;
        });
      } else {
        this.filters.push(kind);
      }
      this.renderGraph();
    },

    connectedTo(nodeId) {
      if (!nodeId) {
        return [];
      }
      var ids = new Set();
      this.edges.forEach(function (e) {
        var s = e.source && e.source.id !== undefined ? e.source.id : e.source;
        var t = e.target && e.target.id !== undefined ? e.target.id : e.target;
        if (s === nodeId) {
          ids.add(t);
        }
        if (t === nodeId) {
          ids.add(s);
        }
      });
      return this.nodes.filter(function (n) {
        return ids.has(n.id);
      });
    },

    /**
     * Reset pan/zoom to identity (1:1, top-left origin behavior matches d3 default view).
     */
    resetGraphView() {
      var svg = d3.select('#gm-svg');
      if (svg.empty() || !this._gmZoomBehavior || typeof d3 === 'undefined') {
        return;
      }
      this._gmLastTransform = d3.zoomIdentity;
      svg
        .transition()
        .duration(280)
        .call(this._gmZoomBehavior.transform, d3.zoomIdentity);
    },

    /**
     * Fit all currently simulated node positions into the viewport with padding.
     * Uses the standard translate(center).scale(k).translate(-cx,-cy) composition.
     */
    fitGraphToView(zoomBias) {
      if (typeof d3 === 'undefined') {
        return;
      }
      var canvas = document.getElementById('graph-memory-canvas');
      var svg = d3.select('#gm-svg');
      if (!canvas || svg.empty() || !this._gmZoomBehavior || !this.simulation) {
        return;
      }
      var nds = this.simulation.nodes();
      if (!nds || !nds.length) {
        return;
      }
      var W = canvas.clientWidth || 800;
      var H = canvas.clientHeight || 500;
      var pad = 56;
      var labelTop = 52;
      var minX = Infinity;
      var minY = Infinity;
      var maxX = -Infinity;
      var maxY = -Infinity;
      var kindR = { persona: 14, episode: 10 };
      nds.forEach(function (n) {
        if (n.x == null || n.y == null || !Number.isFinite(n.x) || !Number.isFinite(n.y)) {
          return;
        }
        var rr = kindR[n.kind] != null ? kindR[n.kind] : 8;
        minX = Math.min(minX, n.x - rr - 6);
        maxX = Math.max(maxX, n.x + rr + 6);
        minY = Math.min(minY, n.y - rr - labelTop);
        maxY = Math.max(maxY, n.y + rr + 10);
      });
      if (!Number.isFinite(minX) || !Number.isFinite(maxX)) {
        return;
      }
      var bw = Math.max(maxX - minX, 64);
      var bh = Math.max(maxY - minY, 64);
      var k = Math.min((W - 2 * pad) / bw, (H - 2 * pad) / bh);
      var bias = typeof zoomBias === 'number' && Number.isFinite(zoomBias) ? zoomBias : 1;
      k = k * bias;
      k = Math.max(this._gmZoomMin, Math.min(k, this._gmZoomMax));
      var cx = (minX + maxX) / 2;
      var cy = (minY + maxY) / 2;
      var t = d3.zoomIdentity.translate(W / 2, H / 2).scale(k).translate(-cx, -cy);
      this._gmLastTransform = t;
      svg.transition().duration(420).call(this._gmZoomBehavior.transform, t);
    },

    renderGraph() {
      if (typeof d3 === 'undefined') {
        return;
      }
      var filteredNodes = this.nodes.filter(function (n) {
        return this.filters.includes(n.kind);
      }, this);
      var nodeIds = new Set(filteredNodes.map(function (n) {
        return n.id;
      }));
      var filteredEdges = this.edges.filter(function (e) {
        var s = e.source && e.source.id !== undefined ? e.source.id : e.source;
        var t = e.target && e.target.id !== undefined ? e.target.id : e.target;
        return nodeIds.has(s) && nodeIds.has(t);
      });

      var svg = d3.select('#gm-svg');
      var canvas = document.getElementById('graph-memory-canvas');
      if (!canvas) {
        return;
      }
      var W = canvas.clientWidth || 800;
      var H = canvas.clientHeight || 500;

      svg.select('#gm-links').selectAll('*').remove();
      svg.select('#gm-nodes').selectAll('*').remove();
      svg.select('#gm-labels').selectAll('*').remove();

      if (filteredNodes.length === 0) {
        return;
      }

      var simNodes = filteredNodes.map(function (n) {
        return Object.assign({}, n);
      });
      var simEdges = filteredEdges.map(function (e) {
        return {
          rel: e.rel,
          inferred: !!e.inferred,
          source: e.source && e.source.id !== undefined ? e.source.id : e.source,
          target: e.target && e.target.id !== undefined ? e.target.id : e.target,
        };
      });

      if (this.simulation) {
        this.simulation.stop();
      }
      var self = this;
      this.simulation = d3
        .forceSimulation(simNodes)
        .force(
          'link',
          d3
            .forceLink(simEdges)
            .id(function (d) {
              return d.id;
            })
            .distance(function () {
              return 80 + Math.random() * 40;
            })
            .strength(0.4)
        )
        .force('charge', d3.forceManyBody().strength(-220))
        .force('center', d3.forceCenter(W / 2, H / 2))
        .force('collision', d3.forceCollide(28))
        .alphaDecay(0.025);

      if (self._gmZoomBehavior) {
        svg.on('.zoom', null);
      }
      var zoom = d3
        .zoom()
        .scaleExtent([self._gmZoomMin, self._gmZoomMax])
        // Keep page scrolling usable: plain wheel scrolls the page.
        // Power users can still wheel-zoom the graph with Ctrl/Cmd + wheel.
        .filter(function (event) {
          if (!event) {
            return true;
          }
          if (event.type === 'wheel') {
            return !!(event.ctrlKey || event.metaKey);
          }
          return true;
        })
        .on('zoom', function (event) {
          self._gmLastTransform = event.transform;
          svg.select('#gm-links').attr('transform', event.transform);
          svg.select('#gm-nodes').attr('transform', event.transform);
          svg.select('#gm-labels').attr('transform', event.transform);
        });
      self._gmZoomBehavior = zoom;
      svg.call(zoom);
      var startTransform = self._gmLastTransform != null ? self._gmLastTransform : d3.zoomIdentity;
      svg.call(zoom.transform, startTransform);
      // d3-zoom may set touch-action:none on the SVG, which blocks trackpad/page scroll
      // chaining on some engines; pan-y keeps vertical scroll while Ctrl/Cmd+wheel still zooms.
      try {
        svg.style('touch-action', 'pan-y');
      } catch (eTouch) {
        /* ignore */
      }
      if (self._gmLastTransform == null) {
        // First render: auto-fit after initial force settle, but keep a slightly
        // closer view than full "fit all" so the graph remains readable.
        setTimeout(function () {
          if (self._gmLastTransform != null || !self.simulation) {
            return;
          }
          self.fitGraphToView(1.12);
        }, 220);
      }

      var linkSel = svg
        .select('#gm-links')
        .selectAll('line')
        .data(simEdges)
        .enter()
        .append('line')
        .attr('stroke', function (d) {
          return d.inferred ? 'rgba(124,211,255,0.28)' : 'rgba(148,163,210,0.22)';
        })
        .attr('stroke-width', function (d) {
          return d.rel ? 1.35 : 1.05;
        })
        .attr('stroke-dasharray', function (d) {
          return d.inferred ? '4 3' : null;
        })
        .attr('stroke-linecap', 'round')
        .attr('marker-end', 'url(#gm-arrow)')
        .attr('title', function (d) {
          if (!d.rel) {
            return '';
          }
          return d.inferred ? String(d.rel) + ' (inferred)' : String(d.rel);
        });

      function gmNodeGradientId(kind) {
        var k = kind || 'semantic';
        var safe =
          k === 'runtime_state'
            ? 'runtime_state'
            : ['episode', 'semantic', 'procedural', 'persona'].indexOf(k) >= 0
              ? k
              : 'semantic';
        return 'url(#gm-rad-' + safe + ')';
      }

      var nodeSel = svg
        .select('#gm-nodes')
        .selectAll('circle')
        .data(simNodes)
        .enter()
        .append('circle')
        .attr('r', function (d) {
          return d.kind === 'persona' ? 14 : d.kind === 'episode' ? 10 : 8;
        })
        .attr('fill', function (d) {
          return gmNodeGradientId(d.kind);
        })
        .attr('fill-opacity', 0.98)
        .attr('stroke', function (d) {
          return self.kindColor[d.kind] || '#888';
        })
        .attr('stroke-width', function (d) {
          return d.kind === 'persona' ? 2.2 : 1.75;
        })
        .attr('stroke-opacity', 0.65)
        .attr('filter', 'url(#gm-node-shadow)')
        .attr('class', function (d) {
          return 'gm-node gm-node-' + (d.kind || 'semantic');
        })
        .style('cursor', 'pointer')
        .on('mouseover', function (event, d) {
          var base = d.kind === 'persona' ? 14 : d.kind === 'episode' ? 10 : 8;
          d3.select(this)
            .transition()
            .duration(150)
            .attr('r', base * 1.35)
            .attr('fill-opacity', 1)
            .attr('filter', 'url(#gm-node-active)')
            .attr('stroke-opacity', 0.92);
        })
        .on('mouseout', function (event, d) {
          if (!self.selected || self.selected.id !== d.id) {
            var base = d.kind === 'persona' ? 14 : d.kind === 'episode' ? 10 : 8;
            d3.select(this)
              .transition()
              .duration(150)
              .attr('r', base)
              .attr('fill-opacity', 0.98)
              .attr('filter', 'url(#gm-node-shadow)')
              .attr('stroke-opacity', 0.65);
          }
        })
        .on('click', function (event, d) {
          event.stopPropagation();
          self.selected = d;
          linkSel
            .attr('stroke', function (e) {
              var s = e.source && e.source.id !== undefined ? e.source.id : e.source;
              var t = e.target && e.target.id !== undefined ? e.target.id : e.target;
              return s === d.id || t === d.id ? 'rgba(255,255,255,0.55)' : 'rgba(255,255,255,0.08)';
            })
            .attr('stroke-width', function (e) {
              var s = e.source && e.source.id !== undefined ? e.source.id : e.source;
              var t = e.target && e.target.id !== undefined ? e.target.id : e.target;
              return s === d.id || t === d.id ? 2 : 1;
            });
        })
        .call(
          d3
            .drag()
            .on('start', function (event, d) {
              if (!event.active) {
                self.simulation.alphaTarget(0.3).restart();
              }
              d.fx = d.x;
              d.fy = d.y;
            })
            .on('drag', function (event, d) {
              d.fx = event.x;
              d.fy = event.y;
            })
            .on('end', function (event, d) {
              if (!event.active) {
                self.simulation.alphaTarget(0);
              }
              d.fx = null;
              d.fy = null;
            })
        );

      var labelSel = svg
        .select('#gm-labels')
        .selectAll('text')
        .data(
          simNodes.filter(function (n) {
            return (
              n.kind === 'persona' ||
              n.kind === 'episode' ||
              n.kind === 'procedural' ||
              n.kind === 'semantic'
            );
          })
        )
        .enter()
        .append('text')
        .attr('font-size', function (d) {
          return d.kind === 'persona' || d.kind === 'episode' ? '10px' : '9px';
        })
        .attr('fill', function (d) {
          return self.kindColor[d.kind] || '#888';
        })
        .attr('fill-opacity', 0.92)
        .attr('stroke', 'rgba(8,10,18,0.92)')
        .attr('stroke-width', 0.35)
        .attr('paint-order', 'stroke fill')
        .attr('text-anchor', 'middle')
        .attr('dy', function (d) {
          if (d.kind === 'persona') {
            return -18;
          }
          if (d.kind === 'episode') {
            return -13;
          }
          return -11;
        })
        .attr('pointer-events', 'none')
        .text(function (d) {
          var raw =
            d.kind === 'procedural' || d.kind === 'persona'
              ? self.nodeTextKey(d)
              : d.label || '';
          var lab = String(raw || '');
          var max = d.kind === 'semantic' ? 22 : 28;
          return lab.slice(0, max) + (lab.length > max ? '…' : '');
        });

      svg.on('click', function () {
        self.selected = null;
        linkSel.attr('stroke', 'rgba(255,255,255,0.12)').attr('stroke-width', 1.2);
      });

      this.simulation.on('tick', function () {
        linkSel
          .attr('x1', function (d) {
            return d.source.x;
          })
          .attr('y1', function (d) {
            return d.source.y;
          })
          .attr('x2', function (d) {
            return d.target.x;
          })
          .attr('y2', function (d) {
            return d.target.y;
          });
        nodeSel
          .attr('cx', function (d) {
            return d.x;
          })
          .attr('cy', function (d) {
            return d.y;
          });
        labelSel
          .attr('x', function (d) {
            return d.x;
          })
          .attr('y', function (d) {
            return d.y;
          });
      });
    },
  };
}
