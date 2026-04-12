// ArmaraOS API Client — Fetch wrapper, WebSocket manager, auth injection, toast notifications
'use strict';

// ── Toast Notification System ──
var OpenFangToast = (function() {
  var _container = null;
  var _toastId = 0;

  function getContainer() {
    if (!_container) {
      _container = document.getElementById('toast-container');
      if (!_container) {
        _container = document.createElement('div');
        _container.id = 'toast-container';
        _container.className = 'toast-container';
        document.body.appendChild(_container);
      }
    }
    return _container;
  }

  function toast(message, type, duration) {
    type = type || 'info';
    duration = duration || 4000;
    var id = ++_toastId;
    var el = document.createElement('div');
    el.className = 'toast toast-' + type;
    el.setAttribute('data-toast-id', id);

    var msgSpan = document.createElement('span');
    msgSpan.className = 'toast-msg';
    msgSpan.textContent = message;
    el.appendChild(msgSpan);

    var closeBtn = document.createElement('button');
    closeBtn.className = 'toast-close';
    closeBtn.textContent = '\u00D7';
    closeBtn.onclick = function() { dismissToast(el); };
    el.appendChild(closeBtn);

    el.onclick = function(e) { if (e.target === el) dismissToast(el); };
    getContainer().appendChild(el);

    // Auto-dismiss
    if (duration > 0) {
      setTimeout(function() { dismissToast(el); }, duration);
    }
    return id;
  }

  function dismissToast(el) {
    if (!el || el.classList.contains('toast-dismiss')) return;
    el.classList.add('toast-dismiss');
    setTimeout(function() { if (el.parentNode) el.parentNode.removeChild(el); }, 300);
  }

  function success(msg, duration) { return toast(msg, 'success', duration); }
  function error(msg, duration) { return toast(msg, 'error', duration || 6000); }
  function warn(msg, duration) { return toast(msg, 'warn', duration || 5000); }
  function info(msg, duration) { return toast(msg, 'info', duration); }

  // Styled confirmation modal — replaces native confirm()
  // opts: { confirmLabel?, danger? } — danger defaults true (destructive styling).
  function confirm(title, message, onConfirm, opts) {
    opts = opts || {};
    var overlay = document.createElement('div');
    overlay.className = 'confirm-overlay';

    var modal = document.createElement('div');
    modal.className = 'confirm-modal';

    var titleEl = document.createElement('div');
    titleEl.className = 'confirm-title';
    titleEl.textContent = title;
    modal.appendChild(titleEl);

    var msgEl = document.createElement('div');
    msgEl.className = 'confirm-message';
    msgEl.textContent = message;
    modal.appendChild(msgEl);

    var actions = document.createElement('div');
    actions.className = 'confirm-actions';

    var cancelBtn = document.createElement('button');
    cancelBtn.className = 'btn btn-ghost confirm-cancel';
    cancelBtn.textContent = 'Cancel';
    actions.appendChild(cancelBtn);

    var okBtn = document.createElement('button');
    okBtn.className = opts.danger === false ? 'btn btn-primary confirm-ok' : 'btn btn-danger confirm-ok';
    okBtn.textContent = opts.confirmLabel || 'Confirm';
    actions.appendChild(okBtn);

    modal.appendChild(actions);
    overlay.appendChild(modal);

    function close() { if (overlay.parentNode) overlay.parentNode.removeChild(overlay); document.removeEventListener('keydown', onKey); }
    cancelBtn.onclick = close;
    okBtn.onclick = function() { close(); if (onConfirm) onConfirm(); };
    overlay.addEventListener('click', function(e) { if (e.target === overlay) close(); });

    function onKey(e) { if (e.key === 'Escape') close(); }
    document.addEventListener('keydown', onKey);

    document.body.appendChild(overlay);
    okBtn.focus();
  }

  /** Shown before PUT /api/agents/:id/model (kernel clears canonical session). */
  function modelProviderChangeWarningText() {
    return 'Changing the model or provider clears this agent\'s canonical session memory (unlike editing the system prompt or tool filters). Your chat transcript usually stays on screen, but model-side session continuity resets. Continue?';
  }

  return {
    toast: toast,
    success: success,
    error: error,
    warn: warn,
    info: info,
    confirm: confirm,
    modelProviderChangeWarningText: modelProviderChangeWarningText
  };
})();

