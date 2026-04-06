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
    saveBusy: false,
    downloadBusy: false,

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
      this.refresh();
    },

    breadcrumbParts() {
      var p = (this.currentPath || '').trim();
      if (!p) return [];
      return p.split('/').filter(Boolean);
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
        OpenFangToast.error((e && e.message) ? e.message : String(e));
      }
      this.downloadBusy = false;
    },

    async downloadFile(name) {
      await this.downloadFileByRelativePath(this.pathToChild(name));
    },

    downloadOpenFile() {
      return this.downloadFileByRelativePath(this.filePath);
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
        OpenFangToast.error(e.message || String(e));
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
