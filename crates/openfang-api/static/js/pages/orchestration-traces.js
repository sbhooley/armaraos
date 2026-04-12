// Orchestration traces — multi-agent delegation observability
(function () {
  function shortId(id) {
    if (!id) return '—';
    var s = String(id);
    return s.length > 10 ? s.slice(0, 8) + '…' : s;
  }

  function eventTypeName(et) {
    if (!et || typeof et !== 'object') return '';
    return et.type || '';
  }

  function agentHue(agentId) {
    var s = String(agentId || '');
    var h = 0;
    for (var i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) % 360;
    return h;
  }

  /**
   * @param {unknown} node
   * @param {number} depth
   * @returns {Array<{ agentId: string, depth: number, label: string, hasChildren: boolean }>}
   */
  function flattenDelegationTree(node, depth) {
    depth = depth || 0;
    if (!node || typeof node !== 'object') return [];
    var aid = node.agent_id != null ? String(node.agent_id) : '';
    var rows = [
      {
        agentId: aid,
        depth: depth,
        label: shortId(aid),
        hasChildren: Array.isArray(node.children) && node.children.length > 0,
      },
    ];
    var ch = node.children || [];
    for (var i = 0; i < ch.length; i++) {
      rows = rows.concat(flattenDelegationTree(ch[i], depth + 1));
    }
    return rows;
  }

  /**
   * @param {unknown} node
   * @param {number} depth
   * @returns {Array<{ agentId: string, depth: number, label: string, displayName: string, usedPct: number, used: number, max: number, inherits: boolean }>}
   */
  function flattenQuotaTree(node, depth) {
    depth = depth || 0;
    if (!node || typeof node !== 'object') return [];
    var q = node.quota || {};
    var max = Number(q.max_llm_tokens_per_hour) || 0;
    var used = Number(q.used_llm_tokens) || 0;
    var cap = max > 0 ? max : 1;
    var pct = Math.min(100, (used / cap) * 100);
    var name = node.name != null ? String(node.name) : '';
    var aid = node.agent_id != null ? String(node.agent_id) : '';
    var rows = [
      {
        agentId: aid,
        depth: depth,
        label: shortId(aid),
        displayName: name,
        usedPct: pct,
        used: used,
        max: max,
        inherits: !!q.inherits_parent,
      },
    ];
    var ch = node.children || [];
    for (var i = 0; i < ch.length; i++) {
      rows = rows.concat(flattenQuotaTree(ch[i], depth + 1));
    }
    return rows;
  }

  /**
   * Render interactive D3 force-directed graph from delegation tree.
   * @param {string} containerId - DOM element ID for the SVG container
   * @param {unknown} tree - Delegation tree object with agent_id and children
   */
  function renderD3DelegationGraph(containerId, tree) {
    var container = document.getElementById(containerId);
    if (!container || !tree) return;

    // Clear previous graph
    container.innerHTML = '';

    // Build nodes and links from tree
    var nodes = [];
    var links = [];
    var nodeMap = new Map();

    function traverse(node, parent) {
      if (!node || typeof node !== 'object') return;
      var id = String(node.agent_id || '');
      if (!id) return;

      var n = { id: id, label: shortId(id), hue: agentHue(id) };
      nodes.push(n);
      nodeMap.set(id, n);

      if (parent) {
        links.push({ source: parent.id, target: id });
      }

      var children = node.children || [];
      for (var i = 0; i < children.length; i++) {
        traverse(children[i], n);
      }
    }
    traverse(tree, null);

    if (nodes.length === 0) return;

    // SVG dimensions
    var width = container.clientWidth || 600;
    var height = Math.max(400, nodes.length * 40);

    var svg = d3
      .select(container)
      .append('svg')
      .attr('width', width)
      .attr('height', height)
      .attr('viewBox', [0, 0, width, height])
      .style('max-width', '100%')
      .style('height', 'auto');

    // Create force simulation
    var simulation = d3
      .forceSimulation(nodes)
      .force(
        'link',
        d3
          .forceLink(links)
          .id(function (d) {
            return d.id;
          })
          .distance(80)
      )
      .force('charge', d3.forceManyBody().strength(-200))
      .force('center', d3.forceCenter(width / 2, height / 2))
      .force('collision', d3.forceCollide().radius(30));

    // Add links (edges)
    var link = svg
      .append('g')
      .attr('class', 'd3-links')
      .selectAll('line')
      .data(links)
      .join('line')
      .attr('stroke', '#555')
      .attr('stroke-width', 1.5)
      .attr('stroke-opacity', 0.6);

    // Add nodes
    var node = svg
      .append('g')
      .attr('class', 'd3-nodes')
      .selectAll('g')
      .data(nodes)
      .join('g')
      .call(
        d3
          .drag()
          .on('start', dragstarted)
          .on('drag', dragged)
          .on('end', dragended)
      );

    // Node circles
    node
      .append('circle')
      .attr('r', 20)
      .attr('fill', function (d) {
        return 'hsl(' + d.hue + ', 70%, 50%)';
      })
      .attr('stroke', '#fff')
      .attr('stroke-width', 2)
      .style('cursor', 'pointer')
      .append('title')
      .text(function (d) {
        return d.id;
      });

    // Node labels
    node
      .append('text')
      .text(function (d) {
        return d.label;
      })
      .attr('x', 0)
      .attr('y', 5)
      .attr('text-anchor', 'middle')
      .attr('fill', '#fff')
      .style('font-size', '10px')
      .style('font-family', 'monospace')
      .style('pointer-events', 'none')
      .style('user-select', 'none');

    // Click to copy agent ID
    node.on('click', function (event, d) {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(d.id).then(function () {
          if (typeof OpenFangToast !== 'undefined') {
            OpenFangToast.show('Copied ' + d.label + ' to clipboard', 'success');
          }
        });
      }
    });

    // Update positions on simulation tick
    simulation.on('tick', function () {
      link
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

      node.attr('transform', function (d) {
        return 'translate(' + d.x + ',' + d.y + ')';
      });
    });

    // Drag functions
    function dragstarted(event) {
      if (!event.active) simulation.alphaTarget(0.3).restart();
      event.subject.fx = event.subject.x;
      event.subject.fy = event.subject.y;
    }

    function dragged(event) {
      event.subject.fx = event.x;
      event.subject.fy = event.y;
    }

    function dragended(event) {
      if (!event.active) simulation.alphaTarget(0);
      event.subject.fx = null;
      event.subject.fy = null;
    }
  }

  /**
   * @param {unknown} eventsJson
   * @returns {{ t0: number, t1: number, rows: Array<{ agentId: string, label: string, segments: Array<{ leftPct: number, widthPct: number, kind: string, title: string }> }> } | null}
   */
  function buildGantt(eventsJson) {
    var raw = eventsJson;
    if (!raw) return null;
    var events = Array.isArray(raw) ? raw.slice() : null;
    if (!events || events.length === 0) return null;

    events.sort(function (a, b) {
      return String(a.timestamp || '').localeCompare(String(b.timestamp || ''));
    });

    var times = [];
    for (var i = 0; i < events.length; i++) {
      var ts = Date.parse(events[i].timestamp || '');
      if (Number.isFinite(ts)) times.push(ts);
    }
    if (times.length === 0) return null;
    var t0 = Math.min.apply(null, times);
    var t1 = Math.max.apply(null, times);
    if (t1 <= t0) t1 = t0 + 1000;
    var span = t1 - t0;

    var agents = {};
    function addAgent(id) {
      if (!id) return;
      var k = String(id);
      agents[k] = true;
    }

    for (var j = 0; j < events.length; j++) {
      var ev = events[j];
      addAgent(ev.agent_id);
      addAgent(ev.orchestrator_id);
      var et = ev.event_type;
      var tn = eventTypeName(et);
      if (tn === 'agent_delegated' && et && et.target_agent) addAgent(et.target_agent);
    }

    var agentList = Object.keys(agents).sort();
    if (agentList.length === 0) return null;

    var rows = [];
    for (var r = 0; r < agentList.length; r++) {
      var aid = agentList[r];
      rows.push({ agentId: aid, label: shortId(aid), segments: [] });
    }
    var rowMap = {};
    for (var rr = 0; rr < rows.length; rr++) rowMap[rows[rr].agentId] = rows[rr];

    function placeSegment(agentId, leftPct, widthPct, kind, title) {
      var row = rowMap[String(agentId)];
      if (!row || widthPct <= 0) return;
      leftPct = Math.max(0, Math.min(100 - widthPct, leftPct));
      row.segments.push({
        leftPct: leftPct,
        widthPct: Math.max(widthPct, 0.35),
        kind: kind,
        title: title || '',
      });
    }

    for (var k = 0; k < events.length; k++) {
      var e = events[k];
      var t = Date.parse(e.timestamp || '');
      if (!Number.isFinite(t)) continue;
      var typ = eventTypeName(e.event_type);

      if (typ === 'agent_completed' && e.event_type) {
        var dur = Number(e.event_type.duration_ms) || 0;
        var start = t - dur;
        var left = ((start - t0) / span) * 100;
        var w = (dur / span) * 100;
        if (w < 0.2) w = 0.2;
        placeSegment(
          e.agent_id,
          left,
          w,
          'completed',
          'completed · ' +
            dur +
            ' ms · tokens ' +
            (e.event_type.tokens_in || 0) +
            '/' +
            (e.event_type.tokens_out || 0)
        );
      } else if (typ === 'agent_failed') {
        var lx = ((t - t0) / span) * 100;
        placeSegment(
          e.agent_id,
          lx,
          0.5,
          'failed',
          e.event_type && e.event_type.error ? String(e.event_type.error).slice(0, 80) : 'failed'
        );
      } else if (typ === 'agent_delegated' && e.event_type && e.event_type.target_agent) {
        var lx2 = ((t - t0) / span) * 100;
        placeSegment(e.agent_id, lx2, 0.45, 'delegate', 'delegated');
        placeSegment(e.event_type.target_agent, lx2 + 0.1, 0.45, 'delegate-target', 'received delegation');
      } else {
        var lx3 = ((t - t0) / span) * 100;
        placeSegment(e.agent_id, lx3, 0.4, 'marker', typ || 'event');
      }
    }

    return {
      t0: t0,
      t1: t1,
      spanMs: span,
      rows: rows.filter(function (row) {
        return row.segments.length > 0;
      }),
    };
  }

  /**
   * @param {unknown} costJson
   * @returns {{ maxTok: number, rows: Array<{ label: string, agentId: string, inPct: number, outPct: number, tokensIn: number, tokensOut: number }> } | null}
   */
  function buildHeatmap(costJson) {
    if (!costJson || typeof costJson !== 'object') return null;
    var by = costJson.by_agent;
    if (!Array.isArray(by) || by.length === 0) return null;
    var maxTok = 0;
    for (var i = 0; i < by.length; i++) {
      var line = by[i];
      var sum = (Number(line.tokens_in) || 0) + (Number(line.tokens_out) || 0);
      if (sum > maxTok) maxTok = sum;
    }
    if (maxTok <= 0) maxTok = 1;
    var rows = [];
    for (var j = 0; j < by.length; j++) {
      var L = by[j];
      var tin = Number(L.tokens_in) || 0;
      var tout = Number(L.tokens_out) || 0;
      rows.push({
        label: shortId(L.agent_id),
        agentId: String(L.agent_id || ''),
        tokensIn: tin,
        tokensOut: tout,
        inPct: (tin / maxTok) * 100,
        outPct: (tout / maxTok) * 100,
      });
    }
    return { maxTok: maxTok, rows: rows };
  }

  document.addEventListener('alpine:init', function () {
    Alpine.data('orchestrationTracesPage', function () {
      return {
        agentHue: agentHue,
        loading: false,
        summaries: [],
        traceListFilter: '',
        traceDateFrom: '',
        traceDateTo: '',
        selectedTraceId: null,
        detailEvents: null,
        detailTree: null,
        detailCost: null,
        detailError: null,
        agentIdForQuota: '',
        eventTypeFilter: '',
        ganttData: null,
        heatmapData: null,
        delegationGraphRows: [],
        quotaChartRows: [],

        refreshViz() {
          this.ganttData = buildGantt(this.eventsMatchingFilter());
          this.heatmapData = buildHeatmap(this.detailCost);
          if (this.selectedTraceId && this.detailTree) {
            this.delegationGraphRows = flattenDelegationTree(this.detailTree, 0);
            // Render D3 graph (use setTimeout to ensure DOM is ready)
            var tree = this.detailTree;
            setTimeout(function () {
              renderD3DelegationGraph('d3-delegation-graph-svg', tree);
            }, 50);
          } else {
            this.delegationGraphRows = [];
          }
        },

        eventsMatchingFilter() {
          var ev = this.detailEvents;
          if (!ev || !Array.isArray(ev)) return null;
          var f = (this.eventTypeFilter || '').trim().toLowerCase();
          if (!f) return ev;
          var parts = f
            .split(',')
            .map(function (s) {
              return s.trim().replace(/\s+/g, '_');
            })
            .filter(Boolean);
          if (parts.length === 0) return ev;
          return ev.filter(function (e) {
            var name = eventTypeName(e.event_type);
            for (var i = 0; i < parts.length; i++) {
              if (name.indexOf(parts[i]) !== -1 || name === parts[i]) return true;
            }
            return false;
          });
        },

        filteredSummaries() {
          var list = this.summaries || [];
          var q = (this.traceListFilter || '').trim().toLowerCase();
          if (q) {
            list = list.filter(function (s) {
              return String(s.trace_id || '')
                .toLowerCase()
                .includes(q);
            });
          }
          var from = this.traceDateFrom;
          var to = this.traceDateTo;
          if (!from && !to) return list;
          return list.filter(function (s) {
            var t = Date.parse(s.last_event_at || '');
            if (!Number.isFinite(t)) return true;
            if (from) {
              var tf = new Date(from).getTime();
              if (Number.isFinite(tf) && t < tf) return false;
            }
            if (to) {
              var tt = new Date(to).getTime();
              if (Number.isFinite(tt) && t > tt) return false;
            }
            return true;
          });
        },

        clearDateFilters() {
          this.traceDateFrom = '';
          this.traceDateTo = '';
        },

        copyAgentId(id) {
          if (!id || !navigator.clipboard) return;
          navigator.clipboard.writeText(id).then(
            function () {
              OpenFangToast && OpenFangToast.success('Copied agent id');
            },
            function () {}
          );
        },

        async loadSummaries() {
          this.loading = true;
          this.detailError = null;
          try {
            var data = await OpenFangAPI.get('/api/orchestration/traces?limit=80');
            this.summaries = Array.isArray(data) ? data : [];
          } catch (e) {
            this.summaries = [];
            this.detailError = openFangErrText(e) || String(e);
            OpenFangToast && OpenFangToast.error(this.detailError);
          } finally {
            this.loading = false;
          }
        },

        async selectTrace(id) {
          if (!id) return;
          this.selectedTraceId = id;
          this.detailEvents = null;
          this.detailTree = null;
          this.detailCost = null;
          this.detailError = null;
          this.ganttData = null;
          this.heatmapData = null;
          this.delegationGraphRows = [];
          this.quotaChartRows = [];
          this.eventTypeFilter = '';
          try {
            var enc = encodeURIComponent(id);
            var ev = await OpenFangAPI.get('/api/orchestration/traces/' + enc);
            var tree = await OpenFangAPI.get('/api/orchestration/traces/' + enc + '/tree');
            var cost = await OpenFangAPI.get('/api/orchestration/traces/' + enc + '/cost');
            this.detailEvents = ev;
            this.detailTree = tree;
            this.detailCost = cost;
            this.refreshViz();
          } catch (e) {
            this.detailError = openFangErrText(e) || String(e);
            OpenFangToast && OpenFangToast.error(this.detailError);
          }
        },

        async loadQuotaTree() {
          var raw = (this.agentIdForQuota || '').trim();
          if (!raw) {
            OpenFangToast && OpenFangToast.warn('Enter an agent id or name');
            return;
          }
          this.detailError = null;
          try {
            var enc = encodeURIComponent(raw);
            var q = await OpenFangAPI.get('/api/orchestration/quota-tree/' + enc);
            this.detailTree = q;
            this.selectedTraceId = null;
            this.detailEvents = null;
            this.detailCost = null;
            this.ganttData = null;
            this.heatmapData = null;
            this.delegationGraphRows = [];
            this.quotaChartRows = flattenQuotaTree(q, 0);
          } catch (e) {
            this.detailError = openFangErrText(e) || String(e);
            OpenFangToast && OpenFangToast.error(this.detailError);
          }
        },

        eventTypeLabel(ev) {
          if (!ev || !ev.event_type) return '';
          var t = ev.event_type;
          if (typeof t === 'object' && t.type) return t.type;
          return String(t);
        },

        eventsJsonForDisplay() {
          var fe = this.eventsMatchingFilter();
          if (!fe) return '[]';
          try {
            return JSON.stringify(fe, null, 2);
          } catch (e) {
            return '';
          }
        },

        formatGanttRange() {
          var g = this.ganttData;
          if (!g || !Number.isFinite(g.t0)) return '';
          try {
            return (
              new Date(g.t0).toISOString().slice(11, 23) +
              ' → ' +
              new Date(g.t1).toISOString().slice(11, 23) +
              ' UTC · ' +
              Math.round(g.spanMs || 0) +
              ' ms'
            );
          } catch (e) {
            return '';
          }
        },

        init() {
          this.loadSummaries();
        },
      };
    });
  });
})();