// ── Friendly Error Messages ──
function errorHintForStatus(status, serverHint) {
  if (serverHint) return serverHint;
  if (status === 0 || !status) {
    return 'If you use the desktop app, wait until it finishes starting. For a remote daemon, verify host/port, VPN, and firewall. Use Copy debug info or Generate + copy bundle (sidebar when disconnected) or Settings → System Info → Support — the bundle includes recent daemon logs without SSH.';
  }
  if (status === 401) {
    return 'Open Settings → Security and set your API key, or complete dashboard login.';
  }
  if (status === 403) return 'Your account or API key does not have access to this action.';
  if (status === 404) return 'The resource may have been deleted or the URL is wrong.';
  if (status === 429) return 'Wait a few seconds and retry, or reduce request frequency.';
  if (status === 413) return 'Reduce payload size (smaller file or shorter prompt).';
  if (status === 500) {
    return 'Internal server error. Retry once; if it persists, click Generate + copy bundle (sidebar) or Settings → System Info → Support, then attach the .zip when reporting the issue.';
  }
  if (status === 502 || status === 503) {
    return 'The API may be restarting or overloaded. Confirm the daemon is running, then use Copy debug info or Generate + copy bundle if the problem continues.';
  }
  return '';
}

function friendlyError(status, serverMsg) {
  if (status === 0 || !status) return 'Cannot reach daemon';
  if (status === 401) return 'Not authorized — check your API key or login';
  if (status === 403) return 'Permission denied';
  if (status === 404) return serverMsg || 'Resource not found';
  if (status === 429) return 'Rate limited — try again shortly';
  if (status === 413) return 'Request too large';
  if (status === 500) return 'Server error — see detail and hint below';
  if (status === 502 || status === 503) return 'Daemon unavailable';
  return serverMsg || 'Unexpected error (' + status + ')';
}

function makeApiError(message, status, detail, hint, where, requestId, serverPath) {
  var e = new Error(message);
  e.name = 'OpenFangAPIError';
  e.status = status || 0;
  e.detail = detail || '';
  e.hint = hint || errorHintForStatus(status, '');
  e.where = where || '';
  e.requestId = requestId || '';
  e.serverPath = serverPath || '';
  return e;
}

// Compose a full human-readable error string from an OpenFangAPIError, including
// server detail and hint when present. Safe to call on any value.
function openFangErrText(e) {
  var msg = (e && e.message) || 'Unknown error';
  var detail = e && e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
  var hint = e && e.hint ? (' Hint: ' + e.hint) : '';
  return msg + detail + hint;
}

