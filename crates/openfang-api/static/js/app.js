// ArmaraOS App — Alpine.js init, hash router, global store
'use strict';

/** Persisted dashboard theme: light | system | dark. Default: dark. Syncs legacy openfang-theme-mode. */
function getStoredThemeMode() {
  try {
    return localStorage.getItem('armaraos-theme-mode')
      || localStorage.getItem('openfang-theme-mode')
      || 'dark';
  } catch (e) {
    return 'dark';
  }
}

function effectiveThemeFromMode(mode) {
  if (mode === 'system') {
    return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  return mode;
}

// Marked.js configuration
if (typeof marked !== 'undefined') {
  marked.setOptions({
    breaks: true,
    gfm: true,
    highlight: function(code, lang) {
      if (typeof hljs !== 'undefined' && lang && hljs.getLanguage(lang)) {
        try { return hljs.highlight(code, { language: lang }).value; } catch(e) {}
      }
      return code;
    }
  });
}

/** Tauri 2 desktop shell only: invoke Rust commands when `withGlobalTauri` is enabled. */
function ArmaraosDesktopTauriInvoke(cmd, args) {
  args = args || {};
  return new Promise(function(resolve, reject) {
    try {
      var w = typeof window !== 'undefined' ? window : null;
      var core = w && w.__TAURI__ && w.__TAURI__.core;
      if (!core || typeof core.invoke !== 'function') {
        resolve(null);
        return;
      }
      core.invoke(cmd, args).then(resolve).catch(reject);
    } catch (err) {
      reject(err);
    }
  });
}

function escapeHtml(text) {
  var div = document.createElement('div');
  div.textContent = text || '';
  return div.innerHTML;
}

function renderMarkdown(text) {
  if (!text) return '';
  if (typeof marked !== 'undefined') {
    // Protect LaTeX blocks from marked.js mangling (underscores, backslashes, etc.)
    var latexBlocks = [];
    var protected_ = text;
    // Protect display math $$...$$ first (greedy across lines)
    protected_ = protected_.replace(/\$\$([\s\S]+?)\$\$/g, function(match) {
      var idx = latexBlocks.length;
      latexBlocks.push(match);
      return '\x00LATEX' + idx + '\x00';
    });
    // Protect inline math $...$ (single line, not empty, not starting/ending with space)
    protected_ = protected_.replace(/\$([^\s$](?:[^$]*[^\s$])?)\$/g, function(match) {
      var idx = latexBlocks.length;
      latexBlocks.push(match);
      return '\x00LATEX' + idx + '\x00';
    });
    // Protect \[...\] display math
    protected_ = protected_.replace(/\\\[([\s\S]+?)\\\]/g, function(match) {
      var idx = latexBlocks.length;
      latexBlocks.push(match);
      return '\x00LATEX' + idx + '\x00';
    });
    // Protect \(...\) inline math
    protected_ = protected_.replace(/\\\(([\s\S]+?)\\\)/g, function(match) {
      var idx = latexBlocks.length;
      latexBlocks.push(match);
      return '\x00LATEX' + idx + '\x00';
    });

    var html = marked.parse(protected_);
    // Restore LaTeX blocks
    for (var i = 0; i < latexBlocks.length; i++) {
      html = html.replace('\x00LATEX' + i + '\x00', latexBlocks[i]);
    }
    // Add copy buttons to code blocks
    html = html.replace(/<pre><code/g, '<pre><button class="copy-btn" onclick="copyCode(this)">Copy</button><code');
    // Open external links in new tab
    html = html.replace(/<a\s+href="(https?:\/\/[^"]*)"(?![^>]*target=)([^>]*)>/gi, '<a href="$1" target="_blank" rel="noopener"$2>');
    return html;
  }
  return escapeHtml(text);
}

