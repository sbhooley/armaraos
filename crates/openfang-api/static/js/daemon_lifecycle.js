// Shared Settings + Runtime: reload config/channels/integrations, graceful shutdown
'use strict';

function armaraosDaemonLifecycleControls() {
  return {
    daemonCtlLoading: {
      config: false,
      channels: false,
      integrations: false,
      shutdown: false
    },

    refreshAfterDaemonActions: async function() {
      var keys = ['loadSysInfo', 'loadConfig', 'loadData'];
      for (var i = 0; i < keys.length; i++) {
        var fn = this[keys[i]];
        if (typeof fn === 'function') {
          try {
            await fn.call(this);
          } catch (e) { /* ignore */ }
        }
      }
    },

    daemonPromptReloadConfig: function() {
      var self = this;
      OpenFangToast.confirm(
        'Reload configuration?',
        'Reads config.toml from disk and applies hot-reloadable changes. Some options still require a full daemon restart.',
        function() {
          self.daemonExecuteReloadConfig();
        },
        { confirmLabel: 'Reload', danger: false }
      );
    },

    async daemonExecuteReloadConfig() {
      if (this.daemonCtlLoading.config) return;
      this.daemonCtlLoading.config = true;
      try {
        var j = await OpenFangAPI.post('/api/config/reload', {});
        if (j.status === 'no_changes') {
          OpenFangToast.info('No configuration changes on disk');
        } else if (j.restart_required) {
          OpenFangToast.warn(
            'Configuration partially applied — a full daemon restart is required for some changes.',
            8000
          );
        } else {
          OpenFangToast.success('Configuration reloaded');
        }
        await this.refreshAfterDaemonActions();
      } catch (e) {
        OpenFangToast.error(e.message || String(e));
      }
      this.daemonCtlLoading.config = false;
    },

    daemonPromptReloadChannels: function() {
      var self = this;
      OpenFangToast.confirm(
        'Reload channels?',
        'Reconnects messaging bridges from disk (e.g. WhatsApp, Discord). Brief disruption is possible.',
        function() {
          self.daemonExecuteReloadChannels();
        },
        { confirmLabel: 'Reload channels', danger: false }
      );
    },

    async daemonExecuteReloadChannels() {
      if (this.daemonCtlLoading.channels) return;
      this.daemonCtlLoading.channels = true;
      try {
        var j = await OpenFangAPI.post('/api/channels/reload', {});
        var n = Array.isArray(j.started) ? j.started.length : 0;
        var names = Array.isArray(j.started) && j.started.length ? j.started.join(', ') : '';
        if (names.length > 72) names = names.slice(0, 69) + '…';
        var msg = n ? 'Channels reloaded (' + n + ')' + (names ? ': ' + names : '') : 'Channels reloaded';
        OpenFangToast.success(msg, n ? 6000 : 4000);
        await this.refreshAfterDaemonActions();
      } catch (e) {
        OpenFangToast.error(e.message || String(e));
      }
      this.daemonCtlLoading.channels = false;
    },

    daemonPromptReloadIntegrations: function() {
      var self = this;
      OpenFangToast.confirm(
        'Reload integrations?',
        'Reconnects MCP and extension integrations from config.',
        function() {
          self.daemonExecuteReloadIntegrations();
        },
        { confirmLabel: 'Reload', danger: false }
      );
    },

    async daemonExecuteReloadIntegrations() {
      if (this.daemonCtlLoading.integrations) return;
      this.daemonCtlLoading.integrations = true;
      try {
        var j = await OpenFangAPI.post('/api/integrations/reload', {});
        var c = j.new_connections != null ? j.new_connections : '';
        OpenFangToast.success(
          c !== '' ? 'Integrations reloaded (connections: ' + c + ')' : 'Integrations reloaded'
        );
        await this.refreshAfterDaemonActions();
      } catch (e) {
        OpenFangToast.error(e.message || String(e));
      }
      this.daemonCtlLoading.integrations = false;
    },

    daemonPromptShutdown: function() {
      var self = this;
      OpenFangToast.confirm(
        'Shut down daemon?',
        'Stops the API and kernel gracefully. This dashboard will disconnect. Use the desktop app, openfang start, or your process supervisor to run the daemon again.',
        function() {
          self.daemonExecuteShutdown();
        },
        { confirmLabel: 'Shut down', danger: true }
      );
    },

    async daemonExecuteShutdown() {
      if (this.daemonCtlLoading.shutdown) return;
      this.daemonCtlLoading.shutdown = true;
      OpenFangToast.info('Shutting down…', 5000);
      try {
        await OpenFangAPI.post('/api/shutdown', {});
        OpenFangToast.success('Shutdown acknowledged', 4000);
      } catch (e) {
        var st = e.status;
        if (st === 0 || st === 502 || st === 503) {
          OpenFangToast.info('Daemon stopped or connection closed.', 5000);
        } else {
          OpenFangToast.error(e.message || String(e));
        }
      }
      this.daemonCtlLoading.shutdown = false;
    }
  };
}
