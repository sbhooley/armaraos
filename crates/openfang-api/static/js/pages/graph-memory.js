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

    async init() {
      await this.loadAgents();
      await this.fetchGraph();

      var self = this;
      window.addEventListener('armaraos-kernel-event', function (e) {
        if (self.isGraphMemoryKernelEvent(e.detail)) {
          self.fetchGraph();
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
        this.agents = list
          .map(function (a) {
            var id = String(a.id != null ? a.id : a.agent_id || '').trim();
            if (!id) {
              return null;
            }
            var nm = (a.name && String(a.name).trim()) || 'Agent';
            var short = id.length > 10 ? id.slice(0, 8) + '…' : id;
            return { id: id, label: nm + ' — ' + short };
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
      await this.fetchGraph();
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
        await this.$nextTick();
        this.renderGraph();
      } catch (e) {
        console.error('graph-memory fetch:', e);
      } finally {
        this.loading = false;
      }
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