function copyCode(btn) {
  var code = btn.nextElementSibling;
  if (code) {
    navigator.clipboard.writeText(code.textContent).then(function() {
      btn.textContent = 'Copied!';
      btn.classList.add('copied');
      setTimeout(function() { btn.textContent = 'Copy'; btn.classList.remove('copied'); }, 1500);
    });
  }
}

// Tool category icon SVGs — returns inline SVG for each tool category
function toolIcon(toolName) {
  if (!toolName) return '';
  var n = toolName.toLowerCase();
  var s = 'width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"';
  // File/directory operations
  if (n.indexOf('file_') === 0 || n.indexOf('directory_') === 0)
    return '<svg ' + s + '><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/><path d="M14 2v6h6"/><path d="M16 13H8"/><path d="M16 17H8"/></svg>';
  // Web/fetch
  if (n.indexOf('web_') === 0 || n.indexOf('link_') === 0)
    return '<svg ' + s + '><circle cx="12" cy="12" r="10"/><path d="M2 12h20"/><path d="M12 2a15 15 0 0 1 4 10 15 15 0 0 1-4 10 15 15 0 0 1-4-10 15 15 0 0 1 4-10z"/></svg>';
  // Shell/exec
  if (n.indexOf('shell') === 0 || n.indexOf('exec_') === 0)
    return '<svg ' + s + '><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></svg>';
  // Agent operations
  if (n.indexOf('agent_') === 0)
    return '<svg ' + s + '><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></svg>';
  // Memory/knowledge
  if (n.indexOf('memory_') === 0 || n.indexOf('knowledge_') === 0)
    return '<svg ' + s + '><path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z"/><path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z"/></svg>';
  // Cron/schedule
  if (n.indexOf('cron_') === 0 || n.indexOf('schedule_') === 0)
    return '<svg ' + s + '><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>';
  // Browser/playwright
  if (n.indexOf('browser_') === 0 || n.indexOf('playwright_') === 0)
    return '<svg ' + s + '><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/></svg>';
  // Container/docker
  if (n.indexOf('container_') === 0 || n.indexOf('docker_') === 0)
    return '<svg ' + s + '><path d="M22 12H2"/><path d="M5.45 5.11L2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/></svg>';
  // Image/media
  if (n.indexOf('image_') === 0 || n.indexOf('tts_') === 0)
    return '<svg ' + s + '><rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="8.5" cy="8.5" r="1.5"/><polyline points="21 15 16 10 5 21"/></svg>';
  // Hand tools
  if (n.indexOf('hand_') === 0)
    return '<svg ' + s + '><path d="M18 11V6a2 2 0 0 0-2-2 2 2 0 0 0-2 2"/><path d="M14 10V4a2 2 0 0 0-2-2 2 2 0 0 0-2 2v6"/><path d="M10 10.5V6a2 2 0 0 0-2-2 2 2 0 0 0-2 2v8"/><path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.9-5.7-2.4L3.4 16a2 2 0 0 1 3.2-2.4L8 15"/></svg>';
  // Task/collab
  if (n.indexOf('task_') === 0)
    return '<svg ' + s + '><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11"/></svg>';
  // Default — wrench
  return '<svg ' + s + '><path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/></svg>';
}

