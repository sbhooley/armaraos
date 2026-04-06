// App Store (route id: ainl-library) — browse free AINL programs from synced library; paid catalog placeholder
'use strict';

function ainlLibraryPage() {
  return {
    loading: true,
    loadError: '',
    root: '',
    total: 0,
    libraryPresent: false,
    categories: [],
    programs: [],
    curated: [],
    sync: null,
    search: '',
    /** 'free' | 'paid' — paid is a coming-soon placeholder */
    storeTab: 'free',
    /** Collapsible sections: omitted or true = expanded; false = collapsed */
    expandedSections: {},
    loadHints: false,
    strictValidation: true,
    hintsTruncated: false,
    maxHintsApplied: null,
    expanded: {},
    tryBusy: null,
    outputModal: null,

    isSectionOpen(key) {
      return this.expandedSections[key] !== false;
    },

    toggleSection(key) {
      var cur = this.expandedSections[key];
      var open = cur !== false;
      this.expandedSections = Object.assign({}, this.expandedSections, { [key]: !open });
    },

    sectionCategoryKey(cat) {
      if (!cat) return 'cat-unknown';
      var id = cat.id != null ? String(cat.id) : String(cat.label || 'unknown');
      return 'cat-' + id.replace(/[^a-zA-Z0-9_-]/g, '_');
    },

    get isDesktopShell() {
      try {
        return !!(typeof window !== 'undefined' && window.__TAURI__ && window.__TAURI__.core);
      } catch (e) {
        return false;
      }
    },

    get ainlDesktop() {
      try {
        return Alpine.store('ainl').desktop;
      } catch (e) {
        return null;
      }
    },

    filteredCategories() {
      var q = (this.search || '').trim().toLowerCase();
      if (!q) return this.categories;
      return this.categories.map(function(cat) {
        var progs = (cat.programs || []).filter(function(p) {
          var path = (p.path || '').toLowerCase();
          var name = (p.name || '').toLowerCase();
          var hint = (p.hint || '').toLowerCase();
          return path.indexOf(q) >= 0 || name.indexOf(q) >= 0 || hint.indexOf(q) >= 0;
        });
        return { id: cat.id, label: cat.label, programs: progs };
      }).filter(function(c) { return c.programs.length > 0; });
    },

    get filteredCurated() {
      var q = (this.search || '').trim().toLowerCase();
      var list = Array.isArray(this.curated) ? this.curated : [];
      if (!q) return list;
      return list.filter(function(c) {
        var path = (c.relative_path || '').toLowerCase();
        var name = (c.name || '').toLowerCase();
        return path.indexOf(q) >= 0 || name.indexOf(q) >= 0;
      });
    },

    totalFilteredPrograms() {
      var n = 0;
      this.filteredCategories().forEach(function(c) {
        n += (c.programs || []).length;
      });
      return n;
    },

    async loadLibrary() {
      this.loading = true;
      this.loadError = '';
      try {
        var url = '/api/ainl/library';
        if (this.loadHints) url += '?hints=1';
        var data = await OpenFangAPI.get(url);
        this.root = data.root || '';
        this.total = data.total || 0;
        this.libraryPresent = !!data.library_present;
        this.categories = data.categories || [];
        this.programs = data.programs || [];
        this.sync = data.sync || null;
        this.hintsTruncated = !!data.hints_truncated;
        this.maxHintsApplied = data.max_hints_applied != null ? data.max_hints_applied : null;
      } catch (e) {
        this.loadError = e.message || String(e);
        this.categories = [];
        this.programs = [];
      }
      try {
        var cur = await OpenFangAPI.get('/api/ainl/library/curated');
        this.curated = Array.isArray(cur) ? cur : [];
      } catch (e2) {
        this.curated = [];
      }
      this.loading = false;
    },

    openSettingsAinl() {
      try {
        sessionStorage.setItem('armaraos-settings-tab', 'ainl');
      } catch (e) { /* ignore */ }
      if (typeof window !== 'undefined' && window.location) {
        window.location.hash = 'settings';
      }
    },

    buildAbsolute(rel) {
      if (!rel || !this.root) return '';
      var base = String(this.root).replace(/[/\\]+$/, '');
      var r = String(rel).replace(/^[/\\]+/, '');
      return base + '/' + r.replace(/\\/g, '/');
    },

    navigateToSchedulerWithAinl(relPath) {
      if (!relPath) return;
      try {
        sessionStorage.setItem('armaraos-scheduler-prefill', JSON.stringify({
          actionKind: 'ainl_run',
          ainlPath: relPath,
          cron: '0 9 * * *'
        }));
      } catch (e) { /* ignore */ }
      if (typeof window !== 'undefined' && window.location) {
        window.location.hash = 'scheduler';
      }
    },

    copySchedulerDeepLink(relPath) {
      if (!relPath || typeof window === 'undefined' || !window.location) return;
      var base = window.location.origin + window.location.pathname;
      var link = base + '#scheduler?ainl=' + encodeURIComponent(relPath);
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(link).then(function() {
          OpenFangToast.success('Copied scheduler link');
        }).catch(function() {
          OpenFangToast.info(link);
        });
      } else {
        OpenFangToast.info(link);
      }
    },

    copyText(t) {
      if (!t) return;
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(t).then(function() {
          OpenFangToast.success('Copied');
        }).catch(function() {
          OpenFangToast.info(t);
        });
      } else {
        OpenFangToast.info(t);
      }
    },

    copyCommand(p) {
      var abs = p.absolute || this.buildAbsolute(p.path || '');
      if (!abs) {
        OpenFangToast.warn('Path not available yet — refresh the library.');
        return;
      }
      var line = 'ainl validate ' + (this.strictValidation ? '--strict ' : '') + '"' + abs.replace(/"/g, '\\"') + '"';
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(line).then(function() {
          OpenFangToast.success('Copied command to clipboard');
        }).catch(function() {
          OpenFangToast.warn(line);
        });
      } else {
        OpenFangToast.info(line);
      }
    },

    async trySample(p, mode) {
      mode = mode || 'validate';
      if (!this.isDesktopShell) {
        OpenFangToast.warn('Try sample runs the bundled AINL CLI in the desktop app.');
        return;
      }
      var rel = p.path;
      if (!rel) return;
      this.tryBusy = rel + ':' + mode;
      try {
        var res = await ArmaraosDesktopTauriInvoke('ainl_try_library_file', {
          relativePath: rel,
          mode: mode,
          timeoutSecs: mode === 'run' ? 300 : 120,
          strict: this.strictValidation,
        });
        if (!res) {
          OpenFangToast.error('Desktop command unavailable');
          return;
        }
        if (res.timed_out) {
          var sug = res.suggested_command || '';
          this.outputModal = {
            title: 'Timed out (' + (res.timeout_secs || '') + 's): ' + rel,
            ok: false,
            body: 'The process hit the time limit. Run the same command in a terminal, or adjust timeout in a future build.\n\n' + sug,
            suggestedCommand: sug,
            timedOut: true,
          };
          OpenFangToast.warn('ainl ' + mode + ' timed out — copy command below');
          return;
        }
        var msg = (res.ok ? 'OK: ' : 'Failed: ') + rel;
        var detail = (res.stdout || '') + (res.stderr ? '\n' + res.stderr : '');
        this.outputModal = {
          title: msg,
          ok: !!res.ok,
          body: detail.trim() || '(no output)',
          suggestedCommand: res.suggested_command || '',
          timedOut: false,
        };
        if (res.ok) {
          OpenFangToast.success('ainl ' + mode + ' finished');
        } else {
          OpenFangToast.error('ainl ' + mode + ' reported errors — see details');
        }
      } catch (e) {
        OpenFangToast.error(e.message || String(e));
      }
      this.tryBusy = null;
    },

    closeOutputModal() {
      this.outputModal = null;
    },

    async openFolder() {
      if (!this.isDesktopShell) {
        OpenFangToast.info('Open the folder ' + (this.root || '~/.armaraos/ainl-library') + ' in your file manager.');
        return;
      }
      try {
        await ArmaraosDesktopTauriInvoke('open_ainl_library_dir');
      } catch (e) {
        OpenFangToast.error(e.message || String(e));
      }
    },

    formatSyncTime(u) {
      if (u == null || u === '') return '';
      try {
        return new Date(Number(u) * 1000).toLocaleString();
      } catch (e) {
        return String(u);
      }
    },
  };
}
