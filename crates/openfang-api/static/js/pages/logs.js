// ArmaraOS Logs Page — Live audit (SSE + poll), daemon tracing file, Audit Trail
'use strict';

function logsPage() {
  return {
    tab: 'live',

    // -- Live (audit) logs --
    entries: [],
    levelFilter: '',
    textFilter: '',
    autoRefresh: true,
    hovering: false,
    loading: true,
    loadError: '',
    _pollTimer: null,
    _eventSource: null,
    streamConnected: false,
    streamPaused: false,

    // -- Daemon tracing log --
    daemonEntries: [],
    daemonPath: '',
    daemonLevelFilter: '',
    daemonTextFilter: '',
    hoveringDaemon: false,
    daemonLoading: false,
    daemonLoadError: '',
    streamConnectedDaemon: false,
    streamPausedDaemon: false,
    _daemonEventSource: null,
    _daemonPollTimer: null,
    savedLogLevel: 'info',
    logLevelDraft: 'info',

    // -- Audit trail --
    auditEntries: [],
    tipHash: '',
    chainValid: null,
    filterAction: '',
    auditLoading: false,
    auditLoadError: '',

    liveStreamQuery: function() {
      var q = [];
      if (this.levelFilter) q.push('level=' + encodeURIComponent(this.levelFilter));
      if (this.textFilter) q.push('filter=' + encodeURIComponent(this.textFilter));
      return q.length ? ('?' + q.join('&')) : '';
    },

    initPage: function() {
      var self = this;
      this.startStreaming();
      this.$watch('levelFilter', function() {
        if (self.tab === 'live') self.restartLiveStream();
      });
      this.$watch('textFilter', function() {
        if (self.tab === 'live') self.restartLiveStream();
      });
      this.$watch('daemonLevelFilter', function() {
        if (self.tab === 'daemon') self.restartDaemonStream();
      });
      this.$watch('daemonTextFilter', function() {
        if (self.tab === 'daemon') self.restartDaemonStream();
      });
    },

    onLogsTab: function(name) {
      this.tab = name;
      if (name === 'live') {
        this.stopDaemonStreaming();
        this.startStreaming();
      } else if (name === 'daemon') {
        if (this._eventSource) {
          this._eventSource.close();
          this._eventSource = null;
        }
        this.streamConnected = false;
        if (this._pollTimer) {
          clearInterval(this._pollTimer);
          this._pollTimer = null;
        }
        this.loadDaemonTab();
      } else {
        if (this._eventSource) {
          this._eventSource.close();
          this._eventSource = null;
        }
        this.streamConnected = false;
        if (this._pollTimer) {
          clearInterval(this._pollTimer);
          this._pollTimer = null;
        }
        this.stopDaemonStreaming();
        if (name === 'audit' && !this.auditEntries.length && !this.auditLoading) {
          this.loadAudit();
        }
      }
    },

    restartLiveStream: function() {
      if (this.tab !== 'live') return;
      this.entries = [];
      if (this._eventSource) {
        this._eventSource.close();
        this._eventSource = null;
      }
      if (this._pollTimer) {
        clearInterval(this._pollTimer);
        this._pollTimer = null;
      }
      this.startStreaming();
    },

    startStreaming: function() {
      var self = this;
      if (this._eventSource) {
        this._eventSource.close();
        this._eventSource = null;
      }

      var url = OpenFangAPI.sseUrl('/api/logs/stream' + this.liveStreamQuery());

      try {
        this._eventSource = new EventSource(url);
      } catch (e) {
        this.streamConnected = false;
        this.startPolling();
        return;
      }

      this._eventSource.onopen = function() {
        self.streamConnected = true;
        self.loading = false;
        self.loadError = '';
      };

      this._eventSource.onmessage = function(event) {
        if (self.streamPaused) return;
        try {
          var entry = JSON.parse(event.data);
          var dominated = false;
          for (var i = 0; i < self.entries.length; i++) {
            if (self.entries[i].seq === entry.seq) {
              dominated = true;
              break;
            }
          }
          if (!dominated) {
            self.entries.push(entry);
            if (self.entries.length > 500) {
              self.entries.splice(0, self.entries.length - 500);
            }
            if (self.autoRefresh && !self.hovering) {
              self.$nextTick(function() {
                var el = document.getElementById('log-container');
                if (el) el.scrollTop = el.scrollHeight;
              });
            }
          }
        } catch (err) {
          // ignore
        }
      };

      this._eventSource.onerror = function() {
        self.streamConnected = false;
        if (self._eventSource) {
          self._eventSource.close();
          self._eventSource = null;
        }
        self.startPolling();
      };
    },

    startPolling: function() {
      var self = this;
      this.streamConnected = false;
      this.fetchLogs();
      if (this._pollTimer) clearInterval(this._pollTimer);
      this._pollTimer = setInterval(function() {
        if (
          self.autoRefresh &&
          !self.hovering &&
          self.tab === 'live' &&
          !self.streamPaused
        ) {
          self.fetchLogs();
        }
      }, 2000);
    },

    async fetchLogs() {
      if (this.loading) this.loadError = '';
      try {
        var data = await OpenFangAPI.get('/api/audit/recent?n=200');
        this.entries = data.entries || [];
        if (this.autoRefresh && !this.hovering) {
          this.$nextTick(function() {
            var el = document.getElementById('log-container');
            if (el) el.scrollTop = el.scrollHeight;
          });
        }
        if (this.loading) this.loading = false;
      } catch (e) {
        if (this.loading) {
          this.loadError = e.message || 'Could not load logs.';
          this.loading = false;
        }
      }
    },

    async loadData() {
      this.loading = true;
      return this.fetchLogs();
    },

    togglePause: function() {
      this.streamPaused = !this.streamPaused;
      if (!this.streamPaused && this.streamConnected) {
        this.$nextTick(function() {
          var el = document.getElementById('log-container');
          if (el) el.scrollTop = el.scrollHeight;
        });
      }
    },

    clearLogs: function() {
      this.entries = [];
    },

    classifyLevel: function(action) {
      if (!action) return 'info';
      var a = action.toLowerCase();
      if (a.indexOf('error') !== -1 || a.indexOf('fail') !== -1 || a.indexOf('crash') !== -1) {
        return 'error';
      }
      if (a.indexOf('warn') !== -1 || a.indexOf('deny') !== -1 || a.indexOf('block') !== -1) {
        return 'warn';
      }
      return 'info';
    },

    get filteredEntries() {
      var self = this;
      var levelF = this.levelFilter;
      var textF = this.textFilter.toLowerCase();
      return this.entries.filter(function(e) {
        if (levelF && self.classifyLevel(e.action) !== levelF) return false;
        if (textF) {
          var haystack = (
            (e.action || '') +
            ' ' +
            (e.detail || '') +
            ' ' +
            (e.agent_id || '')
          ).toLowerCase();
          if (haystack.indexOf(textF) === -1) return false;
        }
        return true;
      });
    },

    get connectionLabel() {
      if (this.streamPaused) return 'Paused';
      if (this.streamConnected) return 'Live';
      if (this._pollTimer) return 'Polling';
      return 'Disconnected';
    },

    get connectionClass() {
      if (this.streamPaused) return 'paused';
      if (this.streamConnected) return 'live';
      if (this._pollTimer) return 'polling';
      return 'disconnected';
    },

    get headerConnectionLabel() {
      if (this.tab === 'daemon') {
        if (this.streamPausedDaemon) return 'Paused';
        if (this.streamConnectedDaemon) return 'Live';
        if (this._daemonPollTimer) return 'Polling';
        return 'Disconnected';
      }
      return this.connectionLabel;
    },

    get headerConnectionClass() {
      if (this.tab === 'daemon') {
        if (this.streamPausedDaemon) return 'paused';
        if (this.streamConnectedDaemon) return 'live';
        if (this._daemonPollTimer) return 'polling';
        return 'disconnected';
      }
      return this.connectionClass;
    },

    exportLogs: function() {
      var lines = this.filteredEntries.map(function(e) {
        return (
          new Date(e.timestamp).toISOString() + ' [' + e.action + '] ' + (e.detail || '')
        );
      });
      var blob = new Blob([lines.join('\n')], { type: 'text/plain' });
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = 'openfang-logs-' + new Date().toISOString().slice(0, 10) + '.txt';
      a.click();
      URL.revokeObjectURL(url);
    },

    stopDaemonStreaming: function() {
      if (this._daemonEventSource) {
        this._daemonEventSource.close();
        this._daemonEventSource = null;
      }
      if (this._daemonPollTimer) {
        clearInterval(this._daemonPollTimer);
        this._daemonPollTimer = null;
      }
      this.streamConnectedDaemon = false;
    },

    async loadDaemonTab() {
      this.daemonLoading = true;
      this.daemonLoadError = '';
      try {
        var st = await OpenFangAPI.get('/api/status');
        this.savedLogLevel = String(st.log_level || 'info').toLowerCase();
        this.logLevelDraft = this.savedLogLevel;
      } catch (e) {
        this.daemonLoadError = e.message || 'Could not load status.';
      }
      try {
        await this.fetchDaemonRecent();
      } catch (e2) {
        if (!this.daemonLoadError) {
          this.daemonLoadError = e2.message || 'Could not load daemon log.';
        }
      }
      this.daemonLoading = false;
      this.startDaemonStreaming();
    },

    daemonStreamQuery: function() {
      var q = [];
      if (this.daemonLevelFilter) {
        q.push('level=' + encodeURIComponent(this.daemonLevelFilter));
      }
      if (this.daemonTextFilter) {
        q.push('filter=' + encodeURIComponent(this.daemonTextFilter));
      }
      return q.length ? ('?' + q.join('&')) : '';
    },

    async fetchDaemonRecent() {
      var q = '/api/logs/daemon/recent?lines=200';
      if (this.daemonLevelFilter) {
        q += '&level=' + encodeURIComponent(this.daemonLevelFilter);
      }
      if (this.daemonTextFilter) {
        q += '&filter=' + encodeURIComponent(this.daemonTextFilter);
      }
      var data = await OpenFangAPI.get(q);
      this.daemonPath = data.path || '';
      this.daemonEntries = (data.lines || []).map(function(row, idx) {
        return { seq: row.seq != null ? row.seq : idx, line: row.line || '' };
      });
    },

    startDaemonStreaming: function() {
      var self = this;
      this.stopDaemonStreaming();
      var path = '/api/logs/daemon/stream' + this.daemonStreamQuery();
      var url = OpenFangAPI.sseUrl(path);
      try {
        this._daemonEventSource = new EventSource(url);
      } catch (e) {
        this.streamConnectedDaemon = false;
        this.startDaemonPolling();
        return;
      }
      this._daemonEventSource.onopen = function() {
        self.streamConnectedDaemon = true;
        self.daemonLoadError = '';
      };
      this._daemonEventSource.onmessage = function(event) {
        if (self.streamPausedDaemon) return;
        try {
          var row = JSON.parse(event.data);
          var line = row.line || '';
          self.daemonEntries.push({ seq: self.daemonEntries.length + 1, line: line });
          if (self.daemonEntries.length > 500) {
            self.daemonEntries.splice(0, self.daemonEntries.length - 500);
          }
          if (self.autoRefresh && !self.hoveringDaemon) {
            self.$nextTick(function() {
              var el = document.getElementById('daemon-log-container');
              if (el) el.scrollTop = el.scrollHeight;
            });
          }
        } catch (err) {
          // ignore
        }
      };
      this._daemonEventSource.onerror = function() {
        self.streamConnectedDaemon = false;
        if (self._daemonEventSource) {
          self._daemonEventSource.close();
          self._daemonEventSource = null;
        }
        self.startDaemonPolling();
      };
    },

    startDaemonPolling: function() {
      var self = this;
      if (this._daemonPollTimer) clearInterval(this._daemonPollTimer);
      this._daemonPollTimer = setInterval(function() {
        if (
          self.tab === 'daemon' &&
          self.autoRefresh &&
          !self.hoveringDaemon &&
          !self.streamPausedDaemon
        ) {
          self.fetchDaemonRecent().catch(function() {});
        }
      }, 2000);
    },

    restartDaemonStream: function() {
      if (this.tab !== 'daemon') return;
      this.daemonEntries = [];
      this.startDaemonStreaming();
    },

    toggleDaemonPause: function() {
      this.streamPausedDaemon = !this.streamPausedDaemon;
      if (!this.streamPausedDaemon && this.streamConnectedDaemon) {
        this.$nextTick(function() {
          var el = document.getElementById('daemon-log-container');
          if (el) el.scrollTop = el.scrollHeight;
        });
      }
    },

    clearDaemonLogs: function() {
      this.daemonEntries = [];
    },

    classifyDaemonLineLevel: function(line) {
      if (!line) return 'info';
      if (line.indexOf(' ERROR ') !== -1) return 'error';
      if (line.indexOf(' WARN ') !== -1) return 'warn';
      if (line.indexOf(' INFO ') !== -1) return 'info';
      if (line.indexOf('DEBUG') !== -1) return 'debug';
      if (line.indexOf('TRACE') !== -1) return 'trace';
      return 'info';
    },

    exportDaemonLogs: function() {
      var blob = new Blob([this.daemonEntries.map(function(e) { return e.line; }).join('\n')], {
        type: 'text/plain',
      });
      var url = URL.createObjectURL(blob);
      var a = document.createElement('a');
      a.href = url;
      a.download = 'armaraos-daemon-log-' + new Date().toISOString().slice(0, 10) + '.txt';
      a.click();
      URL.revokeObjectURL(url);
    },

    async saveLogLevel() {
      try {
        await OpenFangAPI.post('/api/config/set', {
          path: 'log_level',
          value: this.logLevelDraft,
        });
        this.savedLogLevel = String(this.logLevelDraft).toLowerCase();
        OpenFangToast.success(
          'Saved. Restart the daemon for tracing to use the new log level.',
        );
      } catch (e) {
        OpenFangToast.error(e.message || 'Could not save log level.');
      }
    },

    get filteredAuditEntries() {
      var self = this;
      if (!self.filterAction) return self.auditEntries;
      return self.auditEntries.filter(function(e) {
        return e.action === self.filterAction;
      });
    },

    async loadAudit() {
      this.auditLoading = true;
      this.auditLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/audit/recent?n=200');
        this.auditEntries = data.entries || [];
        this.tipHash = data.tip_hash || '';
      } catch (e) {
        this.auditEntries = [];
        this.auditLoadError = e.message || 'Could not load audit log.';
      }
      this.auditLoading = false;
    },

    auditAgentName: function(agentId) {
      if (!agentId) return '-';
      var agents = Alpine.store('app').agents || [];
      var agent = agents.find(function(a) {
        return a.id === agentId;
      });
      return agent ? agent.name : agentId.substring(0, 8) + '...';
    },

    friendlyAction: function(action) {
      if (!action) return 'Unknown';
      var map = {
        AgentSpawn: 'Agent Created',
        AgentKill: 'Agent Stopped',
        AgentTerminated: 'Agent Stopped',
        ToolInvoke: 'Tool Used',
        ToolResult: 'Tool Completed',
        AgentMessage: 'Message',
        NetworkAccess: 'Network Access',
        ShellExec: 'Shell Command',
        FileAccess: 'File Access',
        MemoryAccess: 'Memory Access',
        AuthAttempt: 'Login Attempt',
        AuthSuccess: 'Login Success',
        AuthFailure: 'Login Failed',
        CapabilityDenied: 'Permission Denied',
        RateLimited: 'Rate Limited',
      };
      return map[action] || action.replace(/([A-Z])/g, ' $1').trim();
    },

    async verifyChain() {
      try {
        var data = await OpenFangAPI.get('/api/audit/verify');
        this.chainValid = data.valid === true;
        if (this.chainValid) {
          OpenFangToast.success(
            'Audit chain verified — ' + (data.entries || 0) + ' entries valid',
          );
        } else {
          OpenFangToast.error('Audit chain broken!');
        }
      } catch (e) {
        this.chainValid = false;
        OpenFangToast.error('Chain verification failed: ' + e.message);
      }
    },

    destroy: function() {
      if (this._eventSource) {
        this._eventSource.close();
        this._eventSource = null;
      }
      if (this._pollTimer) {
        clearInterval(this._pollTimer);
        this._pollTimer = null;
      }
      this.stopDaemonStreaming();
    },
  };
}