// Alpine.js global store
document.addEventListener('alpine:init', function() {
  // Restore saved API key on load
  var savedKey = localStorage.getItem('openfang-api-key');
  if (savedKey) OpenFangAPI.setAuthToken(savedKey);

  Alpine.store('ainl', {
    /** Desktop (Tauri) AINL install status; updated by background poll + Settings actions */
    desktop: null,
    /** True until first successful ainl_status reports ok (or user is not on desktop shell) */
    bootstrapping: false,
  });

  Alpine.store('app', {
    agents: [],
    connected: false,
    booting: true,
    wsConnected: false,
    connectionState: 'connected',
    lastError: '',
    /** Longer recovery hint when the daemon is unreachable or returns an error */
    lastErrorHint: '',
    /** Extra detail (e.g. server message body) for copy-debug */
    lastErrorDetail: '',
    /** Method + path for the last failing request (when known) */
    lastErrorWhere: '',
    /** Server route template from JSON `path` (when known) */
    lastErrorServerPath: '',
    /** x-request-id for the last failing request (when known) */
    lastErrorRequestId: '',
    version: '0.1.0',
    agentCount: 0,
    pendingApprovalCount: 0,
    lastPendingApprovalSignature: '',
    pendingAgent: null,
    focusMode: localStorage.getItem('openfang-focus') === 'true',
    showOnboarding: false,
    showAuthPrompt: false,
    authMode: 'apikey',
    sessionUser: null,

    toggleFocusMode() {
      this.focusMode = !this.focusMode;
      localStorage.setItem('openfang-focus', this.focusMode);
    },

    async refreshAgents() {
      try {
        var agents = await OpenFangAPI.get('/api/agents');
        this.agents = Array.isArray(agents) ? agents : [];
        this.agentCount = this.agents.length;
      } catch(e) { /* silent */ }
    },

    async refreshApprovals() {
      try {
        var data = await OpenFangAPI.get('/api/approvals');
        var approvals = Array.isArray(data) ? data : (data.approvals || []);
        var pending = approvals.filter(function(a) { return a.status === 'pending'; });
        var signature = pending
          .map(function(a) { return a.id; })
          .sort()
          .join(',');
        if (pending.length > 0 && signature !== this.lastPendingApprovalSignature && typeof OpenFangToast !== 'undefined') {
          OpenFangToast.warn('An agent is waiting for approval. Open Approvals to review.');
        }
        this.pendingApprovalCount = pending.length;
        this.lastPendingApprovalSignature = signature;
      } catch(e) { /* silent */ }
    },

    async checkStatus() {
      try {
        var s = await OpenFangAPI.get('/api/status');
        this.connected = true;
        this.booting = false;
        this.lastError = '';
        this.lastErrorHint = '';
        this.lastErrorDetail = '';
        this.lastErrorWhere = '';
        this.lastErrorServerPath = '';
        this.lastErrorRequestId = '';
        this.version = s.version || '0.1.0';
        this.agentCount = s.agent_count || 0;
      } catch(e) {
        this.connected = false;
        this.lastError = e.message || 'Unknown error';
        this.lastErrorHint = e.hint || '';
        this.lastErrorDetail = e.detail || '';
        this.lastErrorWhere = e.where || '';
        this.lastErrorServerPath = e.serverPath || '';
        this.lastErrorRequestId = e.requestId || '';
        console.warn('[ArmaraOS] Status check failed:', e.message);
      }
    },

    copyConnectionDebug() {
      var lines = [
        'ArmaraOS connection debug',
        'URL: ' + (typeof window !== 'undefined' ? window.location.origin : ''),
        'Where: ' + (this.lastErrorWhere || ''),
        'API path: ' + (this.lastErrorServerPath || ''),
        'Request ID: ' + (this.lastErrorRequestId || ''),
        'Error: ' + (this.lastError || ''),
        'Hint: ' + (this.lastErrorHint || ''),
        'Detail: ' + (this.lastErrorDetail || ''),
        'Time: ' + new Date().toISOString()
      ];
      var text = lines.join('\n');
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(function() {
          if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Copied debug info');
        }).catch(function() {});
      }
    },

    async generateAndCopyDebugBundle() {
      var bundlePath = '';
      try {
        // Prefer desktop shell command (works even when API key is enabled).
        var bundle = await ArmaraosDesktopTauriInvoke('generate_support_bundle');
        if (bundle && bundle.bundle_path) bundlePath = bundle.bundle_path;
      } catch(e1) { /* ignore */ }

      if (!bundlePath) {
        try {
          var res = await OpenFangAPI.post('/api/support/diagnostics', {});
          bundlePath = (res && res.bundle_path) ? res.bundle_path : '';
        } catch(e2) {
          // If the bundle fails, still copy actionable debug info
          this.copyConnectionDebug();
          if (typeof OpenFangToast !== 'undefined') {
            OpenFangToast.error((e2 && e2.message) ? e2.message : 'Diagnostics bundle failed');
          }
          return;
        }
      }

      var lines = [
        'ArmaraOS debug bundle',
        'Bundle: ' + bundlePath,
        'URL: ' + (typeof window !== 'undefined' ? window.location.origin : ''),
        'Where: ' + (this.lastErrorWhere || ''),
        'Request ID: ' + (this.lastErrorRequestId || ''),
        'Error: ' + (this.lastError || ''),
        'Hint: ' + (this.lastErrorHint || ''),
        'Detail: ' + (this.lastErrorDetail || ''),
        'Time: ' + new Date().toISOString()
      ];
      var text = lines.join('\n');
      if (navigator.clipboard && navigator.clipboard.writeText) {
        navigator.clipboard.writeText(text).then(function() {
          if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Generated + copied debug bundle');
        }).catch(function() {});
      }
    },

    async checkOnboarding() {
      if (localStorage.getItem('openfang-onboarded')) return;
      try {
        var config = await OpenFangAPI.get('/api/config');
        var apiKey = config && config.api_key;
        var noKey = !apiKey || apiKey === 'not set' || apiKey === '';
        if (noKey && this.agentCount === 0) {
          this.showOnboarding = true;
        }
      } catch(e) {
        // If config endpoint fails, still show onboarding if no agents
        if (this.agentCount === 0) this.showOnboarding = true;
      }
    },

    dismissOnboarding() {
      this.showOnboarding = false;
      localStorage.setItem('openfang-onboarded', 'true');
    },

    async checkAuth() {
      try {
        // First check if session-based auth is configured
        var authInfo = await OpenFangAPI.get('/api/auth/check');
        if (authInfo.mode === 'none') {
          // No session auth — fall back to API key detection
          this.authMode = 'apikey';
          this.sessionUser = null;
        } else if (authInfo.mode === 'session') {
          this.authMode = 'session';
          if (authInfo.authenticated) {
            this.sessionUser = authInfo.username;
            this.showAuthPrompt = false;
            return;
          }
          // Session auth enabled but not authenticated — show login prompt
          this.showAuthPrompt = true;
          return;
        }
      } catch(e) { /* ignore — fall through to API key check */ }

      // API key mode detection
      try {
        await OpenFangAPI.get('/api/tools');
        this.showAuthPrompt = false;
      } catch(e) {
        if (e.message && (e.message.indexOf('Not authorized') >= 0 || e.message.indexOf('401') >= 0 || e.message.indexOf('Missing Authorization') >= 0 || e.message.indexOf('Unauthorized') >= 0)) {
          var saved = localStorage.getItem('openfang-api-key');
          if (saved) {
            OpenFangAPI.setAuthToken('');
            localStorage.removeItem('openfang-api-key');
          }
          this.showAuthPrompt = true;
        }
      }
    },

    submitApiKey(key) {
      if (!key || !key.trim()) return;
      OpenFangAPI.setAuthToken(key.trim());
      localStorage.setItem('openfang-api-key', key.trim());
      this.showAuthPrompt = false;
      this.refreshAgents();
    },

    async sessionLogin(username, password) {
      try {
        var result = await OpenFangAPI.post('/api/auth/login', { username: username, password: password });
        if (result.status === 'ok') {
          this.sessionUser = result.username;
          this.showAuthPrompt = false;
          this.refreshAgents();
        } else {
          OpenFangToast.error(result.error || 'Login failed');
        }
      } catch(e) {
        OpenFangToast.error(e.message || 'Login failed');
      }
    },

    async sessionLogout() {
      try {
        await OpenFangAPI.post('/api/auth/logout');
      } catch(e) { /* ignore */ }
      this.sessionUser = null;
      this.showAuthPrompt = true;
    },

    clearApiKey() {
      OpenFangAPI.setAuthToken('');
      localStorage.removeItem('openfang-api-key');
    }
  });

  Alpine.store('kernelEvents', {
    connected: false,
    error: '',
    received: 0,
    items: [],
    last: null,
  });

  /** Subscribe to GET /api/events/stream (kernel bus). Started from app().init. */
  window.ArmaraosKernelSse = (function() {
    var _es = null;
    function isTauriShell() {
      var w = typeof window !== 'undefined' ? window : null;
      return !!(w && w.__TAURI__ && w.__TAURI__.core);
    }
    function maybeToast(ev) {
      if (typeof OpenFangToast === 'undefined' || !ev || !ev.payload) return;
      if (isTauriShell()) return;
      var p = ev.payload;
      if (p.type === 'Lifecycle' && p.data && p.data.event === 'Crashed') {
        var err = (p.data.error || '').slice(0, 220);
        OpenFangToast.error('Agent crashed: ' + err, 8000);
      } else if (p.type === 'System' && p.data && p.data.event === 'KernelStopping') {
        OpenFangToast.warn('Kernel stopping…', 5000);
      } else if (p.type === 'System' && p.data && p.data.event === 'QuotaEnforced') {
        OpenFangToast.warn('Quota enforced for an agent', 6000);
      } else if (p.type === 'System' && p.data && p.data.event === 'HealthCheckFailed') {
        OpenFangToast.warn('Agent health check failed', 6000);
      } else if (p.type === 'System' && p.data && p.data.event === 'CronJobCompleted') {
        var name = p.data.job_name || 'Scheduled job';
        var out = (p.data.output_preview || '').slice(0, 180);
        OpenFangToast.info(name + ': ' + out, 7000);
      } else if (p.type === 'System' && p.data && p.data.event === 'CronJobFailed') {
        var name2 = p.data.job_name || 'Scheduled job';
        var err2 = (p.data.error || '').slice(0, 180);
        OpenFangToast.error(name2 + ' failed: ' + err2, 8000);
      }
    }
    return {
      start: function() {
        if (_es) {
          try { _es.close(); } catch (e) { /* ignore */ }
          _es = null;
        }
        var ke;
        try {
          ke = Alpine.store('kernelEvents');
        } catch (e) {
          return;
        }
        try {
          _es = new EventSource(OpenFangAPI.sseUrl('/api/events/stream'));
        } catch (err) {
          ke.error = 'EventSource unavailable';
          return;
        }
        _es.onopen = function() {
          ke.connected = true;
          ke.error = '';
        };
        _es.onerror = function() {
          ke.connected = false;
          ke.error = 'disconnected';
        };
        _es.onmessage = function(event) {
          if (!event.data) return;
          try {
            var j = JSON.parse(event.data);
            ke.received++;
            ke.last = j;
            if (ke.items.length > 80) ke.items.shift();
            ke.items.push({ id: j.id, ts: j.timestamp, payload: j.payload });
            maybeToast(j);
            window.dispatchEvent(new CustomEvent('armaraos-kernel-event', { detail: j }));
          } catch (e) { /* ignore parse errors */ }
        };
      },
      stop: function() {
        if (_es) {
          try { _es.close(); } catch (e) { /* ignore */ }
          _es = null;
        }
        try {
          Alpine.store('kernelEvents').connected = false;
        } catch (e) { /* ignore */ }
      },
    };
  })();
});

