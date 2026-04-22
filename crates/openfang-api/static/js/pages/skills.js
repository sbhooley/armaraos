// ArmaraOS Skills Page — OpenClaw/ClawHub ecosystem + local skills + MCP servers
'use strict';

function skillsPage() {
  return {
    tab: 'installed',
    skills: [],
    loading: true,
    loadError: '',

    // ClawHub state
    clawhubSearch: '',
    clawhubResults: [],
    clawhubBrowseResults: [],
    clawhubLoading: false,
    clawhubError: '',
    clawhubSort: 'trending',
    clawhubNextCursor: null,
    installingSlug: null,
    installResult: null,
    _searchTimer: null,
    _browseCache: {},    // { key: { ts, data } } client-side 60s cache
    _searchCache: {},

    // Skill detail modal
    skillDetail: null,
    detailLoading: false,
    showSkillCode: false,
    skillCode: '',
    skillCodeFilename: '',
    skillCodeLoading: false,

    // MCP servers
    mcpServers: [],
    mcpLoading: false,

    // MCP guided installer (Skills → MCP)
    mcpPresets: [],
    mcpAvailableIntegrations: [],
    mcpInstalledIntegrations: [],
    mcpPresetId: '',
    mcpSelectedTemplate: null,
    mcpForm: {},
    mcpFieldErrors: {},
    mcpInstallerLoading: false,
    mcpInstallerBusy: false,
    mcpInstallerError: '',
    mcpHostReadiness: null,
    mcpUvBootstrapBusy: false,

    // Custom MCP (primary flow)
    mcpShowPresets: false,
    customMcpTransport: 'stdio',
    customMcpForm: {
      id: '',
      name: '',
      icon: '🔌',
      description: '',
      command: '',
      argsLine: '',
      url: '',
      timeout_secs: '',
      headersText: '',
    },
    customMcpEnvRows: [],
    customMcpFieldErrors: {},

    // Category definitions from the OpenClaw ecosystem
    categories: [
      { id: 'coding', name: 'Coding & IDEs' },
      { id: 'git', name: 'Git & GitHub' },
      { id: 'web', name: 'Web & Frontend' },
      { id: 'devops', name: 'DevOps & Cloud' },
      { id: 'browser', name: 'Browser & Automation' },
      { id: 'search', name: 'Search & Research' },
      { id: 'ai', name: 'AI & LLMs' },
      { id: 'data', name: 'Data & Analytics' },
      { id: 'productivity', name: 'Productivity' },
      { id: 'communication', name: 'Communication' },
      { id: 'media', name: 'Media & Streaming' },
      { id: 'notes', name: 'Notes & PKM' },
      { id: 'security', name: 'Security' },
      { id: 'cli', name: 'CLI Utilities' },
      { id: 'marketing', name: 'Marketing & Sales' },
      { id: 'finance', name: 'Finance' },
      { id: 'smart-home', name: 'Smart Home & IoT' },
      { id: 'docs', name: 'PDF & Documents' },
    ],

    runtimeBadge: function(rt) {
      var r = (rt || '').toLowerCase();
      if (r === 'python' || r === 'py') return { text: 'PY', cls: 'runtime-badge-py' };
      if (r === 'node' || r === 'nodejs' || r === 'js' || r === 'javascript') return { text: 'JS', cls: 'runtime-badge-js' };
      if (r === 'wasm' || r === 'webassembly') return { text: 'WASM · coming soon', cls: 'runtime-badge-wasm runtime-badge-unavailable' };
      if (r === 'prompt_only' || r === 'prompt' || r === 'promptonly') return { text: 'PROMPT', cls: 'runtime-badge-prompt' };
      return { text: r.toUpperCase().substring(0, 4), cls: 'runtime-badge-prompt' };
    },

    sourceBadge: function(source) {
      if (!source) return { text: 'Local', cls: 'badge-dim' };
      switch (source.type) {
        case 'clawhub': return { text: 'ClawHub', cls: 'badge-info' };
        case 'openclaw': return { text: 'OpenClaw', cls: 'badge-info' };
        case 'bundled': return { text: 'Built-in', cls: 'badge-success' };
        default: return { text: 'Local', cls: 'badge-dim' };
      }
    },

    formatDownloads: function(n) {
      if (!n) return '0';
      if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M';
      if (n >= 1000) return (n / 1000).toFixed(1) + 'K';
      return n.toString();
    },

    /** ClawHub vector search returns score (often 0–1). Show as % match when in range. */
    formatClawhubMatchScore: function(score) {
      if (score == null || score === '' || !(typeof score === 'number') || isNaN(score) || score <= 0) return '';
      if (score <= 1) return Math.round(score * 100) + '% match';
      return score.toFixed(2) + ' score';
    },

    /** ClawHub uses Unix ms for updated_at. */
    formatClawhubUpdatedAt: function(ms) {
      if (ms == null || ms === 0) return '';
      var d = new Date(typeof ms === 'number' ? ms : parseInt(ms, 10));
      if (isNaN(d.getTime())) return '';
      return d.toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' });
    },

    async loadSkills() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await OpenFangAPI.get('/api/skills');
        this.skills = (data.skills || []).map(function(s) {
          return {
            name: s.name,
            description: s.description || '',
            version: s.version || '',
            author: s.author || '',
            runtime: s.runtime || 'unknown',
            tools_count: s.tools_count || 0,
            tags: s.tags || [],
            enabled: s.enabled !== false,
            source: s.source || { type: 'local' },
            has_prompt_context: !!s.has_prompt_context
          };
        });
      } catch(e) {
        this.skills = [];
        this.loadError = e.message || 'Could not load skills.';
      }
      this.loading = false;
    },

    async loadData() {
      await this.loadSkills();
    },

    // Debounced search — fires 350ms after user stops typing
    onSearchInput() {
      if (this._searchTimer) clearTimeout(this._searchTimer);
      var q = this.clawhubSearch.trim();
      if (!q) {
        this.clawhubResults = [];
        this.clawhubError = '';
        return;
      }
      var self = this;
      this._searchTimer = setTimeout(function() { self.searchClawHub(); }, 350);
    },

    // ClawHub search
    async searchClawHub() {
      if (!this.clawhubSearch.trim()) {
        this.clawhubResults = [];
        return;
      }
      this.clawhubLoading = true;
      this.clawhubError = '';
      try {
        var data = await OpenFangAPI.get('/api/clawhub/search?q=' + encodeURIComponent(this.clawhubSearch.trim()) + '&limit=20');
        this.clawhubResults = data.items || [];
        if (data.error) this.clawhubError = data.error;
      } catch(e) {
        this.clawhubResults = [];
        this.clawhubError = e.message || 'Search failed';
      }
      this.clawhubLoading = false;
    },

    // Clear search and go back to browse
    clearSearch() {
      this.clawhubSearch = '';
      this.clawhubResults = [];
      this.clawhubError = '';
      if (this._searchTimer) clearTimeout(this._searchTimer);
    },

    // ClawHub browse by sort (with 60s client-side cache)
    async browseClawHub(sort) {
      this.clawhubSort = sort || 'trending';
      var ckey = 'browse:' + this.clawhubSort;
      var cached = this._browseCache[ckey];
      if (cached && (Date.now() - cached.ts) < 60000) {
        this.clawhubBrowseResults = cached.data.items || [];
        this.clawhubNextCursor = cached.data.next_cursor || null;
        return;
      }
      this.clawhubLoading = true;
      this.clawhubError = '';
      this.clawhubNextCursor = null;
      try {
        var data = await OpenFangAPI.get('/api/clawhub/browse?sort=' + this.clawhubSort + '&limit=20');
        this.clawhubBrowseResults = data.items || [];
        this.clawhubNextCursor = data.next_cursor || null;
        if (data.error) this.clawhubError = data.error;
        this._browseCache[ckey] = { ts: Date.now(), data: data };
      } catch(e) {
        this.clawhubBrowseResults = [];
        this.clawhubError = e.message || 'Browse failed';
      }
      this.clawhubLoading = false;
    },

    // ClawHub load more results
    async loadMoreClawHub() {
      if (!this.clawhubNextCursor || this.clawhubLoading) return;
      this.clawhubLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/clawhub/browse?sort=' + this.clawhubSort + '&limit=20&cursor=' + encodeURIComponent(this.clawhubNextCursor));
        this.clawhubBrowseResults = this.clawhubBrowseResults.concat(data.items || []);
        this.clawhubNextCursor = data.next_cursor || null;
      } catch(e) {
        // silently fail on load more
      }
      this.clawhubLoading = false;
    },

    // Show skill detail
    async showSkillDetail(slug) {
      this.detailLoading = true;
      this.skillDetail = null;
      this.installResult = null;
      try {
        var data = await OpenFangAPI.get('/api/clawhub/skill/' + encodeURIComponent(slug));
        this.skillDetail = data;
      } catch(e) {
        OpenFangToast.error('Failed to load skill details');
      }
      this.detailLoading = false;
    },

    closeDetail() {
      this.skillDetail = null;
      this.installResult = null;
      this.showSkillCode = false;
      this.skillCode = '';
      this.skillCodeFilename = '';
    },

    async viewSkillCode(slug) {
      if (this.showSkillCode) {
        this.showSkillCode = false;
        return;
      }
      this.skillCodeLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/clawhub/skill/' + encodeURIComponent(slug) + '/code');
        this.skillCode = data.code || '';
        this.skillCodeFilename = data.filename || 'source';
        this.showSkillCode = true;
      } catch(e) {
        OpenFangToast.error('Could not load skill source code');
      }
      this.skillCodeLoading = false;
    },

    // Install from ClawHub
    async installFromClawHub(slug) {
      this.installingSlug = slug;
      this.installResult = null;
      try {
        var data = await OpenFangAPI.post('/api/clawhub/install', { slug: slug });
        this.installResult = data;
        if (data.warnings && data.warnings.length > 0) {
          OpenFangToast.success('Skill "' + data.name + '" installed with ' + data.warnings.length + ' warning(s)');
        } else {
          OpenFangToast.success('Skill "' + data.name + '" installed successfully');
        }
        // Update installed state in detail modal if open
        if (this.skillDetail && this.skillDetail.slug === slug) {
          this.skillDetail.installed = true;
        }
        await this.loadSkills();
      } catch(e) {
        var msg = e.message || 'Install failed';
        if (msg.includes('already_installed')) {
          OpenFangToast.error('Skill is already installed');
        } else if (msg.includes('SecurityBlocked')) {
          OpenFangToast.error('Skill blocked by security scan');
        } else {
          OpenFangToast.error('Install failed: ' + msg);
        }
      }
      this.installingSlug = null;
    },

    // Uninstall
    uninstallSkill: function(name) {
      var self = this;
      OpenFangToast.confirm('Uninstall Skill', 'Uninstall skill "' + name + '"? This cannot be undone.', async function() {
        try {
          await OpenFangAPI.post('/api/skills/uninstall', { name: name });
          OpenFangToast.success('Skill "' + name + '" uninstalled');
          await self.loadSkills();
        } catch(e) {
          OpenFangToast.error('Failed to uninstall skill: ' + openFangErrText(e));
        }
      });
    },

    // Create prompt-only skill
    async createDemoSkill(skill) {
      try {
        await OpenFangAPI.post('/api/skills/create', {
          name: skill.name,
          description: skill.description,
          runtime: 'prompt_only',
          prompt_context: skill.prompt_context || skill.description
        });
        OpenFangToast.success('Skill "' + skill.name + '" created');
        this.tab = 'installed';
        await this.loadSkills();
      } catch(e) {
        OpenFangToast.error('Failed to create skill: ' + openFangErrText(e));
      }
    },

    // Load MCP servers
    async loadMcpServers() {
      this.mcpLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/mcp/servers');
        this.mcpServers = data;
      } catch(e) {
        this.mcpServers = { configured: [], connected: [], total_configured: 0, total_connected: 0 };
      }
      this.mcpLoading = false;
    },

    resetCustomMcpForm: function() {
      this.customMcpTransport = 'stdio';
      this.customMcpForm = {
        id: '',
        name: '',
        icon: '🔌',
        description: '',
        command: '',
        argsLine: '',
        url: '',
        timeout_secs: '',
        headersText: '',
      };
      this.customMcpEnvRows = [];
      this.customMcpFieldErrors = {};
    },

    addCustomMcpEnvRow: function() {
      this.customMcpEnvRows.push({
        name: '',
        label: '',
        help: '',
        is_secret: true,
        value: '',
      });
    },

    removeCustomMcpEnvRow: function(idx) {
      this.customMcpEnvRows.splice(idx, 1);
    },

    buildCustomMcpPayload: function() {
      var transport;
      var t = this.customMcpTransport;
      if (t === 'stdio') {
        var args = [];
        var line = (this.customMcpForm.argsLine || '').trim();
        if (line) {
          line.split(/\s+/).forEach(function(a) {
            if (a) args.push(a);
          });
        }
        transport = { type: 'stdio', command: (this.customMcpForm.command || '').trim(), args: args };
      } else if (t === 'sse') {
        transport = { type: 'sse', url: (this.customMcpForm.url || '').trim() };
      } else {
        transport = { type: 'http', url: (this.customMcpForm.url || '').trim() };
      }
      var env = (this.customMcpEnvRows || []).map(function(row) {
        return {
          name: (row.name || '').trim(),
          label: (row.label || '').trim() || (row.name || '').trim(),
          help: (row.help || '').trim(),
          is_secret: !!row.is_secret,
          value: row.value != null ? String(row.value) : '',
        };
      }).filter(function(row) {
        return row.name.length > 0;
      });
      var headers = [];
      var ht = (this.customMcpForm.headersText || '').split('\n');
      ht.forEach(function(line) {
        var s = (line || '').trim();
        if (s) headers.push(s);
      });
      var payload = {
        id: (this.customMcpForm.id || '').trim(),
        name: (this.customMcpForm.name || '').trim(),
        icon: (this.customMcpForm.icon || '').trim() || '🔌',
        transport: transport,
        env: env,
      };
      var desc = (this.customMcpForm.description || '').trim();
      if (desc) payload.description = desc;
      var to = (this.customMcpForm.timeout_secs || '').trim();
      if (to && !isNaN(parseInt(to, 10))) payload.timeout_secs = parseInt(to, 10);
      if (headers.length && (t === 'sse' || t === 'http')) payload.headers = headers;
      return payload;
    },

    async validateCustomMcp() {
      this.customMcpFieldErrors = {};
      this.mcpInstallerBusy = true;
      try {
        var payload = this.buildCustomMcpPayload();
        var res = await OpenFangAPI.post('/api/integrations/custom/validate', payload);
        if (res.field_errors && Object.keys(res.field_errors).length) {
          this.customMcpFieldErrors = res.field_errors;
          OpenFangToast.error('Fix the highlighted fields and try again.');
        } else if (res.ok === false) {
          OpenFangToast.error('Validation failed.');
        } else {
          OpenFangToast.success('Looks good — you can add this MCP server now.');
        }
      } catch(e) {
        OpenFangToast.error('Validation failed: ' + openFangErrText(e));
      }
      this.mcpInstallerBusy = false;
    },

    async installCustomMcp() {
      this.customMcpFieldErrors = {};
      this.mcpInstallerBusy = true;
      try {
        var payload = this.buildCustomMcpPayload();
        var res = await OpenFangAPI.post('/api/integrations/custom/add', payload);
        OpenFangToast.success(res.message || 'Custom MCP installed');
        this.resetCustomMcpForm();
        await this.loadMcpPanel();
        var cid = res.id;
        if (cid) {
          try {
            await OpenFangAPI.post('/api/integrations/' + encodeURIComponent(cid) + '/reconnect', {});
          } catch(e2) {
            /* ignore */
          }
        }
        await this.loadMcpPanel();
      } catch(e) {
        OpenFangToast.error('Install failed: ' + openFangErrText(e));
      }
      this.mcpInstallerBusy = false;
    },

    isCustomMcpInstalled: function() {
      var id = (this.customMcpForm.id || '').trim();
      if (!id) return false;
      return this.isMcpIntegrationInstalled(id);
    },

    async loadMcpInstallerData() {
      this.mcpInstallerLoading = true;
      this.mcpInstallerError = '';
      try {
        var presets = await OpenFangAPI.get('/api/integrations/mcp-presets');
        this.mcpPresets = (presets.presets || []).slice().sort(function(a, b) {
          return (a.order || 0) - (b.order || 0);
        });
        var avail = await OpenFangAPI.get('/api/integrations/available');
        this.mcpAvailableIntegrations = avail.integrations || [];
        var inst = await OpenFangAPI.get('/api/integrations');
        this.mcpInstalledIntegrations = inst.installed || [];
        if (!this.mcpPresetId && this.mcpPresets.length) {
          this.mcpPresetId = this.mcpPresets[0].preset_id;
        }
        this.applyMcpPresetSelection();
        try {
          this.mcpHostReadiness = await OpenFangAPI.get('/api/system/mcp-host-readiness');
        } catch (e2) {
          this.mcpHostReadiness = null;
        }
      } catch(e) {
        this.mcpInstallerError = e.message || String(e);
      }
      this.mcpInstallerLoading = false;
    },

    async loadMcpPanel() {
      await Promise.all([this.loadMcpServers(), this.loadMcpInstallerData()]);
    },

    findMcpTemplate: function(integrationId) {
      return (this.mcpAvailableIntegrations || []).find(function(t) { return t.id === integrationId; }) || null;
    },

    isMcpIntegrationInstalled: function(integrationId) {
      return (this.mcpInstalledIntegrations || []).some(function(x) { return x.id === integrationId; });
    },

    applyMcpPresetSelection: function() {
      this.mcpFieldErrors = {};
      this.mcpInstallerError = '';
      var preset = (this.mcpPresets || []).find(function(p) { return p.preset_id === this.mcpPresetId; }.bind(this));
      if (!preset) {
        this.mcpSelectedTemplate = null;
        this.mcpForm = {};
        return;
      }
      var tpl = this.findMcpTemplate(preset.integration_id);
      this.mcpSelectedTemplate = tpl;
      var form = {};
      if (tpl && tpl.required_env) {
        tpl.required_env.forEach(function(e) {
          form[e.name] = '';
        });
      }
      if (preset.integration_id === 'filesystem') {
        form.allowed_paths = '';
      }
      if (preset.integration_id === 'apple-caldav') {
        form.DAV_PROVIDER = form.DAV_PROVIDER || 'icloud';
      }
      this.mcpForm = form;
    },

    async validateMcpPreset() {
      if (!this.mcpSelectedTemplate) return;
      this.mcpFieldErrors = {};
      this.mcpInstallerBusy = true;
      try {
        var preset = (this.mcpPresets || []).find(function(p) { return p.preset_id === this.mcpPresetId; }.bind(this));
        var id = preset ? preset.integration_id : '';
        var payload = { id: id, env: this.buildMcpInstallEnv(), config: this.buildMcpInstallConfig() };
        var res = await OpenFangAPI.post('/api/integrations/validate', payload);
        if (res.field_errors && Object.keys(res.field_errors).length) {
          this.mcpFieldErrors = res.field_errors;
          OpenFangToast.error('Fix the highlighted fields and try again.');
        } else {
          OpenFangToast.success('Looks good — you can install now.');
        }
      } catch(e) {
        OpenFangToast.error('Validation failed: ' + openFangErrText(e));
      }
      this.mcpInstallerBusy = false;
    },

    buildMcpInstallEnv: function() {
      var env = {};
      var tpl = this.mcpSelectedTemplate;
      if (!tpl || !tpl.required_env) return env;
      var self = this;
      tpl.required_env.forEach(function(field) {
        var v = (self.mcpForm[field.name] != null) ? String(self.mcpForm[field.name]) : '';
        env[field.name] = v;
      });
      return env;
    },

    buildMcpInstallConfig: function() {
      var cfg = {};
      var preset = (this.mcpPresets || []).find(function(p) { return p.preset_id === this.mcpPresetId; }.bind(this));
      if (preset && preset.integration_id === 'filesystem') {
        cfg.allowed_paths = (this.mcpForm.allowed_paths != null) ? String(this.mcpForm.allowed_paths) : '';
      }
      return cfg;
    },

    async installMcpPreset() {
      var preset = (this.mcpPresets || []).find(function(p) { return p.preset_id === this.mcpPresetId; }.bind(this));
      if (!preset) return;
      this.mcpFieldErrors = {};
      this.mcpInstallerBusy = true;
      try {
        var payload = {
          id: preset.integration_id,
          env: this.buildMcpInstallEnv(),
          config: this.buildMcpInstallConfig(),
        };
        // OAuth-only templates (no required env) — still send empty objects so the configured installer runs.
        if (preset.integration_id === 'google-calendar') {
          payload.env = payload.env || {};
          payload.config = payload.config || {};
        }
        var res = await OpenFangAPI.post('/api/integrations/add', payload);
        OpenFangToast.success(res.message || 'Installed');
        await this.loadMcpPanel();
        // Best-effort reconnect for the new integration
        try {
          await OpenFangAPI.post('/api/integrations/' + encodeURIComponent(preset.integration_id) + '/reconnect', {});
        } catch(e2) {
          // ignore — user can hit Reconnect manually
        }
        await this.loadMcpPanel();
      } catch(e) {
        OpenFangToast.error('Install failed: ' + openFangErrText(e));
      }
      this.mcpInstallerBusy = false;
    },

    async reconnectMcpIntegration(integrationId) {
      try {
        await OpenFangAPI.post('/api/integrations/' + encodeURIComponent(integrationId) + '/reconnect', {});
        OpenFangToast.success('Reconnect requested');
        await this.loadMcpPanel();
      } catch(e) {
        OpenFangToast.error('Reconnect failed: ' + openFangErrText(e));
      }
    },

    async mcpBootstrapUv() {
      this.mcpUvBootstrapBusy = true;
      try {
        var res = await OpenFangAPI.post('/api/system/bootstrap-uv', {});
        if (res && res.ok === false) {
          OpenFangToast.error(res.error || 'Install failed');
        } else {
          OpenFangToast.success((res && res.message) || 'uv install completed');
        }
        await this.loadMcpInstallerData();
      } catch (e) {
        OpenFangToast.error(openFangErrText(e));
      }
      this.mcpUvBootstrapBusy = false;
    },

    async mcpCopyUvInstallCommand() {
      var cmd = (this.mcpHostReadiness && this.mcpHostReadiness.uv_install_sh) || 'curl -LsSf https://astral.sh/uv/install.sh | sh';
      try {
        if (typeof copyTextToClipboard === 'function') {
          await copyTextToClipboard(cmd);
          OpenFangToast.success('Copied install command');
        }
      } catch (e) { /* ignore */ }
    },

    async registerMcpIntegrationLegacy(integrationId) {
      try {
        await OpenFangAPI.post('/api/integrations/add', { id: integrationId });
        OpenFangToast.success('Integration registered');
        await this.loadMcpPanel();
      } catch(e) {
        OpenFangToast.error('Register failed: ' + openFangErrText(e));
      }
    },

    mcpCurrentPreset: function() {
      var self = this;
      return (this.mcpPresets || []).find(function(p) { return p.preset_id === self.mcpPresetId; }) || null;
    },

    mcpCurrentIntegrationId: function() {
      var p = this.mcpCurrentPreset();
      return p && p.integration_id ? p.integration_id : '';
    },

    // Category search on ClawHub
    searchCategory: function(cat) {
      this.clawhubSearch = cat.name;
      this.searchClawHub();
    },

    // Quick start skills (prompt-only, zero deps)
    quickStartSkills: [
      { name: 'code-review-guide', description: 'Adds code review best practices and checklist to agent context.', prompt_context: 'You are an expert code reviewer. When reviewing code:\n1. Check for bugs and logic errors\n2. Evaluate code style and readability\n3. Look for security vulnerabilities\n4. Suggest performance improvements\n5. Verify error handling\n6. Check test coverage' },
      { name: 'writing-style', description: 'Configurable writing style guide for content generation.', prompt_context: 'Follow these writing guidelines:\n- Use clear, concise language\n- Prefer active voice over passive voice\n- Keep paragraphs short (3-4 sentences)\n- Use bullet points for lists\n- Maintain consistent tone throughout' },
      { name: 'api-design', description: 'REST API design patterns and conventions.', prompt_context: 'When designing REST APIs:\n- Use nouns for resources, not verbs\n- Use HTTP methods correctly (GET, POST, PUT, DELETE)\n- Return appropriate status codes\n- Use pagination for list endpoints\n- Version your API\n- Document all endpoints' },
      { name: 'security-checklist', description: 'OWASP-aligned security review checklist.', prompt_context: 'Security review checklist (OWASP aligned):\n- Input validation on all user inputs\n- Output encoding to prevent XSS\n- Parameterized queries to prevent SQL injection\n- Authentication and session management\n- Access control checks\n- CSRF protection\n- Security headers\n- Error handling without information leakage' },
    ],

    // Check if skill is installed by slug
    isSkillInstalled: function(slug) {
      return this.skills.some(function(s) {
        return s.source && s.source.type === 'clawhub' && s.source.slug === slug;
      });
    },

    // Check if skill is installed by name
    isSkillInstalledByName: function(name) {
      return this.skills.some(function(s) { return s.name === name; });
    },
  };
}
