// ArmaraOS Setup Wizard — First-run guided setup (Provider + Agent + Channel)
'use strict';

/** Escape a string for use inside TOML triple-quoted strings ("""\n...\n"""). */
function wizardTomlMultilineEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"""/g, '""\\"');
}

/** Escape a string for use inside a TOML basic (single-line) string ("..."). */
function wizardTomlBasicEscape(s) {
  return s.replace(/\\/g, '\\\\').replace(/"/g, '\\"').replace(/\n/g, '\\n').replace(/\r/g, '\\r').replace(/\t/g, '\\t');
}

function wizardPage() {
  return {
    step: 1,
    totalSteps: 6,
    loading: false,
    error: '',

    // Step 2: Provider setup
    providers: [],
    /** Default selection before/after load — OpenRouter is the recommended path */
    selectedProvider: 'openrouter',
    apiKeyInput: '',
    testingProvider: false,
    testResult: null,
    savingKey: false,
    keySaved: false,
    /** For providers that use an API key: true only after a successful /test in this flow (blocks Next until verified). */
    step2ConnectionVerified: false,
    /** Bumped when the user switches provider or starts a new save, so stale /test responses are ignored. */
    providerVerifyToken: 0,

    // Step 3: Agent creation
    templates: [
      {
        id: 'assistant',
        name: 'Armara',
        description: 'Your personal assistant powered by AI Native Language — everyday tasks, answers, web search, building, and more.',
        icon: 'GA',
        category: 'General',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'balanced',
        system_prompt: 'You are Armara, a personal assistant powered by AI Native Language, running in ArmaraOS. Be helpful, clear, and concise. Ask clarifying questions when needed.'
      },
      {
        id: 'coder',
        name: 'Code Helper',
        description: 'A programming-focused agent that writes, reviews, and debugs code across multiple languages.',
        icon: 'CH',
        category: 'Development',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'precise',
        system_prompt: 'You are an expert programmer. Help users write clean, efficient code. Explain your reasoning. Follow best practices and conventions for the language being used.'
      },
      {
        id: 'researcher',
        name: 'Researcher',
        description: 'An analytical agent that breaks down complex topics, synthesizes information, and provides cited summaries.',
        icon: 'RS',
        category: 'Research',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'balanced',
        system_prompt: 'You are a research analyst. Break down complex topics into clear explanations. Provide structured analysis with key findings. Cite sources when available.'
      },
      {
        id: 'writer',
        name: 'Writer',
        description: 'A creative writing agent that helps with drafting, editing, and improving written content of all kinds.',
        icon: 'WR',
        category: 'Writing',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'creative',
        system_prompt: 'You are a skilled writer and editor. Help users create polished content. Adapt your tone and style to match the intended audience. Offer constructive suggestions for improvement.'
      },
      {
        id: 'data-analyst',
        name: 'Data Analyst',
        description: 'A data-focused agent that helps analyze datasets, create queries, and interpret statistical results.',
        icon: 'DA',
        category: 'Development',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'precise',
        system_prompt: 'You are a data analysis expert. Help users understand their data, write SQL/Python queries, and interpret results. Present findings clearly with actionable insights.'
      },
      {
        id: 'devops',
        name: 'DevOps Engineer',
        description: 'A systems-focused agent for CI/CD, infrastructure, Docker, and deployment troubleshooting.',
        icon: 'DO',
        category: 'Development',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'precise',
        system_prompt: 'You are a DevOps engineer. Help with CI/CD pipelines, Docker, Kubernetes, infrastructure as code, and deployment. Prioritize reliability and security.'
      },
      {
        id: 'support',
        name: 'Customer Support',
        description: 'A professional, empathetic agent for handling customer inquiries and resolving issues.',
        icon: 'CS',
        category: 'Business',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'balanced',
        system_prompt: 'You are a professional customer support representative. Be empathetic, patient, and solution-oriented. Acknowledge concerns before offering solutions. Escalate complex issues appropriately.'
      },
      {
        id: 'tutor',
        name: 'Tutor',
        description: 'A patient educational agent that explains concepts step-by-step and adapts to the learner\'s level.',
        icon: 'TU',
        category: 'General',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'balanced',
        system_prompt: 'You are a patient and encouraging tutor. Explain concepts step by step, starting from fundamentals. Use analogies and examples. Check understanding before moving on. Adapt to the learner\'s pace.'
      },
      {
        id: 'api-designer',
        name: 'API Designer',
        description: 'An agent specialized in RESTful API design, OpenAPI specs, and integration architecture.',
        icon: 'AD',
        category: 'Development',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'precise',
        system_prompt: 'You are an API design expert. Help users design clean, consistent RESTful APIs following best practices. Cover endpoint naming, request/response schemas, error handling, and versioning.'
      },
      {
        id: 'meeting-notes',
        name: 'Meeting Notes',
        description: 'Summarizes meeting transcripts into structured notes with action items and key decisions.',
        icon: 'MN',
        category: 'Business',
        provider: 'openrouter',
        model: 'stepfun/step-3.5-flash:free',
        profile: 'precise',
        system_prompt: 'You are a meeting summarizer. When given a meeting transcript or notes, produce a structured summary with: key decisions, action items (with owners), discussion highlights, and follow-up questions.'
      }
    ],
    selectedTemplate: 0,
    agentName: 'my-assistant',
    creatingAgent: false,
    createdAgent: null,

    // Step 3: Category filtering
    templateCategory: 'All',
    get templateCategories() {
      var cats = { 'All': true };
      this.templates.forEach(function(t) { if (t.category) cats[t.category] = true; });
      return Object.keys(cats);
    },
    get filteredTemplates() {
      var cat = this.templateCategory;
      if (cat === 'All') return this.templates;
      return this.templates.filter(function(t) { return t.category === cat; });
    },

    // Step 3: Profile/tool descriptions
    profileDescriptions: {
      minimal: { label: 'Minimal', desc: 'Read-only file access' },
      coding: { label: 'Coding', desc: 'Files + shell + web fetch' },
      research: { label: 'Research', desc: 'Web search + file read/write' },
      balanced: { label: 'Balanced', desc: 'General-purpose tool set' },
      precise: { label: 'Precise', desc: 'Focused tool set for accuracy' },
      creative: { label: 'Creative', desc: 'Full tools with creative emphasis' },
      full: { label: 'Full', desc: 'All 35+ tools' }
    },
    profileInfo: function(name) { return this.profileDescriptions[name] || { label: name, desc: '' }; },

    /** Same provider/model logic as createAgent() — for honest UI under the agent name field */
    get previewAgentProvider() {
      var tpl = this.templates[this.selectedTemplate];
      if (!tpl) return '';
      if (this.selectedProviderObj && this.providerIsConfigured(this.selectedProviderObj)) {
        return this.selectedProviderObj.id;
      }
      return tpl.provider;
    },
    get previewAgentModel() {
      var tpl = this.templates[this.selectedTemplate];
      if (!tpl) return '';
      if (this.selectedProviderObj && this.providerIsConfigured(this.selectedProviderObj)) {
        return this.defaultModelForProvider(this.selectedProviderObj.id) || tpl.model;
      }
      return tpl.model;
    },

    // Step 4: Try It chat
    tryItMessages: [],
    tryItInput: '',
    tryItSending: false,
    suggestedMessages: {
      'General': ['What can you help me with?', 'Tell me a fun fact', 'Summarize the latest AI news'],
      'Development': ['Write a Python hello world', 'Explain async/await', 'Review this code snippet'],
      'Research': ['Explain quantum computing simply', 'Compare React vs Vue', 'What are the latest trends in AI?'],
      'Writing': ['Help me write a professional email', 'Improve this paragraph', 'Write a blog intro about AI'],
      'Business': ['Draft a meeting agenda', 'How do I handle a complaint?', 'Create a project status update']
    },
    get currentSuggestions() {
      var tpl = this.templates[this.selectedTemplate];
      var cat = tpl ? tpl.category : 'General';
      return this.suggestedMessages[cat] || this.suggestedMessages['General'];
    },
    async sendTryItMessage(text) {
      if (!text || !text.trim() || !this.createdAgent || this.tryItSending) return;
      text = text.trim();
      this.tryItInput = '';
      this.tryItMessages.push({ role: 'user', text: text });
      this.tryItSending = true;
      try {
        var res = await OpenFangAPI.post('/api/agents/' + this.createdAgent.id + '/message', { message: text });
        this.tryItMessages.push({ role: 'agent', text: res.response || '(no response)' });
      } catch(e) {
        this.tryItMessages.push({ role: 'agent', text: 'Error: ' + (e.message || 'Could not reach agent') });
      }
      this.tryItSending = false;
    },

    // Step 5: Channel setup (optional)
    channelType: '',
    channelOptions: [
      {
        name: 'telegram',
        display_name: 'Telegram',
        icon: 'TG',
        description: 'Connect your agent to a Telegram bot for messaging.',
        token_label: 'Bot Token',
        token_placeholder: '123456:ABC-DEF1234ghIkl-zyx57W2v1u123ew11',
        token_env: 'TELEGRAM_BOT_TOKEN',
        help: 'Create a bot via @BotFather on Telegram to get your token.'
      },
      {
        name: 'discord',
        display_name: 'Discord',
        icon: 'DC',
        description: 'Connect your agent to a Discord server via bot token.',
        token_label: 'Bot Token',
        token_placeholder: 'MTIz...abc',
        token_env: 'DISCORD_BOT_TOKEN',
        help: 'Create a Discord application at discord.com/developers and add a bot.'
      },
      {
        name: 'slack',
        display_name: 'Slack',
        icon: 'SL',
        description: 'Connect your agent to a Slack workspace.',
        token_label: 'Bot Token',
        token_placeholder: 'xoxb-...',
        token_env: 'SLACK_BOT_TOKEN',
        help: 'Create a Slack app at api.slack.com/apps and install it to your workspace.'
      }
    ],
    channelToken: '',
    configuringChannel: false,
    channelConfigured: false,

    // Step 5: Summary
    setupSummary: {
      provider: '',
      agent: '',
      channel: '',
      schedule: ''
    },

    // ── Lifecycle ──

    async loadData() {
      this.loading = true;
      this.error = '';
      try {
        await this.loadProviders();
        // Always select OpenRouter when the catalog includes it (recommended default on wizard entry).
        var providers = this.providers;
        var hasOpenRouter = providers.some(function(p) { return p.id === 'openrouter'; });
        if (hasOpenRouter) {
          this.selectedProvider = 'openrouter';
        } else {
          var unconfigured = providers.filter(function(p) {
            return p.auth_status !== 'configured' && p.api_key_env;
          });
          if (unconfigured.length > 0) {
            this.selectedProvider = unconfigured[0].id;
          } else if (providers.length > 0) {
            this.selectedProvider = providers[0].id;
          } else {
            this.selectedProvider = '';
          }
        }
      } catch(e) {
        this.error = e.message || 'Could not load setup data.';
      }
      this.loading = false;
      if (this.step === 2) {
        var self = this;
        queueMicrotask(function() { self.maybeAutoVerifyStep2Provider(); });
      }
    },

    // ── Navigation ──

    nextStep() {
      if (this.step === 2 && !this.wizardProviderReady) return;
      if (this.step === 3 && !this.createdAgent) {
        // Skip "Try It" if no agent was created
        this.step = 5;
      } else if (this.step < this.totalSteps) {
        this.step++;
      }
      if (this.step === 2) {
        var self = this;
        queueMicrotask(function() { self.maybeAutoVerifyStep2Provider(); });
      }
    },

    prevStep() {
      if (this.step === 5 && !this.createdAgent) {
        // Skip back past "Try It" if no agent was created
        this.step = 3;
      } else if (this.step > 1) {
        this.step--;
      }
      if (this.step === 2) {
        var self = this;
        queueMicrotask(function() { self.maybeAutoVerifyStep2Provider(); });
      }
    },

    goToStep(n) {
      if (n >= 1 && n <= this.totalSteps) {
        if (n === 4 && !this.createdAgent) return; // Can't go to Try It without agent
        // Do not allow skipping the provider step via the progress bar (must match Next button rules).
        if (n > 2 && !this.wizardProviderReady) return;
        this.step = n;
        if (n === 2) {
          var self = this;
          queueMicrotask(function() { self.maybeAutoVerifyStep2Provider(); });
        }
      }
    },

    stepLabel(n) {
      var labels = ['Welcome', 'Provider', 'Agent', 'Try It', 'Channel', 'Done'];
      return labels[n - 1] || '';
    },

    /** True when the currently selected LLM provider is ready (same rule used for progress-bar jumps). */
    get wizardProviderReady() {
      var p = this.selectedProviderObj;
      if (!p) return false;
      if (this.selectedProvider === 'claude-code') {
        return this.claudeCodeDetected || this.providerIsConfigured(p);
      }
      if (!p.api_key_env) {
        return this.providerIsConfigured(p);
      }
      return this.providerIsConfigured(p) && this.step2ConnectionVerified;
    },

    get canGoNext() {
      if (this.step === 2) return this.wizardProviderReady;
      if (this.step === 3) return this.agentName.trim().length > 0;
      return true;
    },

    /** Primary label for step 2 forward button (never implies "skip" when disabled). */
    get wizardStep2ContinueLabel() {
      if (this.wizardProviderReady) return 'Next';
      if (this.step === 2 && this.savingKey) return 'Saving and verifying…';
      if (this.step === 2 && this.testingProvider) return 'Verifying connection…';
      if (this.selectedProvider === 'claude-code') return 'Detect Claude Code to continue';
      return 'Save API key to continue';
    },

    claudeCodeDetected: false,

    get hasConfiguredProvider() {
      var self = this;
      return this.providers.some(function(p) {
        return p.auth_status === 'configured';
      });
    },

    // ── Step 2: Providers ──

    async loadProviders() {
      try {
        var data = await OpenFangAPI.get('/api/providers');
        this.providers = data.providers || [];
      } catch(e) { this.providers = []; }
    },

    get selectedProviderObj() {
      var self = this;
      var match = this.providers.filter(function(p) { return p.id === self.selectedProvider; });
      return match.length > 0 ? match[0] : null;
    },

    get popularProviders() {
      var popular = ['openrouter', 'anthropic', 'openai', 'gemini', 'groq', 'deepseek', 'claude-code'];
      return this.providers.filter(function(p) {
        return popular.indexOf(p.id) >= 0;
      }).sort(function(a, b) {
        return popular.indexOf(a.id) - popular.indexOf(b.id);
      });
    },

    get otherProviders() {
      var popular = ['openrouter', 'anthropic', 'openai', 'gemini', 'groq', 'deepseek', 'claude-code'];
      return this.providers.filter(function(p) {
        return popular.indexOf(p.id) < 0;
      });
    },

    selectProvider(id) {
      this.providerVerifyToken++;
      this.selectedProvider = id;
      this.apiKeyInput = '';
      this.testResult = null;
      this.step2ConnectionVerified = false;
      var match = this.providers.filter(function(p) { return p.id === id; });
      var p = match.length > 0 ? match[0] : null;
      if (!p || !this.providerIsConfigured(p)) {
        this.keySaved = false;
      }
      var self = this;
      queueMicrotask(function() { self.maybeAutoVerifyStep2Provider(); });
    },

    /**
     * If the selected provider already has a key on the daemon, run a connection test automatically
     * so the user does not advance (or think they are ready) without a working outbound call.
     */
    async maybeAutoVerifyStep2Provider() {
      if (this.step !== 2) return;
      var p = this.selectedProviderObj;
      if (!p || this.selectedProvider === 'claude-code') return;
      if (!p.api_key_env) {
        this.step2ConnectionVerified = this.providerIsConfigured(p);
        return;
      }
      if (!this.providerIsConfigured(p)) {
        this.step2ConnectionVerified = false;
        return;
      }
      await this.runProviderConnectionTest({
        successToast: false,
        errorToast: true,
      });
    },

    providerHelp: function(id) {
      var help = {
        anthropic: { url: 'https://console.anthropic.com/settings/keys', text: 'Get your key from the Anthropic Console' },
        openai: { url: 'https://platform.openai.com/api-keys', text: 'Get your key from the OpenAI Platform' },
        gemini: { url: 'https://aistudio.google.com/apikey', text: 'Get your key from Google AI Studio' },
        groq: { url: 'https://console.groq.com/keys', text: 'Get your key from the Groq Console (free tier available)' },
        deepseek: { url: 'https://platform.deepseek.com/api_keys', text: 'Get your key from the DeepSeek Platform (very affordable)' },
        openrouter: { url: 'https://openrouter.com', text: 'Get a free API key at openrouter.com (then create a key in the dashboard). ArmaraOS defaults to the free model stepfun/step-3.5-flash:free unless you pick a paid model or another provider.' },
        mistral: { url: 'https://console.mistral.ai/api-keys', text: 'Get your key from the Mistral Console' },
        together: { url: 'https://api.together.xyz/settings/api-keys', text: 'Get your key from Together AI' },
        fireworks: { url: 'https://fireworks.ai/account/api-keys', text: 'Get your key from Fireworks AI' },
        perplexity: { url: 'https://www.perplexity.ai/settings/api', text: 'Get your key from Perplexity Settings' },
        cohere: { url: 'https://dashboard.cohere.com/api-keys', text: 'Get your key from the Cohere Dashboard' },
        xai: { url: 'https://console.x.ai/', text: 'Get your key from the xAI Console' },
        'claude-code': { url: 'https://docs.anthropic.com/en/docs/claude-code', text: 'Install: npm install -g @anthropic-ai/claude-code && claude auth (no API key needed)' }
      };
      return help[id] || null;
    },

    providerIsConfigured(p) {
      return p && p.auth_status === 'configured';
    },

    async saveKey() {
      var provider = this.selectedProviderObj;
      if (!provider) return;
      var key = this.apiKeyInput.trim();
      if (!key) {
        OpenFangToast.error('Please enter an API key');
        return;
      }
      this.savingKey = true;
      this.step2ConnectionVerified = false;
      try {
        await OpenFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/key', { key: key });
        this.providerVerifyToken++;
        await this.loadProviders();
        var ok = await this.runProviderConnectionTest({
          successToast: false,
          errorToast: false,
        });
        if (ok) {
          this.apiKeyInput = '';
          this.keySaved = true;
          this.setupSummary.provider = provider.display_name;
          var ms = this.testResult && this.testResult.latency_ms != null ? this.testResult.latency_ms : '?';
          OpenFangToast.success(provider.display_name + ' key saved and verified (' + ms + 'ms)');
        } else {
          this.keySaved = false;
          var detail =
            (this.testResult && this.testResult.error) ||
            (this.testResult && this.testResult.status !== 'ok' ? 'connection test failed' : '') ||
            'connection test failed';
          OpenFangToast.error(
            provider.display_name + ' key was saved, but verification failed: ' + detail + '. Fix the key or click Test connection, then continue.'
          );
        }
      } catch(e) {
        OpenFangToast.error('Failed to save key: ' + e.message);
        this.step2ConnectionVerified = false;
        this.testingProvider = false;
      }
      this.savingKey = false;
    },

    /**
     * POST /api/providers/:id/test and update step2ConnectionVerified + testResult.
     * @param {{ successToast?: boolean, errorToast?: boolean }} options
     * @returns {Promise<boolean>}
     */
    async runProviderConnectionTest(options) {
      options = options || {};
      var successToast = options.successToast !== false;
      var errorToast = options.errorToast !== false;
      var provider = this.selectedProviderObj;
      if (!provider) return false;
      var tokenAtStart = this.providerVerifyToken;
      this.testingProvider = true;
      this.testResult = null;
      var ok = false;
      try {
        var result = await OpenFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/test', {});
        if (tokenAtStart !== this.providerVerifyToken) {
          return false;
        }
        this.testResult = result;
        ok = result.status === 'ok';
        if (ok) {
          this.step2ConnectionVerified = true;
          if (successToast) {
            OpenFangToast.success(provider.display_name + ' connected (' + (result.latency_ms || '?') + 'ms)');
          }
        } else {
          this.step2ConnectionVerified = false;
          if (errorToast) {
            OpenFangToast.error(provider.display_name + ': ' + (result.error || 'Connection failed'));
          }
        }
      } catch(e) {
        if (tokenAtStart !== this.providerVerifyToken) {
          return false;
        }
        this.testResult = { status: 'error', error: e.message };
        this.step2ConnectionVerified = false;
        if (errorToast) {
          OpenFangToast.error('Connection test failed: ' + e.message);
        }
        ok = false;
      } finally {
        if (tokenAtStart === this.providerVerifyToken) {
          this.testingProvider = false;
        }
      }
      return ok;
    },

    async testKey() {
      await this.runProviderConnectionTest({ successToast: true, errorToast: true });
    },

    async detectClaudeCode() {
      this.testingProvider = true;
      this.testResult = null;
      try {
        var result = await OpenFangAPI.post('/api/providers/claude-code/test', {});
        this.testResult = result;
        if (result.status === 'ok') {
          this.claudeCodeDetected = true;
          this.keySaved = true;
          this.step2ConnectionVerified = true;
          this.setupSummary.provider = 'Claude Code';
          OpenFangToast.success('Claude Code detected (' + (result.latency_ms || '?') + 'ms)');
        } else {
          this.step2ConnectionVerified = false;
          this.testResult = { status: 'error', error: 'Claude Code CLI not detected' };
          OpenFangToast.error('Claude Code CLI not detected. Make sure you\'ve run: npm install -g @anthropic-ai/claude-code && claude auth');
        }
      } catch(e) {
        this.step2ConnectionVerified = false;
        this.testResult = { status: 'error', error: e.message };
        OpenFangToast.error('Claude Code CLI not detected. Make sure you\'ve run: npm install -g @anthropic-ai/claude-code && claude auth');
      }
      this.testingProvider = false;
    },

    // ── Step 3: Agent creation ──

    selectTemplate(index) {
      this.selectedTemplate = index;
      var tpl = this.templates[index];
      if (tpl) {
        this.agentName = tpl.name.toLowerCase().replace(/\s+/g, '-');
      }
    },

    async createAgent() {
      var tpl = this.templates[this.selectedTemplate];
      if (!tpl) return;
      var name = this.agentName.trim();
      if (!name) {
        OpenFangToast.error('Please enter a name for your agent');
        return;
      }

      // Use the provider the user just configured, or the template default
      var provider = tpl.provider;
      var model = tpl.model;
      if (this.selectedProviderObj && this.providerIsConfigured(this.selectedProviderObj)) {
        provider = this.selectedProviderObj.id;
        // Use a sensible default model for the provider
        model = this.defaultModelForProvider(provider) || tpl.model;
      }

      var toml = '[agent]\n';
      toml += 'name = "' + wizardTomlBasicEscape(name) + '"\n';
      toml += 'description = "' + wizardTomlBasicEscape(tpl.description) + '"\n';
      toml += 'profile = "' + tpl.profile + '"\n\n';
      toml += '[model]\nprovider = "' + provider + '"\n';
      toml += 'model = "' + model + '"\n';
      toml += 'system_prompt = """\n' + wizardTomlMultilineEscape(tpl.system_prompt) + '\n"""\n';

      this.creatingAgent = true;
      try {
        var res = await OpenFangAPI.post('/api/agents', { manifest_toml: toml });
        if (res.agent_id) {
          this.createdAgent = { id: res.agent_id, name: res.name || name };
          this.setupSummary.agent = res.name || name;
          OpenFangToast.success('Agent "' + (res.name || name) + '" created');
          await Alpine.store('app').refreshAgents();
        } else {
          OpenFangToast.error('Failed: ' + (res.error || 'Unknown error'));
        }
      } catch(e) {
        OpenFangToast.error('Failed to create agent: ' + e.message);
      }
      this.creatingAgent = false;
    },

    defaultModelForProvider(providerId) {
      var defaults = {
        anthropic: 'claude-sonnet-4-20250514',
        openai: 'gpt-4o',
        gemini: 'gemini-2.5-flash',
        groq: 'llama-3.3-70b-versatile',
        deepseek: 'deepseek-chat',
        openrouter: 'stepfun/step-3.5-flash:free',
        mistral: 'mistral-large-latest',
        together: 'meta-llama/Llama-3-70b-chat-hf',
        fireworks: 'accounts/fireworks/models/llama-v3p1-70b-instruct',
        perplexity: 'llama-3.1-sonar-large-128k-online',
        cohere: 'command-r-plus',
        xai: 'grok-2',
        'claude-code': 'claude-code/sonnet'
      };
      return defaults[providerId] || '';
    },

    // ── Step 5: Channel setup ──

    selectChannel(name) {
      if (this.channelType === name) {
        this.channelType = '';
        this.channelToken = '';
      } else {
        this.channelType = name;
        this.channelToken = '';
      }
    },

    get selectedChannelObj() {
      var self = this;
      var match = this.channelOptions.filter(function(ch) { return ch.name === self.channelType; });
      return match.length > 0 ? match[0] : null;
    },

    async configureChannel() {
      var ch = this.selectedChannelObj;
      if (!ch) return;
      var token = this.channelToken.trim();
      if (!token) {
        OpenFangToast.error('Please enter the ' + ch.token_label);
        return;
      }
      this.configuringChannel = true;
      try {
        // Channel configure endpoint expects canonical field keys (e.g. bot_token_env),
        // not raw env var names. It will write the secret to secrets.env and set the env var.
        await OpenFangAPI.post('/api/channels/' + ch.name + '/configure', {
          fields: { bot_token_env: token }
        });
        this.channelConfigured = true;
        this.setupSummary.channel = ch.display_name;
        OpenFangToast.success(ch.display_name + ' configured and activated.');
      } catch(e) {
        OpenFangToast.error('Failed: ' + (e.message || 'Unknown error'));
      }
      this.configuringChannel = false;
    },

    // ── Step 6: Finish ──

    async finish() {
      localStorage.setItem('openfang-onboarded', 'true');
      Alpine.store('app').showOnboarding = false;
      // If we created an agent, automatically create a simple scheduled job so
      // the first-run flow ends with something visible in Scheduler + Logs.
      try {
        if (this.createdAgent && this.createdAgent.id) {
          var created = await OpenFangAPI.post('/api/schedules', {
            name: 'Daily check-in',
            cron: '0 9 * * *',
            agent_id: this.createdAgent.id,
            message: 'Daily check-in: summarize what changed since yesterday and suggest 1 next action.',
            enabled: true
          });
          if (created && created.id) {
            this.setupSummary.schedule = created.name || 'Daily check-in';
            // Run once immediately so the user sees output right away.
            try {
              await OpenFangAPI.post('/api/schedules/' + created.id + '/run', {});
            } catch (e2) { /* ignore */ }
          }
        }
      } catch (e) { /* ignore */ }
      // Navigate to agents with chat if an agent was created, otherwise overview
      if (this.createdAgent) {
        var agent = this.createdAgent;
        Alpine.store('app').pendingAgent = { id: agent.id, name: agent.name, model_provider: '?', model_name: '?' };
        window.location.hash = 'agents';
      } else {
        window.location.hash = 'overview';
      }
    },

    finishAndDismiss() {
      localStorage.setItem('openfang-onboarded', 'true');
      Alpine.store('app').showOnboarding = false;
      window.location.hash = 'overview';
    }
  };
}
