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

/** Curated AINL cron monitors (health/budget); success is intentionally quiet. See `cron_success_suppresses_session_append` in the kernel. */
function armaraosRoutineMonitorCronJobName(name) {
  if (!name) return false;
  var n = String(name);
  return (
    n === 'armaraos-agent-health-monitor' ||
    n === 'armaraos-system-health-monitor' ||
    n === 'armaraos-daily-budget-digest' ||
    n === 'armaraos-budget-threshold-alert' ||
    n === 'armaraos-ainl-health-weekly'
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

/** Opens https URLs (Terms, Privacy, etc.); uses Tauri `open_external_url` in desktop shell. */
function armaraosOpenExternalUrl(ev, url) {
  if (!url) return;
  try {
    if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
  } catch (e0) { /* ignore */ }
  try {
    var w = typeof window !== 'undefined' ? window : null;
    var core = w && w.__TAURI__ && w.__TAURI__.core;
    if (core && typeof core.invoke === 'function') {
      core.invoke('open_external_url', { url: String(url) }).catch(function() {
        try {
          w.open(String(url), '_blank', 'noopener,noreferrer');
        } catch (e1) { /* ignore */ }
      });
      return;
    }
  } catch (e2) { /* ignore */ }
  try {
    if (typeof window !== 'undefined') window.open(String(url), '_blank', 'noopener,noreferrer');
  } catch (e3) { /* ignore */ }
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

var NOTIFY_RELEASE_DISMISS_KEY = 'armaraos-notify-dismissed-release-tag';

function armaraosNotifyReleaseDismissedTag() {
  try {
    return String(localStorage.getItem(NOTIFY_RELEASE_DISMISS_KEY) || '');
  } catch (e) {
    return '';
  }
}

function armaraosNotifyPersistReleaseDismiss(tag) {
  if (!tag) return;
  try {
    localStorage.setItem(NOTIFY_RELEASE_DISMISS_KEY, String(tag));
  } catch (e) { /* ignore */ }
}

/** Dismissed PyPI `ainativelang` version for the “AINL update on PyPI” bell row — `armaraos-notify-dismissed-ainl-pypi`. */
var NOTIFY_AINL_PYPI_DISMISS_KEY = 'armaraos-notify-dismissed-ainl-pypi';

function armaraosNotifyAinlDismissedVersion() {
  try {
    return String(localStorage.getItem(NOTIFY_AINL_PYPI_DISMISS_KEY) || '');
  } catch (e) {
    return '';
  }
}

function armaraosNotifyPersistAinlDismiss(version) {
  if (!version) return;
  try {
    localStorage.setItem(NOTIFY_AINL_PYPI_DISMISS_KEY, String(version));
  } catch (e) { /* ignore */ }
}

/** Latest blog posts from ainativelang.com (JSON); polled by the notification center. */
var ARMARAOS_BLOG_FEED_URL = 'https://ainativelang.com/blog/feed.json';
var NOTIFY_BLOG_SEEDED_KEY = 'armaraos-blog-notify-seeded-v1';
var NOTIFY_BLOG_IGNORED_SLUGS_KEY = 'armaraos-blog-notify-ignored-slugs';
var NOTIFY_BLOG_DISMISSED_SLUGS_KEY = 'armaraos-notify-dismissed-blog-slugs';
var NOTIFY_BLOG_DISMISS_MAX = 200;
var NOTIFY_BLOG_BASELINE_COUNT = 3;

function armaraosNotifyBlogLoadIgnoredSet() {
  try {
    var raw = localStorage.getItem(NOTIFY_BLOG_IGNORED_SLUGS_KEY);
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

function armaraosNotifyBlogLoadDismissedSet() {
  try {
    var raw = localStorage.getItem(NOTIFY_BLOG_DISMISSED_SLUGS_KEY);
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

function armaraosNotifyPersistBlogDismiss(slug) {
  if (!slug) return;
  try {
    var raw = localStorage.getItem(NOTIFY_BLOG_DISMISSED_SLUGS_KEY);
    var arr = raw ? JSON.parse(raw) : [];
    if (!Array.isArray(arr)) arr = [];
    var s = String(slug);
    if (arr.indexOf(s) < 0) arr.push(s);
    if (arr.length > NOTIFY_BLOG_DISMISS_MAX) arr = arr.slice(-NOTIFY_BLOG_DISMISS_MAX);
    localStorage.setItem(NOTIFY_BLOG_DISMISSED_SLUGS_KEY, JSON.stringify(arr));
  } catch (e) { /* ignore */ }
}

/** First successful fetch: ignore archive beyond the newest `NOTIFY_BLOG_BASELINE_COUNT` posts. */
function armaraosNotifyBlogMaybeSeed(posts) {
  if (!Array.isArray(posts) || posts.length === 0) return;
  try {
    if (localStorage.getItem(NOTIFY_BLOG_SEEDED_KEY) === '1') return;
    var beyond = posts.slice(NOTIFY_BLOG_BASELINE_COUNT);
    var ignored = [];
    for (var i = 0; i < beyond.length; i++) {
      var sl = beyond[i] && beyond[i].slug != null ? String(beyond[i].slug).trim() : '';
      if (sl) ignored.push(sl);
    }
    localStorage.setItem(NOTIFY_BLOG_IGNORED_SLUGS_KEY, JSON.stringify(ignored));
    localStorage.setItem(NOTIFY_BLOG_SEEDED_KEY, '1');
  } catch (e) { /* ignore */ }
}

function armaraosSemverTuple(s) {
  if (!s) return null;
  var t = String(s).trim();
  if (t.charAt(0) === 'v' || t.charAt(0) === 'V') t = t.slice(1);
  var core = t.split(/[-+]/)[0];
  var parts = core.split('.');
  var out = [];
  for (var i = 0; i < 3; i++) {
    var n = parseInt(parts[i], 10);
    out.push(isNaN(n) ? 0 : n);
  }
  return out;
}

/** True when `a` is strictly newer than `b` (core x.y.z only; ignores pre-release suffix on tag). */
function armaraosSemverGreater(a, b) {
  var ta = armaraosSemverTuple(a);
  var tb = armaraosSemverTuple(b);
  if (!ta || !tb) return false;
  for (var i = 0; i < 3; i++) {
    if (ta[i] > tb[i]) return true;
    if (ta[i] < tb[i]) return false;
  }
  return false;
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
    /** Latest `GET /api/system/daemon-resources` payload for the sidebar CPU/MEM badges. */
    daemonResources: null,
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
    /** Per-agent eco mode map (`agentId -> off|balanced|aggressive`), persisted in ui-prefs.json. */
    ecoModesByAgent: (function() { try { return JSON.parse(localStorage.getItem('armaraos-eco-modes-v1') || '{}'); } catch(e) { return {}; } })(),
    /**
     * Bell rows for kernel **AgentAssistantReply**: `all` (always), `hidden` (skip while that agent’s chat is open + visible), `off`.
     * Persisted in ui-prefs.json as **notify_chat_replies** (+ localStorage for instant UI).
     */
    notifyChatReplies: (function() {
      try {
        var m = localStorage.getItem('armaraos-notify-chat-replies');
        if (m === 'off' || m === 'hidden' || m === 'all') return m;
      } catch (e) { /* ignore */ }
      return 'all';
    })(),
    /** Last full ui-prefs payload loaded from disk (preserves unknown keys on save). */
    _uiPrefsRaw: {},
    _uiPrefsLoaded: false,
    normalizeEcoMode(mode) {
      return (mode === 'off' || mode === 'balanced' || mode === 'aggressive') ? mode : 'off';
    },
    normalizeNotifyChatReplies(mode) {
      var m = mode != null ? String(mode) : '';
      return (m === 'off' || m === 'hidden' || m === 'all') ? m : 'all';
    },
    setNotifyChatReplies(mode) {
      var next = this.normalizeNotifyChatReplies(mode);
      if (next === this.notifyChatReplies) return;
      this.notifyChatReplies = next;
      try { localStorage.setItem('armaraos-notify-chat-replies', next); } catch (e) { /* ignore */ }
      this._saveUiPrefs();
    },

    getAgentEcoMode(agentId, fallbackMode) {
      var fallback = this.normalizeEcoMode(fallbackMode || 'off');
      if (agentId == null || agentId === '') return fallback;
      var id = String(agentId);
      var map = this.ecoModesByAgent || {};
      return this.normalizeEcoMode(map[id] || fallback);
    },

    setAgentEcoMode(agentId, mode) {
      if (agentId == null || agentId === '') return;
      var id = String(agentId);
      var nextMode = this.normalizeEcoMode(mode);
      var prev = this.ecoModesByAgent || {};
      var next = Object.assign({}, prev);
      next[id] = nextMode;
      this.ecoModesByAgent = next;
      try { localStorage.setItem('armaraos-eco-modes-v1', JSON.stringify(next)); } catch(e) { /* ignore */ }
      this._saveUiPrefs();
    },

    /** When inline chat is open on #agents, the agent id being viewed (null = picker). */
    agentsPageChatAgentId: null,
    /** agentId -> count of unread assistant-side updates (replaced immutably for Alpine). */
    chatUnreadCounts: {},
    /** agentId -> optional detail line for the notification center (e.g. kernel message preview). */
    chatUnreadPreview: {},
    /** agentId -> last unread bump timestamp (ms epoch), used for auto-decay. */
    chatUnreadLastTs: {},
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

    setChatUnreadPreview(agentId, text) {
      if (!agentId) return;
      var id = String(agentId);
      var t = text != null ? String(text).trim().slice(0, 300) : '';
      var p = Object.assign({}, this.chatUnreadPreview || {});
      if (!t) {
        if (!p[id]) return;
        delete p[id];
      } else {
        p[id] = t;
      }
      this.chatUnreadPreview = p;
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

    bumpAgentChatUnread(agentId, messagePreview) {
      if (!agentId || this.isChatSurfaceActiveForAgent(agentId)) return;
      if (messagePreview != null && String(messagePreview).trim()) {
        this.setChatUnreadPreview(String(agentId), String(messagePreview).trim().slice(0, 300));
      }
      var prev = this.chatUnreadCounts || {};
      var next = Object.assign({}, prev);
      var id0 = String(agentId);
      next[id0] = (next[id0] || 0) + 1;
      this.chatUnreadCounts = next;
      var tsm = Object.assign({}, this.chatUnreadLastTs || {});
      tsm[id0] = Date.now();
      this.chatUnreadLastTs = tsm;
      this.updateTabTitle();
      try {
        var nctr = Alpine.store('notifyCenter');
        nctr.syncChatUnreadRows();
        if (nctr && !nctr.panelOpen) {
          nctr._announceIfClosed(armaraosAgentDisplayName(id0) + ' — new message');
        }
      } catch (e) { /* ignore */ }
    },

    clearAgentChatUnread(agentId) {
      if (!agentId) return;
      var id0 = String(agentId);
      var pprev = this.chatUnreadPreview || {};
      if (pprev[id0] != null) {
        var pp2 = Object.assign({}, pprev);
        delete pp2[id0];
        this.chatUnreadPreview = pp2;
      }
      var prev = this.chatUnreadCounts || {};
      var hadCount = (prev[id0] || 0) > 0;
      if (!hadCount) {
        try {
          Alpine.store('notifyCenter').syncChatUnreadRows();
        } catch (e) { /* ignore */ }
        return;
      }
      var next = Object.assign({}, prev);
      delete next[id0];
      this.chatUnreadCounts = next;
      var ts2 = Object.assign({}, this.chatUnreadLastTs || {});
      delete ts2[id0];
      this.chatUnreadLastTs = ts2;
      this.updateTabTitle();
      try {
        Alpine.store('notifyCenter').syncChatUnreadRows();
      } catch (e2) { /* ignore */ }
    },

    clearAllChatUnread() {
      this.chatUnreadCounts = {};
      this.chatUnreadPreview = {};
      this.chatUnreadLastTs = {};
      this.updateTabTitle();
      try {
        Alpine.store('notifyCenter').syncChatUnreadRows();
      } catch (e) { /* ignore */ }
    },

    clearFleetStatusUnread() {
      this.clearAllChatUnread();
    },

    decayChatUnreadBadges() {
      var counts = this.chatUnreadCounts || {};
      var keys = Object.keys(counts);
      if (!keys.length) return;

      // If no user-facing agents are currently running, stale fleet/title unread badges
      // should clear automatically.
      var agents = this.primaryAgentsForSidebar();
      var running = 0;
      for (var i = 0; i < agents.length; i++) {
        if (String((agents[i] && agents[i].state) || '') === 'Running') running += 1;
      }
      if (running === 0) {
        this.clearAllChatUnread();
        return;
      }

      var now = Date.now();
      var maxAgeMs = 120000;
      var last = this.chatUnreadLastTs || {};
      var nextCounts = Object.assign({}, counts);
      var nextPrev = Object.assign({}, this.chatUnreadPreview || {});
      var nextTs = Object.assign({}, last);
      var changed = false;
      for (var j = 0; j < keys.length; j++) {
        var aid = keys[j];
        var ts = Number(last[aid]) || 0;
        if (ts <= 0) {
          nextTs[aid] = now;
          changed = true;
          continue;
        }
        if ((now - ts) >= maxAgeMs && !this.isChatSurfaceActiveForAgent(aid)) {
          delete nextCounts[aid];
          delete nextPrev[aid];
          delete nextTs[aid];
          changed = true;
        }
      }
      if (!changed) return;
      this.chatUnreadCounts = nextCounts;
      this.chatUnreadPreview = nextPrev;
      this.chatUnreadLastTs = nextTs;
      this.updateTabTitle();
      try {
        Alpine.store('notifyCenter').syncChatUnreadRows();
      } catch (e3) { /* ignore */ }
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
        this._uiPrefsRaw = (prefs && typeof prefs === 'object' && !Array.isArray(prefs)) ? Object.assign({}, prefs) : {};
        this._uiPrefsLoaded = true;
        if (Array.isArray(prefs.pinned_agents)) {
          this.pinnedAgentIds = prefs.pinned_agents;
          try { localStorage.setItem('armaraos-pinned-agents', JSON.stringify(prefs.pinned_agents)); } catch(e) { /* ignore */ }
        }
        if (prefs && typeof prefs.agent_eco_modes === 'object' && prefs.agent_eco_modes !== null && !Array.isArray(prefs.agent_eco_modes)) {
          var raw = prefs.agent_eco_modes;
          var norm = {};
          Object.keys(raw).forEach(function(agentId) {
            var v = raw[agentId];
            if (typeof v === 'string') {
              var mode = (v === 'off' || v === 'balanced' || v === 'aggressive') ? v : null;
              if (mode) norm[String(agentId)] = mode;
            }
          });
          this.ecoModesByAgent = norm;
          try { localStorage.setItem('armaraos-eco-modes-v1', JSON.stringify(norm)); } catch(e2) { /* ignore */ }
        }
        if (prefs && prefs.notify_chat_replies != null) {
          var ncr = this.normalizeNotifyChatReplies(prefs.notify_chat_replies);
          this.notifyChatReplies = ncr;
          try { localStorage.setItem('armaraos-notify-chat-replies', ncr); } catch (e4) { /* ignore */ }
        }
        try { window.dispatchEvent(new CustomEvent('armaraos-ui-prefs-loaded')); } catch(e3) { /* ignore */ }
      } catch(e) {
        this._uiPrefsLoaded = false;
        /* keep localStorage-seeded values on failure */
      }
    },

    /** Persist current UI prefs to the server (fire-and-forget). */
    _saveUiPrefs() {
      var base = (this._uiPrefsRaw && typeof this._uiPrefsRaw === 'object' && !Array.isArray(this._uiPrefsRaw))
        ? Object.assign({}, this._uiPrefsRaw)
        : {};
      var prefs = Object.assign(base, {
        pinned_agents: this.pinnedAgentIds || [],
        agent_eco_modes: this.ecoModesByAgent || {},
        notify_chat_replies: this.normalizeNotifyChatReplies(this.notifyChatReplies)
      });
      this._uiPrefsRaw = Object.assign({}, prefs);
      var self = this;
      if (!this._uiPrefsLoaded) {
        OpenFangAPI.get('/api/ui-prefs')
          .then(function(remote) {
            var rb = (remote && typeof remote === 'object' && !Array.isArray(remote)) ? remote : {};
            var merged = Object.assign({}, rb, prefs);
            self._uiPrefsRaw = Object.assign({}, merged);
            self._uiPrefsLoaded = true;
            return OpenFangAPI.put('/api/ui-prefs', merged);
          })
          .catch(function() {
            return OpenFangAPI.put('/api/ui-prefs', prefs).catch(function() { /* ignore */ });
          });
        return;
      }
      OpenFangAPI.put('/api/ui-prefs', prefs).catch(function() { /* ignore */ });
    },

    /** Merge a partial object into ui-prefs and persist, preserving all unrelated keys. */
    saveUiPrefsPatch(patch) {
      if (!patch || typeof patch !== 'object' || Array.isArray(patch)) return;
      var base = (this._uiPrefsRaw && typeof this._uiPrefsRaw === 'object' && !Array.isArray(this._uiPrefsRaw))
        ? Object.assign({}, this._uiPrefsRaw)
        : {};
      var next = Object.assign(base, patch);
      this._uiPrefsRaw = Object.assign({}, next);
      var self = this;
      if (!this._uiPrefsLoaded) {
        OpenFangAPI.get('/api/ui-prefs')
          .then(function(remote) {
            var rb = (remote && typeof remote === 'object' && !Array.isArray(remote)) ? remote : {};
            var merged = Object.assign({}, rb, patch);
            self._uiPrefsRaw = Object.assign({}, merged);
            self._uiPrefsLoaded = true;
            return OpenFangAPI.put('/api/ui-prefs', merged);
          })
          .catch(function() {
            return OpenFangAPI.put('/api/ui-prefs', next).catch(function() { /* ignore */ });
          });
        return;
      }
      OpenFangAPI.put('/api/ui-prefs', next).catch(function() { /* ignore */ });
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

    _formatDaemonBytes(bytes) {
      var u = typeof bytes === 'number' && !isNaN(bytes) ? bytes : 0;
      if (u <= 0) return '0 B';
      if (u < 1024) return u + ' B';
      if (u < 1024 * 1024) return (u / 1024).toFixed(1) + ' KB';
      if (u < 1024 * 1024 * 1024) return (u / (1024 * 1024)).toFixed(1) + ' MB';
      return (u / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
    },

    daemonResourcesCpuText() {
      var dr = this.daemonResources;
      if (!dr) return '\u2026';
      if (dr.supported === false) return 'n/a';
      var v = typeof dr.cpu_percent === 'number' ? dr.cpu_percent : 0;
      return (Math.round(v * 10) / 10).toFixed(1) + '%';
    },

    daemonResourcesCpuBar() {
      var dr = this.daemonResources;
      if (!dr || dr.supported === false) return 0;
      var v = typeof dr.cpu_percent === 'number' ? dr.cpu_percent : 0;
      return Math.max(0, Math.min(100, v));
    },

    daemonResourcesMemText() {
      var dr = this.daemonResources;
      if (!dr) return '\u2026';
      if (dr.supported === false) return 'n/a';
      var v = typeof dr.memory_percent === 'number' ? dr.memory_percent : 0;
      return (Math.round(v * 10) / 10).toFixed(1) + '%';
    },

    daemonResourcesMemBar() {
      var dr = this.daemonResources;
      if (!dr || dr.supported === false) return 0;
      var v = typeof dr.memory_percent === 'number' ? dr.memory_percent : 0;
      return Math.max(0, Math.min(100, v));
    },

    daemonResourcesTooltipCpu() {
      var dr = this.daemonResources;
      if (!dr) return 'Daemon CPU usage';
      if (dr.supported === false) {
        return 'CPU usage is not available on this platform build.';
      }
      var pct = (typeof dr.cpu_percent === 'number' ? dr.cpu_percent : 0).toFixed(1);
      var cores = (typeof dr.cpu_cores_equivalent === 'number' ? dr.cpu_cores_equivalent : 0).toFixed(2);
      var n = dr.logical_cpus != null ? String(dr.logical_cpus) : '?';
      return 'Daemon CPU (average): ' + pct + '% of total CPU (' + cores + ' core-eq, ' + n + ' logical CPUs).';
    },

    daemonResourcesTooltipMem() {
      var dr = this.daemonResources;
      if (!dr) return 'Daemon memory usage';
      if (dr.supported === false) {
        return 'Memory usage is not available on this platform build.';
      }
      var rss = this._formatDaemonBytes(dr.memory_rss_bytes || 0);
      var tot = this._formatDaemonBytes(dr.memory_total_bytes || 0);
      var pct = (typeof dr.memory_percent === 'number' ? dr.memory_percent : 0).toFixed(1);
      return 'Daemon RSS: ' + rss + ' / ' + tot + ' RAM (' + pct + '%).';
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
        var self = this;
        OpenFangAPI.get('/api/system/daemon-resources')
          .then(function(dr) {
            self.daemonResources = dr;
          })
          .catch(function() { /* ignore */ });
      } catch(e) {
        this.connected = false;
        this.daemonResources = null;
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
    /**
     * Drive notification-center rows from the same `chatUnreadCounts` as the sidebar,
     * Fleet header badge, and All Agents nav badge. Removes legacy per-agent
     * `agent-reply-*` rows (replaced by unified `chat-unread-*`).
     */
    syncChatUnreadRows() {
      var app;
      try {
        app = Alpine.store('app');
      } catch (e) {
        return;
      }
      if (!app) return;
      var counts = app.chatUnreadCounts || {};
      var previews = app.chatUnreadPreview || {};
      var raw = this.items || [];
      var base = raw.filter(function(x) {
        if (!x) return true;
        if (x.kind === 'chat_unread') return false;
        if (x.id && String(x.id).indexOf('chat-unread-') === 0) return false;
        if (x.id && String(x.id).indexOf('agent-reply-') === 0) return false;
        return true;
      });
      var agentIds = [];
      for (var k in counts) {
        if (!Object.prototype.hasOwnProperty.call(counts, k)) continue;
        if ((counts[k] || 0) > 0) agentIds.push(String(k));
      }
      agentIds.sort(function(a, b) {
        return armaraosAgentDisplayName(a)
          .toLowerCase()
          .localeCompare(armaraosAgentDisplayName(b).toLowerCase());
      });
      var chatRows = [];
      for (var i = 0; i < agentIds.length; i++) {
        var ag = agentIds[i];
        var c = counts[ag] || 0;
        if (c <= 0) continue;
        var label = armaraosAgentDisplayName(ag);
        var title = c > 1 ? label + ' · ' + c + ' new' : label + ' — new message';
        var det = (previews[ag] && String(previews[ag]).trim())
          ? String(previews[ag]).trim().slice(0, 300)
          : 'Open All Agents to read the reply.';
        chatRows.push({
          id: 'chat-unread-' + ag,
          kind: 'chat_unread',
          title: title,
          detail: det,
          href: '#agents',
          severity: 'info',
          ts: Date.now()
        });
      }
      var merged = chatRows.concat(base);
      if (merged.length > 48) merged.length = 48;
      this.items = merged;
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
      if (String(id).indexOf('chat-unread-') === 0) {
        var aid0 = id.slice('chat-unread-'.length);
        if (aid0) {
          try {
            Alpine.store('app').clearAgentChatUnread(aid0);
          } catch (eChat) { /* ignore */ }
        }
        return;
      }
      if (String(id).indexOf('k-') === 0) {
        armaraosNotifyPersistKernelDismiss(id);
      }
      if (String(id).indexOf('release-') === 0) {
        armaraosNotifyPersistReleaseDismiss(id.slice('release-'.length));
      }
      if (String(id).indexOf('ainl-pypi-') === 0) {
        armaraosNotifyPersistAinlDismiss(id.slice('ainl-pypi-'.length));
      }
      if (String(id).indexOf('blog-') === 0) {
        armaraosNotifyPersistBlogDismiss(id.slice('blog-'.length));
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
      try {
        var a = Alpine.store('app');
        a.chatUnreadCounts = {};
        a.chatUnreadPreview = {};
        a.updateTabTitle();
      } catch (e) { /* ignore */ }
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
    syncAppReleaseUpdate(daemonVersion, latestTag, htmlUrl) {
      if (!daemonVersion || !latestTag) return;
      var dismissed = armaraosNotifyReleaseDismissedTag();
      if (dismissed && dismissed === String(latestTag)) {
        this.items = (this.items || []).filter(function(x) {
          return !(x && x.id && String(x.id).indexOf('release-') === 0);
        });
        return;
      }
      if (!armaraosSemverGreater(latestTag, daemonVersion)) {
        this.items = (this.items || []).filter(function(x) {
          return !(x && x.id && String(x.id).indexOf('release-') === 0);
        });
        return;
      }
      this._prepend({
        id: 'release-' + String(latestTag),
        kind: 'update',
        title: 'ArmaraOS update available',
        detail:
          'This daemon is v' +
          String(daemonVersion) +
          ' · GitHub latest is ' +
          String(latestTag) +
          '.',
        href: htmlUrl || '#settings',
        severity: 'info',
        ts: Date.now()
      });
    },
    /** Host `pip show` version vs PyPI **ainativelang** latest (6h poll + System tab check). */
    syncAinlPypiUpdate(pipVersion, pypiLatest) {
      var strip = function() {
        this.items = (this.items || []).filter(function(x) {
          return !(x && x.id && String(x.id).indexOf('ainl-pypi-') === 0);
        });
      }.bind(this);
      if (!pypiLatest) {
        strip();
        return;
      }
      var pv = String(pypiLatest).trim();
      if (!pv) {
        strip();
        return;
      }
      var dismissed = armaraosNotifyAinlDismissedVersion();
      if (dismissed && dismissed === pv) {
        strip();
        return;
      }
      var installed = pipVersion != null && pipVersion !== '' ? String(pipVersion).trim() : '';
      if (!installed) {
        strip();
        return;
      }
      if (!armaraosSemverGreater(pv, installed)) {
        strip();
        return;
      }
      this._prepend({
        id: 'ainl-pypi-' + pv,
        kind: 'update',
        title: 'AINL package update on PyPI',
        detail:
          'Installed `ainativelang` ' +
          installed +
          ' · PyPI latest is ' +
          pv +
          '.',
        href: '#settings',
        severity: 'info',
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
    /**
     * Replace blog rows from `GET https://ainativelang.com/blog/feed.json` (`posts` newest-first).
     * First run seeds “archive ignore” past the newest NOTIFY_BLOG_BASELINE_COUNT posts; dismissals persist per slug.
     */
    syncBlogFeedFromJson(data) {
      var posts = data && data.posts;
      if (!Array.isArray(posts) || posts.length === 0) return;
      armaraosNotifyBlogMaybeSeed(posts);
      var ignored = armaraosNotifyBlogLoadIgnoredSet();
      var dismissed = armaraosNotifyBlogLoadDismissedSet();
      this.items = (this.items || []).filter(function(x) {
        return !(x && (x.kind === 'blog' || (x.id && String(x.id).indexOf('blog-') === 0)));
      });
      var eligible = [];
      for (var i = 0; i < posts.length; i++) {
        var p = posts[i];
        if (!p || p.slug == null) continue;
        var slug = String(p.slug).trim();
        if (!slug) continue;
        if (ignored[slug]) continue;
        if (dismissed[slug]) continue;
        var title = p.title != null ? String(p.title).trim() : '';
        if (!title) title = 'Blog update';
        var pubTitle = 'Latest News & Updates · ' + title;
        var detail = '';
        if (p.description != null && String(p.description).trim()) {
          detail = String(p.description).trim().slice(0, 280);
        }
        var url = p.url != null ? String(p.url).trim() : '';
        if (!url) {
          url = 'https://ainativelang.com/blog/' + encodeURIComponent(slug);
        }
        var ts = Date.now();
        if (p.date) {
          var parsed = Date.parse(String(p.date));
          if (!isNaN(parsed)) ts = parsed;
        }
        eligible.push({ slug: slug, title: pubTitle, detail: detail, url: url, ts: ts });
      }
      for (var j = eligible.length - 1; j >= 0; j--) {
        var e = eligible[j];
        this._prepend({
          id: 'blog-' + e.slug,
          kind: 'blog',
          title: e.title,
          detail: e.detail,
          href: e.url,
          severity: 'info',
          ts: e.ts
        });
      }
    },
    /** Notification row primary action: external https in browser / desktop shell; hash routes in-dashboard. */
    openRow(ev, row) {
      if (!row || !row.href) return;
      var h = String(row.href).trim();
      if (!h) return;
      if (h.indexOf('http://') === 0 || h.indexOf('https://') === 0) {
        try {
          if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
        } catch (e0) { /* ignore */ }
        armaraosOpenExternalUrl(ev, h);
        this.close();
        return;
      }
      try {
        if (ev && typeof ev.preventDefault === 'function') ev.preventDefault();
      } catch (e1) { /* ignore */ }
      if (h.charAt(0) === '#') {
        try {
          window.location.hash = h.slice(1);
        } catch (e2) { /* ignore */ }
      }
      this.close();
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
          if (d.event === 'WorkflowRunFinished') {
            var wfn = d.workflow_name || 'Workflow';
            var okwf = !!d.ok;
            this._prepend({
              id: kid,
              kind: 'workflow',
              title: okwf ? ('Workflow finished: ' + wfn) : ('Workflow failed: ' + wfn),
              detail: String(d.summary || '').slice(0, 320),
              href: '#workflows',
              severity: okwf ? 'info' : 'error',
              ts: Date.now()
            });
            return;
          }
          if (d.event === 'AgentAssistantReply') {
            var aidAr2 = String(d.agent_id || '');
            var replyMode2 = 'all';
            try {
              replyMode2 = Alpine.store('app').normalizeNotifyChatReplies(Alpine.store('app').notifyChatReplies);
            } catch (eRm) { replyMode2 = 'all'; }
            if (replyMode2 === 'off') {
              return;
            }
            if (replyMode2 === 'hidden' && aidAr2) {
              try {
                if (Alpine.store('app').isChatSurfaceActiveForAgent(aidAr2)) return;
              } catch (eHid) { /* ignore */ }
            }
            if (aidAr2) {
              var pr2 = (d.message_preview || '').slice(0, 300);
              try {
                Alpine.store('app').bumpAgentChatUnread(aidAr2, pr2);
              } catch (eBump) { /* ignore */ }
            }
            return;
          }
          if (d.event === 'CronJobCompleted') {
            if (armaraosRoutineMonitorCronJobName(d.job_name)) {
              return;
            }
            var nmOk = d.job_name || 'Scheduled job';
            var outOk = (d.output_preview || '').slice(0, 220);
            var kindOk = d.action_kind ? String(d.action_kind) : '';
            var titleOk = nmOk + ' finished';
            if (kindOk === 'ainl_run') titleOk = 'AINL program finished';
            else if (kindOk === 'agent_turn') titleOk = 'Scheduled agent turn finished';
            this._prepend({
              id: kid,
              kind: 'scheduler',
              title: titleOk,
              detail: outOk,
              href: '#scheduler',
              severity: 'info',
              ts: Date.now()
            });
            return;
          }
          if (d.event === 'CronJobFailed') {
            var nm = d.job_name || 'Scheduled job';
            var er = (d.error || '').slice(0, 220);
            var kindFail = d.action_kind ? String(d.action_kind) : '';
            var titleFail = nm + ' failed';
            if (kindFail === 'ainl_run') titleFail = 'AINL program run failed';
            else if (kindFail === 'agent_turn') titleFail = 'Scheduled agent turn failed';
            else if (kindFail === 'workflow_run') titleFail = 'Scheduled workflow failed';
            this._prepend({
              id: kid,
              kind: 'agent_error',
              title: titleFail,
              detail: er,
              href: '#scheduler',
              severity: 'error',
              ts: Date.now()
            });
            return;
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
        if (armaraosRoutineMonitorCronJobName(p.data.job_name)) {
          return;
        }
        var name = p.data.job_name || 'Scheduled job';
        var out = (p.data.output_preview || '').slice(0, 180);
        OpenFangToast.info(name + ': ' + out, 7000);
      } else if (p.type === 'System' && p.data && p.data.event === 'WorkflowRunFinished') {
        var wn = p.data.workflow_name || 'Workflow';
        if (p.data.ok) {
          OpenFangToast.info('Workflow finished: ' + wn, 6000);
        } else {
          OpenFangToast.error('Workflow failed: ' + wn, 8000);
        }
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
      setTimeout(function() {
        self.hydrateSidebarTooltips();
      }, 0);

      // Listen for OS theme changes (only matters when mode is 'system')
      window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', function(e) {
        if (self.themeMode === 'system') {
          self.theme = e.matches ? 'dark' : 'light';
        }
      });

      // Hash routing
      var validPages = ['overview','agents','bookmarks','sessions','graph-memory','trajectories','approvals','comms','network','workflows','scheduler','channels','skills','hands','ainl-library','home-files','analytics','logs','timeline','runtime','orchestration-traces','settings','wizard'];
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
          try {
            if (typeof window !== 'undefined' && window.__ARMARAOS_ANALYTICS__ && window.__ARMARAOS_ANALYTICS__.nav) {
              window.__ARMARAOS_ANALYTICS__.nav(pagePart, query);
            }
          } catch (eNav) { /* ignore */ }
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
      function armaraosPollDaemonReleaseUpdate() {
        if (typeof OpenFangAPI === 'undefined' || !OpenFangAPI.get) return;
        Promise.all([
          OpenFangAPI.get('/api/version').catch(function() {
            return null;
          }),
          OpenFangAPI.get('/api/version/github-latest').catch(function() {
            return null;
          }),
          OpenFangAPI.get('/api/ainl/runtime-version').catch(function() {
            return null;
          }),
        ]).then(function(triple) {
          var v = triple[0];
          var gh = triple[1];
          var ar = triple[2];
          if (v && v.version && gh && gh.tag_name) {
            try {
              Alpine.store('notifyCenter').syncAppReleaseUpdate(v.version, gh.tag_name, gh.html_url);
            } catch (eR) { /* ignore */ }
          }
          if (ar) {
            try {
              Alpine.store('notifyCenter').syncAinlPypiUpdate(ar.pip_version, ar.pypi_latest_version);
            } catch (eA) { /* ignore */ }
          }
        });
      }
      armaraosPollDaemonReleaseUpdate();
      setInterval(armaraosPollDaemonReleaseUpdate, 6 * 60 * 60 * 1000);
      function armaraosPollBlogFeed() {
        if (typeof fetch !== 'function') return;
        fetch(ARMARAOS_BLOG_FEED_URL, { credentials: 'omit', cache: 'default' })
          .then(function(r) {
            if (!r.ok) return Promise.reject(new Error('blog feed http'));
            return r.json();
          })
          .then(function(data) {
            try {
              Alpine.store('notifyCenter').syncBlogFeedFromJson(data);
            } catch (eSync) { /* ignore */ }
          })
          .catch(function() { /* offline / blocked */ });
      }
      armaraosPollBlogFeed();
      setInterval(armaraosPollBlogFeed, 60 * 60 * 1000);
      Alpine.store('app').checkOnboarding();
      Alpine.store('app').checkAuth();
      Alpine.store('app').loadUiPrefs();
      try {
        Alpine.store('notifyCenter').syncChatUnreadRows();
      } catch (eUnr) { /* ignore */ }
      setInterval(function() {
        self.pollStatus();
        try { Alpine.store('app').decayChatUnreadBadges(); } catch (eDecay) { /* ignore */ }
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
      this.hydrateSidebarTooltips();
    },

    /** Populate nav tooltips from labels so collapsed icon rail stays discoverable. */
    hydrateSidebarTooltips() {
      try {
        var items = document.querySelectorAll('.sidebar .nav-item');
        for (var i = 0; i < items.length; i++) {
          var item = items[i];
          if (!item) continue;
          var text = String(item.getAttribute('title') || '').trim();
          if (!text) {
            var label = item.querySelector('.nav-label');
            text = label ? String(label.textContent || '').replace(/\s+/g, ' ').trim() : '';
          }
          if (!text) continue;
          item.setAttribute('data-nav-tooltip', text);
          if (!item.getAttribute('title')) item.setAttribute('title', text);
          if (!item.getAttribute('aria-label')) item.setAttribute('aria-label', text);
        }
      } catch (e) { /* ignore */ }
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
      try {
        if (typeof window !== 'undefined' && window.__ARMARAOS_ANALYTICS__ && window.__ARMARAOS_ANALYTICS__.refreshEngagement) {
          window.__ARMARAOS_ANALYTICS__.refreshEngagement();
        }
      } catch (eEng) { /* ignore */ }
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
