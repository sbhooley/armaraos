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

/** Kernel-spawned internal chat agents (hidden from sidebar; grouped under Chat → Automation & probe). */
function isInternalAutomationProbeChatAgentName(name) {
  var n = name != null ? String(name) : '';
  return (
    n.startsWith('allowlist-probe') ||
    n.startsWith('offline-cron') ||
    n.startsWith('allow-ir-off')
  );
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

/** True when the dashboard runs inside the ArmaraOS desktop app (Tauri), not a normal browser tab. */
function isArmaraosDesktopShell() {
  try {
    var w = typeof window !== 'undefined' ? window : null;
    var core = w && w.__TAURI__ && w.__TAURI__.core;
    return !!(core && typeof core.invoke === 'function');
  } catch (e) {
    return false;
  }
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

/**
 * Updates Alpine.store('ainl') fields used by the sidebar WS / AINL / SSE badge row.
 * Desktop: Tauri `ainl_status` (bundled runtime). Browser: throttled GET /api/ainl/library/curated.
 */
async function armaraosRefreshAinlSidebarBadge(connected) {
  try {
    var ainl = Alpine.store('ainl');
    if (!connected) {
      ainl.sidebarOk = false;
      ainl.sidebarTitle = 'Connect to the daemon to use AINL';
      return;
    }
    if (isArmaraosDesktopShell()) {
      if (ainl.bootstrapping) {
        ainl.sidebarOk = false;
        ainl.sidebarTitle = 'Installing AINL runtime…';
        return;
      }
      var d = ainl.desktop;
      if (d && d.ok) {
        ainl.sidebarOk = true;
        ainl.sidebarTitle = 'AINL ready (cli + MCP in desktop bundle)';
        return;
      }
      ainl.sidebarOk = false;
      var det = d && d.detail ? String(d.detail).trim() : '';
      ainl.sidebarTitle = det ? det.slice(0, 200) : 'AINL not ready — open Settings → Runtime';
      return;
    }
    var now = Date.now();
    if (ainl.hostLastProbe && now - ainl.hostLastProbe < 25000) {
      return;
    }
    ainl.hostLastProbe = now;
    try {
      await OpenFangAPI.get('/api/ainl/library/curated');
      ainl.hostLibraryOk = true;
      ainl.sidebarOk = true;
      ainl.sidebarTitle = 'AINL library API available on this daemon';
    } catch (e) {
      ainl.hostLibraryOk = false;
      ainl.sidebarOk = false;
      ainl.sidebarTitle = 'AINL library API not reachable';
    }
  } catch (e) { /* ignore */ }
}

/**
 * Fallback for WebViews (e.g. Tauri/WKWebView) where navigator.clipboard.writeText
 * rejects or is unavailable despite a user gesture.
 */
function copyTextViaExecCommand(text) {
  var ta = document.createElement('textarea');
  ta.value = text;
  ta.setAttribute('readonly', '');
  ta.style.position = 'fixed';
  ta.style.left = '-9999px';
  ta.style.top = '0';
  document.body.appendChild(ta);
  ta.focus();
  ta.select();
  try {
    ta.setSelectionRange(0, text.length);
  } catch (e) { /* IE */ }
  var ok = false;
  try {
    ok = document.execCommand('copy');
  } catch (e) {}
  document.body.removeChild(ta);
  return ok;
}

/** Resolves true on success. Tries Clipboard API, then execCommand. */
function copyTextToClipboard(text) {
  text = text == null ? '' : String(text);
  return new Promise(function(resolve, reject) {
    if (navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
      navigator.clipboard.writeText(text).then(function() {
        resolve(true);
      }).catch(function() {
        if (copyTextViaExecCommand(text)) resolve(true);
        else reject(new Error('Clipboard write failed'));
      });
    } else {
      if (copyTextViaExecCommand(text)) resolve(true);
      else reject(new Error('Clipboard API unavailable'));
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
    copyTextToClipboard(code.textContent || '').then(function() {
      btn.textContent = 'Copied!';
      btn.classList.add('copied');
      setTimeout(function() { btn.textContent = 'Copy'; btn.classList.remove('copied'); }, 1500);
    }).catch(function() {
      if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Could not copy');
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

/** EventTarget JSON: `{ type: 'Agent', value: '<uuid>' }` from kernel SSE. */
function armaraosEventTargetAgentId(target) {
  if (!target || typeof target !== 'object') return '';
  if (target.type === 'Agent' && target.value != null) return String(target.value);
  return '';
}

/** After a WS/SSE unread signal, align assistant-count baseline so digest polling does not double-count. */
var _armaraosDigestSyncTimers = {};
function armaraosScheduleDigestBaselineSync(agentId) {
  if (!agentId) return;
  var id = String(agentId);
  if (_armaraosDigestSyncTimers[id]) clearTimeout(_armaraosDigestSyncTimers[id]);
  _armaraosDigestSyncTimers[id] = setTimeout(function() {
    _armaraosDigestSyncTimers[id] = null;
    if (typeof OpenFangAPI === 'undefined' || !OpenFangAPI.get) return;
    OpenFangAPI.get('/api/agents/' + encodeURIComponent(id) + '/session/digest')
      .then(function(d) {
        var ac = (d && typeof d.assistant_message_count === 'number') ? d.assistant_message_count : 0;
        try { Alpine.store('app').setChatAssistantBaseline(id, ac); } catch (e) { /* ignore */ }
      })
      .catch(function() {});
  }, 450);
}

function armaraosAgentDisplayName(agentId) {
  if (!agentId) return '';
  try {
    var agents = Alpine.store('app').agents || [];
    for (var i = 0; i < agents.length; i++) {
      if (agents[i].id === agentId) return agents[i].name || agentId;
    }
  } catch (e) { /* ignore */ }
  var s = String(agentId);
  return s.length > 14 ? s.slice(0, 8) + '…' : s;
}

function armaraosAgentActivityLineFromSystemPayload(p) {
  if (!p || p.type !== 'System' || !p.data || p.data.event !== 'AgentActivity') return '';
  var d = p.data;
  var phase = d.phase || '';
  var det = d.detail;
  if (phase === 'thinking') return 'Thinking…';
  if (phase === 'tool_use') return 'Using tool: ' + (det || 'tool');
  if (phase === 'streaming') return 'Writing response…';
  if (det) return phase ? (phase + ': ' + det) : String(det);
  return phase || '';
}

/** Persist dismissed kernel notification ids (event ids) across reloads — key `armaraos-notify-dismissed-kernel`. */
var NOTIFY_KERNEL_DISMISS_KEY = 'armaraos-notify-dismissed-kernel';
var NOTIFY_KERNEL_DISMISS_MAX = 400;
var _armaraosKernelDismissCache = null;

function armaraosNotifyLoadKernelDismissSet() {
  try {
    var raw = localStorage.getItem(NOTIFY_KERNEL_DISMISS_KEY);
    var arr = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(arr)) return {};
    var o = {};
    for (var i = 0; i < arr.length; i++) {
      if (arr[i]) o[String(arr[i])] = true;
    }
    return o;
  } catch (e) {
    return {};
  }
}

function armaraosNotifyKernelDismissSetCached() {
  if (!_armaraosKernelDismissCache) {
    _armaraosKernelDismissCache = armaraosNotifyLoadKernelDismissSet();
  }
  return _armaraosKernelDismissCache;
}

function armaraosNotifyInvalidateKernelDismissCache() {
  _armaraosKernelDismissCache = null;
}

function armaraosNotifyIsKernelDismissed(id) {
  if (!id || String(id).indexOf('k-') !== 0) return false;
  return !!armaraosNotifyKernelDismissSetCached()[String(id)];
}

function armaraosNotifyPersistKernelDismiss(id) {
  if (!id || String(id).indexOf('k-') !== 0) return;
  try {
    var raw = localStorage.getItem(NOTIFY_KERNEL_DISMISS_KEY);
    var arr = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(arr)) arr = [];
    var s = String(id);
    if (arr.indexOf(s) < 0) arr.push(s);
    if (arr.length > NOTIFY_KERNEL_DISMISS_MAX) arr = arr.slice(-NOTIFY_KERNEL_DISMISS_MAX);
    localStorage.setItem(NOTIFY_KERNEL_DISMISS_KEY, JSON.stringify(arr));
    armaraosNotifyInvalidateKernelDismissCache();
  } catch (e) { /* ignore */ }
}

var _armaraosNotifyTrapHandler = null;

function armaraosNotifyFocusableSelector() {
  return 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
}

function armaraosNotifyFocusFirstInPanel() {
  var panel = document.getElementById('notify-center-panel');
  if (!panel) return;
  var sel = armaraosNotifyFocusableSelector();
  var list = panel.querySelectorAll(sel);
  var focusables = [];
  for (var i = 0; i < list.length; i++) {
    var el = list[i];
    try {
      if (el.offsetParent !== null || el.getClientRects().length > 0) focusables.push(el);
    } catch (e) {
      focusables.push(el);
    }
  }
  if (focusables.length > 0) {
    try {
      focusables[0].focus();
    } catch (e2) { /* ignore */ }
  } else {
    try {
      panel.focus();
    } catch (e3) { /* ignore */ }
  }
}

function armaraosNotifyTrapInstall(notifyStore) {
  armaraosNotifyTrapRemove();
  _armaraosNotifyTrapHandler = function(e) {
    if (e.key !== 'Tab' || !notifyStore || !notifyStore.panelOpen) return;
    var panel = document.getElementById('notify-center-panel');
    if (!panel) return;
    var sel = armaraosNotifyFocusableSelector();
    var nodes = panel.querySelectorAll(sel);
    var list = [];
    for (var i = 0; i < nodes.length; i++) {
      var el = nodes[i];
      try {
        if (el.offsetParent !== null || el.getClientRects().length > 0) list.push(el);
      } catch (ex) {
        list.push(el);
      }
    }
    if (list.length === 0) return;
    var first = list[0];
    var last = list[list.length - 1];
    var active = document.activeElement;
    if (!panel.contains(active)) return;
    if (e.shiftKey) {
      if (active === first) {
        e.preventDefault();
        last.focus();
      }
    } else {
      if (active === last) {
        e.preventDefault();
        first.focus();
      }
    }
  };
  document.addEventListener('keydown', _armaraosNotifyTrapHandler, true);
}

function armaraosNotifyTrapRemove() {
  if (_armaraosNotifyTrapHandler) {
    document.removeEventListener('keydown', _armaraosNotifyTrapHandler, true);
    _armaraosNotifyTrapHandler = null;
  }
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
    /** Browser-only: last curated-library probe (ms); throttles /api/ainl/library/curated */
    hostLastProbe: 0,
    hostLibraryOk: null,
    /** Sidebar badge: green when AINL is usable on this shell */
    sidebarOk: false,
    sidebarTitle: '',
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
    /** Per-agent status line from kernel SSE (loop phase + inter-agent sends). */
    agentActivityLines: {},
    /** Dashboard hash route (mirrors root `page`) for unread / chat visibility. */
    dashboardPage: 'agents',
    /** Pinned agent IDs (floated to top of Quick Open sidebar list).
     * Seeded from localStorage for instant display; authoritative copy is server-side
     * in ui-prefs.json so it survives reinstalls and upgrades. */
    pinnedAgentIds: (function() { try { return JSON.parse(localStorage.getItem('armaraos-pinned-agents') || '[]'); } catch(e) { return []; } })(),
    /** When inline chat is open on #agents, the agent id being viewed (null = picker). */
    agentsPageChatAgentId: null,
    /** agentId -> count of unread assistant-side updates (replaced immutably for Alpine). */
    chatUnreadCounts: {},
    /** agentId -> last seen assistant_message_count from GET .../session/digest (poll + dedupe). */
    chatAssistantBaseline: {},

    setChatAssistantBaseline(agentId, count) {
      if (agentId == null || agentId === '') return;
      var id = String(agentId);
      var n = typeof count === 'number' && !isNaN(count) ? Math.max(0, Math.floor(count)) : 0;
      var prev = this.chatAssistantBaseline || {};
      var next = Object.assign({}, prev);
      next[id] = n;
      this.chatAssistantBaseline = next;
    },

    /** Call when opening a chat so digest polling does not treat history as new. */
    primeAssistantBaselineForAgent(agentId) {
      var self = this;
      if (agentId == null || agentId === '') return;
      var id = String(agentId);
      OpenFangAPI.get('/api/agents/' + encodeURIComponent(id) + '/session/digest')
        .then(function(d) {
          var ac = (d && typeof d.assistant_message_count === 'number') ? d.assistant_message_count : 0;
          self.setChatAssistantBaseline(id, ac);
        })
        .catch(function() {});
    },

    /** Digest poll: bump unread when assistant messages grew while user is not in that chat. */
    ingestSessionDigestFromPoll(agentId, assistantCount) {
      var id = String(agentId);
      var ac = typeof assistantCount === 'number' && !isNaN(assistantCount) ? Math.max(0, Math.floor(assistantCount)) : 0;
      var base = this.chatAssistantBaseline || {};
      var prev = base[id];
      if (prev === undefined) {
        this.setChatAssistantBaseline(id, ac);
        return;
      }
      if (ac > prev && !this.isChatSurfaceActiveForAgent(id)) {
        this.bumpAgentChatUnread(id);
      }
      this.setChatAssistantBaseline(id, ac);
    },

    runSessionDigestPollRound() {
      var self = this;
      if (!this.connected) return;
      var agents = this.primaryAgentsForSidebar();
      var seen = {};
      agents.forEach(function(a) {
        if (!a || a.id == null) return;
        var id = String(a.id);
        seen[id] = true;
        if (self.isChatSurfaceActiveForAgent(id)) return;
        OpenFangAPI.get('/api/agents/' + encodeURIComponent(id) + '/session/digest')
          .then(function(d) {
            var ac = (d && typeof d.assistant_message_count === 'number') ? d.assistant_message_count : 0;
            self.ingestSessionDigestFromPoll(id, ac);
          })
          .catch(function() {});
      });
      try {
        var wsId = typeof OpenFangAPI.getWsAgentId === 'function' ? OpenFangAPI.getWsAgentId() : null;
        if (wsId && !seen[String(wsId)] && !self.isChatSurfaceActiveForAgent(String(wsId))) {
          var wid = String(wsId);
          OpenFangAPI.get('/api/agents/' + encodeURIComponent(wid) + '/session/digest')
            .then(function(d) {
              var ac = (d && typeof d.assistant_message_count === 'number') ? d.assistant_message_count : 0;
              self.ingestSessionDigestFromPoll(wid, ac);
            })
            .catch(function() {});
        }
      } catch (eW) { /* ignore */ }
    },

    agentHasChatUnread(agentId) {
      if (!agentId) return false;
      var c = this.chatUnreadCounts || {};
      return (c[agentId] || 0) > 0;
    },

    get chatUnreadTotal() {
      var c = this.chatUnreadCounts || {};
      var t = 0;
      for (var k in c) {
        if (Object.prototype.hasOwnProperty.call(c, k) && c[k] > 0) t += c[k];
      }
      return t;
    },

    /** True when this agent's inline chat is visible (agents page + chat open + tab focused). */
    isChatSurfaceActiveForAgent(agentId) {
      if (!agentId) return false;
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return false;
      if (this.dashboardPage !== 'agents') return false;
      return this.agentsPageChatAgentId === agentId;
    },

    bumpAgentChatUnread(agentId) {
      if (!agentId || this.isChatSurfaceActiveForAgent(agentId)) return;
      var prev = this.chatUnreadCounts || {};
      var next = Object.assign({}, prev);
      next[agentId] = (next[agentId] || 0) + 1;
      this.chatUnreadCounts = next;
      this.updateTabTitle();
    },

    clearAgentChatUnread(agentId) {
      if (!agentId) return;
      var prev = this.chatUnreadCounts || {};
      if (!prev[agentId]) return;
      var next = Object.assign({}, prev);
      delete next[agentId];
      this.chatUnreadCounts = next;
      this.updateTabTitle();
    },

    updateTabTitle() {
      try {
        var n = this.chatUnreadTotal;
        document.title = n > 0 ? '(' + n + ') ArmaraOS' : 'ArmaraOS';
      } catch (e) { /* ignore */ }
    },

    /** Load UI preferences from server (pinned agents etc.) and merge into local state. */
    async loadUiPrefs() {
      try {
        var prefs = await OpenFangAPI.get('/api/ui-prefs');
        if (Array.isArray(prefs.pinned_agents)) {
          this.pinnedAgentIds = prefs.pinned_agents;
          try { localStorage.setItem('armaraos-pinned-agents', JSON.stringify(prefs.pinned_agents)); } catch(e) { /* ignore */ }
        }
      } catch(e) { /* keep localStorage-seeded values on failure */ }
    },

    /** Persist current UI prefs to the server (fire-and-forget). */
    _saveUiPrefs() {
      var prefs = { pinned_agents: this.pinnedAgentIds || [] };
      OpenFangAPI.put('/api/ui-prefs', prefs).catch(function() { /* ignore */ });
    },

    /** Pin or unpin an agent in the Quick Open sidebar list. */
    togglePinAgent(agentId) {
      if (!agentId) return;
      var id = String(agentId);
      var prev = this.pinnedAgentIds || [];
      var next = prev.indexOf(id) >= 0
        ? prev.filter(function(x) { return x !== id; })
        : prev.concat([id]);
      this.pinnedAgentIds = next;
      try { localStorage.setItem('armaraos-pinned-agents', JSON.stringify(next)); } catch (e) { /* ignore */ }
      this._saveUiPrefs();
    },

    isAgentPinned(agentId) {
      if (!agentId) return false;
      return (this.pinnedAgentIds || []).indexOf(String(agentId)) >= 0;
    },

    /** Record a recent agent visit for the "Jump back in" overview strip. */
    recordRecentAgent(agent) {
      if (!agent || !agent.id) return;
      try {
        var raw = localStorage.getItem('armaraos-recent-agents');
        var list = raw ? JSON.parse(raw) : [];
        var id = String(agent.id);
        list = list.filter(function(x) { return x.id !== id; });
        list.unshift({ id: id, name: agent.name || '', emoji: (agent.identity && agent.identity.emoji) || '', ts: Date.now() });
        localStorage.setItem('armaraos-recent-agents', JSON.stringify(list.slice(0, 5)));
      } catch (e) { /* ignore */ }
    },

    setAgentActivityLine(agentId, text) {
      if (!agentId || !text) return;
      var self = this;
      var ts = Date.now();
      if (!this.agentActivityLines) this.agentActivityLines = {};
      this.agentActivityLines[agentId] = { text: text, ts: ts };
      setTimeout(function() {
        try {
          var cur = self.agentActivityLines && self.agentActivityLines[agentId];
          if (cur && cur.ts === ts) {
            delete self.agentActivityLines[agentId];
          }
        } catch (e) { /* ignore */ }
      }, 18000);
    },

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

    /** User-facing agents for the sidebar (excludes internal automation / probe chats). Pinned agents float to top. */
    primaryAgentsForSidebar() {
      var agents = this.agents || [];
      var pinned = this.pinnedAgentIds || [];
      var filtered = agents.filter(function(a) {
        return !isInternalAutomationProbeChatAgentName(a && a.name);
      });
      return filtered.slice().sort(function(a, b) {
        var ap = pinned.indexOf(String(a.id)) >= 0;
        var bp = pinned.indexOf(String(b.id)) >= 0;
        if (ap && !bp) return -1;
        if (!ap && bp) return 1;
        return 0;
      });
    },

    /** Open inline chat for this agent (Agents page) from anywhere (e.g. sidebar). */
    openAgentChat(agent) {
      if (!agent) return;
      this.recordRecentAgent(agent);
      this.pendingAgent = agent;
      var h = (window.location.hash || '').replace(/^#/, '');
      if (h !== 'agents') {
        window.location.hash = 'agents';
      }
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
        try {
          Alpine.store('notifyCenter').syncPendingApprovals(pending.length, signature);
        } catch (eN) { /* ignore */ }
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

  /** Persistent notification center (bell): approvals, budget threshold, kernel agent/system events. */
  Alpine.store('notifyCenter', {
    panelOpen: false,
    items: [],
    /** Screen-reader live region (panel closed): new items. */
    liveRegionText: '',
    approvalDismissSig: '',
    budgetSnoozeUntil: 0,
    _healthNotifyAt: {},
    _notifyFocusBefore: null,

    toggle() {
      if (this.panelOpen) {
        this.close();
      } else {
        this.open();
      }
    },
    open() {
      if (this.panelOpen) return;
      try {
        this._notifyFocusBefore = document.activeElement;
      } catch (e) {
        this._notifyFocusBefore = null;
      }
      this.panelOpen = true;
      var self = this;
      setTimeout(function() {
        armaraosNotifyTrapInstall(self);
        armaraosNotifyFocusFirstInPanel();
      }, 0);
    },
    close() {
      this.panelOpen = false;
      armaraosNotifyTrapRemove();
      var el = this._notifyFocusBefore;
      this._notifyFocusBefore = null;
      if (el && typeof el.focus === 'function') {
        try {
          el.focus();
        } catch (e2) { /* ignore */ }
      }
    },
    badgeCount() {
      return (this.items || []).length;
    },
    _announceIfClosed(title) {
      if (this.panelOpen) return;
      var self = this;
      this.liveRegionText = '';
      requestAnimationFrame(function() {
        requestAnimationFrame(function() {
          self.liveRegionText = 'New notification: ' + (title || 'update');
        });
      });
    },
    dismiss(id) {
      if (!id) return;
      if (String(id).indexOf('k-') === 0) {
        armaraosNotifyPersistKernelDismiss(id);
      }
      if (id === 'approval-pending') {
        try {
          this.approvalDismissSig = Alpine.store('app').lastPendingApprovalSignature || '';
        } catch (e) {
          this.approvalDismissSig = '';
        }
      } else if (id === 'budget-alert') {
        this.budgetSnoozeUntil = Date.now() + 3600000;
      }
      this.items = (this.items || []).filter(function(x) { return x.id !== id; });
    },
    clearAll() {
      this.items = [];
      this.approvalDismissSig = '';
      this.budgetSnoozeUntil = 0;
      this._healthNotifyAt = {};
    },
    _prepend(row) {
      if (row && row.id && String(row.id).indexOf('k-') === 0 && armaraosNotifyIsKernelDismissed(row.id)) {
        return;
      }
      var list = (this.items || []).filter(function(x) { return x.id !== row.id; });
      list.unshift(row);
      if (list.length > 48) list.length = 48;
      this.items = list;
      if (!this.panelOpen && row && row.title) {
        this._announceIfClosed(row.title);
      }
    },
    syncPendingApprovals(count, sig) {
      sig = sig != null ? String(sig) : '';
      if (!count) {
        this.items = (this.items || []).filter(function(x) { return x.id !== 'approval-pending'; });
        this.approvalDismissSig = '';
        return;
      }
      if (sig && sig === this.approvalDismissSig) return;
      this._prepend({
        id: 'approval-pending',
        kind: 'approval',
        title: count === 1 ? 'Pending approval' : ('Pending approvals (' + count + ')'),
        detail: 'A sensitive tool call needs your review.',
        href: '#approvals',
        severity: 'warn',
        ts: Date.now()
      });
    },
    syncBudgetStatus(b) {
      if (!b) return;
      if (Date.now() < (this.budgetSnoozeUntil || 0)) return;
      var thr = (typeof b.alert_threshold === 'number' && !isNaN(b.alert_threshold)) ? b.alert_threshold : 0.85;
      var rows = [
        { label: 'Hourly', pct: Number(b.hourly_pct) || 0, lim: Number(b.hourly_limit) || 0, sp: Number(b.hourly_spend) || 0 },
        { label: 'Daily', pct: Number(b.daily_pct) || 0, lim: Number(b.daily_limit) || 0, sp: Number(b.daily_spend) || 0 },
        { label: 'Monthly', pct: Number(b.monthly_pct) || 0, lim: Number(b.monthly_limit) || 0, sp: Number(b.monthly_spend) || 0 }
      ];
      var worst = null;
      for (var i = 0; i < rows.length; i++) {
        var r = rows[i];
        if (r.lim > 0 && r.pct >= thr && (!worst || r.pct > worst.pct)) worst = r;
      }
      if (!worst) {
        this.items = (this.items || []).filter(function(x) { return x.id !== 'budget-alert'; });
        return;
      }
      var pctStr = (worst.pct * 100).toFixed(0);
      this._prepend({
        id: 'budget-alert',
        kind: 'budget',
        title: 'Budget: ' + worst.label + ' at ' + pctStr + '%',
        detail: 'Spend $' + worst.sp.toFixed(2) + ' of $' + worst.lim.toFixed(2) + ' (alert at ' + (thr * 100).toFixed(0) + '%).',
        href: '#settings',
        severity: worst.pct >= 0.98 ? 'error' : 'warn',
        ts: Date.now()
      });
    },
    ingestKernelEvent(j) {
      if (!j || !j.payload) return;
      var p = j.payload;
      var kid = 'k-' + (j.id != null ? String(j.id) : String(Date.now()));
      if (armaraosNotifyIsKernelDismissed(kid)) return;
      try {
        if (p.type === 'Lifecycle' && p.data && p.data.event === 'Crashed') {
          var err = (p.data.error || '').slice(0, 280);
          var aid = (p.data.agent_id != null) ? String(p.data.agent_id) : '';
          this._prepend({
            id: kid,
            kind: 'agent_error',
            title: 'Agent crashed',
            detail: (err || aid || 'See logs for details.').slice(0, 320),
            href: '#agents',
            severity: 'error',
            ts: Date.now()
          });
          return;
        }
        if (p.type === 'System' && p.data) {
          var d = p.data;
          if (d.event === 'QuotaEnforced') {
            this._prepend({
              id: kid,
              kind: 'budget',
              title: 'Quota enforced',
              detail: 'Spent $' + (Number(d.spent) || 0).toFixed(3) + ' / $' + (Number(d.limit) || 0).toFixed(3) + ' (agent ' + String(d.agent_id || '') + ').',
              href: '#settings',
              severity: 'warn',
              ts: Date.now()
            });
            return;
          }
          if (d.event === 'QuotaWarning') {
            this._prepend({
              id: kid,
              kind: 'budget',
              title: 'Quota warning',
              detail: (d.resource || 'resource') + ' at ' + (d.usage_percent != null ? Number(d.usage_percent).toFixed(0) : '?') + '% (agent ' + String(d.agent_id || '') + ').',
              href: '#analytics',
              severity: 'warn',
              ts: Date.now()
            });
            return;
          }
          if (d.event === 'HealthCheckFailed') {
            var aid2 = String((d.agent_id != null) ? d.agent_id : 'unknown');
            var nowMs = Date.now();
            var prev = this._healthNotifyAt[aid2] || 0;
            if (nowMs - prev < 90000) return;
            this._healthNotifyAt[aid2] = nowMs;
            var u2 = d.unresponsive_secs;
            var det = (typeof u2 === 'number') ? ('Unresponsive for ~' + u2 + 's') : 'Agent unresponsive';
            this._prepend({
              id: kid,
              kind: 'agent_error',
              title: 'Health check failed',
              detail: det,
              href: '#agents',
              severity: 'warn',
              ts: Date.now()
            });
            return;
          }
          if (d.event === 'CronJobFailed') {
            var nm = d.job_name || 'Scheduled job';
            var er = (d.error || '').slice(0, 220);
            this._prepend({
              id: kid,
              kind: 'agent_error',
              title: nm + ' failed',
              detail: er,
              href: '#scheduler',
              severity: 'error',
              ts: Date.now()
            });
          }
        }
      } catch (e) { /* ignore */ }
    }
  });

  /** Subscribe to GET /api/events/stream (kernel bus). Started from app().init. */
  window.ArmaraosKernelSse = (function() {
    var _es = null;
    /** Debounce repeated HealthCheckFailed per agent (recovery loops, SSE reconnects). */
    var _healthFailToastAt = {};
    function maybeToast(ev) {
      if (typeof OpenFangToast === 'undefined' || !ev || !ev.payload) return;
      // Always show in-app toasts for kernel SSE events. Native OS notifications
      // (desktop) are best-effort and often blocked by OS permissions / DND; relying
      // on them alone left users with no feedback.
      var p = ev.payload;
      if (p.type === 'Lifecycle' && p.data && p.data.event === 'Crashed') {
        var err = (p.data.error || '').slice(0, 220);
        OpenFangToast.error('Agent crashed: ' + err, 8000);
      } else if (p.type === 'System' && p.data && p.data.event === 'KernelStopping') {
        OpenFangToast.warn('Kernel stopping…', 5000);
      } else if (p.type === 'System' && p.data && p.data.event === 'QuotaEnforced') {
        OpenFangToast.warn('Quota enforced for an agent', 6000);
      } else if (p.type === 'System' && p.data && p.data.event === 'HealthCheckFailed') {
        var aid = String((p.data.agent_id != null) ? p.data.agent_id : 'unknown');
        var nowMs = Date.now();
        var prev = _healthFailToastAt[aid] || 0;
        if (nowMs - prev < 90000) {
          return;
        }
        _healthFailToastAt[aid] = nowMs;
        var uSecs = p.data.unresponsive_secs;
        var detail = (typeof uSecs === 'number') ? (' (~' + uSecs + 's since activity)') : '';
        OpenFangToast.warn('Agent health check failed' + detail, 7000);
      } else if (p.type === 'System' && p.data && p.data.event === 'CronJobCompleted') {
        var name = p.data.job_name || 'Scheduled job';
        var out = (p.data.output_preview || '').slice(0, 180);
        OpenFangToast.info(name + ': ' + out, 7000);
      } else if (p.type === 'System' && p.data && p.data.event === 'CronJobFailed') {
        var name2 = p.data.job_name || 'Scheduled job';
        var err2 = (p.data.error || '').slice(0, 180);
        OpenFangToast.error(name2 + ' failed: ' + err2, 8000);
      } else if (p.type === 'System' && p.data && p.data.event === 'ApprovalPending') {
        try { Alpine.store('app').refreshApprovals(); } catch (eAp) { /* ignore */ }
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
            try {
              Alpine.store('notifyCenter').ingestKernelEvent(j);
            } catch (eN) { /* ignore */ }
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
    isDesktopShell() {
      return isArmaraosDesktopShell();
    },
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
      var validPages = ['overview','agents','bookmarks','sessions','approvals','comms','network','workflows','scheduler','channels','skills','hands','ainl-library','home-files','analytics','logs','timeline','runtime','settings','wizard'];
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
        'approval': 'approvals',
        'app-store': 'ainl-library'
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
          if (query && pagePart === 'home-files') {
            try {
              var paramsHf = new URLSearchParams(query);
              var hfPath = paramsHf.get('path');
              if (hfPath) {
                sessionStorage.setItem('armaraos-home-prefill-path', hfPath);
              }
            } catch (eHf1) { /* ignore */ }
            try {
              if (window.history && window.history.replaceState) {
                var uHf = new URL(window.location.href);
                uHf.hash = 'home-files';
                window.history.replaceState({}, '', uHf.pathname + uHf.search + uHf.hash);
              }
            } catch (eHf2) { /* ignore */ }
          }
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
          try {
            var dash = Alpine.store('app');
            dash.dashboardPage = pagePart;
            if (pagePart !== 'agents') dash.agentsPageChatAgentId = null;
          } catch (e1) { /* ignore */ }
        }
      }
      window.addEventListener('hashchange', handleHash);
      handleHash();

      // Keyboard shortcuts
      document.addEventListener('keydown', function(e) {
        // Ctrl+K — open command palette
        if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
          e.preventDefault();
          document.dispatchEvent(new CustomEvent('open-command-palette'));
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
        // Escape — close mobile menu + notification panel
        if (e.key === 'Escape') {
          self.mobileMenuOpen = false;
          try {
            Alpine.store('notifyCenter').close();
          } catch (eN) { /* ignore */ }
        }
      });

      // Connection state listener
      OpenFangAPI.onConnectionChange(function(state) {
        Alpine.store('app').connectionState = state;
      });

      // Initial data load
      this.pollStatus();
      Alpine.store('app').refreshApprovals();
      OpenFangAPI.get('/api/budget')
        .then(function(b) {
          try {
            Alpine.store('notifyCenter').syncBudgetStatus(b);
          } catch (eB) { /* ignore */ }
        })
        .catch(function() { /* ignore */ });
      Alpine.store('app').checkOnboarding();
      Alpine.store('app').checkAuth();
      Alpine.store('app').loadUiPrefs();
      setInterval(function() {
        self.pollStatus();
        Alpine.store('app').refreshApprovals();
        OpenFangAPI.get('/api/budget')
          .then(function(b) {
            try {
              Alpine.store('notifyCenter').syncBudgetStatus(b);
            } catch (eB) { /* ignore */ }
          })
          .catch(function() { /* ignore */ });
      }, 5000);

      setInterval(function() {
        try { Alpine.store('app').runSessionDigestPollRound(); } catch (eDig) { /* ignore */ }
      }, 24000);

      if (typeof window.ArmaraosKernelSse !== 'undefined' && window.ArmaraosKernelSse.start) {
        window.ArmaraosKernelSse.start();
      }

      if (!window.__armaraosChatUnreadVisibilityBound) {
        window.__armaraosChatUnreadVisibilityBound = true;
        document.addEventListener('visibilitychange', function() {
          if (document.visibilityState !== 'visible') return;
          try {
            var st = Alpine.store('app');
            var id = st.agentsPageChatAgentId;
            if (id && st.isChatSurfaceActiveForAgent(id)) st.clearAgentChatUnread(id);
          } catch (e) { /* ignore */ }
        });
      }

      /** Unread badges while away from chat: WS stays connected after leaving #agents; frames still arrive here. */
      if (!window.__armaraosWsUnreadBound) {
        window.__armaraosWsUnreadBound = true;
        window.addEventListener('armaraos-agent-ws', function(ev) {
          var d = ev && ev.detail;
          if (!d || !d.data || !d.agentId) return;
          var t = d.data.type;
          if (t !== 'response' && t !== 'canvas') return;
          try {
            Alpine.store('app').bumpAgentChatUnread(d.agentId);
            armaraosScheduleDigestBaselineSync(d.agentId);
          } catch (e2) { /* ignore */ }
        });
      }

      if (!window.__armaraosKernelActivityBound) {
        window.__armaraosKernelActivityBound = true;
        window.addEventListener('armaraos-kernel-event', function(ev) {
          try {
            var j = ev.detail;
            if (!j || !j.payload) return;
            var p = j.payload;
            var src = String(j.source || '');
            var app = Alpine.store('app');
            if (p.type === 'System' && p.data && p.data.event === 'AgentActivity') {
              var line = armaraosAgentActivityLineFromSystemPayload(p);
              if (line && src) app.setAgentActivityLine(src, line);
            }
            if (p.type === 'Message' && p.data && p.data.role === 'agent') {
              var toId = armaraosEventTargetAgentId(j.target);
              var preview = (p.data.content || '').slice(0, 120);
              var toName = armaraosAgentDisplayName(toId);
              if (src) app.setAgentActivityLine(src, '\u2192 ' + toName + ': ' + preview);
              // Recipient agent gets an unread badge (inter-agent, etc.) when not viewing that chat.
              if (toId) {
                app.bumpAgentChatUnread(toId);
                armaraosScheduleDigestBaselineSync(toId);
              }
            }
          } catch (e) { /* ignore */ }
        });
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
                armaraosRefreshAinlSidebarBadge(Alpine.store('app').connected);
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
              try {
                armaraosRefreshAinlSidebarBadge(Alpine.store('app').connected);
              } catch (e0) { /* ignore */ }
              if (attempts < maxAttempts) setTimeout(tick, delayMs);
              else {
                try {
                  Alpine.store('ainl').bootstrapping = false;
                  armaraosRefreshAinlSidebarBadge(Alpine.store('app').connected);
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
      try {
        var dash = Alpine.store('app');
        dash.dashboardPage = p;
        if (p !== 'agents') dash.agentsPageChatAgentId = null;
      } catch (e) { /* ignore */ }
      window.location.hash = p;
      this.mobileMenuOpen = false;
    },

    /** Get started nav: re-click while on overview reveals Setup Wizard for onboarded users. */
    navigateOverview() {
      if (this.page === 'overview') {
        window.dispatchEvent(new CustomEvent('openfang-overview-nav-same-page'));
      } else {
        this.navigate('overview');
      }
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
      var wsLive = OpenFangAPI.isWsConnected();
      this.wsConnected = wsLive;
      try { store.wsConnected = wsLive; } catch (eWs) { /* ignore */ }
      await armaraosRefreshAinlSidebarBadge(store.connected);
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
