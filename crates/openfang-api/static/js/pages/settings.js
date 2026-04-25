// ArmaraOS Settings Page — Provider Hub, Model Catalog, Config, Tools + Security, Network, Migration tabs
'use strict';

function settingsPage() {
  // `Object.assign` invokes source getters once and copies returned values as
  // plain properties. Use methods (`filteredToolsList`) for derived lists, not
  // `get filteredTools()` accessors, or the UI stays stuck on the initial snapshot.
  return Object.assign(armaraosDaemonLifecycleControls(), {
    tab: 'providers',
    sysInfo: {},
    usageData: [],
    tools: [],
    /** Per-agent inventory from GET /api/agents/:id/llm-tools (what the model receives). */
    llmToolsAgents: [],
    llmToolsAgentId: '',
    llmToolsLoading: false,
    llmToolsError: '',
    llmToolsResult: null,
    llmToolSearch: '',
    config: {},
    providers: [],
    models: [],
    toolSearch: '',
    modelSearch: '',
    modelProviderFilter: '',
    modelTierFilter: '',
    /** Set when GET /api/models fails or returns an unexpected shape (otherwise the UI looks empty). */
    modelsLoadError: '',
    /** Set when GET /api/providers fails (Providers hub + Models tab diagnostics). */
    providersLoadError: '',
    catalogRefreshing: false,
    showCustomModelForm: false,
    editingCustomModelId: '',
    customModelId: '',
    customModelProvider: 'openrouter',
    customModelContext: 128000,
    customModelMaxOutput: 8192,
    customModelStatus: '',
    providerKeyInputs: {},
    providerUrlInputs: {},
    providerUrlSaving: {},
    providerDeleting: {},
    providerTesting: {},
    providerTestResults: {},
    copilotOAuth: { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 },
    customProviderName: '',
    customProviderUrl: '',
    customProviderKey: '',
    customProviderStatus: '',
    addingCustomProvider: false,
    loading: true,
    /** From GET /api/system/mcp-host-readiness (Google Workspace MCP + uvx). */
    mcpHostReadiness: null,
    googleOauthClientId: '',
    googleOauthClientSecret: '',
    googleOauthSaving: false,
    uvBootstrapBusy: false,
    loadError: '',
    loadErrorDetail: '',
    loadErrorHint: '',
    loadErrorRequestId: '',
    loadErrorWhere: '',
    loadErrorServerPath: '',
    graphMemoryAgents: [],
    graphMemoryAgentId: '',
    graphMemoryControls: {
      memory_enabled: true,
      temporary_mode: false,
      shared_memory_enabled: false,
      include_episodic_hints: true,
      include_semantic_facts: true,
      include_conflicts: true,
      include_procedural_hints: true,
      include_suggested_pattern_candidates: true,
    },
    graphMemoryControlsSaving: false,

    // -- Desktop (Tauri) AINL status (synced via Alpine.store('ainl').desktop) --
    ainlDesktopLoading: false,
    ainlDesktopError: '',
    ainlBootstrapProgress: '',
    ainlVersionInfo: null,
    ainlVersionLoading: false,
    ainlUpgradeLoading: false,
    /** ISO timestamp of last PyPI/GitHub version check (desktop AINL tab). */
    ainlVersionCheckedAt: null,

    // -- Desktop (Tauri) app updates --
    updateChecking: false,
    updateInfo: null,
    updaterPrefs: null,
    releaseChannelSaving: false,
    daemonUpdateChecking: false,
    daemonUpdateInfo: null,

    // -- Support / diagnostics --
    diagGenerating: false,
    diagBundlePath: '',
    diagBundleFilename: '',
    diagRelativePath: '',
    diagDownloadsPath: '',
    diagError: '',

    get configSchemaLine() {
      var a = this.sysInfo.config_schema_version;
      var b = this.sysInfo.config_schema_version_binary;
      if (a == null && b == null) return '—';
      var effective = a != null ? a : '?';
      var binary = b != null ? b : '?';
      var mismatch = (a != null && b != null && a !== b) ? ' ⚠ mismatch' : '';
      return String(effective) + ' (binary ' + String(binary) + ')' + mismatch;
    },

    // -- Dynamic config state --
    configSchema: null,
    configValues: {},
    configDirty: {},
    configSaving: {},

    // -- Security state --
    securityData: null,
    secLoading: false,
    verifyingChain: false,
    chainResult: null,

    // -- Voice settings state (Settings → Voice tab) --
    voiceLoading: false,
    voiceData: null,
    sayVoices: [],
    customVoices: [],
    voicePrefDraft: { preferred_say_voice: '', prefer_macos_say: false, custom_piper_voice: '', kokoro_voice: 'af_heart' },
    voicePrefServer: { preferred_say_voice: '', prefer_macos_say: false, custom_piper_voice: '', kokoro_voice: 'af_heart' },
    voicePrefSaving: false,
    voicePrefStatus: '',
    voiceUploading: false,
    voiceUploadError: '',
    onnxFileInput: null,
    onnxJsonFileInput: null,
    onnxIdInput: '',
    /** MIT-licensed Kokoro-82M voice ids (American/British/Japanese/etc). Populated statically;
        the runtime auto-downloads the chosen one from HuggingFace `onnx-community/Kokoro-82M-v1.0-ONNX`. */
    kokoroVoices: [
      'af_heart', 'af_bella', 'af_nicole', 'af_sarah', 'af_sky',
      'am_adam', 'am_michael',
      'bf_emma', 'bf_isabella',
      'bm_george', 'bm_lewis'
    ],

    /** Computed rows for the auto-installed components panel (Voice tab). */
    get voiceComponentRows() {
      var c = (this.voiceData && this.voiceData.components) || {};
      return [
        { key: 'whisper_cli', label: 'Whisper CLI (speech-to-text engine)', ok: !!(c.whisper_cli && c.whisper_cli.exists), path: c.whisper_cli && c.whisper_cli.path },
        { key: 'whisper_model', label: 'Whisper ggml-base model (~150 MiB)', ok: !!(c.whisper_model && c.whisper_model.exists), path: c.whisper_model && c.whisper_model.path },
        { key: 'piper_binary', label: 'Piper binary', ok: !!(c.piper_binary && c.piper_binary.exists), path: c.piper_binary && c.piper_binary.path },
        { key: 'piper_voice', label: 'Piper bundled voice (en_US-lessac-medium)', ok: !!(c.piper_voice && c.piper_voice.exists), path: c.piper_voice && c.piper_voice.path },
        { key: 'active_piper_voice', label: 'Piper active voice (resolved at synth time)', ok: !!(c.active_piper_voice && c.active_piper_voice.exists), path: c.active_piper_voice && c.active_piper_voice.path },
        { key: 'macos_say', label: 'macOS /usr/bin/say (built-in fallback)', ok: !!(this.voiceData && (this.voiceData.macos_say_binary_present || this.voiceData.macos_say_ready)), path: '/usr/bin/say' },
        { key: 'kokoro', label: 'Kokoro-82M model (opt-in, ~310 MiB)', ok: !!(c.kokoro_model && c.kokoro_model.exists), path: c.kokoro_model && c.kokoro_model.path },
      ];
    },

    coreFeatures: [
      {
        name: 'Path Traversal Prevention', key: 'path_traversal',
        description: 'Blocks directory escape attacks (../) in all file operations. Two-phase validation: syntactic rejection of path components, then canonicalization to normalize symlinks.',
        threat: 'Directory escape, privilege escalation via symlinks',
        impl: 'host_functions.rs — safe_resolve_path() + safe_resolve_parent()'
      },
      {
        name: 'SSRF Protection', key: 'ssrf_protection',
        description: 'Blocks outbound requests to private IPs, localhost, and cloud metadata endpoints (AWS/GCP/Azure). Validates DNS resolution results to defeat rebinding attacks.',
        threat: 'Internal network reconnaissance, cloud credential theft',
        impl: 'host_functions.rs — is_ssrf_target() + is_private_ip()'
      },
      {
        name: 'Capability-Based Access Control', key: 'capability_system',
        description: 'Deny-by-default permission system. Every agent operation (file I/O, network, shell, memory, spawn) requires an explicit capability grant in the manifest.',
        threat: 'Unauthorized resource access, sandbox escape',
        impl: 'host_functions.rs — check_capability() on every host function'
      },
      {
        name: 'Privilege Escalation Prevention', key: 'privilege_escalation_prevention',
        description: 'When a parent agent spawns a child, the kernel enforces child capabilities are a subset of parent capabilities. No agent can grant rights it does not have.',
        threat: 'Capability escalation through agent spawning chains',
        impl: 'kernel_handle.rs — spawn_agent_checked()'
      },
      {
        name: 'Subprocess Environment Isolation', key: 'subprocess_isolation',
        description: 'Child processes (shell tools) inherit only a safe allow-list of environment variables. API keys, database passwords, and secrets are never leaked to subprocesses.',
        threat: 'Secret exfiltration via child process environment',
        impl: 'subprocess_sandbox.rs — env_clear() + SAFE_ENV_VARS'
      },
      {
        name: 'Security Headers', key: 'security_headers',
        description: 'Every HTTP response includes CSP, X-Frame-Options: DENY, X-Content-Type-Options: nosniff, Referrer-Policy, and X-XSS-Protection headers.',
        threat: 'XSS, clickjacking, MIME sniffing, content injection',
        impl: 'middleware.rs — security_headers()'
      },
      {
        name: 'Wire Protocol Authentication', key: 'wire_hmac_auth',
        description: 'Agent-to-agent OFP connections use HMAC-SHA256 mutual authentication with nonce-based handshake and constant-time signature comparison (subtle crate).',
        threat: 'Man-in-the-middle attacks on mesh network',
        impl: 'peer.rs — hmac_sign() + hmac_verify()'
      },
      {
        name: 'Request ID Tracking', key: 'request_id_tracking',
        description: 'Every API request receives a unique UUID (x-request-id header) and is logged with method, path, status code, and latency for full traceability.',
        threat: 'Untraceable actions, forensic blind spots',
        impl: 'middleware.rs — request_logging()'
      }
    ],

    configurableFeatures: [
      {
        name: 'API Rate Limiting', key: 'rate_limiter',
        description: 'GCRA (Generic Cell Rate Algorithm) with cost-aware tokens. Different endpoints cost different amounts — spawning an agent costs 50 tokens, health check costs 1.',
        configHint: 'Hard-coded: 500 tokens/minute per IP. Edit rate_limiter.rs to tune.',
        valueKey: 'rate_limiter'
      },
      {
        name: 'WebSocket Connection Limits', key: 'websocket_limits',
        description: 'Per-IP connection cap prevents connection exhaustion. Idle timeout closes abandoned connections. Message rate limiting prevents flooding.',
        configHint: 'Hard-coded: 5 connections/IP, 30min idle timeout, 64KB max message. Edit ws.rs to tune.',
        valueKey: 'websocket_limits'
      },
      {
        name: 'WASM Dual Metering', key: 'wasm_sandbox',
        description: 'WASM modules run with two independent resource limits: fuel metering (CPU instruction count) and epoch interruption (wall-clock timeout with watchdog thread).',
        configHint: 'Default: 1M fuel units, 30s timeout. Configurable per-agent via SandboxConfig.',
        valueKey: 'wasm_sandbox'
      },
      {
        name: 'Bearer Token Authentication', key: 'auth',
        description: 'All non-health endpoints require Authorization: Bearer header. When no API key is configured, all requests are restricted to localhost only.',
        configHint: 'Set api_key in ~/.openfang/config.toml for remote access. Empty = localhost only.',
        valueKey: 'auth'
      }
    ],

    monitoringFeatures: [
      {
        name: 'Merkle Audit Trail', key: 'audit_trail',
        description: 'Every security-critical action is appended to an immutable, tamper-evident log. Each entry is cryptographically linked to the previous via SHA-256 hash chain.',
        configHint: 'Always active. Verify chain integrity from the Audit Log page.',
        valueKey: 'audit_trail'
      },
      {
        name: 'Information Flow Taint Tracking', key: 'taint_tracking',
        description: 'Labels data by provenance (ExternalNetwork, UserInput, PII, Secret, UntrustedAgent) and blocks unsafe flows: external data cannot reach shell_exec, secrets cannot reach network.',
        configHint: 'Always active. Prevents data flow attacks automatically.',
        valueKey: 'taint_tracking'
      },
      {
        name: 'Ed25519 Manifest Signing', key: 'manifest_signing',
        description: 'Agent manifests can be cryptographically signed with Ed25519. Verify manifest integrity before loading to prevent supply chain tampering.',
        configHint: 'Available for use. Sign manifests with ed25519-dalek for verification.',
        valueKey: 'manifest_signing'
      }
    ],

    // -- Peers state --
    peers: [],
    peersLoading: false,
    peersLoadError: '',
    _peerPollTimer: null,

    // -- Migration state --
    migStep: 'intro',
    detecting: false,
    scanning: false,
    migrating: false,
    sourcePath: '',
    targetPath: '',
    scanResult: null,
    migResult: null,

    // -- PostHog dashboard analytics (Settings → System; optional compile-time key) --
    dashboardAnalyticsConfigured: false,
    dashboardAnalyticsOk: false,
    dashboardReplayOk: false,

    loadDashboardAnalyticsPrefs() {
      this.dashboardAnalyticsConfigured = false;
      this.dashboardAnalyticsOk = false;
      this.dashboardReplayOk = false;
      try {
        var a = typeof window !== 'undefined' ? window.__ARMARAOS_ANALYTICS__ : null;
        if (!a || typeof a.configured !== 'function' || !a.configured()) return;
        this.dashboardAnalyticsConfigured = true;
        this.dashboardAnalyticsOk = typeof a.allow === 'function' ? !!a.allow() : false;
        this.dashboardReplayOk = typeof a.replayAllow === 'function' ? !!a.replayAllow() : false;
      } catch (e) { /* ignore */ }
    },

    applyDashboardAnalyticsConsent() {
      try {
        if (!this.dashboardAnalyticsOk) this.dashboardReplayOk = false;
        var a = typeof window !== 'undefined' ? window.__ARMARAOS_ANALYTICS__ : null;
        if (!a || typeof a.setConsent !== 'function') return;
        a.setConsent(!!this.dashboardAnalyticsOk, !!this.dashboardReplayOk);
      } catch (e) { /* ignore */ }
    },

    // -- Settings load --
    async loadSettings() {
      try {
        var stTab = sessionStorage.getItem('armaraos-settings-tab');
        if (stTab) {
          this.tab = stTab;
          sessionStorage.removeItem('armaraos-settings-tab');
        }
      } catch (e) { /* ignore */ }
      this.loading = true;
      clearPageLoadError(this);
      // Always start Settings with a clean model filter state; stale webview
      // autofill/search state should never hide the whole catalog.
      this.modelSearch = '';
      this.modelProviderFilter = '';
      this.modelTierFilter = '';
      this.modelsLoadError = '';
      this.providersLoadError = '';
      try {
        await Promise.all([
          this.loadSysInfo(),
          this.loadUsage(),
          this.loadTools(),
          this.loadLlmToolsAgents(),
          this.loadMcpHostReadiness(),
          this.loadConfig(),
          this.loadProviders(),
          this.loadModels(),
          this.loadUpdaterPrefs(),
          this.loadGraphMemoryControls()
        ]);
        // Fail-safe: if both arrays are empty, retry once serially so transient
        // startup races do not leave the Models tab blank.
        if (!this.models.length && !this.providers.length) {
          await this.loadProviders();
          await this.loadModels();
        }
      } catch(e) {
        applyPageLoadError(this, e, 'Could not load settings.');
      }
      this.loadDashboardAnalyticsPrefs();
      try {
        if (typeof window !== 'undefined' && typeof window.armaraosAnalyticsInit === 'function') {
          window.armaraosAnalyticsInit();
        }
      } catch (eA) { /* ignore */ }
      try {
        var A = typeof window !== 'undefined' ? window.__ARMARAOS_ANALYTICS__ : null;
        if (A && typeof A.usageSnapshot === 'function' && A.configured && A.configured()) {
          var provs2 = this.providers || [];
          var nConf = 0;
          for (var pi = 0; pi < provs2.length; pi++) {
            if (provs2[pi] && provs2[pi].auth_status === 'configured') nConf++;
          }
          var ac =
            this.sysInfo && typeof this.sysInfo.agent_count === 'number' ? this.sysInfo.agent_count : undefined;
          var dpk = '';
          if (this.sysInfo && this.sysInfo.default_provider) {
            dpk = String(this.sysInfo.default_provider).split(/[:/]/)[0] || '';
          }
          A.usageSnapshot({
            agent_count: ac,
            providers_configured: nConf,
            default_provider_kind: dpk || undefined,
          });
        }
      } catch (eS) { /* ignore */ }
      this.loading = false;
    },

    copySettingsLoadErrorDebug() {
      copyPageLoadErrorDebug(this, 'ArmaraOS settings load error');
    },

    async loadData() { return this.loadSettings(); },

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
      var ch = this.updaterPrefs.release_channel || 'stable';
      this.releaseChannelSaving = true;
      try {
        await ArmaraosDesktopTauriInvoke('set_release_channel', { channel: ch });
        await this.loadUpdaterPrefs();
        OpenFangToast && OpenFangToast.success('Update channel saved');
      } catch(e) {
        OpenFangToast && OpenFangToast.error(openFangErrText(e) || String(e));
      }
      this.releaseChannelSaving = false;
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
        try {
          var ar = await OpenFangAPI.get('/api/ainl/runtime-version');
          Alpine.store('notifyCenter').syncAinlPypiUpdate(ar.pip_version, ar.pypi_latest_version);
        } catch (eAinl) { /* ignore */ }
      } catch(e) {
        err = e.message || String(e);
        this.daemonUpdateInfo = { error: err };
      }
      if (this.isDesktopShell) {
        try {
          await ArmaraosDesktopTauriInvoke('report_daemon_update_check', { error: err });
          await this.loadUpdaterPrefs();
        } catch(e2) { /* ignore */ }
      }
      this.daemonUpdateChecking = false;
    },

    async loadSysInfo() {
      try {
        var ver = await OpenFangAPI.get('/api/version');
        var status = await OpenFangAPI.get('/api/status');
        this.sysInfo = {
          version: ver.version || '-',
          platform: ver.platform || '-',
          arch: ver.arch || '-',
          uptime_seconds: status.uptime_seconds || 0,
          agent_count: status.agent_count || 0,
          default_provider: status.default_provider || '-',
          default_model: status.default_model || '-',
          api_listen: status.api_listen || status.listen || '-',
          home_dir: status.home_dir || '-',
          log_level: status.log_level || '-',
          network_enabled: !!status.network_enabled,
          config_schema_version: status.config_schema_version != null ? status.config_schema_version : null,
          config_schema_version_binary: status.config_schema_version_binary != null ? status.config_schema_version_binary : null
        };
      } catch(e) { throw e; }
    },

    async loadGraphMemoryControls() {
      try {
        var agentsResp = await OpenFangAPI.get('/api/agents');
        this.graphMemoryAgents = Array.isArray(agentsResp)
          ? agentsResp
          : ((agentsResp && agentsResp.agents) || []);
        if (!this.graphMemoryAgents.length) {
          this.graphMemoryAgentId = '';
          return;
        }
        if (!this.graphMemoryAgentId) {
          this.graphMemoryAgentId = String(this.graphMemoryAgents[0].id || '');
        }
        await this.onGraphMemoryAgentChange();
      } catch (e) {
        this.graphMemoryAgents = [];
      }
    },

    async onGraphMemoryAgentChange() {
      if (!this.graphMemoryAgentId) return;
      try {
        var data = await OpenFangAPI.get(
          '/api/graph-memory/controls?agent_id=' + encodeURIComponent(this.graphMemoryAgentId)
        );
        var c = (data && data.controls) || {};
        this.graphMemoryControls = {
          memory_enabled: c.memory_enabled !== false,
          temporary_mode: !!c.temporary_mode,
          shared_memory_enabled: !!c.shared_memory_enabled,
          include_episodic_hints: c.include_episodic_hints !== false,
          include_semantic_facts: c.include_semantic_facts !== false,
          include_conflicts: c.include_conflicts !== false,
          include_procedural_hints: c.include_procedural_hints !== false,
          include_suggested_pattern_candidates: c.include_suggested_pattern_candidates !== false,
        };
      } catch (e) {
        // keep current values on transient errors
      }
    },

    async saveGraphMemoryControls() {
      if (!this.graphMemoryAgentId || this.graphMemoryControlsSaving) return;
      this.graphMemoryControlsSaving = true;
      try {
        await OpenFangAPI.put('/api/graph-memory/controls', {
          agent_id: this.graphMemoryAgentId,
          memory_enabled: !!this.graphMemoryControls.memory_enabled,
          temporary_mode: !!this.graphMemoryControls.temporary_mode,
          shared_memory_enabled: !!this.graphMemoryControls.shared_memory_enabled,
          include_episodic_hints: !!this.graphMemoryControls.include_episodic_hints,
          include_semantic_facts: !!this.graphMemoryControls.include_semantic_facts,
          include_conflicts: !!this.graphMemoryControls.include_conflicts,
          include_procedural_hints: !!this.graphMemoryControls.include_procedural_hints,
          include_suggested_pattern_candidates:
            !!this.graphMemoryControls.include_suggested_pattern_candidates,
        });
        OpenFangToast && OpenFangToast.success('Graph memory controls saved');
      } catch (e) {
        OpenFangToast &&
          OpenFangToast.error(openFangErrText(e) || 'Failed to save graph memory controls');
      }
      this.graphMemoryControlsSaving = false;
    },

    async loadUsage() {
      try {
        var data = await OpenFangAPI.get('/api/usage');
        this.usageData = data.agents || [];
      } catch(e) { this.usageData = []; }
    },

    async loadTools() {
      try {
        var data = await OpenFangAPI.get('/api/tools');
        this.tools = data.tools || [];
      } catch(e) { this.tools = []; }
    },

    /** Load agent ids for the LLM tool inventory picker; does not throw (errors stay local). */
    async loadLlmToolsAgents() {
      this.llmToolsError = '';
      try {
        var data = await OpenFangAPI.get('/api/agents');
        var agents = data && data.agents != null ? data.agents : data;
        if (!Array.isArray(agents)) agents = [];
        this.llmToolsAgents = agents;
        var firstId = '';
        for (var i = 0; i < agents.length; i++) {
          var a = agents[i];
          if (a && (a.id || a.agent_id)) {
            firstId = String(a.id || a.agent_id);
            break;
          }
        }
        if (!this.llmToolsAgentId && firstId) {
          this.llmToolsAgentId = firstId;
        }
        if (this.llmToolsAgentId) {
          await this.fetchLlmToolsInventory();
        } else {
          this.llmToolsResult = null;
        }
      } catch (e) {
        this.llmToolsAgents = [];
        this.llmToolsResult = null;
        this.llmToolsError = openFangErrText(e) || String(e);
      }
    },

    async fetchLlmToolsInventory() {
      if (!this.llmToolsAgentId) {
        this.llmToolsResult = null;
        return;
      }
      this.llmToolsLoading = true;
      this.llmToolsError = '';
      try {
        this.llmToolsResult = await OpenFangAPI.get(
          '/api/agents/' + encodeURIComponent(this.llmToolsAgentId) + '/llm-tools'
        );
      } catch (e) {
        this.llmToolsResult = null;
        this.llmToolsError = openFangErrText(e) || String(e);
      }
      this.llmToolsLoading = false;
    },

    async loadMcpHostReadiness() {
      try {
        this.mcpHostReadiness = await OpenFangAPI.get('/api/system/mcp-host-readiness');
      } catch (e) {
        this.mcpHostReadiness = null;
      }
    },

    async saveGoogleWorkspaceOAuth() {
      var id = (this.googleOauthClientId || '').trim();
      if (!id) {
        if (OpenFangToast) OpenFangToast.error('Enter a Google OAuth Client ID from Google Cloud Console.');
        return;
      }
      this.googleOauthSaving = true;
      try {
        await OpenFangAPI.post('/api/integrations/google-workspace/oauth', {
          GOOGLE_OAUTH_CLIENT_ID: id,
          GOOGLE_OAUTH_CLIENT_SECRET: (this.googleOauthClientSecret || '').trim()
        });
        if (OpenFangToast) OpenFangToast.success('Saved. MCP servers that use these env vars will reconnect.');
        await this.loadMcpHostReadiness();
      } catch (e) {
        if (OpenFangToast) OpenFangToast.error('Save failed: ' + (e && e.message ? e.message : String(e)));
      }
      this.googleOauthSaving = false;
    },

    async bootstrapUvFromSettings() {
      this.uvBootstrapBusy = true;
      try {
        var res = await OpenFangAPI.post('/api/system/bootstrap-uv', {});
        if (res && res.ok === false) {
          if (OpenFangToast) OpenFangToast.error(res.error || 'Install failed');
        } else {
          if (OpenFangToast) OpenFangToast.success((res && res.message) || 'uv installed — restart the daemon if `uvx` is still not found');
        }
        await this.loadMcpHostReadiness();
      } catch (e) {
        if (OpenFangToast) OpenFangToast.error('Install failed: ' + (e && e.message ? e.message : String(e)));
      }
      this.uvBootstrapBusy = false;
    },

    async copyUvInstallCommand() {
      var cmd = (this.mcpHostReadiness && this.mcpHostReadiness.uv_install_sh) || 'curl -LsSf https://astral.sh/uv/install.sh | sh';
      try {
        if (typeof copyTextToClipboard === 'function') {
          await copyTextToClipboard(cmd);
          if (OpenFangToast) OpenFangToast.success('Copied install command');
        }
      } catch (e) { /* ignore */ }
    },

    async loadConfig() {
      try {
        this.config = await OpenFangAPI.get('/api/config');
        this._normalizeConfigEco();
      } catch(e) { this.config = {}; }
    },

    _normalizeConfigEco() {
      var c = this.config;
      if (!c || typeof c !== 'object') return;
      if (typeof c.efficient_mode !== 'string' || c.efficient_mode === '') {
        c.efficient_mode = 'off';
      }
      if (!c.adaptive_eco || typeof c.adaptive_eco !== 'object') {
        c.adaptive_eco = { enabled: false, enforce: false };
      } else {
        if (typeof c.adaptive_eco.enabled !== 'boolean') c.adaptive_eco.enabled = !!c.adaptive_eco.enabled;
        if (typeof c.adaptive_eco.enforce !== 'boolean') c.adaptive_eco.enforce = !!c.adaptive_eco.enforce;
      }
    },

    async saveEfficientMode() {
      var mode = (this.config && this.config.efficient_mode) || 'off';
      try {
        await OpenFangAPI.post('/api/config/set', { path: 'efficient_mode', value: mode });
        try { localStorage.setItem('armaraos-eco-mode', mode); } catch(e2) {}
        OpenFangToast && OpenFangToast.success('Efficient mode saved — takes effect on next message');
      } catch(e) {
        OpenFangToast && OpenFangToast.error('Failed to save: ' + (e && e.message ? e.message : String(e)));
      }
    },

    async setAdaptiveEcoEnabled(checked) {
      if (!this.config) this.config = {};
      this._normalizeConfigEco();
      this.config.adaptive_eco.enabled = !!checked;
      try {
        await OpenFangAPI.post('/api/config/set', { path: 'adaptive_eco.enabled', value: !!checked });
        OpenFangToast && OpenFangToast.success('Adaptive eco updated — takes effect on next message');
      } catch(e) {
        OpenFangToast && OpenFangToast.error('Failed to save adaptive eco: ' + (e && e.message ? e.message : String(e)));
        try { await this.loadConfig(); } catch (e2) { /* ignore */ }
      }
    },

    async setAdaptiveEcoEnforce(checked) {
      if (!this.config) this.config = {};
      this._normalizeConfigEco();
      this.config.adaptive_eco.enforce = !!checked;
      try {
        await OpenFangAPI.post('/api/config/set', { path: 'adaptive_eco.enforce', value: !!checked });
        OpenFangToast && OpenFangToast.success('Enforce mode updated — takes effect on next message');
      } catch(e) {
        OpenFangToast && OpenFangToast.error('Failed to save: ' + (e && e.message ? e.message : String(e)));
        try { await this.loadConfig(); } catch (e2) { /* ignore */ }
      }
    },

    async loadProviders() {
      this.providersLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/providers');
        var plist = data && data.providers;
        if (!Array.isArray(plist) && Array.isArray(data)) plist = data;
        this.providers = Array.isArray(plist) ? plist : [];
        for (var i = 0; i < this.providers.length; i++) {
          var p = this.providers[i];
          if (this.canEditProviderUrl(p)) {
            if (!this.providerUrlInputs[p.id]) {
              this.providerUrlInputs[p.id] = p.base_url || '';
            }
            if (this.providerUrlSaving[p.id] === undefined) {
              this.providerUrlSaving[p.id] = false;
            }
          }
        }
      } catch (e) {
        this.providers = [];
        this.providersLoadError = openFangErrText(e) || String(e);
      }
    },

    async loadModels() {
      this.modelsLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/models');
        var mlist = data && data.models;
        if (!Array.isArray(mlist) && data && Array.isArray(data.data)) mlist = data.data;
        if (!Array.isArray(mlist) && Array.isArray(data)) mlist = data;
        this.models = Array.isArray(mlist)
          ? mlist.map(function(m) {
              if (!m) return null;
              if (typeof m === 'string') {
                return {
                  id: m,
                  display_name: m,
                  provider: 'unknown',
                  tier: 'custom',
                  context_window: null,
                  max_output_tokens: null,
                  input_cost_per_m: 0,
                  output_cost_per_m: 0,
                  available: true,
                };
              }
              var id = m.id || m.model || m.name || '';
              var provider =
                m.provider || m.owned_by || m.vendor || m.provider_id || 'unknown';
              var tier = m.tier != null ? m.tier : (m.category || m.level || 'custom');
              return Object.assign({}, m, {
                id: String(id || ''),
                display_name: m.display_name || m.name || String(id || ''),
                provider: String(provider || 'unknown'),
                tier: String(tier || 'custom').toLowerCase(),
              });
            }).filter(Boolean)
          : [];
        // If stale filters/search hide all rows, auto-clear once so the catalog
        // cannot appear empty while models are present.
        if (this.models.length > 0 && this.filteredModelsList().length === 0) {
          if ((this.modelSearch || '').trim() || this.modelProviderFilter || this.modelTierFilter) {
            this.modelSearch = '';
            this.modelProviderFilter = '';
            this.modelTierFilter = '';
          }
        }
        var total = data && typeof data.total === 'number' ? data.total : null;
        if (this.models.length === 0 && total != null && total > 0) {
          this.modelsLoadError =
            'Server reported ' +
            total +
            ' models but returned an empty list. Try Refresh catalog, or restart the daemon.';
        }
      } catch (e) {
        this.models = [];
        this.modelsLoadError = openFangErrText(e) || String(e);
      }
    },

    resetCustomModelForm() {
      this.editingCustomModelId = '';
      this.customModelId = '';
      this.customModelProvider = 'openrouter';
      this.customModelContext = 128000;
      this.customModelMaxOutput = 8192;
      this.customModelStatus = '';
    },

    startEditCustomModel(model) {
      if (!model) return;
      this.showCustomModelForm = true;
      this.editingCustomModelId = String(model.id || '');
      this.customModelId = String(model.id || '');
      this.customModelProvider = String(model.provider || 'openrouter');
      this.customModelContext = model.context_window || 128000;
      this.customModelMaxOutput = model.max_output_tokens || 8192;
      this.customModelStatus = '';
    },

    /** Reload providers + models (e.g. after a transient API failure left the catalog empty). */
    async refreshModelCatalog() {
      if (this.catalogRefreshing) return;
      this.catalogRefreshing = true;
      try {
        await Promise.all([this.loadProviders(), this.loadModels()]);
        if (!this.modelsLoadError && !this.providersLoadError) {
          OpenFangToast && OpenFangToast.success('Catalog refreshed');
        }
      } finally {
        this.catalogRefreshing = false;
      }
    },

    async addCustomModel() {
      var id = this.customModelId.trim();
      if (!id) return;
      var isEdit = !!this.editingCustomModelId;
      this.customModelStatus = isEdit ? 'Saving...' : 'Adding...';
      try {
        var payload = {
          id: id,
          provider: this.customModelProvider || 'openrouter',
          context_window: this.customModelContext || 128000,
          max_output_tokens: this.customModelMaxOutput || 8192,
        };
        if (isEdit) {
          await OpenFangAPI.put(
            '/api/models/custom/' + encodeURIComponent(this.editingCustomModelId),
            payload
          );
          this.customModelStatus = 'Saved!';
        } else {
          await OpenFangAPI.post('/api/models/custom', payload);
          this.customModelStatus = 'Added!';
        }
        this.resetCustomModelForm();
        this.showCustomModelForm = false;
        await this.loadModels();
      } catch(e) {
        this.customModelStatus = 'Error: ' + (e.message || 'Failed');
      }
    },

    async deleteCustomModel(modelId) {
      if (!confirm('Delete custom model "' + modelId + '"?')) return;
      try {
        await OpenFangAPI.del('/api/models/custom/' + encodeURIComponent(modelId));
        OpenFangToast.success('Model deleted');
        if (this.editingCustomModelId === modelId) {
          this.resetCustomModelForm();
          this.showCustomModelForm = false;
        }
        await this.loadModels();
      } catch(e) {
        OpenFangToast.error('Failed to delete: ' + openFangErrText(e));
      }
    },

    async loadConfigSchema() {
      try {
        var results = await Promise.all([
          OpenFangAPI.get('/api/config/schema').catch(function() { return {}; }),
          OpenFangAPI.get('/api/config')
        ]);
        this.configSchema = results[0].sections || null;
        this.configValues = results[1] || {};
      } catch(e) { /* silent */ }
    },

    isConfigDirty(section, field) {
      return this.configDirty[section + '.' + field] === true;
    },

    markConfigDirty(section, field) {
      this.configDirty[section + '.' + field] = true;
    },

    async saveConfigField(section, field, value) {
      var key = section + '.' + field;
      // Root-level fields (api_key, api_listen, log_level) use just the field name
      var sectionMeta = this.configSchema && this.configSchema[section];
      var path = (sectionMeta && sectionMeta.root_level) ? field : key;
      this.configSaving[key] = true;
      try {
        await OpenFangAPI.post('/api/config/set', { path: path, value: value });
        this.configDirty[key] = false;
        OpenFangToast.success('Saved ' + field);
      } catch(e) {
        OpenFangToast.error('Failed to save: ' + openFangErrText(e));
      }
      this.configSaving[key] = false;
    },

    /** Host-wide tools table search — must be a method: `Object.assign` snapshots getters as static values. */
    filteredToolsList() {
      var q = (this.toolSearch || '').toLowerCase().trim();
      var list = this.tools || [];
      if (!q) return list;
      return list.filter(function(t) {
        if (!t) return false;
        var nm = (t.name || '').toLowerCase();
        return nm.indexOf(q) !== -1 ||
               (t.description || '').toLowerCase().indexOf(q) !== -1;
      });
    },

    /** Per-agent LLM tools search — must be a method (see `filteredToolsList`). */
    filteredLlmToolsList() {
      var r = this.llmToolsResult;
      if (!r || !Array.isArray(r.tools)) return [];
      var q = (this.llmToolSearch || '').toLowerCase().trim();
      if (!q) return r.tools;
      return r.tools.filter(function(t) {
        if (!t) return false;
        var n = (t.name || '').toLowerCase();
        var d = (t.description || '').toLowerCase();
        return n.indexOf(q) !== -1 || d.indexOf(q) !== -1;
      });
    },

    /** Models table — must be a method (see `filteredToolsList` for why). */
    filteredModelsList() {
      var self = this;
      var pf = (self.modelProviderFilter || '').trim();
      var tf = (self.modelTierFilter || '').trim().toLowerCase();
      var list = self.models || [];
      return list.filter(function(m) {
        if (!m) return false;
        var pid = m.id != null ? String(m.id) : '';
        var pprovider = m.provider != null ? String(m.provider) : '';
        if (pf && pprovider !== pf) return false;
        if (tf) {
          var mt = m.tier != null ? String(m.tier).toLowerCase() : '';
          if (mt !== tf) return false;
        }
        if (self.modelSearch) {
          var q = self.modelSearch.toLowerCase();
          if (pid.toLowerCase().indexOf(q) === -1 &&
              (m.display_name || '').toLowerCase().indexOf(q) === -1) return false;
        }
        return true;
      });
    },

    /** Provider filter dropdown values — method (see `filteredToolsList`). */
    uniqueProviderNamesList() {
      var seen = {};
      (this.models || []).forEach(function(m) {
        if (m && m.provider) seen[m.provider] = true;
      });
      // Keep the provider filter populated even when /api/models fails but
      // /api/providers succeeds.
      (this.providers || []).forEach(function(p) {
        if (p && p.id) seen[p.id] = true;
      });
      return Object.keys(seen).sort();
    },

    /** Tier filter dropdown values — method (see `filteredToolsList`). */
    uniqueTiersList() {
      var seen = {};
      (this.models || []).forEach(function(m) {
        if (m && m.tier != null && String(m.tier).length) {
          seen[String(m.tier).toLowerCase()] = true;
        }
      });
      return Object.keys(seen).sort();
    },

    get isDesktopShell() {
      var w = typeof window !== 'undefined' ? window : null;
      var core = w && w.__TAURI__ && w.__TAURI__.core;
      return !!(core && typeof core.invoke === 'function');
    },

    async checkForDesktopUpdates() {
      if (!this.isDesktopShell) return;
      this.updateChecking = true;
      this.updateInfo = null;
      try {
        var info = await ArmaraosDesktopTauriInvoke('check_for_updates');
        this.updateInfo = info;
        await this.loadUpdaterPrefs();
        if (!info) {
          OpenFangToast.error('Update check returned no data');
        } else if (info.available && info.installable) {
          var v = (info.version || 'unknown');
          if (!confirm('ArmaraOS v' + v + ' is available. Install now? The app will restart.')) {
            this.updateChecking = false;
            return;
          }
          OpenFangToast.info('Installing update…');
          await ArmaraosDesktopTauriInvoke('install_update');
        } else if (info.available && !info.installable) {
          var v2 = (info.version || 'unknown');
          var u = info.download_url || 'https://github.com/sbhooley/armaraos/releases';
          OpenFangToast.info('Update available (v' + v2 + '). Opening download page…', 7000);
          try {
            await ArmaraosDesktopTauriInvoke('open_external_url', { url: u });
          } catch (e) {
            window.open(u, '_blank', 'noopener,noreferrer');
          }
        } else {
          OpenFangToast.success('Up to date');
        }
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
        await this.loadUpdaterPrefs();
      }
      this.updateChecking = false;
    },

    _applyDiagnosticsResult(res) {
      this.diagBundlePath = (res && res.bundle_path) ? res.bundle_path : '';
      this.diagBundleFilename = (res && res.bundle_filename) ? res.bundle_filename : '';
      this.diagRelativePath = (res && res.relative_path) ? res.relative_path : '';
      if (!this.diagBundleFilename && this.diagBundlePath) {
        try {
          var norm = String(this.diagBundlePath).replace(/\\/g, '/');
          var segs = norm.split('/').filter(Boolean);
          this.diagBundleFilename = segs.length ? segs[segs.length - 1] : '';
        } catch (e) { /* ignore */ }
      }
      if (!this.diagRelativePath && this.diagBundleFilename) {
        this.diagRelativePath = 'support/' + this.diagBundleFilename;
      }
    },

    async generateDiagnosticsBundle() {
      this.diagError = '';
      this.diagBundlePath = '';
      this.diagBundleFilename = '';
      this.diagRelativePath = '';
      this.diagDownloadsPath = '';
      this.diagGenerating = true;
      try {
        var res = null;
        if (this.isDesktopShell) {
          try {
            res = await ArmaraosDesktopTauriInvoke('generate_support_bundle');
          } catch (e0) {
            res = null;
          }
        }
        if (!res || !res.bundle_path) {
          res = await OpenFangAPI.post('/api/support/diagnostics', {});
        }
        this._applyDiagnosticsResult(res);
        if (!this.diagBundlePath) {
          throw new Error('No bundle path returned');
        }
        if (this.isDesktopShell) {
          try {
            var copyOut = await ArmaraosDesktopTauriInvoke('copy_diagnostics_to_downloads', {
              bundlePath: this.diagBundlePath,
            });
            if (copyOut && copyOut.downloads_path) {
              this.diagDownloadsPath = copyOut.downloads_path;
              OpenFangToast &&
                OpenFangToast.success('Diagnostics saved to Downloads and home/support folder');
            } else {
              OpenFangToast && OpenFangToast.success('Diagnostics bundle created in home/support folder');
            }
          } catch (eCopy) {
            OpenFangToast &&
              OpenFangToast.warn(
                'Bundle created, but copy to Downloads failed: ' + (eCopy.message || String(eCopy))
              );
            if (this.diagBundleFilename) {
              try {
                await OpenFangAPI.downloadDiagnosticsZip(this.diagBundleFilename);
                OpenFangToast &&
                  OpenFangToast.info('Started a browser-style download as a fallback — check Downloads');
              } catch (eDl) {
                OpenFangToast &&
                  OpenFangToast.info(
                    'You can open Home folder → support and copy the .zip manually (' +
                      (this.diagBundleFilename || 'armaraos-diagnostics-*.zip') +
                      ').'
                  );
              }
            }
          }
        } else if (this.diagBundleFilename) {
          try {
            await OpenFangAPI.downloadDiagnosticsZip(this.diagBundleFilename);
            OpenFangToast &&
              OpenFangToast.success('Download started — save completes to your default downloads folder');
          } catch (eDl) {
            OpenFangToast &&
              OpenFangToast.warn(
                'Bundle created on the server; browser download failed: ' + (eDl.message || String(eDl))
              );
            OpenFangToast &&
              OpenFangToast.info('Open Home folder → support to download the .zip from the dashboard.');
          }
        } else {
          OpenFangToast && OpenFangToast.success('Diagnostics bundle generated');
        }
      } catch (e) {
        this.diagError = e && (e.message || String(e)) || 'Diagnostics bundle failed';
        OpenFangToast && OpenFangToast.error(this.diagError);
      }
      this.diagGenerating = false;
    },

    openHomeFolderSupport() {
      window.location.hash = 'home-files?path=support';
    },

    async openSupportEmail() {
      var subject = encodeURIComponent('ArmaraOS Support — Bug Report');
      var bodyPlain =
        'Describe the bug here:\n\n' +
        (this.diagBundlePath ? 'Bundle on disk: ' + this.diagBundlePath + '\n\n' : '') +
        'Thanks!';
      if (this.isDesktopShell) {
        try {
          var mailOut = await ArmaraosDesktopTauriInvoke('compose_support_email', {
            bundle_path: this.diagBundlePath || null,
          });
          if (mailOut && mailOut.attach_failed) {
            var hint =
              'Could not attach the zip automatically. Add the diagnostics .zip manually — ';
            if (this.diagDownloadsPath) {
              hint += 'it should be in Downloads, or ';
            }
            hint += this.diagBundlePath
              ? 'open Home folder → support: ' + this.diagBundlePath
              : 'generate diagnostics first, then try again.';
            OpenFangToast && OpenFangToast.warn(hint);
          }
          return;
        } catch (e) {
          try {
            await ArmaraosDesktopTauriInvoke('open_external_url', {
              url:
                'mailto:ainativelang@gmail.com?subject=' +
                subject +
                '&body=' +
                encodeURIComponent(bodyPlain),
            });
            return;
          } catch (e2) {
            /* fall through */
          }
        }
      }
      try {
        window.location.href =
          'mailto:ainativelang@gmail.com?subject=' +
          subject +
          '&body=' +
          encodeURIComponent(bodyPlain);
      } catch (e3) {
        /* ignore */
      }
    },

    get ainlDesktop() {
      try {
        return Alpine.store('ainl').desktop;
      } catch (e) {
        return null;
      }
    },

    _setAinlDesktopStore(st) {
      try {
        Alpine.store('ainl').desktop = st;
        Alpine.store('ainl').bootstrapping = !(st && st.ok);
      } catch (e) { /* ignore */ }
    },

    async loadAinlDesktop(quiet) {
      if (!this.isDesktopShell) return;
      if (!quiet) {
        this.ainlDesktopLoading = true;
        this.ainlDesktopError = '';
      }
      try {
        var st = await ArmaraosDesktopTauriInvoke('ainl_status');
        this._setAinlDesktopStore(st);
      } catch (e) {
        if (!quiet) {
          this.ainlDesktopError = e.message || String(e);
          this._setAinlDesktopStore(null);
        }
      }
      if (!quiet) this.ainlDesktopLoading = false;
    },

    async loadAinlVersions() {
      if (!this.isDesktopShell) return;
      this.ainlVersionLoading = true;
      try {
        var info = await ArmaraosDesktopTauriInvoke('ainl_check_versions');
        this.ainlVersionInfo = info;
      } catch (e) {
        this.ainlVersionInfo = { pypi_error: e.message || String(e) };
      }
      this.ainlVersionCheckedAt = new Date().toISOString();
      this.ainlVersionLoading = false;
    },

    formatAinlVersionCheckedAt() {
      if (!this.ainlVersionCheckedAt) return '';
      try {
        return new Date(this.ainlVersionCheckedAt).toLocaleString();
      } catch (e) {
        return '';
      }
    },

    async upgradeAinlFromPip() {
      if (!this.isDesktopShell) return;
      if (!confirm('Upgrade AINL in the app virtualenv from PyPI? This may take a minute.')) return;
      this.ainlUpgradeLoading = true;
      try {
        var st = await ArmaraosDesktopTauriInvoke('upgrade_ainl_pip');
        this._setAinlDesktopStore(st);
        await this.loadAinlVersions();
        if (st && st.ok) {
          OpenFangToast.success(st.detail || 'AINL upgraded');
        } else if (st) {
          OpenFangToast.warn(st.detail || 'Upgrade finished with warnings');
        }
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
      }
      this.ainlUpgradeLoading = false;
    },

    init() {
      // Set up AINL bootstrap progress event listener for desktop
      if (this.isDesktopShell && window.__TAURI__) {
        window.__TAURI__.event.listen('ainl-bootstrap-progress', (event) => {
          this.ainlBootstrapProgress = event.payload || '';
        }).catch(err => console.warn('Failed to listen for AINL progress:', err));
      }
    },

    async retryAinlBootstrap() {
      if (!this.isDesktopShell) return;
      this.ainlDesktopLoading = true;
      this.ainlDesktopError = '';
      this.ainlBootstrapProgress = 'Starting bootstrap...';
      try {
        var st = await ArmaraosDesktopTauriInvoke('ensure_ainl_installed');
        this._setAinlDesktopStore(st);
        if (st && st.ok) {
          OpenFangToast.success(st.detail || 'AINL is ready');
        } else if (st) {
          OpenFangToast.warn(st.detail || 'AINL bootstrap incomplete');
        } else {
          OpenFangToast.error('AINL bootstrap returned no data');
        }
      } catch (e) {
        this.ainlDesktopError = e.message || String(e);
        OpenFangToast.error(this.ainlDesktopError);
      }
      this.ainlDesktopLoading = false;
      this.ainlBootstrapProgress = '';
    },

    async retryAinlHostOnly() {
      if (!this.isDesktopShell) return;
      this.ainlDesktopLoading = true;
      this.ainlDesktopError = '';
      try {
        var st = await ArmaraosDesktopTauriInvoke('ensure_armaraos_ainl_host');
        await this.loadAinlDesktop();
        if (st && st.ok) {
          OpenFangToast.success(st.detail || 'Host integration updated');
        } else if (st) {
          OpenFangToast.warn(st.detail || 'Host integration issue');
        }
      } catch (e) {
        this.ainlDesktopError = e.message || String(e);
        OpenFangToast.error(this.ainlDesktopError);
      }
      this.ainlDesktopLoading = false;
    },

    async openAinlLibraryFolder() {
      if (!this.isDesktopShell) return;
      try {
        await ArmaraosDesktopTauriInvoke('open_ainl_library_dir');
      } catch (e) {
        OpenFangToast.error(openFangErrText(e) || String(e));
      }
    },

    /** Opens python.org/downloads in the system browser (whitelisted in Tauri). */
    async openPythonDownloadsPage() {
      if (!this.isDesktopShell) return;
      try {
        await ArmaraosDesktopTauriInvoke('open_external_url', {
          url: 'https://www.python.org/downloads/',
        });
      } catch (e) {
        window.open('https://www.python.org/downloads/', '_blank', 'noopener,noreferrer');
      }
    },

    providerAuthClass(p) {
      if (p.auth_status === 'configured' || p.auth_status === 'not_required') return 'auth-configured';
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') return 'auth-not-set';
      return 'auth-no-key';
    },

    providerAuthText(p) {
      if (p.auth_status === 'configured') return 'Configured';
      if (p.auth_status === 'not_required') return 'No Key Needed';
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') {
        if (p.id === 'claude-code' || p.id === 'qwen-code') return 'Not Installed';
        return 'Not Set';
      }
      return 'No Key Needed';
    },

    providerCardClass(p) {
      if (p.auth_status === 'configured' || p.auth_status === 'not_required') return 'configured';
      if (p.auth_status === 'not_set' || p.auth_status === 'missing') return 'not-configured';
      return 'no-key';
    },

    /** Subprocess LLM drivers (no API key in ArmaraOS; readiness = CLI on PATH + auth). */
    isCliLlmProvider(p) {
      return !!(p && (p.id === 'claude-code' || p.id === 'qwen-code'));
    },

    isCustomProvider(p) {
      return !!(p && p.is_custom);
    },

    canEditProviderUrl(p) {
      return !!(p && (p.is_local || this.isCustomProvider(p)));
    },

    canRemoveProviderUrl(p) {
      return this.canEditProviderUrl(p) && !!((p.base_url || '').trim());
    },

    tierBadgeClass(tier) {
      if (!tier) return '';
      var t = tier.toLowerCase();
      if (t === 'frontier') return 'tier-frontier';
      if (t === 'smart') return 'tier-smart';
      if (t === 'balanced') return 'tier-balanced';
      if (t === 'fast') return 'tier-fast';
      return '';
    },

    formatCost(cost) {
      if (!cost && cost !== 0) return '-';
      return '$' + cost.toFixed(4);
    },

    formatContext(ctx) {
      if (!ctx) return '-';
      if (ctx >= 1000000) return (ctx / 1000000).toFixed(1) + 'M';
      if (ctx >= 1000) return Math.round(ctx / 1000) + 'K';
      return String(ctx);
    },

    formatUptime(secs) {
      if (!secs) return '-';
      var h = Math.floor(secs / 3600);
      var m = Math.floor((secs % 3600) / 60);
      var s = secs % 60;
      if (h > 0) return h + 'h ' + m + 'm';
      if (m > 0) return m + 'm ' + s + 's';
      return s + 's';
    },

    async saveProviderKey(provider) {
      var key = this.providerKeyInputs[provider.id];
      if (!key || !key.trim()) { OpenFangToast.error('Please enter an API key'); return; }
      try {
        var resp = await OpenFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/key', { key: key.trim() });
        if (resp && resp.switched_default) {
          OpenFangToast.warn(resp.message || 'Default provider was switched to ' + provider.display_name);
        } else {
          OpenFangToast.success('API key saved for ' + provider.display_name);
        }
        this.providerKeyInputs[provider.id] = '';
        await this.loadProviders();
        await this.loadModels();
      } catch(e) {
        OpenFangToast.error('Failed to save key: ' + openFangErrText(e));
      }
    },

    async removeProviderKey(provider) {
      try {
        await OpenFangAPI.del('/api/providers/' + encodeURIComponent(provider.id) + '/key');
        OpenFangToast.success('API key removed for ' + provider.display_name);
        await this.loadProviders();
        await this.loadModels();
      } catch(e) {
        OpenFangToast.error('Failed to remove key: ' + openFangErrText(e));
      }
    },

    async startCopilotOAuth() {
      this.copilotOAuth.polling = true;
      this.copilotOAuth.userCode = '';
      try {
        var resp = await OpenFangAPI.post('/api/providers/github-copilot/oauth/start', {});
        this.copilotOAuth.userCode = resp.user_code;
        this.copilotOAuth.verificationUri = resp.verification_uri;
        this.copilotOAuth.pollId = resp.poll_id;
        this.copilotOAuth.interval = resp.interval || 5;
        window.open(resp.verification_uri, '_blank');
        this.pollCopilotOAuth();
      } catch(e) {
        OpenFangToast.error('Failed to start Copilot login: ' + openFangErrText(e));
        this.copilotOAuth.polling = false;
      }
    },

    pollCopilotOAuth() {
      var self = this;
      setTimeout(async function() {
        if (!self.copilotOAuth.pollId) return;
        try {
          var resp = await OpenFangAPI.get('/api/providers/github-copilot/oauth/poll/' + self.copilotOAuth.pollId);
          if (resp.status === 'complete') {
            OpenFangToast.success('GitHub Copilot authenticated successfully!');
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
            await self.loadProviders();
            await self.loadModels();
          } else if (resp.status === 'pending') {
            if (resp.interval) self.copilotOAuth.interval = resp.interval;
            self.pollCopilotOAuth();
          } else if (resp.status === 'expired') {
            OpenFangToast.error('Device code expired. Please try again.');
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          } else if (resp.status === 'denied') {
            OpenFangToast.error('Access denied by user.');
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          } else {
            OpenFangToast.error('OAuth error: ' + (resp.error || resp.status));
            self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
          }
        } catch(e) {
          OpenFangToast.error('Poll error: ' + openFangErrText(e));
          self.copilotOAuth = { polling: false, userCode: '', verificationUri: '', pollId: '', interval: 5 };
        }
      }, self.copilotOAuth.interval * 1000);
    },

    async testProvider(provider) {
      this.providerTesting[provider.id] = true;
      this.providerTestResults[provider.id] = null;
      try {
        var result = await OpenFangAPI.post('/api/providers/' + encodeURIComponent(provider.id) + '/test', {});
        this.providerTestResults[provider.id] = result;
        if (result.status === 'ok') {
          OpenFangToast.success(provider.display_name + ' connected (' + (result.latency_ms || '?') + 'ms)');
          await this.loadProviders();
          await this.loadModels();
        } else {
          OpenFangToast.error(provider.display_name + ': ' + (result.error || 'Connection failed'));
        }
      } catch(e) {
        this.providerTestResults[provider.id] = { status: 'error', error: e.message };
        OpenFangToast.error('Test failed: ' + openFangErrText(e));
      }
      this.providerTesting[provider.id] = false;
    },

    /** Same as Test; primary label on Settings for CLI providers before first success. */
    async detectCliProvider(provider) {
      await this.testProvider(provider);
    },

    async saveProviderUrl(provider) {
      var url = this.providerUrlInputs[provider.id];
      if (!url || !url.trim()) { OpenFangToast.error('Please enter a base URL'); return; }
      url = url.trim();
      if (url.indexOf('http://') !== 0 && url.indexOf('https://') !== 0) {
        OpenFangToast.error('URL must start with http:// or https://'); return;
      }
      this.providerUrlSaving[provider.id] = true;
      try {
        var result = await OpenFangAPI.put('/api/providers/' + encodeURIComponent(provider.id) + '/url', { base_url: url });
        if (result.reachable) {
          OpenFangToast.success(provider.display_name + ' URL saved &mdash; reachable (' + (result.latency_ms || '?') + 'ms)');
        } else {
          OpenFangToast.warn(provider.display_name + ' URL saved but not reachable');
        }
        await this.loadProviders();
      } catch(e) {
        OpenFangToast.error('Failed to save URL: ' + openFangErrText(e));
      }
      this.providerUrlSaving[provider.id] = false;
    },

    async deleteCustomProvider(provider) {
      if (!this.isCustomProvider(provider)) return;
      if (!confirm('Permanently delete custom provider "' + provider.id + '"? This removes its base URL, stored API key, and all catalog models for this provider.')) return;
      this.providerDeleting[provider.id] = true;
      try {
        await OpenFangAPI.del('/api/providers/' + encodeURIComponent(provider.id));
        OpenFangToast.success('Deleted provider "' + provider.id + '"');
        delete this.providerUrlInputs[provider.id];
        await this.loadProviders();
        await this.loadModels();
      } catch (e) {
        OpenFangToast.error('Failed to delete provider: ' + openFangErrText(e));
      }
      this.providerDeleting[provider.id] = false;
    },

    async removeProviderUrl(provider) {
      if (!this.canRemoveProviderUrl(provider)) {
        OpenFangToast.error('No custom URL to remove for ' + provider.display_name);
        return;
      }
      if (!confirm('Remove URL override for "' + provider.display_name + '"?')) return;
      this.providerUrlSaving[provider.id] = true;
      try {
        var result = await OpenFangAPI.del('/api/providers/' + encodeURIComponent(provider.id) + '/url');
        if (result.provider_removed) {
          OpenFangToast.success('Removed custom provider "' + provider.id + '"');
        } else {
          OpenFangToast.success('URL override removed for ' + provider.display_name);
        }
        await this.loadProviders();
        await this.loadModels();
      } catch(e) {
        OpenFangToast.error('Failed to remove URL: ' + openFangErrText(e));
      }
      this.providerUrlSaving[provider.id] = false;
    },

    async addCustomProvider() {
      var name = this.customProviderName.trim().toLowerCase().replace(/[^a-z0-9-]/g, '-').replace(/-+/g, '-');
      if (!name) { OpenFangToast.error('Please enter a provider name'); return; }
      var url = this.customProviderUrl.trim();
      if (!url) { OpenFangToast.error('Please enter a base URL'); return; }
      if (url.indexOf('http://') !== 0 && url.indexOf('https://') !== 0) {
        OpenFangToast.error('URL must start with http:// or https://'); return;
      }
      this.addingCustomProvider = true;
      this.customProviderStatus = '';
      try {
        var result = await OpenFangAPI.put('/api/providers/' + encodeURIComponent(name) + '/url', { base_url: url });
        if (this.customProviderKey.trim()) {
          await OpenFangAPI.post('/api/providers/' + encodeURIComponent(name) + '/key', { key: this.customProviderKey.trim() });
        }
        this.customProviderName = '';
        this.customProviderUrl = '';
        this.customProviderKey = '';
        this.customProviderStatus = '';
        OpenFangToast.success('Provider "' + name + '" added' + (result.reachable ? ' (reachable)' : ' (not reachable yet)'));
        await this.loadProviders();
      } catch(e) {
        this.customProviderStatus = 'Error: ' + (e.message || 'Failed');
        OpenFangToast.error('Failed to add provider: ' + openFangErrText(e));
      }
      this.addingCustomProvider = false;
    },

    // -- Security methods --
    async loadSecurity() {
      this.secLoading = true;
      try {
        this.securityData = await OpenFangAPI.get('/api/security');
      } catch(e) {
        this.securityData = null;
      }
      this.secLoading = false;
    },

    // --------------------------------------------------------------------
    // Voice tab — local TTS / STT installation status, voice picker, upload
    // --------------------------------------------------------------------

    /** Fetch /api/system/local-voice + /say-voices + /voices in parallel and seed the draft from server values. */
    async loadVoiceSettings(refresh) {
      var self = this;
      self.voiceLoading = !refresh;
      try {
        var results = await Promise.allSettled([
          OpenFangAPI.get('/api/system/local-voice'),
          OpenFangAPI.get('/api/system/local-voice/say-voices'),
          OpenFangAPI.get('/api/system/local-voice/voices'),
        ]);
        if (results[0].status === 'fulfilled') {
          self.voiceData = results[0].value || null;
        }
        if (results[1].status === 'fulfilled') {
          self.sayVoices = (results[1].value && results[1].value.voices) || [];
          // Sort: premium > enhanced > default, alphabetical within tier.
          var rank = function(t) { return t === 'premium' ? 0 : t === 'enhanced' ? 1 : 2; };
          self.sayVoices.sort(function(a, b) {
            var dr = rank(a.tier) - rank(b.tier);
            if (dr !== 0) return dr;
            return (a.id || '').localeCompare(b.id || '');
          });
        }
        if (results[2].status === 'fulfilled') {
          self.customVoices = (results[2].value && results[2].value.voices) || [];
        }
        // Keep draft + server snapshot aligned with what the daemon reports (includes prefs
        // applied immediately after `PUT /api/system/local-voice/preference`).
        if (self.voiceData) {
          var srv = {
            preferred_say_voice: self.voiceData.preferred_say_voice || '',
            prefer_macos_say: !!self.voiceData.prefer_macos_say,
            custom_piper_voice: self.voiceData.custom_piper_voice || '',
            kokoro_voice: (self.voiceData.kokoro && self.voiceData.kokoro.voice) || 'af_heart',
          };
          self.voicePrefServer = srv;
          self.voicePrefDraft = Object.assign({}, srv);
        }
      } catch(e) {
        // network or auth error — keep whatever we had
      }
      self.voiceLoading = false;
    },

    /** Discard local edits to the voice preferences and reload from disk. */
    resetVoicePrefDraft() {
      this.voicePrefDraft = Object.assign({}, this.voicePrefServer);
      this.voicePrefStatus = '';
    },

    /** PUT /api/system/local-voice/preference and update the "server" snapshot on success. */
    async saveVoicePreference() {
      this.voicePrefSaving = true;
      this.voicePrefStatus = '';
      try {
        var body = {
          preferred_say_voice: this.voicePrefDraft.preferred_say_voice || null,
          prefer_macos_say: !!this.voicePrefDraft.prefer_macos_say,
          custom_piper_voice: this.voicePrefDraft.custom_piper_voice || null,
          kokoro_voice: this.voicePrefDraft.kokoro_voice || null,
        };
        var res = await OpenFangAPI.put('/api/system/local-voice/preference', body);
        this.voicePrefStatus = '✓ Saved. ' + (res && res.hint ? res.hint : 'Voice picks apply immediately.');
        await this.loadVoiceSettings(true);
      } catch(e) {
        this.voicePrefStatus = 'Save failed: ' + (e && e.message ? e.message : 'unknown error');
      }
      this.voicePrefSaving = false;
    },

    /** Multipart upload — POST /api/system/local-voice/voices/upload — refresh the list on success. */
    async uploadCustomVoice() {
      if (!this.onnxFileInput) {
        this.voiceUploadError = 'Choose a .onnx file first.';
        return;
      }
      this.voiceUploading = true;
      this.voiceUploadError = '';
      try {
        var form = new FormData();
        form.append('voice', this.onnxFileInput);
        if (this.onnxJsonFileInput) {
          form.append('metadata', this.onnxJsonFileInput);
        }
        if (this.onnxIdInput && this.onnxIdInput.trim()) {
          form.append('id', this.onnxIdInput.trim());
        }
        var hdrs = {};
        var token = (window.OpenFangAPI && OpenFangAPI.getToken && OpenFangAPI.getToken()) || '';
        if (token) hdrs['Authorization'] = 'Bearer ' + token;
        var resp = await fetch(OpenFangAPI.baseUrl + '/api/system/local-voice/voices/upload', {
          method: 'POST',
          headers: hdrs,
          body: form,
        });
        var json = await resp.json().catch(function() { return {}; });
        if (!resp.ok || json.ok === false) {
          throw new Error(json.error || ('HTTP ' + resp.status));
        }
        // Auto-select the newly uploaded voice (matches user expectation).
        if (json.id) {
          this.voicePrefDraft.custom_piper_voice = json.id;
          // Persist pick immediately so the next `loadVoiceSettings` (and the daemon) do not
          // drop the selection — `config.toml` + in-memory `[local_voice]` sync via PUT.
          try {
            await OpenFangAPI.put('/api/system/local-voice/preference', {
              preferred_say_voice: this.voicePrefDraft.preferred_say_voice || null,
              prefer_macos_say: !!this.voicePrefDraft.prefer_macos_say,
              custom_piper_voice: json.id,
              kokoro_voice: this.voicePrefDraft.kokoro_voice || null,
            });
          } catch (e2) {
            this.voiceUploadError =
              'Voice uploaded but failed to set as active: ' +
              (e2 && e2.message ? e2.message : 'unknown error') +
              ' — click Save voice preferences.';
          }
        }
        this.onnxFileInput = null;
        this.onnxJsonFileInput = null;
        this.onnxIdInput = '';
        // Reset only the file inputs inside the Voice tab (never clear unrelated uploads).
        var vroot = this.$refs && this.$refs.voiceSettingsPanel;
        if (vroot) {
          vroot.querySelectorAll('input[type="file"]').forEach(function(el) {
            el.value = '';
          });
        }
        await this.loadVoiceSettings(true);
      } catch(e) {
        this.voiceUploadError = 'Upload failed: ' + (e && e.message ? e.message : 'unknown error');
      }
      this.voiceUploading = false;
    },

    /** DELETE /api/system/local-voice/voices/{id} after a confirm prompt. */
    async deleteCustomVoice(id) {
      if (!id) return;
      if (!window.confirm('Remove uploaded voice "' + id + '"? The .onnx file will be deleted from disk.')) return;
      try {
        await OpenFangAPI.delete('/api/system/local-voice/voices/' + encodeURIComponent(id));
        // If the deleted voice was the active pick, clear it in config + daemon so UI and TTS agree.
        if (this.voicePrefDraft.custom_piper_voice === id || this.voicePrefServer.custom_piper_voice === id) {
          this.voicePrefDraft.custom_piper_voice = '';
          try {
            await OpenFangAPI.put('/api/system/local-voice/preference', {
              preferred_say_voice: this.voicePrefDraft.preferred_say_voice || null,
              prefer_macos_say: !!this.voicePrefDraft.prefer_macos_say,
              custom_piper_voice: null,
              kokoro_voice: this.voicePrefDraft.kokoro_voice || null,
            });
          } catch (e2) {
            this.voicePrefStatus =
              'Voice removed from disk but failed to clear active selection in config: ' +
              (e2 && e2.message ? e2.message : 'unknown error') +
              ' — click Save voice preferences.';
          }
        }
        await this.loadVoiceSettings(true);
      } catch(e) {
        this.voicePrefStatus = 'Delete failed: ' + (e && e.message ? e.message : 'unknown error');
      }
    },

    /** Local browser preview using the Web Speech API — works on macOS without round-tripping the daemon. */
    previewSayVoice(voiceId) {
      if (!('speechSynthesis' in window)) {
        this.voicePrefStatus = 'Preview unsupported in this browser.';
        return;
      }
      try {
        var u = new SpeechSynthesisUtterance("Hi, I'm " + (voiceId || 'the system voice') + ". This is how I'll sound when responding to you.");
        var voices = window.speechSynthesis.getVoices();
        var match = voices.find(function(v) { return v.name === voiceId || v.name === voiceId.replace(/ \(.*\)$/, ''); });
        if (match) u.voice = match;
        window.speechSynthesis.cancel();
        window.speechSynthesis.speak(u);
      } catch(e) {
        this.voicePrefStatus = 'Preview failed: ' + (e && e.message ? e.message : 'unknown');
      }
    },

    /** Open System Settings → Accessibility → Spoken Content (macOS only). Falls back to a hint. */
    openMacosVoiceDownload() {
      try {
        if (window.armaraosOpenExternalUrl) {
          window.armaraosOpenExternalUrl({ preventDefault: function(){} }, 'x-apple.systempreferences:com.apple.preference.universalaccess?Speech');
          return;
        }
      } catch(e) { /* ignore */ }
      this.voicePrefStatus = 'Open System Settings → Accessibility → Spoken Content → System Voice → Manage Voices to install Premium / Enhanced voices.';
    },

    isActive(key) {
      if (!this.securityData) return true;
      var core = this.securityData.core_protections || {};
      if (core[key] !== undefined) return core[key];
      return true;
    },

    getConfigValue(key) {
      if (!this.securityData) return null;
      var cfg = this.securityData.configurable || {};
      return cfg[key] || null;
    },

    getMonitoringValue(key) {
      if (!this.securityData) return null;
      var mon = this.securityData.monitoring || {};
      return mon[key] || null;
    },

    formatConfigValue(feature) {
      var val = this.getConfigValue(feature.valueKey);
      if (!val) return feature.configHint;
      switch (feature.valueKey) {
        case 'rate_limiter':
          return 'Algorithm: ' + (val.algorithm || 'GCRA') + ' | ' + (val.tokens_per_minute || 500) + ' tokens/min per IP';
        case 'websocket_limits':
          return 'Max ' + (val.max_per_ip || 5) + ' conn/IP | ' + Math.round((val.idle_timeout_secs || 1800) / 60) + 'min idle timeout | ' + Math.round((val.max_message_size || 65536) / 1024) + 'KB max msg';
        case 'wasm_sandbox':
          return 'Fuel: ' + (val.fuel_metering ? 'ON' : 'OFF') + ' | Epoch: ' + (val.epoch_interruption ? 'ON' : 'OFF') + ' | Timeout: ' + (val.default_timeout_secs || 30) + 's';
        case 'auth':
          return 'Mode: ' + (val.mode || 'unknown') + (val.api_key_set ? ' (key configured)' : ' (no key set)');
        default:
          return feature.configHint;
      }
    },

    formatMonitoringValue(feature) {
      var val = this.getMonitoringValue(feature.valueKey);
      if (!val) return feature.configHint;
      switch (feature.valueKey) {
        case 'audit_trail':
          return (val.enabled ? 'Active' : 'Disabled') + ' | ' + (val.algorithm || 'SHA-256') + ' | ' + (val.entry_count || 0) + ' entries logged';
        case 'taint_tracking':
          var labels = val.tracked_labels || [];
          return (val.enabled ? 'Active' : 'Disabled') + ' | Tracking: ' + labels.join(', ');
        case 'manifest_signing':
          return 'Algorithm: ' + (val.algorithm || 'Ed25519') + ' | ' + (val.available ? 'Available' : 'Not available');
        default:
          return feature.configHint;
      }
    },

    async verifyAuditChain() {
      this.verifyingChain = true;
      this.chainResult = null;
      try {
        var res = await OpenFangAPI.get('/api/audit/verify');
        this.chainResult = res;
      } catch(e) {
        this.chainResult = { valid: false, error: e.message };
      }
      this.verifyingChain = false;
    },

    // -- Peers methods --
    async loadPeers() {
      this.peersLoading = true;
      this.peersLoadError = '';
      try {
        var data = await OpenFangAPI.get('/api/peers');
        this.peers = (data.peers || []).map(function(p) {
          return {
            node_id: p.node_id,
            node_name: p.node_name,
            address: p.address,
            state: p.state,
            agent_count: (p.agents || []).length,
            protocol_version: p.protocol_version || 1
          };
        });
      } catch(e) {
        this.peers = [];
        this.peersLoadError = e.message || 'Could not load peers.';
      }
      this.peersLoading = false;
    },

    startPeerPolling() {
      var self = this;
      this.stopPeerPolling();
      this._peerPollTimer = setInterval(async function() {
        if (self.tab !== 'network') { self.stopPeerPolling(); return; }
        try {
          var data = await OpenFangAPI.get('/api/peers');
          self.peers = (data.peers || []).map(function(p) {
            return {
              node_id: p.node_id,
              node_name: p.node_name,
              address: p.address,
              state: p.state,
              agent_count: (p.agents || []).length,
              protocol_version: p.protocol_version || 1
            };
          });
        } catch(e) { /* silent */ }
      }, 15000);
    },

    stopPeerPolling() {
      if (this._peerPollTimer) { clearInterval(this._peerPollTimer); this._peerPollTimer = null; }
    },

    // -- Migration methods --
    async autoDetect() {
      this.detecting = true;
      try {
        var data = await OpenFangAPI.get('/api/migrate/detect');
        if (data.detected && data.scan) {
          this.sourcePath = data.path;
          this.scanResult = data.scan;
          this.migStep = 'preview';
        } else {
          this.migStep = 'not_found';
        }
      } catch(e) {
        this.migStep = 'not_found';
      }
      this.detecting = false;
    },

    async scanPath() {
      if (!this.sourcePath) return;
      this.scanning = true;
      try {
        var data = await OpenFangAPI.post('/api/migrate/scan', { path: this.sourcePath });
        if (data.error) {
          OpenFangToast.error('Scan error: ' + data.error);
          this.scanning = false;
          return;
        }
        this.scanResult = data;
        this.migStep = 'preview';
      } catch(e) {
        OpenFangToast.error('Scan failed: ' + openFangErrText(e));
      }
      this.scanning = false;
    },

    async runMigration(dryRun) {
      this.migrating = true;
      try {
        var target = this.targetPath;
        if (!target) target = '';
        var data = await OpenFangAPI.post('/api/migrate', {
          source: 'openclaw',
          source_dir: this.sourcePath || (this.scanResult ? this.scanResult.path : ''),
          target_dir: target,
          dry_run: dryRun
        });
        this.migResult = data;
        this.migStep = 'result';
      } catch(e) {
        this.migResult = { status: 'failed', error: e.message };
        this.migStep = 'result';
      }
      this.migrating = false;
    },

    destroy() {
      this.stopPeerPolling();
    }
  });
}
