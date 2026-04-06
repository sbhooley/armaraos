// ArmaraOS Agents Page — Multi-step spawn wizard, detail view with tabs, file editor, personality presets
'use strict';

/** Escape a string for use inside TOML triple-quoted strings ("""\n...\n""").
 *  Backslashes are escaped, and runs of 3+ consecutive double-quotes are
 *  broken up so the TOML parser never sees an unintended closing delimiter.
 */
function tomlMultilineEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"""/g, '""\\"');
}

/** Escape a string for use inside a TOML basic (single-line) string ("...").
 *  Backslashes, double-quotes, and common control chars are escaped.
 */
function tomlBasicEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
}

var SPAWN_DEFAULT_PROVIDER = 'openrouter';
var SPAWN_DEFAULT_MODEL = 'stepfun/step-3.5-flash:free';

function spawnModelFromManifestToml(toml) {
  var provider = '?';
  var model = '?';
  var mp = toml.match(/provider\s*=\s*"([^"]*)"/);
  var mm = toml.match(/model\s*=\s*"([^"]*)"/);
  if (mp) provider = mp[1];
  if (mm) model = mm[1];
  return { provider: provider, model: model };
}

function agentsPage() {
  return {
    tab: 'agents',
    activeChatAgent: null,
    /** Collapsed by default: internal automation / probe agent chats */
    systemChatAgentsExpanded: false,
    // -- Agents state --
    showSpawnModal: false,
    showDetailModal: false,
    detailAgent: null,
    spawnMode: 'wizard',
    spawning: false,
    spawnToml: '',
    filterState: 'all',
    loading: true,
    loadError: '',
    loadErrorDetail: '',
    loadErrorHint: '',
    loadErrorRequestId: '',
    loadErrorWhere: '',
    loadErrorServerPath: '',
    spawnForm: {
      name: '',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      systemPrompt: 'You are a helpful assistant.',
      profile: 'full',
      caps: { memory_read: true, memory_write: true, network: false, shell: false, agent_spawn: false }
    },

    // -- Multi-step wizard state --
    spawnProviders: [],       // populated from /api/providers on wizard open
    spawnProvidersLoading: false,
    spawnStep: 1,
    spawnIdentity: { emoji: '', color: '#FF5C00', archetype: '' },
    selectedPreset: '',
    soulContent: '',
    emojiOptions: [
      '\u{1F916}', '\u{1F4BB}', '\u{1F50D}', '\u{270D}\uFE0F', '\u{1F4CA}', '\u{1F6E0}\uFE0F',
      '\u{1F4AC}', '\u{1F393}', '\u{1F310}', '\u{1F512}', '\u{26A1}', '\u{1F680}',
      '\u{1F9EA}', '\u{1F3AF}', '\u{1F4D6}', '\u{1F9D1}\u200D\u{1F4BB}', '\u{1F4E7}', '\u{1F3E2}',
      '\u{2764}\uFE0F', '\u{1F31F}', '\u{1F527}', '\u{1F4DD}', '\u{1F4A1}', '\u{1F3A8}'
    ],
    archetypeOptions: ['Assistant', 'Researcher', 'Coder', 'Writer', 'DevOps', 'Support', 'Analyst', 'Custom'],
    personalityPresets: [
      { id: 'professional', label: 'Professional', soul: 'Communicate in a clear, professional tone. Be direct and structured. Use formal language and data-driven reasoning. Prioritize accuracy over personality.' },
      { id: 'friendly', label: 'Friendly', soul: 'Be warm, approachable, and conversational. Use casual language and show genuine interest in the user. Add personality to your responses while staying helpful.' },
      { id: 'technical', label: 'Technical', soul: 'Focus on technical accuracy and depth. Use precise terminology. Show your work and reasoning. Prefer code examples and structured explanations.' },
      { id: 'creative', label: 'Creative', soul: 'Be imaginative and expressive. Use vivid language, analogies, and unexpected connections. Encourage creative thinking and explore multiple perspectives.' },
      { id: 'concise', label: 'Concise', soul: 'Be extremely brief and to the point. No filler, no pleasantries. Answer in the fewest words possible while remaining accurate and complete.' },
      { id: 'mentor', label: 'Mentor', soul: 'Be patient and encouraging like a great teacher. Break down complex topics step by step. Ask guiding questions. Celebrate progress and build confidence.' }
    ],

    // -- Detail modal tabs --
    detailTab: 'info',
    agentFiles: [],
    editingFile: null,
    fileContent: '',
    fileSaving: false,
    filesLoading: false,
    configForm: {},
    configSaving: false,
    // -- Tool filters --
    toolFilters: { tool_allowlist: [], tool_blocklist: [] },
    toolFiltersLoading: false,
    newAllowTool: '',
    newBlockTool: '',
    // -- Model switch --
    editingModel: false,
    newModelValue: '',
    editingProvider: false,
    newProviderValue: '',
    modelSaving: false,
    // -- Fallback chain --
    editingFallback: false,
    newFallbackValue: '',

    // -- Templates state --
    tplTemplates: [],
    tplProviders: [],
    tplLoading: false,
    tplLoadError: '',
    selectedCategory: 'All',
    searchQuery: '',

    builtinTemplates: [],

    // ── Profile Descriptions ──
    profileDescriptions: {
      minimal: { label: 'Minimal', desc: 'Read-only file access' },
      coding: { label: 'Coding', desc: 'Files + shell + web fetch' },
      research: { label: 'Research', desc: 'Web search + file read/write' },
      messaging: { label: 'Messaging', desc: 'Agents + memory access' },
      automation: { label: 'Automation', desc: 'All tools except custom' },
      balanced: { label: 'Balanced', desc: 'General-purpose tool set' },
      precise: { label: 'Precise', desc: 'Focused tool set for accuracy' },
      creative: { label: 'Creative', desc: 'Full tools with creative emphasis' },
      full: { label: 'Full', desc: 'All 35+ tools' }
    },
    profileInfo: function(name) {
      return this.profileDescriptions[name] || { label: name, desc: '' };
    },

    // ── Tool Preview in Spawn Modal ──
    spawnProfiles: [],
    spawnProfilesLoaded: false,
    async loadSpawnProfiles() {
      if (this.spawnProfilesLoaded) return;
      try {
        var data = await OpenFangAPI.get('/api/profiles');
        this.spawnProfiles = data.profiles || [];
        this.spawnProfilesLoaded = true;
      } catch(e) { this.spawnProfiles = []; }
    },
    get selectedProfileTools() {
      var pname = this.spawnForm.profile;
      var match = this.spawnProfiles.find(function(p) { return p.name === pname; });
      if (match && match.tools) return match.tools.slice(0, 15);
      return [];
    },

    get agents() { return Alpine.store('app').agents; },

    /** Kernel-spawned agents for AINL allowlist probe / offline cron / IR-off allow (not user-created). */
    isAutomationProbeChatAgent(agent) {
      return isInternalAutomationProbeChatAgentName(agent && agent.name);
    },

    get chatPickerPrimaryAgents() {
      var self = this;
      return this.agents.filter(function(a) { return !self.isAutomationProbeChatAgent(a); });
    },

    get chatPickerSystemAgents() {
      var self = this;
      return this.agents.filter(function(a) { return self.isAutomationProbeChatAgent(a); });
    },

    get filteredAgents() {
      var f = this.filterState;
      if (f === 'all') return this.agents;
      return this.agents.filter(function(a) { return a.state.toLowerCase() === f; });
    },

    get runningCount() {
      return this.agents.filter(function(a) { return a.state === 'Running'; }).length;
    },

    get stoppedCount() {
      return this.agents.filter(function(a) { return a.state !== 'Running'; }).length;
    },

    // -- Templates computed --
    get categories() {
      var cats = { 'All': true };
      this.builtinTemplates.forEach(function(t) { cats[t.category] = true; });
      this.tplTemplates.forEach(function(t) { if (t.category) cats[t.category] = true; });
      return Object.keys(cats);
    },

    get filteredBuiltins() {
      var self = this;
      return this.builtinTemplates.filter(function(t) {
        if (self.selectedCategory !== 'All' && t.category !== self.selectedCategory) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if (t.name.toLowerCase().indexOf(q) === -1 &&
              t.description.toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    get filteredCustom() {
      var self = this;
      return this.tplTemplates.filter(function(t) {
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          if ((t.name || '').toLowerCase().indexOf(q) === -1 &&
              (t.description || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    isProviderConfigured(providerName) {
      if (!providerName) return false;
      var p = this.tplProviders.find(function(pr) { return pr.id === providerName; });
      return p ? p.auth_status === 'configured' : false;
    },

    async init() {
      var self = this;
      this.loading = true;
      clearPageLoadError(this);
      try {
        await Alpine.store('app').refreshAgents();
        await this.loadTemplates();
      } catch(e) {
        applyPageLoadError(this, e, 'Could not load agents. Is the daemon running?');
      }
      this.loading = false;

      // If a pending agent was set (e.g. from wizard or redirect), open chat inline
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        this.activeChatAgent = store.pendingAgent;
      }
      // Watch for future pendingAgent changes
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.activeChatAgent = agent;
        }
      });
    },

    async loadData() {
      this.loading = true;
      clearPageLoadError(this);
      try {
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        applyPageLoadError(this, e, 'Could not load agents.');
      }
      this.loading = false;
    },

    copyAgentsLoadErrorDebug() {
      copyPageLoadErrorDebug(this, 'ArmaraOS agents load error');
    },

    async loadTemplates() {
      this.tplLoading = true;
      this.tplLoadError = '';
      try {
        var results = await Promise.all([
          OpenFangAPI.get('/api/templates'),
          OpenFangAPI.get('/api/providers').catch(function() { return { providers: [] }; })
        ]);
        // Combine static and dynamic templates
        this.builtinTemplates = [
          {
            name: 'Armara',
            description: 'Personal assistant powered by AI Native Language — everyday tasks, answers, web search, building, and more.',
            category: 'General',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'full',
            system_prompt: 'You are Armara, a personal assistant powered by AI Native Language, running in ArmaraOS. Be helpful, clear, and concise. Ask clarifying questions when needed.',
            manifest_toml: 'name = "Armara"\ndescription = "Personal assistant powered by AI Native Language — everyday tasks, answers, web search, building, and more."\nmodule = "builtin:chat"\nprofile = "full"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are Armara, a personal assistant powered by AI Native Language, running in ArmaraOS. Be helpful, clear, and concise. Ask clarifying questions when needed.\n"""'
          },
          {
            name: 'Code Helper',
            description: 'A programming-focused agent that writes, reviews, and debugs code across multiple languages.',
            category: 'Development',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'coding',
            system_prompt: 'You are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.',
            manifest_toml: 'name = "Code Helper"\ndescription = "A programming-focused agent that writes, reviews, and debugs code across multiple languages."\nmodule = "builtin:chat"\nprofile = "coding"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.\n"""'
          },
          {
            name: 'Researcher',
            description: 'An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries.',
            category: 'Research',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'research',
            system_prompt: 'You are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.',
            manifest_toml: 'name = "Researcher"\ndescription = "An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries."\nmodule = "builtin:chat"\nprofile = "research"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.\n"""'
          },
          {
            name: 'Writer',
            description: 'A creative writing agent that helps with drafting, editing, and improving written content of all kinds.',
            category: 'Writing',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'full',
            system_prompt: 'You are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.',
            manifest_toml: 'name = "Writer"\ndescription = "A creative writing agent that helps with drafting, editing, and improving written content of all kinds."\nmodule = "builtin:chat"\nprofile = "full"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.\n"""'
          },
          {
            name: 'Data Analyst',
            description: 'A data-focused agent that helps analyze datasets, create queries, and interpret statistical results.',
            category: 'Development',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'coding',
            system_prompt: 'You are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.',
            manifest_toml: 'name = "Data Analyst"\ndescription = "A data-focused agent that helps analyze datasets, create queries, and interpret statistical results."\nmodule = "builtin:chat"\nprofile = "coding"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.\n"""'
          },
          {
            name: 'DevOps Engineer',
            description: 'A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting.',
            category: 'Development',
            provider: SPAWN_DEFAULT_PROVIDER,
            model: SPAWN_DEFAULT_MODEL,
            profile: 'automation',
            system_prompt: 'You are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.',
            manifest_toml: 'name = "DevOps Engineer"\ndescription = "A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting."\nmodule = "builtin:chat"\nprofile = "automation"\n\n[model]\nprovider = "openrouter"\nmodel = "stepfun/step-3.5-flash:free"\nsystem_prompt = """\nYou are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.\n"""'
          },
          ...results[0].templates || []
        ];
        this.tplProviders = results[1].providers || [];
      } catch(e) {
        this.builtinTemplates = [];
        this.tplLoadError = e.message || 'Could not load templates.';
      }
      this.tplLoading = false;
    },

    chatWithAgent(agent) {
      Alpine.store('app').pendingAgent = agent;
      this.activeChatAgent = agent;
    },

    closeChat() {
      this.activeChatAgent = null;
      try {
        var st = Alpine.store('app');
        st.pendingAgent = null;
        st.agentsPageChatAgentId = null;
      } catch(e) { /* ignore */ }
      OpenFangAPI.wsDisconnect();
    },

    /** Called before leaving the Agents page (sidebar / hash). Detach chat handlers but keep WS alive for unread + smooth return. */
    onAgentsPageLeave() {
      this.activeChatAgent = null;
      try {
        var st = Alpine.store('app');
        st.pendingAgent = null;
        st.agentsPageChatAgentId = null;
      } catch(e) { /* ignore */ }
      try {
        OpenFangAPI.wsClearUiCallbacks();
      } catch (e2) { /* ignore */ }
    },

    /** Map stored archetype strings onto Config tab select values (lowercase canonical ids). */
    normalizeArchetypeForUi(raw) {
      if (raw == null || raw === '') return '';
      var lower = String(raw).trim().toLowerCase();
      if (!lower) return '';
      var canonical = ['assistant', 'researcher', 'coder', 'writer', 'devops', 'support', 'analyst'];
      if (canonical.indexOf(lower) >= 0) return lower;
      return lower;
    },

    applyAgentDetailFromListAgent(agent) {
      if (!this.detailAgent) return;
      this.detailAgent._fallbacks = [];
      this.detailAgent.turn_stats = null;
      this.detailAgent.scheduled_ainl_host_adapter = null;
      var idn = (agent && agent.identity) || {};
      this.configForm = {
        name: (agent && agent.name) || '',
        system_prompt: agent && agent.system_prompt != null ? agent.system_prompt : '',
        emoji: idn.emoji || '',
        color: idn.color || '#FF5C00',
        archetype: this.normalizeArchetypeForUi(idn.archetype),
        vibe: idn.vibe || ''
      };
      this.toolFilters = { tool_allowlist: [], tool_blocklist: [] };
    },

    /** Apply GET /api/agents/:id payload into detail view, config form, and tool filters. */
    applyAgentDetail(full) {
      if (!full || !full.id) return;
      var idn = full.identity || {};
      var m = full.model || {};
      this.detailAgent = Object.assign({}, this.detailAgent || {}, {
        id: full.id,
        name: full.name,
        state: full.state,
        mode: full.mode,
        model_provider: m.provider,
        model_name: m.model,
        profile: full.profile,
        identity: full.identity,
        system_prompt: full.system_prompt
      });
      this.detailAgent._fallbacks = full.fallback_models || [];
      this.detailAgent.turn_stats = full.turn_stats || null;
      this.detailAgent.scheduled_ainl_host_adapter = full.scheduled_ainl_host_adapter || null;
      this.configForm = {
        name: full.name || '',
        system_prompt: full.system_prompt != null ? String(full.system_prompt) : '',
        emoji: idn.emoji || '',
        color: idn.color || '#FF5C00',
        archetype: this.normalizeArchetypeForUi(idn.archetype),
        vibe: idn.vibe || ''
      };
      this.toolFilters = {
        tool_allowlist: (full.tool_allowlist || []).slice(),
        tool_blocklist: (full.tool_blocklist || []).slice()
      };
    },

    formatTurnErrorRate(ts) {
      if (!ts || ts.error_rate == null) return '—';
      if (typeof ts.error_rate === 'number' && !isNaN(ts.error_rate)) {
        return (ts.error_rate * 100).toFixed(1) + '%';
      }
      return String(ts.error_rate);
    },

    formatIsoTime(iso) {
      if (!iso) return '—';
      try {
        var d = new Date(iso);
        return isNaN(d.getTime()) ? String(iso) : d.toLocaleString();
      } catch(e) { return String(iso); }
    },

    async showDetail(agent) {
      this.detailAgent = Object.assign({}, agent);
      this.detailTab = 'info';
      this.agentFiles = [];
      this.editingFile = null;
      this.fileContent = '';
      this.editingFallback = false;
      this.newFallbackValue = '';
      this.applyAgentDetailFromListAgent(agent);
      this.showDetailModal = true;
      try {
        var full = await OpenFangAPI.get('/api/agents/' + agent.id);
        this.applyAgentDetail(full);
      } catch (e) { /* keep list-based fallback */ }
    },

    killAgent(agent) {
      var self = this;
      OpenFangToast.confirm('Stop Agent', 'Stop agent "' + agent.name + '"? The agent will be shut down.', async function() {
        try {
          await OpenFangAPI.del('/api/agents/' + agent.id);
          OpenFangToast.success('Agent "' + agent.name + '" stopped');
          self.showDetailModal = false;
          await Alpine.store('app').refreshAgents();
        } catch(e) {
          OpenFangToast.error('Failed to stop agent: ' + e.message);
        }
      });
    },

    killAllAgents() {
      var list = this.filteredAgents;
      if (!list.length) return;
      OpenFangToast.confirm('Stop All Agents', 'Stop ' + list.length + ' agent(s)? All agents will be shut down.', async function() {
        var errors = [];
        for (var i = 0; i < list.length; i++) {
          try {
            await OpenFangAPI.del('/api/agents/' + list[i].id);
          } catch(e) { errors.push(list[i].name + ': ' + e.message); }
        }
        await Alpine.store('app').refreshAgents();
        if (errors.length) {
          OpenFangToast.error('Some agents failed to stop: ' + errors.join(', '));
        } else {
          OpenFangToast.success(list.length + ' agent(s) stopped');
        }
      });
    },

    // ── Multi-step wizard navigation ──
    async openSpawnWizard() {
      this.showSpawnModal = true;
      this.spawnStep = 1;
      this.spawnMode = 'wizard';
      this.spawnIdentity = { emoji: '', color: '#FF5C00', archetype: '' };
      this.selectedPreset = '';
      this.soulContent = '';
      this.spawnForm.name = '';
      this.spawnForm.provider = SPAWN_DEFAULT_PROVIDER;
      this.spawnForm.model = SPAWN_DEFAULT_MODEL;
      this.spawnForm.systemPrompt = 'You are a helpful assistant.';
      this.spawnForm.profile = 'full';
      this.spawnProvidersLoading = true;
      try {
        var provData = await OpenFangAPI.get('/api/providers').catch(function() { return { providers: [] }; });
        this.spawnProviders = provData.providers || [];
      } catch(e) {
        this.spawnProviders = [];
      }
      this.spawnProvidersLoading = false;
    },

    nextStep() {
      if (this.spawnStep === 1 && !this.spawnForm.name.trim()) {
        OpenFangToast.warn('Please enter an agent name');
        return;
      }
      if (this.spawnStep < 5) this.spawnStep++;
    },

    prevStep() {
      if (this.spawnStep > 1) this.spawnStep--;
    },

    selectPreset(preset) {
      this.selectedPreset = preset.id;
      this.soulContent = preset.soul;
    },

    generateToml() {
      var f = this.spawnForm;
      var si = this.spawnIdentity;
      var lines = [
        'name = "' + tomlBasicEscape(f.name) + '"',
        'module = "builtin:chat"'
      ];
      if (f.profile && f.profile !== 'custom') {
        lines.push('profile = "' + f.profile + '"');
      }
      lines.push('', '[model]');
      lines.push('provider = "' + f.provider + '"');
      lines.push('model = "' + f.model + '"');
      lines.push('system_prompt = """\n' + tomlMultilineEscape(f.systemPrompt) + '\n"""');
      if (f.profile === 'custom') {
        lines.push('', '[capabilities]');
        if (f.caps.memory_read) lines.push('memory_read = ["*"]');
        if (f.caps.memory_write) lines.push('memory_write = ["self.*"]');
        if (f.caps.network) lines.push('network = ["*"]');
        if (f.caps.shell) lines.push('shell = ["*"]');
        if (f.caps.agent_spawn) lines.push('agent_spawn = true');
      }
      return lines.join('\n');
    },

    async setMode(agent, mode) {
      try {
        await OpenFangAPI.put('/api/agents/' + agent.id + '/mode', { mode: mode });
        agent.mode = mode;
        OpenFangToast.success('Mode set to ' + mode);
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenFangToast.error('Failed to set mode: ' + e.message);
      }
    },

    async spawnAgent() {
      this.spawning = true;
      var toml = this.spawnMode === 'wizard' ? this.generateToml() : this.spawnToml;
      var chatModel = spawnModelFromManifestToml(toml);
      if (!toml.trim()) {
        this.spawning = false;
        OpenFangToast.warn('Manifest is empty \u2014 enter agent config first');
        return;
      }

      try {
        var res = await OpenFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          // Post-spawn: update identity + write SOUL.md if personality preset selected
          var patchBody = {};
          if (this.spawnIdentity.emoji) patchBody.emoji = this.spawnIdentity.emoji;
          if (this.spawnIdentity.color) patchBody.color = this.spawnIdentity.color;
          if (this.spawnIdentity.archetype) patchBody.archetype = this.spawnIdentity.archetype;
          if (this.selectedPreset) patchBody.vibe = this.selectedPreset;

          if (Object.keys(patchBody).length) {
            OpenFangAPI.patch('/api/agents/' + res.agent_id + '/config', patchBody).catch(function(e) { console.warn('Post-spawn config patch failed:', e.message); });
          }
          if (this.soulContent.trim()) {
            OpenFangAPI.put('/api/agents/' + res.agent_id + '/files/SOUL.md', { content: '# Soul\n' + this.soulContent }).catch(function(e) { console.warn('SOUL.md write failed:', e.message); });
          }

          this.showSpawnModal = false;
          this.spawnForm.name = '';
          this.spawnToml = '';
          this.spawnStep = 1;
          OpenFangToast.success('Agent "' + (res.name || 'new') + '" spawned');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({
            id: res.agent_id,
            name: res.name,
            model_provider: chatModel.provider,
            model_name: chatModel.model
          });
        } else {
          OpenFangToast.error('Spawn failed: ' + (res.error || 'Unknown error'));
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn agent: ' + e.message);
      }
      this.spawning = false;
    },

    // ── Detail modal: Files tab ──
    async loadAgentFiles() {
      if (!this.detailAgent) return;
      this.filesLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/files');
        this.agentFiles = data.files || [];
      } catch(e) {
        this.agentFiles = [];
        OpenFangToast.error('Failed to load files: ' + e.message);
      }
      this.filesLoading = false;
    },

    async openFile(file) {
      if (!file.exists) {
        // Create with empty content
        this.editingFile = file.name;
        this.fileContent = '';
        return;
      }
      try {
        var data = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(file.name));
        this.editingFile = file.name;
        this.fileContent = data.content || '';
      } catch(e) {
        OpenFangToast.error('Failed to read file: ' + e.message);
      }
    },

    async saveFile() {
      if (!this.editingFile || !this.detailAgent) return;
      this.fileSaving = true;
      try {
        await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/files/' + encodeURIComponent(this.editingFile), { content: this.fileContent });
        OpenFangToast.success(this.editingFile + ' saved');
        await this.loadAgentFiles();
      } catch(e) {
        OpenFangToast.error('Failed to save file: ' + e.message);
      }
      this.fileSaving = false;
    },

    closeFileEditor() {
      this.editingFile = null;
      this.fileContent = '';
    },

    // ── Detail modal: Config tab ──
    async saveConfig() {
      if (!this.detailAgent) return;
      this.configSaving = true;
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', this.configForm);
        OpenFangToast.success('Config updated');
        try {
          var full = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id);
          this.applyAgentDetail(full);
        } catch (e2) { /* ignore */ }
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenFangToast.error('Failed to save config: ' + e.message);
      }
      this.configSaving = false;
    },

    // ── Clone agent ──
    async cloneAgent(agent) {
      var newName = (agent.name || 'agent') + '-copy';
      try {
        var res = await OpenFangAPI.post('/api/agents/' + agent.id + '/clone', { new_name: newName });
        if (res.agent_id) {
          OpenFangToast.success('Cloned as "' + res.name + '"');
          await Alpine.store('app').refreshAgents();
          this.showDetailModal = false;
        }
      } catch(e) {
        OpenFangToast.error('Clone failed: ' + e.message);
      }
    },

    // -- Template methods --
    async spawnFromTemplate(template) {
      try {
        var manifestToml = template.manifest_toml;
        if (!manifestToml) {
          // If template doesn't have manifest_toml, fetch it from the API
          var data = await OpenFangAPI.get('/api/templates/' + encodeURIComponent(template.name));
          manifestToml = data.manifest_toml;
        }
        if (manifestToml) {
          var res = await OpenFangAPI.post('/api/agents', { manifest_toml: manifestToml });
          if (res.agent_id) {
            var mm = spawnModelFromManifestToml(manifestToml);
            OpenFangToast.success('Agent "' + (res.name || template.name) + '" spawned from template');
            await Alpine.store('app').refreshAgents();
            this.chatWithAgent({
              id: res.agent_id,
              name: res.name || template.name,
              model_provider: mm.provider,
              model_name: mm.model
            });
          }
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn from template: ' + e.message);
      }
    },

    // ── Clear agent history ──
    async clearHistory(agent) {
      var self = this;
      OpenFangToast.confirm('Clear History', 'Clear all conversation history for "' + agent.name + '"? This cannot be undone.', async function() {
        try {
          await OpenFangAPI.del('/api/agents/' + agent.id + '/history');
          OpenFangToast.success('History cleared for "' + agent.name + '"');
        } catch(e) {
          OpenFangToast.error('Failed to clear history: ' + e.message);
        }
      });
    },

    // ── Model switch ──
    async changeModel() {
      if (!this.detailAgent || !this.newModelValue.trim()) return;
      this.modelSaving = true;
      try {
        var resp = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: this.newModelValue.trim() });
        var providerInfo = (resp && resp.provider) ? ' (provider: ' + resp.provider + ')' : '';
        OpenFangToast.success('Model changed' + providerInfo + ' (memory reset)');
        this.editingModel = false;
        await Alpine.store('app').refreshAgents();
        // Refresh detailAgent
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        OpenFangToast.error('Failed to change model: ' + e.message);
      }
      this.modelSaving = false;
    },

    // ── Provider switch ──
    async changeProvider() {
      if (!this.detailAgent || !this.newProviderValue.trim()) return;
      this.modelSaving = true;
      try {
        var combined = this.newProviderValue.trim() + '/' + this.detailAgent.model_name;
        var resp = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: combined });
        OpenFangToast.success('Provider changed to ' + (resp && resp.provider ? resp.provider : this.newProviderValue.trim()));
        this.editingProvider = false;
        await Alpine.store('app').refreshAgents();
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        OpenFangToast.error('Failed to change provider: ' + e.message);
      }
      this.modelSaving = false;
    },

    // ── Fallback model chain ──
    async addFallback() {
      if (!this.detailAgent || !this.newFallbackValue.trim()) return;
      var parts = this.newFallbackValue.trim().split('/');
      var provider = parts.length > 1 ? parts[0] : this.detailAgent.model_provider;
      var model = parts.length > 1 ? parts.slice(1).join('/') : parts[0];
      if (!this.detailAgent._fallbacks) this.detailAgent._fallbacks = [];
      this.detailAgent._fallbacks.push({ provider: provider, model: model });
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        OpenFangToast.success('Fallback added: ' + provider + '/' + model);
      } catch(e) {
        OpenFangToast.error('Failed to save fallbacks: ' + e.message);
        this.detailAgent._fallbacks.pop();
      }
      this.editingFallback = false;
      this.newFallbackValue = '';
    },

    async removeFallback(idx) {
      if (!this.detailAgent || !this.detailAgent._fallbacks) return;
      var removed = this.detailAgent._fallbacks.splice(idx, 1);
      try {
        await OpenFangAPI.patch('/api/agents/' + this.detailAgent.id + '/config', {
          fallback_models: this.detailAgent._fallbacks
        });
        OpenFangToast.success('Fallback removed');
      } catch(e) {
        OpenFangToast.error('Failed to save fallbacks: ' + e.message);
        this.detailAgent._fallbacks.splice(idx, 0, removed[0]);
      }
    },

    // ── Tool filters ──
    async loadToolFilters() {
      if (!this.detailAgent) return;
      this.toolFiltersLoading = true;
      try {
        this.toolFilters = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id + '/tools');
      } catch(e) {
        this.toolFilters = { tool_allowlist: [], tool_blocklist: [] };
      }
      this.toolFiltersLoading = false;
    },

    addAllowTool() {
      var t = this.newAllowTool.trim();
      if (t && this.toolFilters.tool_allowlist.indexOf(t) === -1) {
        this.toolFilters.tool_allowlist.push(t);
        this.newAllowTool = '';
        this.saveToolFilters();
      }
    },

    removeAllowTool(tool) {
      this.toolFilters.tool_allowlist = this.toolFilters.tool_allowlist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    addBlockTool() {
      var t = this.newBlockTool.trim();
      if (t && this.toolFilters.tool_blocklist.indexOf(t) === -1) {
        this.toolFilters.tool_blocklist.push(t);
        this.newBlockTool = '';
        this.saveToolFilters();
      }
    },

    removeBlockTool(tool) {
      this.toolFilters.tool_blocklist = this.toolFilters.tool_blocklist.filter(function(t) { return t !== tool; });
      this.saveToolFilters();
    },

    async saveToolFilters() {
      if (!this.detailAgent) return;
      try {
        await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/tools', this.toolFilters);
      } catch(e) {
        OpenFangToast.error('Failed to update tool filters: ' + e.message);
      }
    },

    /** Add channel_send + event_publish: append to allowlist when restricting tools; always unblock. */
    ensureMessagingTools() {
      var allow = this.toolFilters.tool_allowlist || [];
      var block = (this.toolFilters.tool_blocklist || []).filter(function(t) {
        return t !== 'channel_send' && t !== 'event_publish';
      });
      if (allow.length > 0) {
        ['channel_send', 'event_publish'].forEach(function(t) {
          if (allow.indexOf(t) === -1) allow.push(t);
        });
      }
      this.toolFilters.tool_allowlist = allow;
      this.toolFilters.tool_blocklist = block;
      this.saveToolFilters();
    },

    async spawnBuiltin(t) {
      var toml = 'name = "' + tomlBasicEscape(t.name) + '"\n';
      toml += 'description = "' + tomlBasicEscape(t.description) + '"\n';
      toml += 'module = "builtin:chat"\n';
      toml += 'profile = "' + t.profile + '"\n\n';
      toml += '[model]\nprovider = "' + t.provider + '"\nmodel = "' + t.model + '"\n';
      toml += 'system_prompt = """\n' + tomlMultilineEscape(t.system_prompt) + '\n"""\n';

      try {
        var res = await OpenFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          OpenFangToast.success('Agent "' + t.name + '" spawned');
          await Alpine.store('app').refreshAgents();
          this.chatWithAgent({ id: res.agent_id, name: t.name, model_provider: t.provider, model_name: t.model });
        }
      } catch(e) {
        OpenFangToast.error('Failed to spawn agent: ' + e.message);
      }
    }
  };
}
