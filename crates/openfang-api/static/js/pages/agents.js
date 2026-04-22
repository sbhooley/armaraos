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

/** Fast client checks before spawn or PUT /update (server still validates). */
function validateManifestToml(toml) {
  var s = (toml || '').trim();
  if (!s) return 'Manifest is empty.';
  if (!/^\s*name\s*=/m.test(s)) return 'Manifest must include a top-level name = line.';
  var triples = (s.match(/"""/g) || []).length;
  if (triples % 2 !== 0) return 'Unclosed triple-quoted string (""") in manifest.';
  return '';
}

var SPAWN_DEFAULT_PROVIDER = 'openrouter';
var SPAWN_DEFAULT_MODEL = 'elephant-alpha';

function spawnModelFromManifestToml(toml) {
  var provider = '?';
  var model = '?';
  var mp = toml.match(/provider\s*=\s*"([^"]*)"/);
  var mm = toml.match(/model\s*=\s*"([^"]*)"/);
  if (mp) provider = mp[1];
  if (mm) model = mm[1];
  return { provider: provider, model: model };
}

/** Built-in quick-start cards on Chat → All Agents (do not depend on GET /api/templates). */
function staticChatBuiltinTemplates() {
  return [
    {
      name: 'Armara',
      description: 'Personal assistant powered by AI Native Language — everyday tasks, answers, web search, building, and more.',
      category: 'General',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'full',
      system_prompt: 'You are Armara, a personal assistant powered by AI Native Language, running in ArmaraOS. Be helpful, clear, and concise. Ask clarifying questions when needed.',
      manifest_toml: 'name = "Armara"\ndescription = "Personal assistant powered by AI Native Language — everyday tasks, answers, web search, building, and more."\nmodule = "builtin:chat"\nprofile = "full"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are Armara, a personal assistant powered by AI Native Language, running in ArmaraOS. Be helpful, clear, and concise. Ask clarifying questions when needed.\n"""'
    },
    {
      name: 'Code Helper',
      description: 'A programming-focused agent that writes, reviews, and debugs code across multiple languages.',
      category: 'Development',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'coding',
      system_prompt: 'You are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.',
      manifest_toml: 'name = "Code Helper"\ndescription = "A programming-focused agent that writes, reviews, and debugs code across multiple languages."\nmodule = "builtin:chat"\nprofile = "coding"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.\n"""'
    },
    {
      name: 'Researcher',
      description: 'An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries.',
      category: 'Research',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'research',
      system_prompt: 'You are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.',
      manifest_toml: 'name = "Researcher"\ndescription = "An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries."\nmodule = "builtin:chat"\nprofile = "research"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.\n"""'
    },
    {
      name: 'Writer',
      description: 'A creative writing agent that helps with drafting, editing, and improving written content of all kinds.',
      category: 'Writing',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'full',
      system_prompt: 'You are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.',
      manifest_toml: 'name = "Writer"\ndescription = "A creative writing agent that helps with drafting, editing, and improving written content of all kinds."\nmodule = "builtin:chat"\nprofile = "full"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.\n"""'
    },
    {
      name: 'Data Analyst',
      description: 'A data-focused agent that helps analyze datasets, create queries, and interpret statistical results.',
      category: 'Development',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'coding',
      system_prompt: 'You are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.',
      manifest_toml: 'name = "Data Analyst"\ndescription = "A data-focused agent that helps analyze datasets, create queries, and interpret statistical results."\nmodule = "builtin:chat"\nprofile = "coding"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.\n"""'
    },
    {
      name: 'DevOps Engineer',
      description: 'A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting.',
      category: 'Development',
      provider: SPAWN_DEFAULT_PROVIDER,
      model: SPAWN_DEFAULT_MODEL,
      profile: 'automation',
      system_prompt: 'You are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.',
      manifest_toml: 'name = "DevOps Engineer"\ndescription = "A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting."\nmodule = "builtin:chat"\nprofile = "automation"\n\n[model]\nprovider = "openrouter"\nmodel = "elephant-alpha"\nsystem_prompt = """\nYou are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.\n"""'
    }
  ];
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
    fullManifestToml: '',
    fullManifestLoading: false,
    fullManifestSaving: false,
    fullManifestSectionOpen: false,
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

    builtinTemplates: staticChatBuiltinTemplates(),

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

    fleetPinnedAgentIds: function() {
      try {
        var app = Alpine.store('app');
        var ids = app && Array.isArray(app.pinnedAgentIds) ? app.pinnedAgentIds : [];
        return ids.map(function(x) { return String(x); });
      } catch (e) { return []; }
    },

    sortFleetAgents: function(list) {
      var src = Array.isArray(list) ? list.slice() : [];
      var pinned = this.fleetPinnedAgentIds();
      var rank = {};
      for (var i = 0; i < pinned.length; i++) rank[pinned[i]] = i;
      return src.sort(function(a, b) {
        var aid = String((a && a.id) || '');
        var bid = String((b && b.id) || '');
        var ap = Object.prototype.hasOwnProperty.call(rank, aid);
        var bp = Object.prototype.hasOwnProperty.call(rank, bid);
        if (ap && bp) return rank[aid] - rank[bid];
        if (ap) return -1;
        if (bp) return 1;
        var an = String((a && a.name) || '').toLowerCase();
        var bn = String((b && b.name) || '').toLowerCase();
        var nc = an.localeCompare(bn);
        if (nc !== 0) return nc;
        return aid.localeCompare(bid);
      });
    },

    get chatPickerPrimaryAgents() {
      var self = this;
      return this.sortFleetAgents(this.agents.filter(function(a) { return !self.isAutomationProbeChatAgent(a); }));
    },

    get chatPickerSystemAgents() {
      var self = this;
      return this.sortFleetAgents(this.agents.filter(function(a) { return self.isAutomationProbeChatAgent(a); }));
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

    // ── Fleet vitals (All Agents card grid) ──
    demoMode: false,
    demoProfile: 'standard',
    displayActiveCount: 0,
    tasksToday: 0,
    fleetGraphNodeTotal: 0,
    fleetSpendHour: 0,
    fleetSpendToday: 0,
    fleetSavedHour: 0,
    fleetSavedToday: 0,
    fleetLastCost: null,
    fleetLastSaved: null,
    fleetLastGraphNodeTotal: null,
    _fleetLastSummaryCalls: 0,
    _fleetSpendsMs: [],
    _fleetSpendsAmt: [],
    _fleetSavedsMs: [],
    _fleetSavedsAmt: [],
    _fleetTasksByHour: {},
    _fleetNodesByHour: {},
    _fleetSpendByHour: {},
    _fleetSavedByHour: {},
    _fleetInt: null,
    _fleetDemoInt: null,
    _activitySyncInt: null,
    _idlePulseInt: null,
    _activePulseInt: null,
    _demoKey: null,
    _tweenRaf: null,
    _fleetInFlight: false,
    _graphGen: 0,
    _prevTasksToday: null,
    _prevToolByAgent: {},
    _prevCallByAgent: {},
    _usageCountersInitialized: false,
    _prevGraphNodeByAgent: {},
    _prevPhaseByAgent: {},
    _activitySeenTsByAgent: {},
    _agentStatusByHour: {},
    _agentToolByHour: {},
    _agentNodeByHour: {},
    _agentHourlyPersistTimer: null,
    _hourlyUiPrefsPersistTimer: null,
    usageByAgent: {},
    graphVitalsByAgent: {},
    graphNodeCountByAgent: {},
    agentPinging: {},
    activityFeedByAgent: {},
    _idlePulseTick: 0,

    /** Usage series for small sparkline / activity (0–1 normalized). */
    fleetActivityNorm: 0.08,
    perAgentActivityNorm: {},

    normalizeDemoProfile(raw) {
      var p = raw != null ? String(raw).trim().toLowerCase() : '';
      if (p === 'cinema' || p === 'film') return 'cinema';
      return 'standard';
    },

    readFleetDemoProfileFromEnv() {
      var profile = 'standard';
      try {
        var p = (typeof location !== 'undefined' && location.search) ? new URLSearchParams(location.search) : null;
        if (p) {
          var viaProfile = p.get('demoProfile');
          if (viaProfile) profile = this.normalizeDemoProfile(viaProfile);
          var viaDemo = p.get('demo') || p.get('demoMode');
          if (viaDemo && !/^(1|true|yes|on)$/i.test(String(viaDemo).trim())) {
            profile = this.normalizeDemoProfile(viaDemo);
          }
        }
      } catch (e0) { /* ignore */ }
      try {
        var ls = localStorage.getItem('armaraos-fleet-demo-profile');
        if (ls && profile === 'standard') profile = this.normalizeDemoProfile(ls);
      } catch (e1) { /* ignore */ }
      return profile;
    },

    readFleetDemoFromEnv() {
      var demo = false;
      try {
        var p = (typeof location !== 'undefined' && location.search) ? new URLSearchParams(location.search) : null;
        if (p) {
          var d = p.get('demo') || p.get('demoMode');
          if (d && (/^(1|true|yes|on)$/i.test(String(d).trim()) || this.normalizeDemoProfile(d) === 'cinema')) demo = true;
        }
      } catch (e0) { /* ignore */ }
      try {
        if (!demo && localStorage.getItem('armaraos-fleet-demo') === '1') demo = true;
      } catch (e1) { /* ignore */ }
      return demo;
    },

    applyDemoThemeIfNeeded() {
      if (!this.demoMode) return;
      try {
        localStorage.setItem('armaraos-theme-mode', 'dark');
        document.documentElement.setAttribute('data-theme', 'dark');
      } catch (e) { /* ignore */ }
    },

    toggleFleetDemoMode() {
      this.demoMode = !this.demoMode;
      try {
        localStorage.setItem('armaraos-fleet-demo', this.demoMode ? '1' : '0');
        localStorage.setItem('armaraos-fleet-demo-profile', this.demoProfile || 'standard');
      } catch (e) { /* ignore */ }
      this.applyDemoThemeIfNeeded();
      if (this.demoMode) {
        this.fleetStartDemo();
      } else if (this._fleetDemoInt) {
        try { clearInterval(this._fleetDemoInt); } catch (e2) { /* ignore */ }
        this._fleetDemoInt = null;
      }
    },

    cycleFleetDemoProfile() {
      this.demoProfile = this.demoProfile === 'cinema' ? 'standard' : 'cinema';
      this.demoMode = true;
      try {
        localStorage.setItem('armaraos-fleet-demo', '1');
        localStorage.setItem('armaraos-fleet-demo-profile', this.demoProfile);
      } catch (e) { /* ignore */ }
      this.applyDemoThemeIfNeeded();
      this.fleetStartDemo();
    },

    isDemoPresetActive(preset) {
      var norm = this.normalizeDemoProfile(preset);
      return this.demoMode && this.demoProfile === norm;
    },

    applyDemoPresetUrl(preset) {
      var norm = this.normalizeDemoProfile(preset);
      try {
        var qp = new URLSearchParams((typeof location !== 'undefined' && location.search) ? location.search : '');
        if (norm === 'cinema') {
          qp.set('demo', 'cinema');
          qp.delete('demoProfile');
          qp.delete('demoMode');
        } else {
          qp.set('demo', '1');
          qp.delete('demoProfile');
          qp.delete('demoMode');
        }
        var q = qp.toString();
        var base = (typeof location !== 'undefined') ? location.pathname : '';
        var h = (typeof location !== 'undefined') ? (location.hash || '') : '';
        var nextUrl = base + (q ? ('?' + q) : '') + h;
        if (typeof history !== 'undefined' && history.replaceState) {
          history.replaceState({}, '', nextUrl);
        }
      } catch (e) { /* ignore */ }

      this.demoProfile = norm;
      this.demoMode = true;
      try {
        localStorage.setItem('armaraos-fleet-demo', '1');
        localStorage.setItem('armaraos-fleet-demo-profile', this.demoProfile);
      } catch (e2) { /* ignore */ }
      this.applyDemoThemeIfNeeded();
      this.fleetStartDemo();
    },

    isAgentRunningState(agent) {
      return agent && String(agent.state) === 'Running';
    },

    isReducedMotion() {
      try {
        return window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      } catch (e) { return false; }
    },

    _pad2: function(n) { return n < 10 ? '0' + n : String(n); },
    todayIsoYmd: function() {
      var d = new Date();
      return d.getFullYear() + '-' + this._pad2(d.getMonth() + 1) + '-' + this._pad2(d.getDate());
    },

    fleetHourBucket: function(ts) {
      return Math.floor((ts || Date.now()) / 3600000);
    },

    fleetHourSlots: function() {
      var cur = this.fleetHourBucket(Date.now());
      var out = [];
      for (var i = 7; i >= 0; i--) out.push(cur - i);
      return out;
    },

    hourSlots: function(hours) {
      var h = Math.max(1, Math.floor(Number(hours) || 1));
      var cur = this.fleetHourBucket(Date.now());
      var out = [];
      for (var i = h - 1; i >= 0; i--) out.push(cur - i);
      return out;
    },

    fleetHourLabel: function(bucket) {
      var d = new Date(Number(bucket) * 3600000);
      return this._pad2(d.getHours()) + ':00';
    },

    fleetPruneHourlySeries: function(m) {
      var src = m || {};
      var cur = this.fleetHourBucket(Date.now());
      var min = cur - 23;
      var out = {};
      for (var k in src) {
        if (!Object.prototype.hasOwnProperty.call(src, k)) continue;
        var n = Number(k);
        if (!isFinite(n)) continue;
        if (n >= min && n <= cur) out[String(n)] = Number(src[k]) || 0;
      }
      return out;
    },

    pruneHourlySeries: function(m, keepHours) {
      var src = m || {};
      var cur = this.fleetHourBucket(Date.now());
      var min = cur - Math.max(1, Math.floor(Number(keepHours) || 24)) + 1;
      var out = {};
      for (var k in src) {
        if (!Object.prototype.hasOwnProperty.call(src, k)) continue;
        var n = Number(k);
        if (!isFinite(n)) continue;
        if (n >= min && n <= cur) out[String(n)] = Number(src[k]) || 0;
      }
      return out;
    },

    fleetRecordHourly: function(m, amount, ts) {
      var out = this.fleetPruneHourlySeries(m);
      var key = String(this.fleetHourBucket(ts));
      out[key] = (Number(out[key]) || 0) + (Number(amount) || 0);
      return out;
    },

    recordHourly: function(m, amount, ts, keepHours) {
      var out = this.pruneHourlySeries(m, keepHours);
      var key = String(this.fleetHourBucket(ts));
      out[key] = (Number(out[key]) || 0) + (Number(amount) || 0);
      return out;
    },

    fleetSeriesTodayTotal: function(m) {
      var src = m || {};
      var d = new Date();
      d.setHours(0, 0, 0, 0);
      var start = Math.floor(d.getTime() / 3600000);
      var sum = 0;
      for (var k in src) {
        if (!Object.prototype.hasOwnProperty.call(src, k)) continue;
        var n = Number(k);
        if (!isFinite(n) || n < start) continue;
        sum += Number(src[k]) || 0;
      }
      return Math.max(0, sum);
    },

    fleetMetricHourMap: function(kind) {
      if (kind === 'tasks') return this._fleetTasksByHour || {};
      if (kind === 'nodes') return this._fleetNodesByHour || {};
      if (kind === 'spend') return this._fleetSpendByHour || {};
      if (kind === 'saved') return this._fleetSavedByHour || {};
      return {};
    },

    fleetMetricHourValueText: function(kind, v) {
      var n = Number(v) || 0;
      if (kind === 'spend' || kind === 'saved') return '$' + (Math.round(n * 10000) / 10000).toFixed(4);
      return String(Math.round(n));
    },

    fleetMetricBars: function(kind) {
      var map = this.fleetMetricHourMap(kind);
      var slots = this.fleetHourSlots();
      var vals = slots.map(function(b) { return Math.max(0, Number(map[String(b)]) || 0); });
      var maxv = 0;
      for (var i = 0; i < vals.length; i++) maxv = Math.max(maxv, vals[i]);
      if (maxv <= 0) maxv = 1;
      var self = this;
      return slots.map(function(bucket, idx) {
        var v = vals[idx];
        var pct = (v <= 0) ? 10 : Math.max(16, Math.round((v / maxv) * 100));
        return {
          key: String(kind) + ':' + String(bucket),
          bucket: bucket,
          value: v,
          pct: pct,
          isCurrent: idx === slots.length - 1,
          title: self.fleetHourLabel(bucket) + ' · ' + self.fleetMetricHourValueText(kind, v)
        };
      });
    },

    schedulePersistAgentHourly: function() {
      var self = this;
      if (this._agentHourlyPersistTimer) return;
      this._agentHourlyPersistTimer = setTimeout(function() {
        self._agentHourlyPersistTimer = null;
        try {
          localStorage.setItem('armaraos-agent-hourly-v1', JSON.stringify({
            status: self._agentStatusByHour || {},
            tool: self._agentToolByHour || {},
            node: self._agentNodeByHour || {},
            saved_at: Date.now()
          }));
        } catch (e) { /* ignore */ }
        try {
          localStorage.setItem('armaraos-fleet-hourly-v1', JSON.stringify({
            tasks: self._fleetTasksByHour || {},
            nodes: self._fleetNodesByHour || {},
            spend: self._fleetSpendByHour || {},
            saved: self._fleetSavedByHour || {},
            saved_at: Date.now()
          }));
        } catch (e2) { /* ignore */ }
        self.schedulePersistHourlyUiPrefs();
      }, 300);
    },

    loadPersistedAgentHourly: function() {
      try {
        var raw = localStorage.getItem('armaraos-agent-hourly-v1');
        if (!raw) return;
        var obj = JSON.parse(raw);
        this._agentStatusByHour = obj && obj.status ? obj.status : {};
        this._agentToolByHour = obj && obj.tool ? obj.tool : {};
        this._agentNodeByHour = obj && obj.node ? obj.node : {};
      } catch (e) {
        this._agentStatusByHour = {};
        this._agentToolByHour = {};
        this._agentNodeByHour = {};
      }
      try {
        var rawFleet = localStorage.getItem('armaraos-fleet-hourly-v1');
        if (rawFleet) {
          var f = JSON.parse(rawFleet);
          this._fleetTasksByHour = f && f.tasks ? f.tasks : {};
          this._fleetNodesByHour = f && f.nodes ? f.nodes : {};
          this._fleetSpendByHour = f && f.spend ? f.spend : {};
          this._fleetSavedByHour = f && f.saved ? f.saved : {};
        }
      } catch (e3) {
        this._fleetTasksByHour = this._fleetTasksByHour || {};
        this._fleetNodesByHour = this._fleetNodesByHour || {};
        this._fleetSpendByHour = this._fleetSpendByHour || {};
        this._fleetSavedByHour = this._fleetSavedByHour || {};
      }
    },

    schedulePersistHourlyUiPrefs: function() {
      var self = this;
      if (this._hourlyUiPrefsPersistTimer) return;
      this._hourlyUiPrefsPersistTimer = setTimeout(function() {
        self._hourlyUiPrefsPersistTimer = null;
        self.persistHourlyUiPrefs();
      }, 2200);
    },

    persistHourlyUiPrefs: function() {
      var payload = {
        fleet_hourly_metrics_v1: {
          tasks: this._fleetTasksByHour || {},
          nodes: this._fleetNodesByHour || {},
          spend: this._fleetSpendByHour || {},
          saved: this._fleetSavedByHour || {},
          saved_at: Date.now()
        },
        agent_hourly_activity_v1: {
          status: this._agentStatusByHour || {},
          tool: this._agentToolByHour || {},
          node: this._agentNodeByHour || {},
          saved_at: Date.now()
        }
      };
      try {
        var appStore = Alpine.store('app');
        if (appStore && typeof appStore.saveUiPrefsPatch === 'function') {
          appStore.saveUiPrefsPatch(payload);
          return;
        }
      } catch (e) { /* ignore */ }
      try {
        OpenFangAPI.get('/api/ui-prefs')
          .then(function(prefs) {
            var base = (prefs && typeof prefs === 'object' && !Array.isArray(prefs)) ? prefs : {};
            var next = Object.assign({}, base, payload);
            return OpenFangAPI.put('/api/ui-prefs', next);
          })
          .catch(function() { return null; });
      } catch (e2) { /* ignore */ }
    },

    loadPersistedHourlyUiPrefs: async function() {
      try {
        var prefs = await OpenFangAPI.get('/api/ui-prefs').catch(function() { return null; });
        if (!prefs || typeof prefs !== 'object' || Array.isArray(prefs)) return;
        var fleet = prefs.fleet_hourly_metrics_v1;
        if (fleet && typeof fleet === 'object' && !Array.isArray(fleet)) {
          this._fleetTasksByHour = fleet.tasks ? this.fleetPruneHourlySeries(fleet.tasks) : this._fleetTasksByHour;
          this._fleetNodesByHour = fleet.nodes ? this.fleetPruneHourlySeries(fleet.nodes) : this._fleetNodesByHour;
          this._fleetSpendByHour = fleet.spend ? this.fleetPruneHourlySeries(fleet.spend) : this._fleetSpendByHour;
          this._fleetSavedByHour = fleet.saved ? this.fleetPruneHourlySeries(fleet.saved) : this._fleetSavedByHour;
        }
        var agent = prefs.agent_hourly_activity_v1;
        if (agent && typeof agent === 'object' && !Array.isArray(agent)) {
          this._agentStatusByHour = agent.status ? agent.status : this._agentStatusByHour;
          this._agentToolByHour = agent.tool ? agent.tool : this._agentToolByHour;
          this._agentNodeByHour = agent.node ? agent.node : this._agentNodeByHour;
        }
        this.pruneAgentHourlyMaps();
        this._fleetTasksByHour = this.fleetPruneHourlySeries(this._fleetTasksByHour);
        this._fleetNodesByHour = this.fleetPruneHourlySeries(this._fleetNodesByHour);
        this._fleetSpendByHour = this.fleetPruneHourlySeries(this._fleetSpendByHour);
        this._fleetSavedByHour = this.fleetPruneHourlySeries(this._fleetSavedByHour);
        this.fleetSpendToday = this.fleetSeriesTodayTotal(this._fleetSpendByHour);
        this.fleetSavedToday = this.fleetSeriesTodayTotal(this._fleetSavedByHour);
      } catch (e3) { /* ignore */ }
    },

    pruneAgentHourlyMaps: function() {
      var cap = 36;
      var outStatus = {};
      var outTool = {};
      var outNode = {};
      var srcS = this._agentStatusByHour || {};
      var srcT = this._agentToolByHour || {};
      var srcN = this._agentNodeByHour || {};
      for (var aid in srcS) {
        if (Object.prototype.hasOwnProperty.call(srcS, aid)) outStatus[aid] = this.pruneHourlySeries(srcS[aid], cap);
      }
      for (var aid2 in srcT) {
        if (Object.prototype.hasOwnProperty.call(srcT, aid2)) outTool[aid2] = this.pruneHourlySeries(srcT[aid2], cap);
      }
      for (var aid3 in srcN) {
        if (Object.prototype.hasOwnProperty.call(srcN, aid3)) outNode[aid3] = this.pruneHourlySeries(srcN[aid3], cap);
      }
      this._agentStatusByHour = outStatus;
      this._agentToolByHour = outTool;
      this._agentNodeByHour = outNode;
    },

    recordAgentHourly: function(kind, aid, amount, ts) {
      if (!aid) return;
      var key = String(aid);
      var amt = Number(amount) || 0;
      if (amt <= 0) return;
      if (kind === 'status') {
        var ms = this._agentStatusByHour || {};
        ms[key] = this.recordHourly(ms[key], amt, ts, 36);
        this._agentStatusByHour = Object.assign({}, ms);
      } else if (kind === 'tool') {
        var mt = this._agentToolByHour || {};
        mt[key] = this.recordHourly(mt[key], amt, ts, 36);
        this._agentToolByHour = Object.assign({}, mt);
      } else if (kind === 'node') {
        var mn = this._agentNodeByHour || {};
        mn[key] = this.recordHourly(mn[key], amt, ts, 36);
        this._agentNodeByHour = Object.assign({}, mn);
      }
      this.schedulePersistAgentHourly();
    },

    agentHourlyBars: function(agent) {
      if (!agent || !agent.id) return [];
      var aid = String(agent.id);
      var slots = this.hourSlots(24);
      var status = (this._agentStatusByHour && this._agentStatusByHour[aid]) ? this._agentStatusByHour[aid] : {};
      var tool = (this._agentToolByHour && this._agentToolByHour[aid]) ? this._agentToolByHour[aid] : {};
      var node = (this._agentNodeByHour && this._agentNodeByHour[aid]) ? this._agentNodeByHour[aid] : {};
      var visBump = this.agentStatusVisualBump(agent);
      var maxv = 1;
      for (var i = 0; i < slots.length; i++) {
        var k = String(slots[i]);
        var sv0 = Number(status[k]) || 0;
        if (i === (slots.length - 1)) sv0 += visBump;
        maxv = Math.max(
          maxv,
          sv0,
          Number(tool[k]) || 0,
          Number(node[k]) || 0
        );
      }
      var self = this;
      return slots.map(function(bucket, idx) {
        var k = String(bucket);
        var sv = Math.max(0, Number(status[k]) || 0);
        if (idx === slots.length - 1) sv += visBump;
        var tv = Math.max(0, Number(tool[k]) || 0);
        var nv = Math.max(0, Number(node[k]) || 0);
        var toPct = function(v) {
          if (v <= 0) return 0;
          return Math.max(12, Math.round((v / maxv) * 100));
        };
        return {
          key: aid + ':' + k,
          isCurrent: idx === slots.length - 1,
          statusPct: toPct(sv),
          toolPct: toPct(tv),
          nodePct: toPct(nv),
          title: self.fleetHourLabel(bucket) + ' · status ' + Math.round(sv) + ' · tools ' + Math.round(tv) + ' · nodes ' + Math.round(nv)
        };
      });
    },

    todayCallsFromDaily(daysPayload) {
      if (!daysPayload) return 0;
      var days = daysPayload.days;
      if (!Array.isArray(days) || !days.length) {
        if (typeof daysPayload.today_cost_usd === 'number' && (daysPayload.fallback_today_calls != null)) {
          return Math.max(0, Math.floor(daysPayload.fallback_today_calls));
        }
        return 0;
      }
      var t = this.todayIsoYmd();
      for (var i = 0; i < days.length; i++) {
        if (String(days[i].date) === t) {
          return Math.max(0, Math.floor(Number(days[i].calls) || 0));
        }
      }
      if (days.length) {
        return Math.max(0, Math.floor(Number(days[0].calls) || 0));
      }
      return 0;
    },

    _pushSeries(arr, val, cap) {
      var a = (arr || []).concat();
      a.push(val);
      if (a.length > cap) a = a.slice(a.length - cap, a.length);
      return a;
    },

    _seriesPolyline(ser, w, h) {
      if (!ser || !ser.length) {
        var y0 = h - 2;
        return '2,' + y0 + ' ' + w + ',' + y0;
      }
      var out = [];
      for (var j = 0; j < ser.length; j++) {
        var t = (ser.length <= 1) ? 0 : (j / (ser.length - 1));
        var x = 2 + t * (w - 4);
        // Values are already normalized activity in [0..1]. Use a fixed vertical
        // scale so idle starts near the bottom and activity rises upward.
        var v = Number(ser[j]);
        if (!isFinite(v)) v = 0;
        if (v < 0) v = 0;
        if (v > 1) v = 1;
        var y = (h - 2) - (v * (h - 4));
        out.push((Math.round(x * 100) / 100) + ',' + (Math.round(y * 100) / 100));
      }
      return out.join(' ');
    },

    fleetActivityPolyline: function() {
      return this._seriesPolyline(this.fleetSparks || [], 120, 28);
    },
    perAgentActivityPolyline: function(aid) {
      var s = (this.perAgentSparks && this.perAgentSparks[aid]) ? this.perAgentSparks[aid] : [0, 0];
      return this._seriesPolyline(s, 120, 22);
    },

    fleetSparks: [],
    perAgentSparks: {},

    agentsFleetTweakSparks(fromCallsDelta) {
      var v = Math.max(0, Number(fromCallsDelta) || 0);
      var s = 0.22 * Math.log1p(v) + 0.08;
      s = Math.min(1, s);
      this.fleetActivityNorm = 0.65 * this.fleetActivityNorm + 0.35 * s;
      this.fleetSparks = this._pushSeries(this.fleetSparks, this.fleetActivityNorm, 32);
    },

    agentTweakSpark(agentId) {
      var id = String(agentId);
      var m = (this.perAgentActivityNorm && this.perAgentActivityNorm[id]) != null
        ? this.perAgentActivityNorm[id]
        : 0.25;
      m = 0.62 * m + 0.38 * Math.min(1, 0.15 + 0.02 * (Math.random() * 1.0));
      var nextM = Object.assign({}, this.perAgentActivityNorm || {});
      nextM[id] = m;
      this.perAgentActivityNorm = nextM;
      var ar = (this.perAgentSparks && this.perAgentSparks[id]) ? this.perAgentSparks[id].concat() : [];
      ar.push(m);
      if (ar.length > 20) ar = ar.slice(-20, ar.length);
      var ps = Object.assign({}, this.perAgentSparks);
      ps[id] = ar;
      this.perAgentSparks = ps;
    },

    agentTweakSparkToward(agentId, target, jitter) {
      var id = String(agentId);
      var m = (this.perAgentActivityNorm && this.perAgentActivityNorm[id]) != null
        ? this.perAgentActivityNorm[id]
        : 0.25;
      var t = Math.max(0.02, Math.min(1, Number(target) || 0.25));
      var j = Math.max(0, Math.min(0.2, Number(jitter) || 0));
      var n = (Math.random() * 2 - 1) * j;
      var next = 0.68 * m + 0.32 * Math.max(0.02, Math.min(1, t + n));
      var nextM = Object.assign({}, this.perAgentActivityNorm || {});
      nextM[id] = next;
      this.perAgentActivityNorm = nextM;
      var ar = (this.perAgentSparks && this.perAgentSparks[id]) ? this.perAgentSparks[id].concat() : [];
      ar.push(next);
      if (ar.length > 20) ar = ar.slice(-20, ar.length);
      var ps = Object.assign({}, this.perAgentSparks);
      ps[id] = ar;
      this.perAgentSparks = ps;
    },

    phaseSparkTarget: function(phase) {
      if (phase === 'tool') return 0.84;
      if (phase === 'thinking') return 0.72;
      if (phase === 'streaming') return 0.9;
      if (phase === 'running') return 0.6;
      if (phase === 'waiting') return 0.34;
      if (phase === 'error') return 0.8;
      return 0.28;
    },

    trackAgentPhaseTransitions: function() {
      var prev = Object.assign({}, this._prevPhaseByAgent || {});
      var next = Object.assign({}, prev);
      var self = this;
      (this.agents || []).forEach(function(a) {
        if (!a || !a.id) return;
        var aid = String(a.id);
        var cur = self.agentCurrentPhaseClass(a);
        var was = prev[aid];
        next[aid] = cur;
        if (was != null && was !== cur) {
          self.fleetOnAgentPing(aid, 1);
          self.agentTweakSparkToward(aid, self.phaseSparkTarget(cur), 0.06);
          self.recordAgentHourly('status', aid, 1, Date.now());
        }
      });
      this._prevPhaseByAgent = next;
    },

    nudgeDisplayActive: function() {
      var self = this;
      var target = this.runningCount;
      var start = (typeof this.displayActiveCount === 'number' && !isNaN(this.displayActiveCount))
        ? this.displayActiveCount
        : target;
      if (this._tweenRaf) {
        try { cancelAnimationFrame(this._tweenRaf); } catch (e) { /* ignore */ }
        this._tweenRaf = null;
      }
      if (this.isReducedMotion()) {
        this.displayActiveCount = target;
        return;
      }
      var t0 = (typeof performance !== 'undefined' && performance.now) ? performance.now() : Date.now();
      var dur = 300;
      function step(now) {
        var t = (typeof performance !== 'undefined' && performance.now) ? performance.now() : now;
        var u = Math.min(1, (t - t0) / dur);
        var s = 0.5 - 0.5 * Math.cos(u * Math.PI);
        self.displayActiveCount = Math.round(start + (target - start) * s);
        if (u < 1) self._tweenRaf = requestAnimationFrame(step);
        else {
          self.displayActiveCount = target;
          self._tweenRaf = null;
        }
      }
      this._tweenRaf = requestAnimationFrame(step);
    },

    fleetBuildSpendThisHour: function() {
      var t = Date.now();
      var c = 0.0;
      for (var i = 0; i < (this._fleetSpendsMs || []).length; i++) {
        if (t - this._fleetSpendsMs[i] <= 3600000) c += this._fleetSpendsAmt[i] || 0;
      }
      this.fleetSpendHour = Math.max(0, c);
    },

    fleetSummaryTotalSavedUsd: function(s) {
      if (!s || typeof s !== 'object') return null;
      var q = (s.quota_enforcement && typeof s.quota_enforcement === 'object') ? s.quota_enforcement : null;
      var cs = (s.compression_savings && typeof s.compression_savings === 'object') ? s.compression_savings : null;
      if (!q && !cs) return null;
      var qSaved = q && typeof q.total_est_cost_avoided_usd === 'number' ? q.total_est_cost_avoided_usd : 0;
      var csSaved = cs && typeof cs.estimated_total_cost_saved_usd === 'number' ? cs.estimated_total_cost_saved_usd : 0;
      return Math.max(0, qSaved + csSaved);
    },

    fleetBuildSavedThisHour: function() {
      var t = Date.now();
      var c = 0.0;
      for (var i = 0; i < (this._fleetSavedsMs || []).length; i++) {
        if (t - this._fleetSavedsMs[i] <= 3600000) c += this._fleetSavedsAmt[i] || 0;
      }
      this.fleetSavedHour = Math.max(0, c);
    },

    fleetOnSummary: function(s) {
      if (!s) return;
      var calls = typeof s.call_count === 'number' ? s.call_count : 0;
      if (this._fleetLastSummaryCalls > 0) {
        var d = Math.max(0, calls - this._fleetLastSummaryCalls);
        this.agentsFleetTweakSparks(d);
        if (d > 0) {
          this.fleetOnAgentPing('__global__', d);
        }
      }
      this._fleetLastSummaryCalls = calls;
      var cost = typeof s.total_cost_usd === 'number' ? s.total_cost_usd : 0;
      if (this.fleetLastCost != null) {
        var del = cost - (this.fleetLastCost || 0);
        if (del > 0) {
          this._fleetSpendsMs = this._pushSeries(this._fleetSpendsMs, Date.now(), 800);
          this._fleetSpendsAmt = this._pushSeries(this._fleetSpendsAmt, del, 800);
          this._fleetSpendByHour = this.fleetRecordHourly(this._fleetSpendByHour, del, Date.now());
        }
      }
      this.fleetLastCost = cost;
      this.fleetBuildSpendThisHour();

      var saved = this.fleetSummaryTotalSavedUsd(s);
      if (saved != null) {
        if (this.fleetLastSaved != null) {
          var sdel = saved - (this.fleetLastSaved || 0);
          if (sdel > 0) {
            this._fleetSavedsMs = this._pushSeries(this._fleetSavedsMs, Date.now(), 800);
            this._fleetSavedsAmt = this._pushSeries(this._fleetSavedsAmt, sdel, 800);
            this._fleetSavedByHour = this.fleetRecordHourly(this._fleetSavedByHour, sdel, Date.now());
          }
        }
        this.fleetLastSaved = saved;
        this.fleetSavedToday = this.fleetSeriesTodayTotal(this._fleetSavedByHour);
      }
      this.fleetBuildSavedThisHour();
    },

    fleetOnAgentPing: function(aid, steps) {
      if (this.isReducedMotion()) return;
      if (steps == null || (typeof steps === 'number' && steps <= 0)) return;
      var v = Math.max(0, Number(steps) || 0);
      var spike = Math.min(1, 0.2 + (0.28 * Math.log1p(v)));
      this.fleetActivityNorm = 0.56 * (Number(this.fleetActivityNorm) || 0.08) + 0.44 * spike;
      this.fleetSparks = this._pushSeries(this.fleetSparks || [], this.fleetActivityNorm, 32);
      var o = Object.assign({}, this.agentPinging);
      o[aid] = true;
      this.agentPinging = o;
      var self = this;
      setTimeout(function() {
        var p = Object.assign({}, self.agentPinging);
        p[aid] = false;
        self.agentPinging = p;
      }, 250);
    },

    pickVitalsText: function(n) {
      if (!n) return '';
      var p = n.vitals_phase;
      var tr = n.vitals_trust;
      if (p && tr != null) return p + ' · t ' + (Math.round((Number(tr) || 0) * 100) / 100);
      if (p) return String(p);
      if (tr != null) return 'trust ' + (Math.round((Number(tr) || 0) * 100) / 100);
      if (n.vitals_gate) return 'gate: ' + String(n.vitals_gate);
      return '';
    },

    graphScanVitals: function(nodes) {
      if (!Array.isArray(nodes)) return null;
      for (var i = 0; i < Math.min(80, nodes.length); i++) {
        var o = nodes[i];
        if (o && (o.vitals_phase || o.vitals_trust != null || o.vitals_gate)) return o;
      }
      return null;
    },

    loadGraphVitalsForAgent: async function(aid) {
      try {
        var res = await OpenFangAPI.get(
          '/api/graph-memory?agent_id=' + encodeURIComponent(aid) + '&limit=2000&since_seconds=7776000&edge_mode=strict'
        );
        var n = (res && res.nodes) ? res.nodes : [];
        var c = n.length;
        var prev = (this._prevGraphNodeByAgent && this._prevGraphNodeByAgent[aid] != null)
          ? Number(this._prevGraphNodeByAgent[aid]) || 0
          : null;
        this.graphNodeCountByAgent = Object.assign({}, this.graphNodeCountByAgent, (function() { var o = {}; o[aid] = c; return o; })());
        this.graphVitalsByAgent = Object.assign({}, this.graphVitalsByAgent, (function() {
          var o = {};
          o[aid] = n.length ? this.pickVitalsText(this.graphScanVitals(n)) : '';
          return o;
        }).call(this));
        this._prevGraphNodeByAgent = Object.assign({}, this._prevGraphNodeByAgent, (function() { var o = {}; o[aid] = c; return o; })());

        if (prev != null && c > prev) {
          var delta = c - prev;
          this.fleetOnAgentPing(aid, delta);
          this.agentTweakSpark(aid);
        }
      } catch (e) {
        this.graphVitalsByAgent = Object.assign({}, this.graphVitalsByAgent, (function() { var o = {}; o[aid] = ''; return o; })());
        this.graphNodeCountByAgent = Object.assign({}, this.graphNodeCountByAgent, (function() { var o = {}; o[aid] = 0; return o; })());
        this._prevGraphNodeByAgent = Object.assign({}, this._prevGraphNodeByAgent, (function() { var o = {}; o[aid] = 0; return o; })());
      }
    },

    loadFleetGraphNodeTotal: async function(agents) {
      var list = (agents || []).filter(function(a) { return a && a.id; }).slice(0, 8);
      var sum = 0;
      for (var i = 0; i < list.length; i++) {
        try {
          var r = await OpenFangAPI.get(
            '/api/graph-memory?agent_id=' + encodeURIComponent(String(list[i].id)) + '&limit=2000&since_seconds=7776000&edge_mode=strict'
          );
          var arr = (r && r.nodes) ? r.nodes : [];
          sum += arr.length;
        } catch (e) { /* ignore */ }
      }
      if (this.fleetLastGraphNodeTotal != null && sum > this.fleetLastGraphNodeTotal) {
        this._fleetNodesByHour = this.fleetRecordHourly(
          this._fleetNodesByHour,
          (sum - this.fleetLastGraphNodeTotal),
          Date.now()
        );
      }
      this.fleetLastGraphNodeTotal = sum;
      this.fleetGraphNodeTotal = sum;
    },

    refreshFleetVitals: async function() {
      if (this._fleetInFlight) return;
      if (this.activeChatAgent) return;
      this._fleetInFlight = true;
      var self = this;
      var prev = this._prevToolByAgent || {};
      var prevCalls = this._prevCallByAgent || {};
      var initializingUsageCounters = !this._usageCountersInitialized;
      var usageRows = { agents: [] };
      var summary = null;
      var daily = { days: [] };
      var prevTasksToday = (this._prevTasksToday != null) ? Number(this._prevTasksToday) || 0 : null;
      try { summary = await OpenFangAPI.get('/api/usage/summary').catch(function() { return null; }); } catch (e1) { /* ignore */ }
      try { daily = await OpenFangAPI.get('/api/usage/daily').catch(function() { return { days: [] }; }); } catch (e2) { daily = { days: [] }; }
      this.tasksToday = this.todayCallsFromDaily(daily);
      if (prevTasksToday != null && this.tasksToday > prevTasksToday) {
        this._fleetTasksByHour = this.fleetRecordHourly(
          this._fleetTasksByHour,
          this.tasksToday - prevTasksToday,
          Date.now()
        );
      } else if (this.tasksToday > 0) {
        var curKey = String(this.fleetHourBucket(Date.now()));
        var curVal = Number((this._fleetTasksByHour || {})[curKey]) || 0;
        if (curVal <= 0) {
          this._fleetTasksByHour = this.fleetRecordHourly(this._fleetTasksByHour, 1, Date.now());
        }
      }
      this._prevTasksToday = this.tasksToday;
      if (typeof daily.today_cost_usd === 'number') this.fleetSpendToday = Math.max(0, Number(daily.today_cost_usd) || 0);
      if (summary) this.fleetOnSummary(summary);
      else { this.fleetBuildSpendThisHour(); this.fleetBuildSavedThisHour(); }
      try { usageRows = await OpenFangAPI.get('/api/usage').catch(function() { return { agents: [] }; }); } catch (e3) { usageRows = { agents: [] }; }
      var m = {};
      (usageRows && usageRows.agents ? usageRows.agents : []).forEach(function(r) {
        m[String(r.agent_id)] = {
          tool_calls: r.tool_calls || 0,
          call_count: r.call_count || 0,
          cost_usd: r.cost_usd || 0,
          total_tokens: r.total_tokens || 0
        };
      });
      for (var id in m) {
        if (!Object.prototype.hasOwnProperty.call(m, id)) continue;
        var ptc = (prev[id] != null) ? prev[id] : 0;
        var pcc = (prevCalls[id] != null) ? prevCalls[id] : 0;
        if (!initializingUsageCounters && m[id].tool_calls > ptc) {
          var tdel = m[id].tool_calls - ptc;
          self.fleetOnAgentPing(id, tdel);
          self.recordAgentHourly('tool', id, tdel, Date.now());
        }
        if (!initializingUsageCounters && m[id].call_count > pcc) {
          var cdel = m[id].call_count - pcc;
          self.recordAgentHourly('status', id, cdel, Date.now());
        }
        if (m[id].tool_calls !== ptc) {
          self.agentTweakSpark(id);
        }
      }
      for (var id2 in m) {
        if (Object.prototype.hasOwnProperty.call(m, id2)) {
          this._prevToolByAgent[id2] = m[id2].tool_calls;
          this._prevCallByAgent[id2] = m[id2].call_count;
        }
      }
      this._usageCountersInitialized = true;
      this.usageByAgent = m;
      this.trackAgentPhaseTransitions();
      this.nudgeDisplayActive();
      this._graphGen = (this._graphGen + 1) % 3;
      if (this._graphGen === 0) {
        try {
          await this.loadFleetGraphNodeTotal(this.chatPickerPrimaryAgents);
        } catch (e1) { /* ignore */ }
      }
      var toScan = (this.chatPickerPrimaryAgents || []).map(function(x) { return x.id; }).slice(0, 12);
      await Promise.all(toScan.map((function(id) {
        return this.loadGraphVitalsForAgent(String(id)).catch(function() { /* ignore */ });
      }).bind(this)));
      this.schedulePersistAgentHourly();
      this._fleetInFlight = false;
    },

    formatSecondsAgo: function(iso) {
      if (!iso) return '—';
      var t;
      try { t = new Date(iso).getTime(); } catch (e) { return '—'; }
      if (isNaN(t)) return '—';
      var s = Math.max(0, Math.floor((Date.now() - t) / 1000));
      if (s < 60) return s + 's';
      if (s < 3600) return Math.floor(s / 60) + 'm';
      return Math.floor(s / 3600) + 'h';
    },

    lastActivityLabel: function(agent) {
      if (!agent) return '—';
      var lines = null;
      try { lines = Alpine.store('app').agentActivityLines; } catch (e) { lines = null; }
      var a = lines && lines[agent.id];
      if (a && a.ts) {
        return 'Last activity · ' + this.formatSecondsAgo(new Date(a.ts).toISOString());
      }
      return 'Last activity · ' + this.formatSecondsAgo(agent && agent.last_active);
    },

    toolCallsForAgent: function(a) {
      if (!a) return 0;
      var u = this.usageByAgent && this.usageByAgent[a.id];
      if (u && u.tool_calls != null) return u.tool_calls;
      return 0;
    },

    nodeDeltaText: function(a) {
      if (!a) return '—';
      var n = this.graphNodeCountByAgent && this.graphNodeCountByAgent[a.id];
      if (n == null) return 'Knowledge · —';
      if (n === 0) return 'Knowledge · 0 nodes';
      if (n >= 2000) return 'Knowledge · 2000+ nodes';
      return 'Knowledge · ' + n + ' nodes';
    },

    cognitiveVitals: function(a) {
      if (!a) return '';
      var t = (this.graphVitalsByAgent && this.graphVitalsByAgent[a.id]) ? this.graphVitalsByAgent[a.id] : '';
      if (t) return t;
      var m = (a.mode != null) ? String(a.mode).toLowerCase() : 'full';
      if (m === 'observe') return 'Observe';
      if (m === 'assist') return 'Assist';
      if (m === 'full') return 'Full';
      return m;
    },

    fleetLogLine: function(a) {
      if (!a) return '';
      var lines = null;
      try { lines = Alpine.store('app').agentActivityLines; } catch (e) { lines = null; }
      var aal = lines && lines[a.id];
      if (aal && aal.text) return aal.text;
      return (a.model_name || 'model') + ' · ' + (a.model_provider || '');
    },

    getAgentActivityEntry: function(agent) {
      if (!agent || !agent.id) return null;
      var lines = null;
      try { lines = Alpine.store('app').agentActivityLines; } catch (e) { lines = null; }
      return lines && lines[agent.id] ? lines[agent.id] : null;
    },

    agentCurrentPhaseClass: function(agent) {
      if (!agent) return 'idle';
      var st = String(agent.state || '');
      if (st === 'Crashed') return 'error';
      if (st !== 'Running') return 'idle';
      var entry = this.getAgentActivityEntry(agent);
      if (!entry || !entry.text) return 'waiting';
      var t = String(entry.text).toLowerCase();
      if (t.indexOf('thinking') >= 0) return 'thinking';
      if (t.indexOf('using tool') >= 0 || t.indexOf('tool') >= 0) return 'tool';
      if (t.indexOf('writing response') >= 0 || t.indexOf('stream') >= 0 || t.indexOf('reply') >= 0) return 'streaming';
      if (t.indexOf('waiting') >= 0) return 'waiting';
      if (t.indexOf('awaiting input') >= 0 || t.indexOf('idle') >= 0) return 'waiting';
      return 'running';
    },

    agentCurrentPhaseLabel: function(agent) {
      var ph = this.agentCurrentPhaseClass(agent);
      if (ph === 'error') return 'Error';
      if (ph === 'thinking') return 'Thinking';
      if (ph === 'tool') return 'Tool run';
      if (ph === 'streaming') return 'Responding';
      if (ph === 'waiting') return 'Waiting';
      if (ph === 'running') return 'Live';
      return 'Idle';
    },

    agentPhaseGlyph: function(agent) {
      var ph = this.agentCurrentPhaseClass(agent);
      if (ph === 'thinking') return '…';
      if (ph === 'tool') return '⚙';
      if (ph === 'streaming') return '▸';
      if (ph === 'waiting') return '○';
      if (ph === 'running') return '●';
      if (ph === 'error') return '!';
      return '·';
    },

    agentCurrentPhaseDetail: function(agent) {
      if (!agent) return '';
      var entry = this.getAgentActivityEntry(agent);
      if (entry && entry.text) return String(entry.text);
      if (String(agent.state || '') === 'Running') {
        return 'Awaiting input';
      }
      if (String(agent.state || '') === 'Crashed') return 'Agent crashed — inspect diagnostics';
      return 'No live activity';
    },

    agentCurrentPhaseFreshness: function(agent) {
      var entry = this.getAgentActivityEntry(agent);
      if (entry && entry.ts) {
        try {
          return this.formatSecondsAgo(new Date(entry.ts).toISOString()) + ' ago';
        } catch (e) { /* ignore */ }
      }
      return this.lastActivityLabel(agent).replace('Last activity · ', '') + ' ago';
    },

    agentCurrentPhaseIntensity: function(agent) {
      var ph = this.agentCurrentPhaseClass(agent);
      var isIdleLike = (ph === 'idle' || ph === 'waiting');
      var isActiveLike = (ph === 'tool' || ph === 'thinking' || ph === 'streaming' || ph === 'running');
      var base = 25;
      if (ph === 'thinking') base = 58;
      else if (ph === 'tool') base = 74;
      else if (ph === 'streaming') base = 90;
      else if (ph === 'running') base = 46;
      else if (ph === 'waiting') base = 28;
      else if (ph === 'error') base = 100;

      var ts = null;
      var entry = this.getAgentActivityEntry(agent);
      if (entry && entry.ts) ts = Number(entry.ts) || null;
      if (!ts && agent && agent.last_active) {
        try { ts = new Date(agent.last_active).getTime(); } catch (e) { ts = null; }
      }
      var freshnessBoost = 0;
      if (ts) {
        var age = Math.max(0, Math.floor((Date.now() - ts) / 1000));
        if (isIdleLike) {
          if (age <= 5) freshnessBoost = 3;
          else if (age <= 15) freshnessBoost = 1;
          else if (age <= 45) freshnessBoost = 0;
          else if (age >= 180) freshnessBoost = -6;
        } else {
          if (age <= 5) freshnessBoost = 14;
          else if (age <= 15) freshnessBoost = 8;
          else if (age <= 45) freshnessBoost = 3;
          else if (age >= 180) freshnessBoost = -8;
        }
      }
      var pingBoost = (agent && agent.id && this.agentPinging && this.agentPinging[agent.id])
        ? (isActiveLike ? 12 : 2)
        : 0;
      var idleWave = 0;
      if (isIdleLike) {
        var sid = String((agent && agent.id) || '');
        var seed = 0;
        for (var i = 0; i < sid.length; i++) seed = (seed + sid.charCodeAt(i)) % 97;
        var t = Number(this._idlePulseTick || 0);
        // Two low-amplitude waves with agent-specific phase offsets keep idle bars moving
        // at varied intervals instead of sitting at a fixed midpoint.
        idleWave =
          (Math.sin((t + seed) / (2.6 + (seed % 4) * 0.35)) * 4.6) +
          (Math.sin((t + seed * 0.7) / (4.8 + (seed % 5) * 0.42)) * 2.9);
      }
      var activeWave = 0;
      if (isActiveLike) {
        var sid2 = String((agent && agent.id) || '');
        var seed2 = 0;
        for (var j = 0; j < sid2.length; j++) seed2 = (seed2 + sid2.charCodeAt(j)) % 131;
        var t2 = Number(this._idlePulseTick || 0);
        // Subtle motion while actively working keeps the monitor alive during long calls.
        activeWave =
          (Math.sin((t2 + seed2) / (2.1 + (seed2 % 3) * 0.25)) * 4.2) +
          (Math.sin((t2 + seed2 * 0.55) / (5.4 + (seed2 % 4) * 0.35)) * 2.2);
      }
      var out = Math.round(base + freshnessBoost + pingBoost + idleWave + activeWave);
      if (out < 25) out = 25;
      if (isIdleLike && out > 34) out = 34;
      if (out > 100) out = 100;
      return out;
    },

    agentWorkloadBand: function(agent) {
      var n = Number(this.agentCurrentPhaseIntensity(agent)) || 0;
      if (n >= 90) return 'crit';
      if (n >= 80) return 'hot';
      if (n >= 70) return 'warn';
      if (n > 50) return 'elevated';
      return 'normal';
    },

    // Visual-only bump for the in-progress hour so bars feel alive while work
    // is ongoing. This is not persisted and does not affect finalized hourly totals.
    agentStatusVisualBump: function(agent) {
      if (!agent) return 0;
      var ph = this.agentCurrentPhaseClass(agent);
      var n = Number(this.agentCurrentPhaseIntensity(agent)) || 0;
      if (ph === 'tool' || ph === 'thinking' || ph === 'streaming' || ph === 'running') {
        return Math.max(0, Math.min(0.5, (n - 35) / 120));
      }
      if (ph === 'waiting' || ph === 'idle') {
        return Math.max(0, Math.min(0.12, (n - 20) / 150));
      }
      return 0;
    },

    tickActivePhaseMicroActivity: function() {
      var self = this;
      var list = (this.agents || []).filter(function(a) {
        return a && a.id && String(a.state || '') === 'Running';
      });
      list.forEach(function(a) {
        var ph = self.agentCurrentPhaseClass(a);
        var ts = null;
        var entry = self.getAgentActivityEntry(a);
        if (entry && entry.ts) ts = Number(entry.ts) || null;
        if (!ts && a.last_active) {
          try { ts = new Date(a.last_active).getTime(); } catch (e) { ts = null; }
        }
        var ageSec = ts ? Math.max(0, Math.floor((Date.now() - ts) / 1000)) : 9999;
        if (ageSec > 300) return;
        if (ph === 'tool') self.agentTweakSparkToward(a.id, self.phaseSparkTarget(ph), 0.06);
        else if (ph === 'thinking') self.agentTweakSparkToward(a.id, self.phaseSparkTarget(ph), 0.055);
        else if (ph === 'streaming') self.agentTweakSparkToward(a.id, self.phaseSparkTarget(ph), 0.045);
        else if (ph === 'running') self.agentTweakSparkToward(a.id, self.phaseSparkTarget(ph), 0.04);
        else if (ph === 'waiting' || ph === 'idle') self.agentTweakSparkToward(a.id, self.phaseSparkTarget(ph), 0.025);
      });
      this.trackAgentPhaseTransitions();
    },

    syncAgentActivityFeeds: function() {
      var lines = null;
      try { lines = Alpine.store('app').agentActivityLines; } catch (e) { lines = null; }
      if (!lines) return;
      var next = Object.assign({}, this.activityFeedByAgent || {});
      var seen = Object.assign({}, this._activitySeenTsByAgent || {});
      var changed = false;
      for (var aid in lines) {
        if (!Object.prototype.hasOwnProperty.call(lines, aid)) continue;
        var ent = lines[aid];
        if (!ent || !ent.text || !ent.ts) continue;
        var ts = Number(ent.ts) || 0;
        var prevTs = Number(seen[aid] || 0);
        if (ts <= prevTs) continue;
        var row = {
          ts: ts,
          text: String(ent.text),
          phase: (function(t) {
            var low = String(t || '').toLowerCase();
            if (low.indexOf('thinking') >= 0) return 'thinking';
            if (low.indexOf('using tool') >= 0 || low.indexOf('tool') >= 0) return 'tool';
            if (low.indexOf('writing response') >= 0 || low.indexOf('stream') >= 0 || low.indexOf('reply') >= 0) return 'streaming';
            if (low.indexOf('waiting') >= 0) return 'waiting';
            return 'running';
          })(ent.text)
        };
        var txt = String(ent.text || '');
        var mNodes = txt.match(/graph\s+memory\s*\+\s*(\d+)\s*nodes?/i);
        if (!mNodes) mNodes = txt.match(/(?:wrote|written|added)\s+(\d+)\s+nodes?/i);
        if (mNodes) {
          var nAdd = Number(mNodes[1]) || 0;
          if (nAdd > 0) this.recordAgentHourly('node', aid, nAdd, ts);
        }
        var arr = (next[aid] || []).slice();
        arr.unshift(row);
        if (arr.length > 5) arr = arr.slice(0, 5);
        next[aid] = arr;
        seen[aid] = ts;
        changed = true;
      }
      if (changed) {
        this.activityFeedByAgent = next;
        this._activitySeenTsByAgent = seen;
      }
    },

    agentActivityFeed: function(agent) {
      if (!agent || !agent.id) return [];
      return (this.activityFeedByAgent && this.activityFeedByAgent[agent.id]) ? this.activityFeedByAgent[agent.id] : [];
    },

    fleetActionsStop: function(ev) {
      if (ev) {
        try { if (ev.stopPropagation) ev.stopPropagation(); } catch (e) { /* ignore */ }
        try { if (ev.preventDefault) ev.preventDefault(); } catch (e2) { /* ignore */ }
      }
    },

    isFleetAgentPinned: function(agent) {
      if (!agent || !agent.id) return false;
      try {
        var app = Alpine.store('app');
        return !!(app && typeof app.isAgentPinned === 'function' && app.isAgentPinned(String(agent.id)));
      } catch (e) { return false; }
    },

    toggleFleetAgentPin: function(agent, ev) {
      this.fleetActionsStop(ev);
      if (!agent || !agent.id) return;
      try {
        var app = Alpine.store('app');
        if (app && typeof app.togglePinAgent === 'function') app.togglePinAgent(String(agent.id));
      } catch (e) { /* ignore */ }
    },

    fleetClickCard: function(agent, ev) {
      this.chatWithAgent(agent);
    },

    fleetStartPeriodic: function() {
      this.fleetLastCost = null;
      this._fleetLastSummaryCalls = 0;
      this._prevTasksToday = null;
      this._prevToolByAgent = {};
      this._prevCallByAgent = {};
      this._usageCountersInitialized = false;
      this._prevPhaseByAgent = {};
      this.displayActiveCount = this.runningCount;
      var self = this;
      this.refreshFleetVitals();
      if (this._fleetInt) { try { clearInterval(this._fleetInt); } catch (e) { /* ignore */ } this._fleetInt = null; }
      this._fleetInt = setInterval(function() { self.refreshFleetVitals(); }, 5000);
      this.nudgeDisplayActive();
    },

    fleetTeardown: function() {
      if (this._fleetInt) { try { clearInterval(this._fleetInt); } catch (e) { /* ignore */ } this._fleetInt = null; }
      if (this._fleetDemoInt) { try { clearInterval(this._fleetDemoInt); } catch (e) { /* ignore */ } this._fleetDemoInt = null; }
      if (this._activitySyncInt) { try { clearInterval(this._activitySyncInt); } catch (e0) { /* ignore */ } this._activitySyncInt = null; }
      if (this._idlePulseInt) { try { clearInterval(this._idlePulseInt); } catch (e00) { /* ignore */ } this._idlePulseInt = null; }
      if (this._activePulseInt) { try { clearInterval(this._activePulseInt); } catch (e000) { /* ignore */ } this._activePulseInt = null; }
      if (this._agentHourlyPersistTimer) { try { clearTimeout(this._agentHourlyPersistTimer); } catch (e0000) { /* ignore */ } this._agentHourlyPersistTimer = null; }
      if (this._hourlyUiPrefsPersistTimer) { try { clearTimeout(this._hourlyUiPrefsPersistTimer); } catch (e0001) { /* ignore */ } this._hourlyUiPrefsPersistTimer = null; }
      if (this._tweenRaf) { try { cancelAnimationFrame(this._tweenRaf); } catch (e) { /* ignore */ } this._tweenRaf = null; }
      if (this._demoKey) { try { document.removeEventListener('keydown', this._demoKey, true); } catch (e) { /* ignore */ } this._demoKey = null; }
    },

    fleetStartDemo: function() {
      if (!this.demoMode) return;
      var self = this;
      if (this._fleetDemoInt) { try { clearInterval(this._fleetDemoInt); } catch (e) { /* ignore */ } this._fleetDemoInt = null; }
      this._fleetDemoInt = setInterval(function() {
        if (self.activeChatAgent) return;
        var a = (self.chatPickerPrimaryAgents && self.chatPickerPrimaryAgents[0]) ? self.chatPickerPrimaryAgents[0] : null;
        if (a) {
          self.agentTweakSpark(a.id);
        }
        self.fleetOnAgentPing('__global__', 0.1);
        self.fleetOnSummary({ call_count: (self._fleetLastSummaryCalls || 0) + 1, total_cost_usd: (self.fleetLastCost != null ? self.fleetLastCost : 0) + 0.0004 });
        self.fleetBuildSpendThisHour();
        self.nudgeDisplayActive();
      }, 2000);
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
      this.demoProfile = this.readFleetDemoProfileFromEnv();
      this.demoMode = this.readFleetDemoFromEnv();
      this.applyDemoThemeIfNeeded();
      this.loadPersistedAgentHourly();
      this.pruneAgentHourlyMaps();
      this.loadPersistedHourlyUiPrefs();
      this._demoKey = function(e) {
        if (e.ctrlKey && e.shiftKey && (e.key === 'd' || e.key === 'D')) {
          e.preventDefault();
          self.toggleFleetDemoMode();
          if (self.demoMode) self.fleetStartDemo();
        } else if (e.ctrlKey && e.shiftKey && (e.key === 'c' || e.key === 'C')) {
          e.preventDefault();
          self.cycleFleetDemoProfile();
        }
      };
      try { document.addEventListener('keydown', this._demoKey, true); } catch (eK) { /* ignore */ }
      this.$nextTick(function() {
        self.syncAgentActivityFeeds();
        self.fleetStartPeriodic();
        if (self._activitySyncInt) { try { clearInterval(self._activitySyncInt); } catch (eA) { /* ignore */ } self._activitySyncInt = null; }
        self._activitySyncInt = setInterval(function() { self.syncAgentActivityFeeds(); }, 650);
        if (self._idlePulseInt) { try { clearInterval(self._idlePulseInt); } catch (eB) { /* ignore */ } self._idlePulseInt = null; }
        self._idlePulseInt = setInterval(function() {
          self._idlePulseTick = (self._idlePulseTick + 1) % 10000;
        }, 1200);
        if (self._activePulseInt) { try { clearInterval(self._activePulseInt); } catch (eC) { /* ignore */ } self._activePulseInt = null; }
        self._activePulseInt = setInterval(function() {
          self.tickActivePhaseMicroActivity();
        }, 1100);
        if (self.demoMode) self.fleetStartDemo();
      });
      this.$watch('activeChatAgent', function(v) {
        if (!v) {
          self.$nextTick(function() { self.refreshFleetVitals(); });
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
      var self = this;
      this.tplLoading = true;
      this.tplLoadError = '';
      var builtins = staticChatBuiltinTemplates();
      try {
        var results = await Promise.all([
          OpenFangAPI.get('/api/templates').catch(function(e) {
            self.tplLoadError =
              (e && e.message ? 'Could not load disk templates (' + e.message + ').' : 'Could not load disk templates.') +
              ' Quick-start cards below still work.';
            return { templates: [] };
          }),
          OpenFangAPI.get('/api/providers').catch(function() { return { providers: [] }; })
        ]);
        var extra = (results[0] && results[0].templates) || [];
        this.builtinTemplates = builtins.concat(extra);
        this.tplProviders = (results[1] && results[1].providers) || [];
      } catch (e) {
        this.builtinTemplates = builtins;
        this.tplLoadError =
          (this.tplLoadError ? this.tplLoadError + ' ' : '') +
          ((e && e.message) || 'Unexpected error while loading templates.');
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
      this.fleetTeardown();
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
        vibe: idn.vibe || '',
        max_iterations: (agent && agent.max_iterations) || null,
        ainl_runtime_engine: (agent && agent.ainl_runtime_engine) === true,
        native_planner_enabled: (agent && agent.native_planner_enabled) === true
      };
      this.fullManifestToml = '';
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
        system_prompt: full.system_prompt,
        ainl_runtime_engine: full.ainl_runtime_engine === true,
        native_planner_enabled: full.native_planner_enabled === true
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
        vibe: idn.vibe || '',
        max_iterations: full.max_iterations != null ? full.max_iterations : null,
        ainl_runtime_engine: full.ainl_runtime_engine === true,
        native_planner_enabled: full.native_planner_enabled === true
      };
      this.toolFilters = {
        tool_allowlist: (full.tool_allowlist || []).slice(),
        tool_blocklist: (full.tool_blocklist || []).slice()
      };
      this.fullManifestToml = full.manifest_toml != null ? String(full.manifest_toml) : '';
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
          OpenFangToast.error('Failed to stop agent: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to set mode: ' + openFangErrText(e));
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
      var spawnErr = validateManifestToml(toml);
      if (spawnErr) {
        this.spawning = false;
        OpenFangToast.warn(spawnErr);
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
        OpenFangToast.error('Failed to spawn agent: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to load files: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to read file: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to save file: ' + openFangErrText(e));
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
        OpenFangToast.success('Config updated (partial — chat session preserved)');
        try {
          var full = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id);
          this.applyAgentDetail(full);
        } catch (e2) { /* ignore */ }
        await Alpine.store('app').refreshAgents();
      } catch(e) {
        OpenFangToast.error('Failed to save config: ' + openFangErrText(e));
      }
      this.configSaving = false;
    },

    async reloadFullManifestToml() {
      if (!this.detailAgent) return;
      this.fullManifestLoading = true;
      try {
        var full = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id);
        this.applyAgentDetail(full);
      } catch (e) {
        OpenFangToast.error('Could not load manifest: ' + openFangErrText(e));
      }
      this.fullManifestLoading = false;
    },

    promptApplyFullManifest() {
      var self = this;
      var err = validateManifestToml(this.fullManifestToml);
      if (err) {
        OpenFangToast.warn(err);
        return;
      }
      var msg =
        'Replace the entire running manifest for "' + (this.detailAgent && this.detailAgent.name) + '"?\n\n' +
        '• Clears this agent\'s canonical session memory (chat transcript may still appear in the UI).\n' +
        '• Reloads autonomous schedules in-process (no daemon restart).\n' +
        '• Writes an audit entry (AgentManifestUpdate) for compliance.\n\n' +
        'For small edits (prompt, model, identity), use Save Config above instead — it does not clear the session.';
      OpenFangToast.confirm(
        'Apply full manifest?',
        msg,
        function() { self.applyFullManifestPut(); },
        { danger: true, confirmLabel: 'Apply full manifest' }
      );
    },

    async applyFullManifestPut() {
      if (!this.detailAgent) return;
      var err = validateManifestToml(this.fullManifestToml);
      if (err) {
        OpenFangToast.warn(err);
        return;
      }
      this.fullManifestSaving = true;
      try {
        var res = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/update', {
          manifest_toml: this.fullManifestToml
        });
        var line1 = 'Full manifest applied.';
        if (res && res.note) line1 += '\n' + res.note;
        var line2 =
          '\nAudit: Logs → Audit trail, or GET /api/audit/recent (AgentManifestUpdate).';
        OpenFangToast.success(line1 + line2, 10000);
        try {
          var full = await OpenFangAPI.get('/api/agents/' + this.detailAgent.id);
          this.applyAgentDetail(full);
        } catch (e2) { /* ignore */ }
        await Alpine.store('app').refreshAgents();
      } catch (e) {
        OpenFangToast.error('Full manifest apply failed: ' + openFangErrText(e));
      }
      this.fullManifestSaving = false;
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
        OpenFangToast.error('Clone failed: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to spawn from template: ' + openFangErrText(e));
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
          OpenFangToast.error('Failed to clear history: ' + openFangErrText(e));
        }
      });
    },

    // ── Model switch ──
    changeModel() {
      if (!this.detailAgent || !this.newModelValue.trim()) return;
      var cur = ((this.detailAgent.model_provider || '').trim() + '/' + (this.detailAgent.model_name || '').trim()).replace(/^\/+/, '');
      var next = this.newModelValue.trim();
      if (next === cur) {
        this.editingModel = false;
        return;
      }
      var self = this;
      OpenFangToast.confirm(
        'Switch model?',
        OpenFangToast.modelProviderChangeWarningText(),
        function() { self._changeModelApply(); },
        { danger: false, confirmLabel: 'Switch' }
      );
    },

    async _changeModelApply() {
      if (!this.detailAgent || !this.newModelValue.trim()) return;
      this.modelSaving = true;
      try {
        var resp = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: this.newModelValue.trim() });
        var providerInfo = (resp && resp.provider) ? ' (provider: ' + resp.provider + ')' : '';
        OpenFangToast.success('Model changed' + providerInfo + ' (canonical session reset)');
        this.editingModel = false;
        await Alpine.store('app').refreshAgents();
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        OpenFangToast.error('Failed to change model: ' + openFangErrText(e));
      }
      this.modelSaving = false;
    },

    // ── Provider switch ──
    changeProvider() {
      if (!this.detailAgent || !this.newProviderValue.trim()) return;
      if (this.newProviderValue.trim() === (this.detailAgent.model_provider || '').trim()) {
        this.editingProvider = false;
        return;
      }
      var self = this;
      OpenFangToast.confirm(
        'Switch provider?',
        OpenFangToast.modelProviderChangeWarningText(),
        function() { self._changeProviderApply(); },
        { danger: false, confirmLabel: 'Switch' }
      );
    },

    async _changeProviderApply() {
      if (!this.detailAgent || !this.newProviderValue.trim()) return;
      this.modelSaving = true;
      try {
        var combined = this.newProviderValue.trim() + '/' + this.detailAgent.model_name;
        var resp = await OpenFangAPI.put('/api/agents/' + this.detailAgent.id + '/model', { model: combined });
        OpenFangToast.success('Provider changed to ' + (resp && resp.provider ? resp.provider : this.newProviderValue.trim()) + ' (canonical session reset)');
        this.editingProvider = false;
        await Alpine.store('app').refreshAgents();
        var agents = Alpine.store('app').agents;
        for (var i = 0; i < agents.length; i++) {
          if (agents[i].id === this.detailAgent.id) { this.detailAgent = agents[i]; break; }
        }
      } catch(e) {
        OpenFangToast.error('Failed to change provider: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to save fallbacks: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to save fallbacks: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to update tool filters: ' + openFangErrText(e));
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
        OpenFangToast.error('Failed to spawn agent: ' + openFangErrText(e));
      }
    }
  };
}
