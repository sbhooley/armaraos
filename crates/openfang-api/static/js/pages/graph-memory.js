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

function graphMemoryPanel() {
  return {
    nodes: [],
    edges: [],
    agents: [],
    agentId: '',
    filters: ['episode', 'semantic', 'procedural', 'persona'],
    selected: null,
    loading: false,
    simulation: null,
    liveTimeline: [],
    timelineLimit: 60,
    seenNodeIdsByAgent: {},
    timelineScope: 'all',
    snapshots: [],
    auditEntries: [],
    selectedSnapshotId: '',
    viewMode: 'live',
    governanceBusy: false,
    snapshotLabel: '',
    runCaptures: { A: null, B: null },
    runResults: { A: null, B: null },

    kindColor: {
      episode: '#7c9ef5',
      semantic: '#6be8a0',
      procedural: '#f5c75a',
      persona: '#c97cf5',
    },

    isGraphMemoryKernelEvent(d) {
      var p = d && d.payload;
      return !!(p && p.type === 'System' && p.data && p.data.event === 'GraphMemoryWrite');
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
      return true;
    },

    summarizeNode(node) {
      var label = String((node && node.label) || '').trim();
      if (!label) {
        label = 'Untitled memory node';
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
      this.liveTimeline.unshift(entry);
      if (this.liveTimeline.length > this.timelineLimit) {
        this.liveTimeline = this.liveTimeline.slice(0, this.timelineLimit);
      }
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
      var items = await this.resolveWriteDetails(aid, kind);
      if (!items.length) {
        items = [
          {
            nodeId: '',
            nodeKind: this.timelineKindClass(kind),
            nodeLabel: 'New ' + this.writeKindLabel(kind) + ' stored',
          },
        ];
      }
      var self = this;
      items.forEach(function (item, idx) {
        self.pushTimelineEntry({
          id:
            String(whenMs) +
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
        });
      });
      if (this.viewMode === 'live' && String(this.agentId || '') === aid) {
        await this.fetchGraph();
      }
    },

    async init() {
      await this.loadAgents();
      await this.fetchGraph();
      await this.refreshGovernanceData();

      var self = this;
      window.addEventListener('armaraos-kernel-event', function (e) {
        if (self.isGraphMemoryKernelEvent(e.detail)) {
          self.handleGraphMemoryKernelEvent(e.detail);
        }
      });

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
        var path =
          '/api/graph-memory?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&limit=300';
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

    async refreshGovernanceData() {
      await Promise.all([this.loadSnapshots(), this.loadAudit()]);
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
        var path =
          '/api/graph-memory/snapshot-graph?agent_id=' +
          encodeURIComponent(this.agentId) +
          '&snapshot_id=' +
          encodeURIComponent(this.selectedSnapshotId) +
          '&limit=300';
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

      var zoom = d3
        .zoom()
        .scaleExtent([0.2, 4])
        .on('zoom', function (event) {
          svg.select('#gm-links').attr('transform', event.transform);
          svg.select('#gm-nodes').attr('transform', event.transform);
          svg.select('#gm-labels').attr('transform', event.transform);
        });
      svg.call(zoom);

      var linkSel = svg
        .select('#gm-links')
        .selectAll('line')
        .data(simEdges)
        .enter()
        .append('line')
        .attr('stroke', 'rgba(255,255,255,0.12)')
        .attr('stroke-width', 1.2)
        .attr('marker-end', 'url(#gm-arrow)');

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
          return self.kindColor[d.kind] || '#888';
        })
        .attr('fill-opacity', 0.85)
        .attr('stroke', function (d) {
          return self.kindColor[d.kind] || '#888';
        })
        .attr('stroke-width', 1.5)
        .attr('stroke-opacity', 0.5)
        .attr('filter', function (d) {
          return d.kind === 'persona' ? 'url(#gm-glow)' : null;
        })
        .style('cursor', 'pointer')
        .on('mouseover', function (event, d) {
          d3.select(this)
            .transition()
            .duration(150)
            .attr('r', (d.kind === 'persona' ? 14 : d.kind === 'episode' ? 10 : 8) * 1.4)
            .attr('fill-opacity', 1)
            .attr('filter', 'url(#gm-glow)');
        })
        .on('mouseout', function (event, d) {
          if (!self.selected || self.selected.id !== d.id) {
            d3.select(this)
              .transition()
              .duration(150)
              .attr('r', d.kind === 'persona' ? 14 : d.kind === 'episode' ? 10 : 8)
              .attr('fill-opacity', 0.85)
              .attr('filter', d.kind === 'persona' ? 'url(#gm-glow)' : null);
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
            return n.kind === 'persona' || n.kind === 'episode';
          })
        )
        .enter()
        .append('text')
        .attr('font-size', '10px')
        .attr('fill', function (d) {
          return self.kindColor[d.kind];
        })
        .attr('fill-opacity', 0.8)
        .attr('text-anchor', 'middle')
        .attr('dy', function (d) {
          return d.kind === 'persona' ? -18 : -13;
        })
        .attr('pointer-events', 'none')
        .text(function (d) {
          var lab = d.label || '';
          return lab.slice(0, 28) + (lab.length > 28 ? '…' : '');
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