// Main app component
function app() {
  return {
    page: 'agents',
    themeMode: getStoredThemeMode(),
    theme: (() => {
      return effectiveThemeFromMode(getStoredThemeMode());
    })(),
    sidebarCollapsed: localStorage.getItem('openfang-sidebar') === 'collapsed',
    mobileMenuOpen: false,
    connected: false,
    wsConnected: false,
    version: '0.1.0',
    agentCount: 0,

    get agents() { return Alpine.store('app').agents; },

    init() {
      var self = this;

      // Listen for OS theme changes (only matters when mode is 'system')
      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function(e) {
        if (self.themeMode === 'system') {
          self.theme = e.matches ? 'dark' : 'light';
        }
      });

      // Hash routing
      var validPages = ['overview','agents','sessions','approvals','comms','network','workflows','scheduler','channels','skills','hands','ainl-library','analytics','logs','runtime','settings','wizard'];
      var pageRedirects = {
        'chat': 'agents',
        'templates': 'agents',
        'triggers': 'workflows',
        'cron': 'scheduler',
        'schedules': 'scheduler',
        'memory': 'sessions',
        'audit': 'logs',
        'security': 'settings',
        'peers': 'settings',
        'migration': 'settings',
        'usage': 'analytics',
        'approval': 'approvals'
      };
      function handleHash() {
        var raw = window.location.hash.replace(/^#/, '') || 'agents';
        var pagePart = raw;
        var query = '';
        var qIdx = raw.indexOf('?');
        if (qIdx >= 0) {
          pagePart = raw.slice(0, qIdx);
          query = raw.slice(qIdx + 1);
        }
        if (pageRedirects[pagePart]) {
          pagePart = pageRedirects[pagePart];
          window.location.hash = pagePart + (query ? '?' + query : '');
          return;
        }
        if (validPages.indexOf(pagePart) >= 0) {
          if (query && pagePart === 'scheduler') {
            try {
              var params = new URLSearchParams(query);
              var ainl = params.get('ainl');
              if (ainl) {
                var payload = {
                  actionKind: 'ainl_run',
                  ainlPath: decodeURIComponent(ainl),
                  cron: params.get('cron') || '0 9 * * *'
                };
                if (params.get('json') === '1' || params.get('json') === 'true') {
                  payload.json_output = true;
                }
                sessionStorage.setItem('armaraos-scheduler-prefill', JSON.stringify(payload));
              }
            } catch (e1) { /* ignore */ }
            try {
              if (window.history && window.history.replaceState) {
                var u = new URL(window.location.href);
                u.hash = 'scheduler';
                window.history.replaceState({}, '', u.pathname + u.search + u.hash);
              }
            } catch (e2) { /* ignore */ }
          }
          if (self.page !== pagePart) {
            window.dispatchEvent(new CustomEvent('page-leave'));
          }
          self.page = pagePart;
        }
      }
      window.addEventListener('hashchange', handleHash);
      handleHash();

      // Keyboard shortcuts
      document.addEventListener('keydown', function(e) {
        // Ctrl+K — focus agent switch / go to agents
        if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
          e.preventDefault();
          self.navigate('agents');
        }
        // Ctrl+N — new agent
        if ((e.ctrlKey || e.metaKey) && e.key === 'n' && !e.shiftKey) {
          e.preventDefault();
          self.navigate('agents');
        }
        // Ctrl+Shift+F — toggle focus mode
        if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key === 'F') {
          e.preventDefault();
          Alpine.store('app').toggleFocusMode();
        }
        // Escape — close mobile menu
        if (e.key === 'Escape') {
          self.mobileMenuOpen = false;
        }
      });

      // Connection state listener
      OpenFangAPI.onConnectionChange(function(state) {
        Alpine.store('app').connectionState = state;
      });

      // Initial data load
      this.pollStatus();
      Alpine.store('app').refreshApprovals();
      Alpine.store('app').checkOnboarding();
      Alpine.store('app').checkAuth();
      setInterval(function() {
        self.pollStatus();
        Alpine.store('app').refreshApprovals();
      }, 5000);

      if (typeof window.ArmaraosKernelSse !== 'undefined' && window.ArmaraosKernelSse.start) {
        window.ArmaraosKernelSse.start();
      }

      // Desktop: poll AINL status while Rust boots the venv + pip (startup ensure_ainl_installed).
      (function armaraosDesktopAinlPoll() {
        var w = typeof window !== 'undefined' ? window : null;
        if (!w || !w.__TAURI__ || !w.__TAURI__.core) return;
        try {
          Alpine.store('ainl').bootstrapping = true;
        } catch (e) { /* ignore */ }
        var attempts = 0;
        // Pip + PyPI can exceed ~3 min on slow networks; keep polling up to ~8 min.
        var maxAttempts = 120;
        var delayMs = 4000;
        function tick() {
          attempts++;
          ArmaraosDesktopTauriInvoke('ainl_status')
            .then(function (st) {
              try {
                Alpine.store('ainl').desktop = st;
                var done = st && st.ok;
                Alpine.store('ainl').bootstrapping = !done;
                if (done) return;
              } catch (e) { /* ignore */ }
              if (attempts < maxAttempts) {
                setTimeout(tick, delayMs);
              } else {
                try {
                  Alpine.store('ainl').bootstrapping = false;
                } catch (e2) { /* ignore */ }
              }
            })
            .catch(function () {
              if (attempts < maxAttempts) setTimeout(tick, delayMs);
              else {
                try {
                  Alpine.store('ainl').bootstrapping = false;
                } catch (e2) { /* ignore */ }
              }
            });
        }
        tick();
      })();
    },

    navigate(p) {
      if (this.page !== p) {
        window.dispatchEvent(new CustomEvent('page-leave'));
      }
      this.page = p;
      window.location.hash = p;
      this.mobileMenuOpen = false;
    },

    setTheme(mode) {
      this.themeMode = mode;
      try {
        localStorage.setItem('armaraos-theme-mode', mode);
        localStorage.setItem('openfang-theme-mode', mode);
      } catch (e) { /* private mode */ }
      this.theme = effectiveThemeFromMode(mode);
      ArmaraosDesktopTauriInvoke('set_dashboard_theme_mode', { mode: mode }).catch(function() {});
    },

    toggleTheme() {
      var modes = ['light', 'system', 'dark'];
      var next = modes[(modes.indexOf(this.themeMode) + 1) % modes.length];
      this.setTheme(next);
    },

    toggleSidebar() {
      this.sidebarCollapsed = !this.sidebarCollapsed;
      localStorage.setItem('openfang-sidebar', this.sidebarCollapsed ? 'collapsed' : 'expanded');
    },

    /** Opens the AINL marketing site; uses Tauri when embedded in desktop so the system browser opens. */
    openAinlAttributionUrl(ev) {
      var url = 'https://ainativelang.com/';
      var w = typeof window !== 'undefined' ? window : null;
      var core = w && w.__TAURI__ && w.__TAURI__.core;
      if (core && typeof core.invoke === 'function') {
        if (ev) ev.preventDefault();
        core.invoke('open_external_url', { url: url }).catch(function() {
          window.open(url, '_blank', 'noopener,noreferrer');
        });
        return;
      }
    },

    async pollStatus() {
      var store = Alpine.store('app');
      await store.checkStatus();
      await store.refreshAgents();
      this.connected = store.connected;
      this.version = store.version;
      this.agentCount = store.agentCount;
      this.wsConnected = OpenFangAPI.isWsConnected();
      this.maybeOfferFirstRunWizard();
    },

    /** One-time redirect to Setup Wizard for new installs (default #agents, no agents, daemon up). */
    maybeOfferFirstRunWizard() {
      try {
        if (typeof localStorage === 'undefined') return;
        if (localStorage.getItem('openfang-onboarded') === 'true') return;
        if (localStorage.getItem('of-first-run-wizard-offered') === 'true') return;
        var store = Alpine.store('app');
        if (!store.connected || store.agentCount > 0) return;
        if (this.page !== 'agents') return;
        localStorage.setItem('of-first-run-wizard-offered', 'true');
        window.location.hash = 'wizard';
      } catch (e) { /* ignore */ }
    }
  };
}
