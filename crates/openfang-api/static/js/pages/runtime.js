// Runtime page — system overview and provider status
document.addEventListener('alpine:init', function() {
  Alpine.data('runtimePage', function() {
    return Object.assign(armaraosDaemonLifecycleControls(), {
      loading: true,
      uptime: '-',
      agentCount: 0,
      version: '-',
      defaultModel: '-',
      platform: '-',
      arch: '-',
      apiListen: '-',
      homeDir: '-',
      logLevel: '-',
      networkEnabled: false,
      providers: [],
      updateChecking: false,
      updateInfo: null,
      updaterPrefs: null,
      daemonUpdateChecking: false,
      daemonUpdateInfo: null,

      get isDesktopShell() {
        try {
          var w = typeof window !== 'undefined' ? window : null;
          var core = w && w.__TAURI__ && w.__TAURI__.core;
          return !!(core && typeof core.invoke === 'function');
        } catch (e) {
          return false;
        }
      },

      semverCompare(a, b) {
        var pa = String(a).split('.').map(function(x) { return parseInt(x, 10) || 0; });
        var pb = String(b).split('.').map(function(x) { return parseInt(x, 10) || 0; });
        for (var i = 0; i < Math.max(pa.length, pb.length); i++) {
          var da = pa[i] || 0;
          var db = pb[i] || 0;
          if (da > db) return 1;
          if (da < db) return -1;
        }
        return 0;
      },

      async loadUpdaterPrefs() {
        if (!this.isDesktopShell) return;
        try {
          var p = await ArmaraosDesktopTauriInvoke('get_desktop_updater_prefs');
          this.updaterPrefs = p || null;
        } catch(e) {
          this.updaterPrefs = null;
        }
      },

      async saveReleaseChannel() {
        if (!this.isDesktopShell || !this.updaterPrefs) return;
        try {
          await ArmaraosDesktopTauriInvoke('set_release_channel', { channel: this.updaterPrefs.release_channel || 'stable' });
          await this.loadUpdaterPrefs();
          OpenFangToast && OpenFangToast.success('Channel saved');
        } catch(e) {
          OpenFangToast && OpenFangToast.error(e.message || String(e));
        }
      },

      async checkDaemonRuntimeUpdate() {
        this.daemonUpdateChecking = true;
        this.daemonUpdateInfo = null;
        var err = null;
        try {
          var ver = await OpenFangAPI.get('/api/version');
          var current = ver.version || '';
          var rel = await OpenFangAPI.get('/api/version/github-latest');
          var tag = String(rel.tag_name || '').replace(/^v/i, '');
          var cmp = this.semverCompare(current, tag);
          this.daemonUpdateInfo = {
            current: current,
            latest: tag,
            url: rel.html_url || 'https://github.com/sbhooley/armaraos/releases',
            upToDate: cmp >= 0
          };
        } catch(e) {
          err = e.message || String(e);
          this.daemonUpdateInfo = { error: err };
        }
        if (this.isDesktopShell) {
          try {
            await ArmaraosDesktopTauriInvoke('report_daemon_update_check', { error: err });
            await this.loadUpdaterPrefs();
          } catch(e2) {}
        }
        this.daemonUpdateChecking = false;
      },

      async loadData() {
        this.loading = true;
        try {
          var results = await Promise.all([
            OpenFangAPI.get('/api/status'),
            OpenFangAPI.get('/api/version'),
            OpenFangAPI.get('/api/providers'),
            OpenFangAPI.get('/api/agents'),
            this.loadUpdaterPrefs()
          ]);
          var status = results[0];
          var ver = results[1];
          var prov = results[2];
          var agents = results[3];

          this.version = ver.version || '-';
          this.platform = ver.platform || '-';
          this.arch = ver.arch || '-';
          this.agentCount = Array.isArray(agents) ? agents.length : 0;
          this.defaultModel = status.default_model || '-';
          this.apiListen = status.api_listen || status.listen || '-';
          this.homeDir = status.home_dir || '-';
          this.logLevel = status.log_level || '-';
          this.networkEnabled = !!status.network_enabled;

          // Compute uptime from uptime_seconds
          var diff = status.uptime_seconds || 0;
          if (diff < 60) this.uptime = diff + 's';
          else if (diff < 3600) this.uptime = Math.floor(diff / 60) + 'm ' + (diff % 60) + 's';
          else if (diff < 86400) this.uptime = Math.floor(diff / 3600) + 'h ' + Math.floor((diff % 3600) / 60) + 'm';
          else this.uptime = Math.floor(diff / 86400) + 'd ' + Math.floor((diff % 86400) / 3600) + 'h';

          this.providers = (prov.providers || []).filter(function(p) {
            return p.auth_status === 'Configured' || p.reachable || p.is_local;
          });
        } catch(e) {
          console.error('Runtime load error:', e);
        }
        this.loading = false;
      }
      ,

      async checkForDesktopUpdates() {
        if (!this.isDesktopShell) {
          OpenFangToast && OpenFangToast.warn('Desktop update checks require the desktop app.');
          return;
        }
        this.updateChecking = true;
        this.updateInfo = null;
        try {
          var info = await ArmaraosDesktopTauriInvoke('check_for_updates');
          this.updateInfo = info;
          await this.loadUpdaterPrefs();
          if (!info) {
            OpenFangToast && OpenFangToast.error('Update check returned no data');
          } else if (info.available && info.installable) {
            var v = (info.version || 'unknown');
            var ok = confirm('ArmaraOS v' + v + ' is available. Install now? The app will restart.');
            if (ok) {
              OpenFangToast && OpenFangToast.info('Installing update…');
              await ArmaraosDesktopTauriInvoke('install_update');
            }
          } else if (info.available && !info.installable) {
            var v2 = (info.version || 'unknown');
            var u = info.download_url || 'https://github.com/sbhooley/armaraos/releases';
            OpenFangToast && OpenFangToast.info('Update available (v' + v2 + '). Opening download page…', 7000);
            try {
              await ArmaraosDesktopTauriInvoke('open_external_url', { url: u });
            } catch (e) {
              window.open(u, '_blank', 'noopener,noreferrer');
            }
          } else {
            OpenFangToast && OpenFangToast.success('Up to date');
          }
        } catch (e) {
          OpenFangToast && OpenFangToast.error(e.message || String(e));
          await this.loadUpdaterPrefs();
        }
        this.updateChecking = false;
      }
    });
  });
});
