// ArmaraOS home — browse files under the daemon home directory (~/.armaraos); optional edits via config allowlist
'use strict';

function homeFilesPage() {
  return {
    loading: false,
    loadError: '',
    rootDisplay: '',
    currentPath: '',
    entries: [],
    truncated: false,
    homeEditAllowlist: false,
    homeEditError: '',
    homeEditMaxBytes: 0,
    homeEditBackup: true,
    fileModal: false,
    fileLoading: false,
    fileError: '',
    filePath: '',
    fileEncoding: '',
    fileContent: '',
    fileSize: 0,
    fileEditable: false,
    fileEditMode: false,
    saveBusy: false,
    downloadBusy: false,
    agents: [],
    moveModal: false,
    moveFromRel: '',
    moveToRel: '',
    moveBusy: false,
    runModal: false,
    runTargetPath: '',
    runAgentId: '',
    /** Set when navigating from agent chat → workspace (Home folder). */
    _prefillRunAgentId: '',
    runBusy: false,
    runOutput: '',

    get isDesktopShell() {
      var w = typeof window !== 'undefined' ? window : null;
      var core = w && w.__TAURI__ && w.__TAURI__.core;
      return !!(core && typeof core.invoke === 'function');
    },

    init() {
      try {
        var pre = sessionStorage.getItem('armaraos-home-prefill-path');
        if (pre) {
          sessionStorage.removeItem('armaraos-home-prefill-path');
          this.currentPath = pre;
        }
      } catch (e) { /* ignore */ }
      try {
        var aid = sessionStorage.getItem('armaraos-home-prefill-agent-id');
        if (aid) {
          sessionStorage.removeItem('armaraos-home-prefill-agent-id');
          this._prefillRunAgentId = String(aid).trim();
        }
      } catch (e2) { /* ignore */ }
      this.refresh();
      this.loadAgentsBestEffort();
    },

    /** Normalize home-relative path for prefix checks (forward slashes, no leading slash). */
    _normHomeRel(p) {
      return String(p || '')
        .trim()
        .replace(/\\/g, '/')
        .replace(/^\/+/, '');
    },

    /**
     * Pick an agent whose workspace_rel_home is a prefix of fileRel (under ArmaraOS home).
     * Longest matching prefix wins so nested layouts resolve to the tightest agent.
     */
    pickAgentForHomePath(fileRel) {
      var pathNorm = this._normHomeRel(fileRel);
      if (!pathNorm) return '';
      var list = Array.isArray(this.agents) ? this.agents : [];
      var bestId = '';
      var bestLen = -1;
      for (var i = 0; i < list.length; i++) {
        var ag = list[i];
        var ws = this._normHomeRel(ag && ag.workspace_rel_home);
        if (!ws) continue;
        if (pathNorm === ws || pathNorm.startsWith(ws + '/')) {
          if (ws.length > bestLen) {
            bestLen = ws.length;
            bestId = String((ag && ag.id) || '').trim();
          }
        }
      }
      return bestId;
    },

    isAinlFileName(name) {
      if (!name) return false;
      var n = String(name).toLowerCase();
      return n.endsWith('.ainl') || n.endsWith('.lang');
    },

    canEditFileText() {
      return this.fileEncoding === 'utf8' && this.fileEditable && this.fileEditMode;
    },

    toggleFileEditMode() {
      if (this.fileEncoding !== 'utf8') return;
      this.fileEditMode = !this.fileEditMode;
    },

    breadcrumbParts() {
      var p = (this.currentPath || '').trim();
      if (!p) return [];
      return p.split('/').filter(Boolean);
    },

    async loadAgentsBestEffort() {
      try {
        var data = await OpenFangAPI.get('/api/agents');
        var list = Array.isArray(data) ? data : data.agents;
        this.agents = Array.isArray(list) ? list : [];
      } catch (e) {
        this.agents = [];
      }
    },

    async refresh() {
      this.loading = true;
      this.loadError = '';
      try {
        var q = this.currentPath
          ? '?path=' + encodeURIComponent(this.currentPath)
          : '';
        var data = await OpenFangAPI.get('/api/armaraos-home/list' + q);
        this.rootDisplay = data.root || '';
        this.entries = Array.isArray(data.entries) ? data.entries : [];
        this.truncated = !!data.truncated;
        if (typeof data.path === 'string') this.currentPath = data.path;
        var he = data.home_edit || {};
        this.homeEditAllowlist = !!he.allowlist_enabled;
        this.homeEditError = he.allowlist_error || '';
        this.homeEditMaxBytes = he.max_bytes != null ? he.max_bytes : 0;
        this.homeEditBackup = he.backup !== false;
      } catch (e) {
        this.loadError = e.message || String(e);
        this.entries = [];
      }
      this.loading = false;
    },

    pathToChild(name) {
      if (!name) return this.currentPath;
      return this.currentPath ? this.currentPath + '/' + name : name;
    },

    enterDir(name) {
      this.currentPath = this.pathToChild(name);
      this.refresh();
    },

    goRoot() {
      this.currentPath = '';
      this.refresh();
    },

    goUp() {
      var parts = this.breadcrumbParts();
      if (!parts.length) return;
      parts.pop();
      this.currentPath = parts.join('/');
      this.refresh();
    },

    crumbNavigate(idx) {
      var parts = this.breadcrumbParts();
      this.currentPath = parts.slice(0, idx + 1).join('/');
      this.refresh();
    },

    async downloadFileByRelativePath(rel) {
      if (!rel) return;
      this.downloadBusy = true;
      try {
        if (this.isDesktopShell && typeof ArmaraosDesktopTauriInvoke === 'function') {
          var out = await ArmaraosDesktopTauriInvoke('copy_home_file_to_downloads', {
            relativePath: rel,
          });
          if (out && out.downloads_path) {
            OpenFangToast.success('Saved to Downloads');
          } else {
            OpenFangToast.success('Copied to Downloads');
          }
        } else {
          await OpenFangAPI.downloadArmaraosHomeFile(rel);
          OpenFangToast.success('Download started');
        }
      } catch (e) {
        OpenFangToast.error(openFangErrText(e));
      }
      this.downloadBusy = false;
    },

    async downloadFile(name) {
      await this.downloadFileByRelativePath(this.pathToChild(name));
    },

    downloadOpenFile() {
      return this.downloadFileByRelativePath(this.filePath);
    },

    openMoveModal(name) {
      this.moveFromRel = this.pathToChild(name);
      this.moveToRel = this.moveFromRel;
      this.moveModal = true;
    },

    closeMoveModal() {
      this.moveModal = false;
      this.moveBusy = false;
    },

    async submitMove() {
      var from = (this.moveFromRel || '').trim();
      var to = (this.moveToRel || '').trim();
      if (!from || !to) {
        OpenFangToast.warn('Enter both source and destination paths.');
        return;
      }
      if (from === to) {
        OpenFangToast.warn('Destination must differ from source.');
        return;
      }
      this.moveBusy = true;
      try {
        await OpenFangAPI.post('/api/armaraos-home/move', { from: from, to: to });
        OpenFangToast.success('Moved');
        this.closeMoveModal();
        await this.refresh();
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
      }
      this.moveBusy = false;
    },

    openRunForPath(relPath) {
      this.runTargetPath = (relPath || '').trim();
      var fromWs = this.pickAgentForHomePath(this.runTargetPath);
      this.runAgentId =
        fromWs || (this._prefillRunAgentId || '') || '';
      this.runOutput = '';
      this.runModal = true;
    },

    closeRunModal() {
      this.runModal = false;
      this.runBusy = false;
    },

    async runAinlAtPath() {
      var p = (this.runTargetPath || '').trim();
      if (!p) return;
      this.runBusy = true;
      this.runOutput = '';
      try {
        var body = { path: p, timeout_secs: 300 };
        if (this.runAgentId) body.agent_id = this.runAgentId;
        var data = await OpenFangAPI.post('/api/armaraos-home/run-ainl', body);
        var ok = !!data.ok;
        var out = typeof data.output === 'string' ? data.output : JSON.stringify(data.output || '');
        this.runOutput = out || (ok ? '(no output)' : '(failed)');
        if (ok) {
          OpenFangToast.success('AINL finished');
        } else {
          OpenFangToast.warn('AINL exited with an error — see output below.');
        }
        await this.refresh();
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
      }
      this.runBusy = false;
    },

    async openFile(name) {
      var rel = this.pathToChild(name);
      this.fileModal = true;
      this.fileLoading = true;
      this.fileError = '';
      this.filePath = rel;
      this.fileEncoding = '';
      this.fileContent = '';
      this.fileSize = 0;
      this.fileEditable = false;
      this.fileEditMode = false;
      try {
        var data = await OpenFangAPI.get(
          '/api/armaraos-home/read?path=' + encodeURIComponent(rel)
        );
        this.fileEncoding = data.encoding || '';
        this.fileContent =
          typeof data.content === 'string' ? data.content : JSON.stringify(data.content);
        this.fileSize = data.size != null ? data.size : 0;
        this.fileEditable = !!data.editable;
        if (data.allowlist_error) {
          this.homeEditError = data.allowlist_error;
        }
      } catch (e) {
        this.fileError = e.message || String(e);
      }
      this.fileLoading = false;
    },

    closeFileModal() {
      this.fileModal = false;
      this.fileEditMode = false;
    },

    async saveFile() {
      if (!this.fileEditable || this.fileEncoding !== 'utf8' || !this.filePath) return;
      var len = new TextEncoder().encode(this.fileContent).length;
      if (this.homeEditMaxBytes > 0 && len > this.homeEditMaxBytes) {
        OpenFangToast.error('Content exceeds home_edit_max_bytes (' + this.homeEditMaxBytes + ').');
        return;
      }
      this.saveBusy = true;
      try {
        await OpenFangAPI.post('/api/armaraos-home/write', {
          path: this.filePath,
          content: this.fileContent,
        });
        OpenFangToast.success(this.homeEditBackup ? 'Saved (backup .bak if file existed)' : 'Saved');
        this.refresh();
        this.closeFileModal();
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
      }
      this.saveBusy = false;
    },

    copyFileContent() {
      var t = this.fileContent;
      if (!t) return;
      if (typeof copyTextToClipboard === 'function') {
        copyTextToClipboard(t).then(function () {
          OpenFangToast.success('Copied');
        }).catch(function () {
          OpenFangToast.warn('Copy failed');
        });
      } else if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(t).then(function () {
          OpenFangToast.success('Copied');
        });
      }
    },
  };
}