// ── API Client ──
var OpenFangAPI = (function() {
  var BASE = window.location.origin;
  var WS_BASE = BASE.replace(/^http/, 'ws');
  var _authToken = '';

  // Connection state tracking
  var _connectionState = 'connected';
  var _reconnectAttempt = 0;
  var _connectionListeners = [];

  function setAuthToken(token) { _authToken = token; }

  function headers() {
    var h = { 'Content-Type': 'application/json' };
    if (_authToken) h['Authorization'] = 'Bearer ' + _authToken;
    return h;
  }

  function setConnectionState(state) {
    if (_connectionState === state) return;
    _connectionState = state;
    _connectionListeners.forEach(function(fn) { fn(state); });
  }

  function onConnectionChange(fn) { _connectionListeners.push(fn); }

  function request(method, path, body) {
    var opts = { method: method, headers: headers() };
    if (body !== undefined) opts.body = JSON.stringify(body);
    return fetch(BASE + path, opts).then(function(r) {
      if (_connectionState !== 'connected') setConnectionState('connected');
      if (!r.ok) {
        // On 401, auto-show auth prompt so the user can re-enter their key
        if (r.status === 401 && typeof Alpine !== 'undefined') {
          try {
            var store = Alpine.store('app');
            if (store && !store.showAuthPrompt) {
              _authToken = '';
              localStorage.removeItem('openfang-api-key');
              store.showAuthPrompt = true;
            }
          } catch(e2) { /* ignore Alpine errors */ }
        }
        return r.text().then(function(text) {
          var msg = '';
          var detail = '';
          var hint = '';
          var requestId = '';
          var serverPath = '';
          try {
            var rid = r.headers && r.headers.get ? r.headers.get('x-request-id') : '';
            if (rid) requestId = String(rid);
          } catch(e3) { /* ignore */ }
          try {
            var json = JSON.parse(text);
            msg = json.error || json.message || r.statusText;
            if (json.detail) detail = String(json.detail);
            else if (json.reason) detail = String(json.reason);
            if (json.hint) hint = String(json.hint);
            if (!requestId && json.request_id) requestId = String(json.request_id);
            if (json.path) serverPath = String(json.path);
          } catch(e2) {
            msg = text || r.statusText;
          }
          var primary = friendlyError(r.status, msg);
          var h = hint || errorHintForStatus(r.status, '');
          throw makeApiError(primary, r.status, detail || msg, h, method + ' ' + path, requestId, serverPath);
        });
      }
      var ct = r.headers.get('content-type') || '';
      if (ct.indexOf('application/json') >= 0) return r.json();
      return r.text().then(function(t) {
        try { return JSON.parse(t); } catch(e) { return { text: t }; }
      });
    }).catch(function(e) {
      if (e.name === 'TypeError' && e.message.includes('Failed to fetch')) {
        setConnectionState('disconnected');
        throw makeApiError(
          'Cannot connect to daemon',
          0,
          e.message,
          errorHintForStatus(0, ''),
          method + ' ' + path,
          '',
          ''
        );
      }
      throw e;
    });
  }

  var _networkHintsCache = { value: null, at: 0 };
  var NETWORK_HINTS_TTL_MS = 60000;

  /** GET /api/system/network-hints (cached ~60s). VPN/tunnel/proxy hints for wizard + chat copy. */
  function getNetworkHints() {
    var now = Date.now();
    if (_networkHintsCache.value && now - _networkHintsCache.at < NETWORK_HINTS_TTL_MS) {
      return Promise.resolve(_networkHintsCache.value);
    }
    return get('/api/system/network-hints').then(function(j) {
      _networkHintsCache = { value: j, at: Date.now() };
      return j;
    });
  }

  function get(path) { return request('GET', path); }
  function post(path, body) { return request('POST', path, body); }
  function put(path, body) { return request('PUT', path, body); }
  function patch(path, body) { return request('PATCH', path, body); }
  function del(path) { return request('DELETE', path); }

  // WebSocket manager with auto-reconnect
  var _ws = null;
  var _wsCallbacks = {};
  var _wsConnected = false;
  var _wsAgentId = null;
  var _reconnectTimer = null;
  var _reconnectAttempts = 0;
  var MAX_RECONNECT = 5;

  function noop() {}

  /** Drop chat UI handlers while keeping the socket (e.g. user navigated away from #agents). */
  function wsClearUiCallbacks() {
    _wsCallbacks = {
      onOpen: noop,
      onMessage: noop,
      onClose: function() {
        try { Alpine.store('app').wsConnected = false; } catch (e) { /* ignore */ }
      },
      onError: function() {
        try { Alpine.store('app').wsConnected = false; } catch (e) { /* ignore */ }
      }
    };
  }

  function getWsAgentId() {
    return _wsAgentId;
  }

  function wsConnect(agentId, callbacks) {
    callbacks = callbacks || {};
    var idStr = String(agentId);
    if (_wsAgentId === idStr && _ws && _ws.readyState === WebSocket.OPEN) {
      _wsCallbacks = callbacks;
      _reconnectAttempts = 0;
      setConnectionState('connected');
      if (callbacks.onOpen) callbacks.onOpen();
      return;
    }
    wsDisconnect();
    _wsCallbacks = callbacks;
    _wsAgentId = idStr;
    _reconnectAttempts = 0;
    _doConnect(idStr);
  }

  function _doConnect(agentId) {
    try {
      var url = WS_BASE + '/api/agents/' + agentId + '/ws';
      if (_authToken) url += '?token=' + encodeURIComponent(_authToken);
      var socket = new WebSocket(url);
      _ws = socket;

      socket.onopen = function() {
        // Guard: ignore if this socket was superseded by a newer connection
        if (_ws !== socket) return;
        _wsConnected = true;
        _reconnectAttempts = 0;
        setConnectionState('connected');
        try { Alpine.store('app').wsConnected = true; } catch (eAlp) { /* ignore */ }
        if (_reconnectAttempt > 0) {
          OpenFangToast.success('Reconnected');
          _reconnectAttempt = 0;
        }
        if (_wsCallbacks.onOpen) _wsCallbacks.onOpen();
      };

      socket.onmessage = function(e) {
        // Ignore frames from a socket we already replaced (agent switch / disconnect race)
        if (_ws !== socket) return;
        try {
          var data = JSON.parse(e.data);
        } catch(parseErr) {
          return; // Ignore malformed JSON frames
        }
        try {
          window.dispatchEvent(new CustomEvent('armaraos-agent-ws', {
            detail: { agentId: _wsAgentId, data: data }
          }));
        } catch (evErr) { /* ignore */ }
        // Dispatch outside try/catch so handler errors are not swallowed
        if (_wsCallbacks.onMessage) _wsCallbacks.onMessage(data);
      };

      socket.onclose = function(e) {
        // Guard: only update state if this is still the active socket.
        // A superseded socket closing must not null-out the new connection.
        if (_ws !== socket) return;
        _wsConnected = false;
        _ws = null;
        if (_wsAgentId && _reconnectAttempts < MAX_RECONNECT && e.code !== 1000) {
          _reconnectAttempts++;
          _reconnectAttempt = _reconnectAttempts;
          setConnectionState('reconnecting');
          if (_reconnectAttempts === 1) {
            OpenFangToast.warn('Connection lost, reconnecting...');
          }
          var delay = Math.min(1000 * Math.pow(2, _reconnectAttempts - 1), 10000);
          _reconnectTimer = setTimeout(function() { _doConnect(_wsAgentId); }, delay);
          return;
        }
        if (_wsAgentId && _reconnectAttempts >= MAX_RECONNECT) {
          setConnectionState('disconnected');
          OpenFangToast.error('Connection lost — switched to HTTP mode', 0);
        }
        if (_wsCallbacks.onClose) _wsCallbacks.onClose();
      };

      socket.onerror = function() {
        // Guard: ignore errors from superseded sockets
        if (_ws !== socket) return;
        _wsConnected = false;
        if (_wsCallbacks.onError) _wsCallbacks.onError();
      };
    } catch(e) {
      _wsConnected = false;
    }
  }

  function wsDisconnect() {
    _wsAgentId = null;
    _reconnectAttempts = MAX_RECONNECT;
    if (_reconnectTimer) { clearTimeout(_reconnectTimer); _reconnectTimer = null; }
    if (_ws) { _ws.close(1000); _ws = null; }
    _wsConnected = false;
    // Drop handlers so a destroyed chat component cannot receive frames after navigation away
    _wsCallbacks = {};
  }

  function wsSend(data) {
    if (_ws && _ws.readyState === WebSocket.OPEN) {
      _ws.send(JSON.stringify(data));
      return true;
    }
    return false;
  }

  function isWsConnected() { return _wsConnected; }

  function getConnectionState() { return _connectionState; }

  function getToken() { return _authToken; }

  /** Full URL for EventSource (SSE). EventSource cannot send Authorization; append token when needed. */
  function sseUrl(path) {
    var url = BASE + path;
    var sep = path.indexOf('?') >= 0 ? '&' : '?';
    if (_authToken) url += sep + 'token=' + encodeURIComponent(_authToken);
    return url;
  }

  /** Download a home-relative file (GET + Bearer / ?token=). Saves via blob link. */
  function downloadArmaraosHomeFile(relPath) {
    if (!relPath || typeof relPath !== 'string') {
      return Promise.reject(new Error('Missing path'));
    }
    var path =
      '/api/armaraos-home/download?path=' + encodeURIComponent(relPath);
    if (_authToken) path += '&token=' + encodeURIComponent(_authToken);
    var opts = { method: 'GET', headers: {}, credentials: 'same-origin' };
    if (_authToken) opts.headers['Authorization'] = 'Bearer ' + _authToken;
    var fallbackName = relPath.split('/').pop() || 'download';
    return fetch(BASE + path, opts).then(function(r) {
      if (!r.ok) {
        return r.text().then(function(text) {
          var msg = 'Download failed';
          try {
            var j = JSON.parse(text);
            msg = j.error || j.message || msg;
          } catch (e2) {
            if (text) msg = text.slice(0, 200);
          }
          throw new Error(msg);
        });
      }
      var cd = r.headers.get('Content-Disposition') || '';
      var name = fallbackName;
      var m = /filename\*=UTF-8''([^;\s]+)|filename="([^"]+)"|filename=([^;\s]+)/i.exec(cd);
      if (m) {
        try {
          name = decodeURIComponent((m[1] || m[2] || m[3] || '').trim().replace(/^"+|"+$/g, ''));
        } catch (e3) { /* keep */ }
      }
      return r.blob().then(function(blob) {
        var url = URL.createObjectURL(blob);
        var a = document.createElement('a');
        a.href = url;
        a.download = name || fallbackName;
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(function() { URL.revokeObjectURL(url); }, 2000);
      });
    });
  }

  /** Download a diagnostics zip by filename (GET + Bearer). Triggers browser save (typically Downloads). */
  function downloadDiagnosticsZip(filename) {
    if (!filename || typeof filename !== 'string') {
      return Promise.reject(new Error('Missing bundle filename'));
    }
    var path = '/api/support/diagnostics/download?name=' + encodeURIComponent(filename);
    if (_authToken) path += '&token=' + encodeURIComponent(_authToken);
    var opts = { method: 'GET', headers: {}, credentials: 'same-origin' };
    if (_authToken) opts.headers['Authorization'] = 'Bearer ' + _authToken;
    return fetch(BASE + path, opts).then(function(r) {
      if (!r.ok) {
        return r.text().then(function(text) {
          var msg = 'Download failed';
          try {
            var j = JSON.parse(text);
            msg = j.error || j.message || msg;
          } catch (e2) {
            if (text) msg = text.slice(0, 200);
          }
          throw new Error(msg);
        });
      }
      var cd = r.headers.get('Content-Disposition') || '';
      var name = filename;
      var m = /filename\*=UTF-8''([^;\s]+)|filename="([^"]+)"|filename=([^;\s]+)/i.exec(cd);
      if (m) {
        try {
          name = decodeURIComponent((m[1] || m[2] || m[3] || '').trim().replace(/^"+|"+$/g, ''));
        } catch (e3) { /* keep filename */ }
      }
      return r.blob().then(function(blob) {
        var url = URL.createObjectURL(blob);
        var a = document.createElement('a');
        a.href = url;
        a.download = name || filename;
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(function() { URL.revokeObjectURL(url); }, 2000);
      });
    });
  }

  function upload(agentId, file) {
    var hdrs = {};
    if (_authToken) hdrs['Authorization'] = 'Bearer ' + _authToken;
	var form = new FormData();
    form.append('file', file);
    form.append('filename', file.name);
    return fetch(BASE + '/api/agents/' + agentId + '/upload', {
      method: 'POST',
      headers: hdrs,
      body: form
    }).then(function(r) {
      if (!r.ok) throw new Error('Upload failed');
      return r.json();
    });
  }

  return {
    /** Same origin as API requests (`window.location.origin`). */
    baseUrl: BASE,
    setAuthToken: setAuthToken,
    getToken: getToken,
    sseUrl: sseUrl,
    getNetworkHints: getNetworkHints,
    get: get,
    post: post,
    put: put,
    patch: patch,
    del: del,
    delete: del,
    upload: upload,
    downloadDiagnosticsZip: downloadDiagnosticsZip,
    downloadArmaraosHomeFile: downloadArmaraosHomeFile,
    wsConnect: wsConnect,
    wsDisconnect: wsDisconnect,
    wsClearUiCallbacks: wsClearUiCallbacks,
    getWsAgentId: getWsAgentId,
    wsSend: wsSend,
    isWsConnected: isWsConnected,
    getConnectionState: getConnectionState,
    onConnectionChange: onConnectionChange
  };
})();
