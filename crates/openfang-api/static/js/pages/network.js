// ArmaraOS Network page — mesh visualization + live agent traffic feed
'use strict';

function networkPage() {
  return {
    loading: true,
    loadError: '',
    netStatus: null,
    peers: [],
    a2aAgents: [],
    channelNodes: [],
    localAgents: [],
    events: [],
    trafficFilter: 'all',
    sseSource: null,
    _pollTimer: null,
    graphLayout: { cx: 250, cy: 158, nodes: [], edges: [], overflow: { peers: 0, a2a: 0, channels: 0 } },

    refreshGraph() {
      this.graphLayout = this.buildGraphLayout();
    },

    async loadData() {
      this.loading = true;
      this.loadError = '';
      try {
        var results = await Promise.all([
          OpenFangAPI.get('/api/network/status').catch(function() { return null; }),
          OpenFangAPI.get('/api/peers').catch(function() { return { peers: [] }; }),
          OpenFangAPI.get('/api/a2a/agents').catch(function() { return { agents: [] }; }),
          OpenFangAPI.get('/api/channels').catch(function() { return { channels: [] }; }),
          OpenFangAPI.get('/api/agents').catch(function() { return []; }),
          OpenFangAPI.get('/api/comms/events?limit=200').catch(function() { return []; })
        ]);
        this.netStatus = results[0];
        this.peers = (results[1].peers || []).map(function(p) {
          return {
            node_id: p.node_id,
            node_name: p.node_name || 'Peer',
            address: p.address,
            state: p.state,
            protocol_version: p.protocol_version || 1
          };
        });
        this.a2aAgents = results[2].agents || [];
        var chans = results[3].channels || [];
        this.channelNodes = chans.filter(function(c) {
          return c.configured;
        }).slice(0, 12);
        var agentsRaw = results[4];
        this.localAgents = Array.isArray(agentsRaw) ? agentsRaw : (agentsRaw.agents || []);
        this.events = Array.isArray(results[5]) ? results[5] : [];
        this.refreshGraph();
        this.startSSE();
        this.startPolling();
      } catch (e) {
        this.loadError = e.message || 'Could not load network data.';
      }
      this.loading = false;
    },

    startPolling() {
      var self = this;
      this.stopPolling();
      this._pollTimer = setInterval(async function() {
        try {
          var ns = await OpenFangAPI.get('/api/network/status');
          self.netStatus = ns;
          self.refreshGraph();
        } catch (e) { /* ignore */ }
        try {
          var pr = await OpenFangAPI.get('/api/peers');
          self.peers = (pr.peers || []).map(function(p) {
            return {
              node_id: p.node_id,
              node_name: p.node_name || 'Peer',
              address: p.address,
              state: p.state,
              protocol_version: p.protocol_version || 1
            };
          });
          self.refreshGraph();
        } catch (e) { /* ignore */ }
      }, 4000);
    },

    stopPolling() {
      if (this._pollTimer) {
        clearInterval(this._pollTimer);
        this._pollTimer = null;
      }
    },

    startSSE() {
      if (this.sseSource) this.sseSource.close();
      var self = this;
      var url = OpenFangAPI.baseUrl + '/api/comms/events/stream';
      if (OpenFangAPI.apiKey) url += '?token=' + encodeURIComponent(OpenFangAPI.apiKey);
      this.sseSource = new EventSource(url);
      this.sseSource.onmessage = function(ev) {
        if (ev.data === 'ping') return;
        try {
          var event = JSON.parse(ev.data);
          self.events.unshift(event);
          if (self.events.length > 200) self.events.length = 200;
        } catch (e) { /* ignore */ }
      };
    },

    stopSSE() {
      if (this.sseSource) {
        this.sseSource.close();
        this.sseSource = null;
      }
    },

    cleanup() {
      this.stopSSE();
      this.stopPolling();
    },

    peerConnected(state) {
      return String(state || '').indexOf('Connected') !== -1;
    },

    /** Polar layout: center = this node; three sectors for OFP peers, A2A, channels */
    buildGraphLayout() {
      var cx = 250;
      var cy = 158;
      var r = 118;
      var sectors = 3;
      var sectorSpan = (2 * Math.PI) / sectors;

      function placeInSector(items, sectorIndex, maxN) {
        var list = items.slice(0, maxN);
        var n = list.length;
        var out = [];
        if (n === 0) return out;
        var start = -Math.PI / 2 + sectorIndex * sectorSpan;
        var pad = Math.min(sectorSpan * 0.12, 0.35);
        var usable = sectorSpan - 2 * pad;
        for (var i = 0; i < n; i++) {
          var t = n === 1 ? 0.5 : i / (n - 1);
          var ang = start + pad + t * usable;
          out.push({
            x: cx + r * Math.cos(ang),
            y: cy + r * Math.sin(ang),
            ang: ang
          });
        }
        return out;
      }

      var maxPeers = 8;
      var maxA2a = 6;
      var maxCh = 8;
      var peerSlice = this.peers.slice(0, maxPeers);
      var a2aSlice = this.a2aAgents.slice(0, maxA2a);
      var chSlice = this.channelNodes.slice(0, maxCh);

      var pos0 = placeInSector(peerSlice, 0, maxPeers);
      var pos1 = placeInSector(a2aSlice, 1, maxA2a);
      var pos2 = placeInSector(chSlice, 2, maxCh);

      var nodes = [];
      var edges = [];

      nodes.push({
        id: 'local',
        kind: 'local',
        x: cx,
        y: cy,
        r: 26,
        label: 'This node',
        sub: this.netStatus && this.netStatus.node_id
          ? String(this.netStatus.node_id).substring(0, 10) + '…'
          : 'ArmaraOS',
        extra: (this.localAgents && this.localAgents.length)
          ? this.localAgents.length + ' agent(s) locally'
          : ''
      });

      for (var pi = 0; pi < peerSlice.length; pi++) {
        var p = peerSlice[pi];
        var pp = pos0[pi];
        var ok = this.peerConnected(p.state);
        nodes.push({
          id: 'peer-' + p.node_id,
          kind: 'peer',
          x: pp.x,
          y: pp.y,
          r: ok ? 16 : 14,
          label: this.trunc(p.node_name, 14),
          sub: 'OFP',
          connected: ok
        });
        edges.push({
          id: 'e-peer-' + pi,
          x1: cx,
          y1: cy,
          x2: pp.x,
          y2: pp.y,
          active: ok
        });
      }

      for (var ai = 0; ai < a2aSlice.length; ai++) {
        var a = a2aSlice[ai];
        var ap = pos1[ai];
        nodes.push({
          id: 'a2a-' + ai,
          kind: 'a2a',
          x: ap.x,
          y: ap.y,
          r: 14,
          label: this.trunc(a.name || 'A2A', 12),
          sub: 'A2A'
        });
        edges.push({
          id: 'e-a2a-' + ai,
          x1: cx,
          y1: cy,
          x2: ap.x,
          y2: ap.y,
          active: true
        });
      }

      for (var ci = 0; ci < chSlice.length; ci++) {
        var c = chSlice[ci];
        var cp = pos2[ci];
        nodes.push({
          id: 'ch-' + c.name,
          kind: 'channel',
          x: cp.x,
          y: cp.y,
          r: 13,
          label: this.trunc(c.display_name || c.name, 12),
          sub: 'Channel'
        });
        edges.push({
          id: 'e-ch-' + ci,
          x1: cx,
          y1: cy,
          x2: cp.x,
          y2: cp.y,
          active: true
        });
      }

      return { cx: cx, cy: cy, nodes: nodes, edges: edges, overflow: {
        peers: Math.max(0, this.peers.length - maxPeers),
        a2a: Math.max(0, this.a2aAgents.length - maxA2a),
        channels: Math.max(0, this.channelNodes.length - maxCh)
      }};
    },

    trunc(s, n) {
      s = String(s || '');
      if (s.length <= n) return s;
      return s.substring(0, n - 1) + '…';
    },

    get filteredEventsList() {
      var f = this.trafficFilter;
      return this.events.filter(function(ev) {
        var k = ev.kind || '';
        if (f === 'all') return true;
        if (f === 'messages') return k === 'agent_message';
        if (f === 'tasks') {
          return k === 'task_posted' || k === 'task_claimed' || k === 'task_completed';
        }
        if (f === 'lifecycle') {
          return k === 'agent_spawned' || k === 'agent_terminated';
        }
        return true;
      });
    },

    eventBadgeClass(kind) {
      switch (kind) {
        case 'agent_message': return 'badge badge-info';
        case 'agent_spawned': return 'badge badge-success';
        case 'agent_terminated': return 'badge badge-danger';
        case 'task_posted': return 'badge badge-warning';
        case 'task_claimed': return 'badge badge-info';
        case 'task_completed': return 'badge badge-success';
        default: return 'badge badge-dim';
      }
    },

    eventLabel(kind) {
      switch (kind) {
        case 'agent_message': return 'Message';
        case 'agent_spawned': return 'Spawned';
        case 'agent_terminated': return 'Terminated';
        case 'task_posted': return 'Task';
        case 'task_claimed': return 'Claimed';
        case 'task_completed': return 'Done';
        default: return kind || 'event';
      }
    },

    timeAgo(dateStr) {
      if (!dateStr) return '';
      var d = new Date(dateStr);
      var secs = Math.floor((Date.now() - d.getTime()) / 1000);
      if (secs < 60) return secs + 's ago';
      if (secs < 3600) return Math.floor(secs / 60) + 'm ago';
      if (secs < 86400) return Math.floor(secs / 3600) + 'h ago';
      return Math.floor(secs / 86400) + 'd ago';
    }
  };
}
