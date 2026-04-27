// ArmaraOS Chat Page — Agent chat with markdown + streaming
'use strict';

/**
 * Module-level per-agent message cache.
 * Lives outside the Alpine component so it survives x-if destroy/recreate
 * cycles (page navigation) and agent switches. Keyed by agent UUID.
 * Values are shallow-cloned message arrays — deep objects (tools, images)
 * are shared references since they are not mutated after finalization.
 */
var _agentMsgCache = {};

/**
 * Per-agent lazy-load pagination state for chat history.
 * Keyed by agent UUID. Lives outside the Alpine component so a navigation
 * away from #agents and back doesn't lose the user's scroll-back depth.
 *
 *   oldestIndex   – server-side index of the oldest message currently visible
 *                   (== the `before` value the next prepend should send)
 *   hasMore       – whether the server says there are older messages on disk
 *   totalVisible  – total visible-message count reported by the server
 *   loadingOlder  – true while a prepend fetch is in flight (debounce guard)
 *   version       – bumped on each agent switch / fresh load; older fetches
 *                   that complete after a switch must throw away their result
 */
var _agentSessionState = {};

/** Page size for the initial latest-history load and each scroll-up prepend. */
var CHAT_HISTORY_PAGE_SIZE = 50;
/** Trigger lazy-load when within this many px of the messages container top. */
var CHAT_HISTORY_TOP_THRESHOLD_PX = 120;

/**
 * Turn raw provider/daemon errors into calm, actionable copy for the chat UI.
 * Technical text is returned separately for tooltips / support.
 * @returns {{ text: string, rateLimited: boolean, rawForDebug: string }}
 */
function humanizeChatError(raw) {
  var s = raw == null ? '' : String(raw);
  var trimmed = s.trim();
  if (!trimmed) {
    return {
      text: 'That reply did not finish. Send your message again — nothing is wrong with your chat.',
      rateLimited: false,
      rawForDebug: s
    };
  }
  var once = trimmed.replace(/^(Request failed:\s*)+/i, 'Request failed: ').replace(/^(HTTP error:\s*)+/i, 'HTTP error: ');
  var low = once.toLowerCase();

  if (/429|rate limit|too many requests|resource_exhausted|resource exhausted|throttl|slow down|over capacity|capacity limit/.test(low)) {
    return {
      text: 'The AI service is limiting traffic right now. Wait a bit, then send again — your conversation is still here.',
      rateLimited: true,
      rawForDebug: s
    };
  }

  if (/decoding response body|error decoding|failed to decode body|invalid chunk|incomplete chunked|unexpected eof|stream ended|broken pipe|connection reset/.test(low)) {
    return {
      text: 'The reply stream was cut off or could not be read. This is usually temporary — send again in a moment.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/timeout|timed out|deadline exceeded|context deadline|took too long/i.test(low)) {
    return {
      text: 'The request ran out of time. Try a shorter message or try again in a moment.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  // Before generic 401/403: providers often use 402/403 for credits or model access — not a bad API key.
  if (/402|payment required|insufficient credits|insufficient balance|not enough credits|exceed.*credit|purchase credits|billing issue|charged error/i.test(low)) {
    return {
      text: 'The provider reported a billing or credit limitation. Confirm balance and model pricing on the provider site (e.g. openrouter.com → Credits). Your API key may still be valid.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/no endpoints found|not allowed to use|does not have access to this model|cannot access this model|model.*not.*available.*account|access.*denied.*model/i.test(low)) {
    return {
      text: 'This model is not available for your account or key (not necessarily an invalid key). Choose another model in Settings or confirm access on the provider site.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  // Genuine missing/wrong key is usually 401 or explicit invalid-key text — not every 403.
  if (/\b401\b|invalid api key|api key not|authentication failed|bad api key|wrong api key/i.test(low)) {
    return {
      text: 'Sign-in with the AI provider failed. Check your API key under Settings, then try again.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/\b403\b|permission denied|forbidden/.test(low)) {
    return {
      text: 'The provider refused access (HTTP 403). That is often model or account restrictions, not a wrong API key — hover the error for details. Confirm the model id and your OpenRouter account can use it.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/model.*not found|does not exist|invalid model|unknown model|no such model/.test(low)) {
    return {
      text: 'This model is not available. Use /model to pick another one.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/context length|maximum context|token limit|too many tokens|maximum.*tokens|input is too long/.test(low)) {
    return {
      text: 'This chat is too long for the model. Use /compact or /new, then continue.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/connection refused|econnrefused|network.*unreachable|no route|dns|getaddrinfo|ssl|tls|certificate|failed to connect|cannot reach/.test(low)) {
    return {
      text: 'Could not reach the AI service. Check your connection, VPN, or firewall, then try again.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  if (/\b5\d\d\b|internal server error|bad gateway|service unavailable|gateway timeout|502|503|504/.test(low)) {
    return {
      text: 'The AI service had a brief problem. Wait a moment and send your message again.',
      rateLimited: false,
      rawForDebug: s
    };
  }

  // Short generic: avoid echoing huge raw blobs in the main line
  return {
    text: 'That reply did not finish. Send your message again — if it keeps happening, check Settings → provider.',
    rateLimited: false,
    rawForDebug: s
  };
}

function chatPage() {
  var msgId = 0;
  return {
    currentAgent: null,
    messages: [],
    inputText: '',
    sending: false,
    messageQueue: [],    // Queue for messages sent while streaming
    thinkingMode: 'off', // 'off' | 'on' | 'stream'
    _wsAgent: null,
    _wsReconnectTimer: null,
    _wsInFlightMsg: null,     // { text, files, images } — message in flight when WS dropped
    _elapsedTimer: null,
    elapsedSeconds: 0,        // live counter shown while agent is working
    currentIteration: 0,      // tracks multi-step iteration number for display
    sessionCostUsd: 0,        // cumulative cost for this chat session (USD)
    showSlashMenu: false,
    slashFilter: '',
    slashIdx: 0,
    attachments: [],
    dragOver: false,
    bookmarkModalOpen: false,
    bookmarkCategoryId: '',
    bookmarkNewCategory: '',
    bookmarkTitle: '',
    bookmarkMsg: null,
    contextPressure: 'low', // green/yellow/orange/red indicator
    /** Cumulative prompt/completion tokens for this chat tab (WS responses only). */
    sessionPromptTokens: 0,
    sessionCompletionTokens: 0,
    /** Wall time (ms) for the last completed turn from server (`turn_wall_ms`). */
    lastTurnWallMs: null,
    /** Subtle indicator: whether memory context was applied on the last completed turn. */
    memoryContextAppliedLastTurn: null,
    /** Timestamp when the memory indicator was last updated. */
    memoryContextAppliedAtMs: null,
    _memoryInjectedBeforeTurn: null,
    /** Ultra Cost-Efficient Mode: "off" | "balanced" | "aggressive" | "adaptive" (loaded from config on init). */
    efficientMode: (function() {
      var stored = localStorage.getItem('armaraos-eco-mode');
      return (stored === 'off' || stored === 'balanced' || stored === 'aggressive' || stored === 'adaptive') ? stored : 'off';
    }()),
    /** Global config default used as fallback when an agent has no explicit eco mode yet. */
    globalEfficientMode: 'off',
    /** Rolling average compression saving % for this session (0 when none). */
    sessionEcoSavedPct: 0,
    _sessionEcoSavedSum: 0,
    _sessionEcoSavedCount: 0,
    /** Last error — friendly line for telemetry (cleared on next send / success). */
    lastStreamError: null,
    /** Original error string for tooltips / support (optional). */
    lastStreamErrorTechnical: null,
    /** True when provider rate-limits; softer badge + color in telemetry. */
    lastStreamErrorIsRateLimited: false,
    /** Top-bar runtime/LLM guidance shown below telemetry strip. */
    runtimeStatusNote: null,
    runtimeStatusTechnical: null,
    runtimeStatusLevel: 'info',
    _typingTimeout: null,
    /** Timestamp (ms) when we last received a non-LLM event (tool/text) while sending.
     *  Used to display "Waiting for LLM · Xs" in the header when the LLM is slow
     *  but the agent loop is still alive. Reset on every tool_start/tool_result/text_delta. */
    _llmWaitSince: 0,
    /** Seconds since we started waiting purely for the LLM (no tool or token events). */
    llmWaitSeconds: 0,
    _llmWaitTimer: null,
    /** When false, streaming updates must not call scrollToBottom() — user scrolled up to read history */
    _chatPinnedToBottom: true,
    // Multi-session state
    sessions: [],
    sessionsOpen: false,
    searchOpen: false,
    searchQuery: '',
    // Voice recording state
    recording: false,
    _mediaRecorder: null,
    _audioChunks: [],
    recordingTime: 0,
    _recordingTimer: null,
    /**
     * When true, send `voice_reply: true` with the next message (HTTP/WS) so the daemon
     * may return Piper TTS if `[local_voice]` is configured. Persists in localStorage.
     * Default off: assistant replies are text unless the user opts in.
     */
    voiceReplyEnabled: (function() {
      var s = localStorage.getItem('armaraos-voice-reply');
      if (s === '1') return true;
      if (s === '0') return false;
      return false;
    }()),
    /** Persistent hint under the input when Piper is unavailable or prefs were reset (not only toast). */
    voiceReplyHint: '',
    // Model autocomplete state
    showModelPicker: false,
    modelPickerList: [],
    modelPickerFilter: '',
    modelPickerIdx: 0,
    // Model switcher dropdown
    showModelSwitcher: false,
    modelSwitcherFilter: '',
    modelSwitcherProviderFilter: '',
    modelSwitcherIdx: 0,
    modelSwitching: false,
    _modelCache: null,
    _modelCacheTime: 0,
    slashCommands: [
      { cmd: '/help', desc: 'Show available commands' },
      { cmd: '/agents', desc: 'Switch to Agents page' },
      { cmd: '/new', desc: 'Reset session (clear history)' },
      { cmd: '/compact', desc: 'Trigger LLM session compaction' },
      { cmd: '/model', desc: 'Show or switch model — switching resets canonical session memory (/model [name])' },
      { cmd: '/stop', desc: 'Cancel current agent run' },
      { cmd: '/usage', desc: 'Show session token usage & cost' },
      { cmd: '/think', desc: 'Toggle extended thinking (/think [on|off|stream])' },
      { cmd: '/context', desc: 'Show context window usage & pressure' },
      { cmd: '/verbose', desc: 'Cycle tool detail level (/verbose [off|on|full])' },
      { cmd: '/queue', desc: 'Check if agent is processing' },
      { cmd: '/status', desc: 'Show system status' },
      { cmd: '/clear', desc: 'Clear chat display' },
      { cmd: '/exit', desc: 'Disconnect from agent' },
      { cmd: '/budget', desc: 'Show spending limits and current costs' },
      { cmd: '/peers', desc: 'Show OFP peer network status' },
      { cmd: '/a2a', desc: 'List discovered external A2A agents' },
      { cmd: '/bookmarks', desc: 'Open saved Bookmarks' },
      { cmd: '/btw', desc: 'Inject context into the running agent loop (/btw <text>)' },
      { cmd: '/redirect', desc: 'Override the running agent loop with a new directive — stops current plan (/redirect <new instruction>)' },
      { cmd: '/t', desc: 'Expand a saved template (/t <name> | /t save <name> | /t list | /t delete <name>)' }
    ],
    tokenCount: 0,

    /** Chat input history (per-agent, persisted to localStorage). Up/Down arrows to cycle. */
    _msgHistory: [],
    _msgHistoryIdx: -1,
    _msgHistoryBuffer: '',

    /** From GET /api/system/network-hints (VPN/tunnel awareness). */
    networkHints: null,
    networkHintsBannerDismissed: false,

    // ── Tip Bar ──
    tipIndex: 0,
    tips: ['Type / for commands', '/think on for reasoning', 'Ctrl+Shift+F for focus mode', 'Drag files to attach', '/model to switch models', '/context to check usage', '/verbose off to hide tool details'],
    tipTimer: null,
    get currentTip() {
      if (localStorage.getItem('of-tips-off') === 'true') return '';
      return this.tips[this.tipIndex % this.tips.length];
    },
    dismissTips: function() { localStorage.setItem('of-tips-off', 'true'); },
    startTipCycle: function() {
      var self = this;
      if (this.tipTimer) clearInterval(this.tipTimer);
      this.tipTimer = setInterval(function() {
        self.tipIndex = (self.tipIndex + 1) % self.tips.length;
      }, 30000);
    },

    /** ArrowUp in textarea: navigate model picker, slash menu, or input history. */
    handleInputArrowUp(e) {
      if (this.showModelPicker) {
        e.preventDefault();
        this.modelPickerIdx = Math.max(0, this.modelPickerIdx - 1);
        return;
      }
      if (this.showSlashMenu) {
        e.preventDefault();
        this.slashIdx = Math.max(0, this.slashIdx - 1);
        return;
      }
      // History: navigate back when cursor is at the very start of the input
      var ta = e.target;
      if (ta && ta.selectionStart === 0 && this._msgHistory.length > 0) {
        if (this._msgHistoryIdx === -1) this._msgHistoryBuffer = this.inputText;
        this._msgHistoryIdx = Math.min(this._msgHistoryIdx + 1, this._msgHistory.length - 1);
        this.inputText = this._msgHistory[this._msgHistoryIdx] || '';
        e.preventDefault();
        var self = this;
        this.$nextTick(function() {
          var el = document.getElementById('msg-input');
          if (el) el.selectionStart = el.selectionEnd = el.value.length;
        });
      }
    },

    /** ArrowDown in textarea: navigate model picker, slash menu, or input history. */
    handleInputArrowDown(e) {
      if (this.showModelPicker) {
        e.preventDefault();
        this.modelPickerIdx = Math.min(this.filteredModelPicker.length - 1, this.modelPickerIdx + 1);
        return;
      }
      if (this.showSlashMenu) {
        e.preventDefault();
        this.slashIdx = Math.min(this.filteredSlashCommands.length - 1, this.slashIdx + 1);
        return;
      }
      // History: navigate forward when in history mode
      if (this._msgHistoryIdx >= 0) {
        e.preventDefault();
        if (this._msgHistoryIdx > 0) {
          this._msgHistoryIdx--;
          this.inputText = this._msgHistory[this._msgHistoryIdx] || '';
        } else {
          this._msgHistoryIdx = -1;
          this.inputText = this._msgHistoryBuffer || '';
          this._msgHistoryBuffer = '';
        }
      }
    },

    // Backward compat helper
    get thinkingEnabled() { return this.thinkingMode !== 'off'; },

    // Context pressure dot color
    get contextDotColor() {
      switch (this.contextPressure) {
        case 'critical': return '#ef4444';
        case 'high': return '#f97316';
        case 'medium': return '#eab308';
        default: return '#22c55e';
      }
    },

    get lastStreamErrorTruncated() {
      var s = this.lastStreamError || '';
      return s.length > 220 ? s.slice(0, 217) + '\u2026' : s;
    },

    /** Tooltip: technical detail for hover; keeps the visible line calm. */
    get lastStreamErrorTooltip() {
      var tech = this.lastStreamErrorTechnical;
      if (!tech) return this.lastStreamError || '';
      var t = String(tech);
      if (t.length > 700) t = t.slice(0, 697) + '\u2026';
      return 'Technical detail: ' + t;
    },

    get lastErrorLooksRateLimited() {
      return !!this.lastStreamErrorIsRateLimited;
    },

    get runtimeStatusHintText() {
      if (this.sending && this.llmWaitSeconds >= 10) {
        return 'Still waiting for the LLM (' + this.llmWaitSeconds + 's). Expected: reply should continue. If it passes ~20s, you can wait, press Stop, or resend.';
      }
      if (this.sending && this.llmWaitSeconds >= 4) {
        return 'Waiting for the LLM (' + this.llmWaitSeconds + 's). This is usually transient during model load or brief network jitter.';
      }
      return this.runtimeStatusNote || '';
    },

    get runtimeStatusHintTitle() {
      if (this.sending && this.llmWaitSeconds >= 6) {
        return 'Live wait status while the model is still processing.';
      }
      var tech = this.runtimeStatusTechnical;
      if (!tech) return this.runtimeStatusNote || '';
      var t = String(tech);
      if (t.length > 700) t = t.slice(0, 697) + '\u2026';
      return 'Technical detail: ' + t;
    },

    get runtimeStatusLooksWarning() {
      if (this.runtimeStatusLevel === 'warn') return true;
      if (this.sending && this.llmWaitSeconds >= 18) return true;
      return false;
    },

    _clearStreamErrorTelemetry: function() {
      this.lastStreamError = null;
      this.lastStreamErrorTechnical = null;
      this.lastStreamErrorIsRateLimited = false;
    },

    _clearRuntimeStatusTelemetry: function() {
      this.runtimeStatusNote = null;
      this.runtimeStatusTechnical = null;
      this.runtimeStatusLevel = 'info';
    },

    _applyAinlRuntimeTelemetry: function(payload, source) {
      if (!payload || typeof payload !== 'object') return;
      var status = String(payload.turn_status || '').toLowerCase();
      var warningCount = Number(payload.warning_count || 0);
      var partial = !!payload.partial_success;
      var note = '';
      var level = 'info';
      if (status && status !== 'ok') {
        level = 'warn';
        note = 'Runtime pre-pass reported ' + status.toUpperCase() + '. Expected: fallback to the normal LLM reply path. If this repeats, disable ainl_runtime_engine for this agent.';
      } else if (partial || warningCount > 0) {
        level = 'warn';
        note = 'Runtime pre-pass completed with ' + warningCount + ' warning' + (warningCount === 1 ? '' : 's') + '. Expected: the LLM reply should still continue this turn. If output looks off, retry once.';
      }
      if (!note) return;
      this.runtimeStatusNote = note;
      try {
        this.runtimeStatusTechnical = JSON.stringify({
          source: source || 'ws',
          telemetry: payload
        });
      } catch (e) {
        this.runtimeStatusTechnical = String(payload);
      }
      this.runtimeStatusLevel = level;
    },

    _applyFriendlyError: function(raw) {
      var h = humanizeChatError(raw);
      this.lastStreamError = h.text;
      this.lastStreamErrorTechnical = h.rawForDebug;
      this.lastStreamErrorIsRateLimited = h.rateLimited;
      return h;
    },

    _recordEcoSaving: function(pct) {
      if (!pct || pct <= 0) return;
      this._sessionEcoSavedSum += pct;
      this._sessionEcoSavedCount++;
      this.sessionEcoSavedPct = Math.round(this._sessionEcoSavedSum / this._sessionEcoSavedCount);
    },

    /** Short suffix for message meta: adaptive confidence + shadow Δ vs recommendation (when present). */
    _buildEcoMetaSuffix: function(data) {
      if (!data) return '';
      var parts = [];
      if (data.adaptive_confidence != null && typeof data.adaptive_confidence === 'number') {
        parts.push('conf ' + Math.round(data.adaptive_confidence * 100) + '%');
      }
      var cf = data.eco_counterfactual;
      if (cf && cf.tokens_saved_delta_recommended_minus_applied != null) {
        var d = cf.tokens_saved_delta_recommended_minus_applied;
        parts.push('\u0394rec ' + (d > 0 ? '+' : '') + d + ' tok');
      }
      return parts.length ? ' | ' + parts.join(' \u00b7 ') : '';
    },

    /** Tooltip JSON for hover (truncated) — full receipt lives on the server / IR. */
    _buildEcoMetaTooltip: function(data) {
      if (!data) return '';
      if (data.adaptive_confidence == null && !data.eco_counterfactual) return '';
      try {
        var o = { adaptive_confidence: data.adaptive_confidence, eco_counterfactual: data.eco_counterfactual };
        var s = JSON.stringify(o);
        return s.length > 900 ? s.slice(0, 897) + '...' : s;
      } catch (e) {
        return '';
      }
    },

    _syncEcoModeToConfig: async function(mode) {
      var normalized = (mode === 'off' || mode === 'balanced' || mode === 'aggressive' || mode === 'adaptive') ? mode : 'off';
      // Skip redundant writes when we already synced this mode.
      if (this._lastSyncedEcoMode === normalized) return;
      this._lastSyncedEcoMode = normalized;
      try {
        await fetch('/api/config/set', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ path: 'efficient_mode', value: normalized })
        });
      } catch(e) {
        // Non-fatal: UI remains responsive; we re-attempt on next switch/toggle.
      }
    },

    _applyAgentEcoMode: function(agent) {
      if (!agent || !agent.id) return;
      var st = Alpine.store('app');
      var mode = st.getAgentEcoMode(agent.id, this.globalEfficientMode || this.efficientMode || 'off');
      this.efficientMode = mode;
      // Ensure first-time fallback becomes an explicit per-agent persisted value.
      st.setAgentEcoMode(agent.id, mode);
      localStorage.setItem('armaraos-eco-mode', mode);
      this._syncEcoModeToConfig(mode);
    },

    formatTokenShort: function(n) {
      n = Number(n) || 0;
      if (n >= 1000000) return (n / 1000000).toFixed(1).replace(/\.0$/, '') + 'M';
      if (n >= 10000) return Math.round(n / 1000) + 'k';
      if (n >= 1000) return (n / 1000).toFixed(1).replace(/\.0$/, '') + 'k';
      return String(n);
    },

    formatWallSeconds: function(ms) {
      if (ms == null || ms === '') return '';
      var s = Number(ms) / 1000;
      return (s >= 100 ? Math.round(s) : s.toFixed(1)) + 's';
    },

    workspacePillStyle() {
      var fallback = '#3b82f6';
      var color = fallback;
      try {
        var c = this.currentAgent && this.currentAgent.identity && this.currentAgent.identity.color
          ? String(this.currentAgent.identity.color).trim()
          : '';
        // Keep styling safe/predictable: only allow #RGB/#RRGGBB values.
        if (/^#(?:[0-9a-fA-F]{3}){1,2}$/.test(c)) color = c;
      } catch (e) { /* fallback */ }
      return '--workspace-pill-accent:' + color;
    },

    get showChatNetworkBanner() {
      if (this.networkHintsBannerDismissed || !this.currentAgent) return false;
      var h = this.networkHints;
      return !!(h && h.likely_vpn);
    },

    dismissChatNetworkBanner() {
      this.networkHintsBannerDismissed = true;
    },

    async refreshChatNetworkHints() {
      try {
        if (typeof OpenFangAPI !== 'undefined' && OpenFangAPI.getNetworkHints) {
          this.networkHints = await OpenFangAPI.getNetworkHints();
        }
      } catch (e) { /* ignore */ }
    },

    get modelDisplayName() {
      if (!this.currentAgent) return '';
      var name = this.currentAgent.model_name || '';
      var short = name.replace(/-\d{8}$/, '');
      return short.length > 24 ? short.substring(0, 22) + '\u2026' : short;
    },

    get switcherProviders() {
      var seen = {};
      (this._modelCache || []).forEach(function(m) { seen[m.provider] = true; });
      return Object.keys(seen).sort();
    },

    get filteredSwitcherModels() {
      var models = this._modelCache || [];
      var provFilter = this.modelSwitcherProviderFilter;
      var textFilter = this.modelSwitcherFilter ? this.modelSwitcherFilter.toLowerCase() : '';
      if (!provFilter && !textFilter) return models;
      return models.filter(function(m) {
        if (provFilter && m.provider !== provFilter) return false;
        if (textFilter) {
          return m.id.toLowerCase().indexOf(textFilter) !== -1 ||
                 (m.display_name || '').toLowerCase().indexOf(textFilter) !== -1 ||
                 m.provider.toLowerCase().indexOf(textFilter) !== -1;
        }
        return true;
      });
    },

    get groupedSwitcherModels() {
      var filtered = this.filteredSwitcherModels;
      var groups = {}, order = [];
      filtered.forEach(function(m) {
        if (!groups[m.provider]) { groups[m.provider] = []; order.push(m.provider); }
        groups[m.provider].push(m);
      });
      return order.map(function(p) {
        return { provider: p.charAt(0).toUpperCase() + p.slice(1), models: groups[p] };
      });
    },

    init() {
      var self = this;

      // Start tip cycle
      this.startTipCycle();

      // Fetch dynamic commands from server
      this.fetchCommands();

      // Load global efficient mode from server config (fallback for agents that
      // have never had an explicit per-agent mode selected).
      OpenFangAPI.get('/api/config').then(function(cfg) {
        if (cfg && typeof cfg.efficient_mode === 'string' && cfg.efficient_mode) {
          self.globalEfficientMode = cfg.efficient_mode;
          if (!self.currentAgent) self.efficientMode = cfg.efficient_mode;
          localStorage.setItem('armaraos-eco-mode', cfg.efficient_mode);
        }
      }).catch(function() {});

      // Ctrl+/ keyboard shortcut
      document.addEventListener('keydown', function(e) {
        if ((e.ctrlKey || e.metaKey) && e.key === '/') {
          e.preventDefault();
          var input = document.getElementById('msg-input');
          if (input) { input.focus(); self.inputText = '/'; }
        }
        // Ctrl+M for model switcher
        if ((e.ctrlKey || e.metaKey) && e.key === 'm' && self.currentAgent) {
          e.preventDefault();
          self.toggleModelSwitcher();
        }
        // Ctrl+F for chat search
        if ((e.ctrlKey || e.metaKey) && e.key === 'f' && self.currentAgent) {
          e.preventDefault();
          self.toggleSearch();
        }
      });

      // Load session + session list when agent changes
      this.$watch('currentAgent', function(agent) {
        try {
          var st = Alpine.store('app');
          st.agentsPageChatAgentId = agent ? agent.id : null;
          if (agent) {
            st.clearAgentChatUnread(agent.id);
            st.primeAssistantBaselineForAgent(agent.id);
          }
        } catch (e) { /* ignore */ }
        if (agent) {
          self._applyAgentEcoMode(agent);
          self.loadSession(agent.id);
          self.loadSessions(agent.id);
          // Load per-agent input history
          try {
            var raw = localStorage.getItem('armaraos-chat-history-' + agent.id);
            self._msgHistory = raw ? JSON.parse(raw) : [];
          } catch (eH) { self._msgHistory = []; }
          self._msgHistoryIdx = -1;
          self._msgHistoryBuffer = '';
        }
      });

      // Check for pending agent from Agents page (set before chat mounted)
      var store = Alpine.store('app');
      if (store.pendingAgent) {
        self.selectAgent(store.pendingAgent);
        store.pendingAgent = null;
      }

      // Watch for future pending agent selections (e.g., user clicks agent while on chat)
      this.$watch('$store.app.pendingAgent', function(agent) {
        if (agent) {
          self.selectAgent(agent);
          Alpine.store('app').pendingAgent = null;
        }
      });

      // When server-side UI prefs arrive after chat mounted, re-apply current
      // agent mode so upgrades/reinstalls pick up persisted per-agent settings.
      window.addEventListener('armaraos-ui-prefs-loaded', function() {
        if (self.currentAgent && self.currentAgent.id) {
          self._applyAgentEcoMode(self.currentAgent);
        }
      });

      this._syncVoiceReplyWithServer();

      // Watch for slash commands + model autocomplete
      this.$watch('inputText', function(val) {
        var modelMatch = val.match(/^\/model\s+(.*)$/i);
        if (modelMatch) {
          self.showSlashMenu = false;
          self.modelPickerFilter = modelMatch[1].toLowerCase();
          if (!self.modelPickerList.length) {
            OpenFangAPI.get('/api/models').then(function(data) {
              self.modelPickerList = (data.models || []).filter(function(m) { return m.available; });
              self.showModelPicker = true;
              self.modelPickerIdx = 0;
            }).catch(function() {});
          } else {
            self.showModelPicker = true;
          }
        } else if (val.startsWith('/')) {
          self.showModelPicker = false;
          self.slashFilter = val.slice(1).toLowerCase();
          self.showSlashMenu = true;
          self.slashIdx = 0;
        } else {
          self.showSlashMenu = false;
          self.showModelPicker = false;
        }
      });
    },

    get filteredModelPicker() {
      if (!this.modelPickerFilter) return this.modelPickerList.slice(0, 15);
      var f = this.modelPickerFilter;
      return this.modelPickerList.filter(function(m) {
        return m.id.toLowerCase().indexOf(f) !== -1 || (m.display_name || '').toLowerCase().indexOf(f) !== -1 || m.provider.toLowerCase().indexOf(f) !== -1;
      }).slice(0, 15);
    },

    pickModel(modelId) {
      this.showModelPicker = false;
      this.inputText = '/model ' + modelId;
      this.sendMessage();
    },

    toggleModelSwitcher() {
      if (this.showModelSwitcher) { this.showModelSwitcher = false; return; }
      var self = this;
      var now = Date.now();
      if (this._modelCache && (now - this._modelCacheTime) < 300000) {
        this.modelSwitcherFilter = '';
        this.modelSwitcherProviderFilter = '';
        this.modelSwitcherIdx = 0;
        this.showModelSwitcher = true;
        this.$nextTick(function() {
          var el = document.getElementById('model-switcher-search');
          if (el) el.focus();
        });
        return;
      }
      OpenFangAPI.get('/api/models').then(function(data) {
        var models = (data.models || []).filter(function(m) { return m.available; });
        self._modelCache = models;
        self._modelCacheTime = Date.now();
        self.modelPickerList = models;
        self.modelSwitcherFilter = '';
        self.modelSwitcherProviderFilter = '';
        self.modelSwitcherIdx = 0;
        self.showModelSwitcher = true;
        self.$nextTick(function() {
          var el = document.getElementById('model-switcher-search');
          if (el) el.focus();
        });
      }).catch(function(e) {
        var msg = e.message || 'Unknown error';
        var detail = e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
        var hint = e.hint ? (' Hint: ' + e.hint) : '';
        OpenFangToast.error('Failed to load models: ' + msg + detail + hint);
      });
    },

    switchModel(model) {
      if (!this.currentAgent) return;
      var prov = (model.provider || '').trim();
      var curProv = (this.currentAgent.model_provider || '').trim();
      if (model.id === this.currentAgent.model_name && (!prov || prov === curProv)) {
        this.showModelSwitcher = false;
        return;
      }
      var self = this;
      OpenFangToast.confirm(
        'Switch model?',
        OpenFangToast.modelProviderChangeWarningText(),
        function() { self._switchModelApply(model); },
        { danger: false, confirmLabel: 'Switch' }
      );
    },

    _switchModelApply(model) {
      var self = this;
      if (!this.currentAgent) return;
      this.modelSwitching = true;
      OpenFangAPI.put('/api/agents/' + this.currentAgent.id + '/model', { model: model.id }).then(function(resp) {
        self.currentAgent.model_name = (resp && resp.model) || model.id;
        self.currentAgent.model_provider = (resp && resp.provider) || model.provider;
        OpenFangToast.success('Switched to ' + (model.display_name || model.id) + ' (canonical session reset)');
        self.showModelSwitcher = false;
        self.modelSwitching = false;
      }).catch(function(e) {
        var msg = e.message || 'Unknown error';
        var detail = e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
        var hint = e.hint ? (' Hint: ' + e.hint) : '';
        OpenFangToast.error('Switch failed: ' + msg + detail + hint);
        self.modelSwitching = false;
      });
    },

    // Fetch dynamic slash commands from server
    fetchCommands: function() {
      var self = this;
      OpenFangAPI.get('/api/commands').then(function(data) {
        if (data.commands && data.commands.length) {
          // Build a set of known cmds to avoid duplicates
          var existing = {};
          self.slashCommands.forEach(function(c) { existing[c.cmd] = true; });
          data.commands.forEach(function(c) {
            if (!existing[c.cmd]) {
              self.slashCommands.push({ cmd: c.cmd, desc: c.desc || '', source: c.source || 'server' });
              existing[c.cmd] = true;
            }
          });
        }
      }).catch(function() { /* silent — use hardcoded list */ });
    },

    get filteredSlashCommands() {
      if (!this.slashFilter) return this.slashCommands;
      var f = this.slashFilter;
      return this.slashCommands.filter(function(c) {
        return c.cmd.toLowerCase().indexOf(f) !== -1 || c.desc.toLowerCase().indexOf(f) !== -1;
      });
    },

    _startElapsed: function() {
      var self = this;
      this.elapsedSeconds = 0;
      this.currentIteration = 0;
      if (this._elapsedTimer) clearInterval(this._elapsedTimer);
      this._elapsedTimer = setInterval(function() { self.elapsedSeconds++; }, 1000);
    },

    _stopElapsed: function() {
      if (this._elapsedTimer) { clearInterval(this._elapsedTimer); this._elapsedTimer = null; }
      this.elapsedSeconds = 0;
      this.currentIteration = 0;
      this._stopLlmWait();
    },

    /** Start counting pure-LLM wait time (no tokens/tools arriving). */
    _startLlmWait: function() {
      var self = this;
      this._llmWaitSince = Date.now();
      if (this._llmWaitTimer) clearInterval(this._llmWaitTimer);
      this._llmWaitTimer = setInterval(function() {
        self.llmWaitSeconds = Math.round((Date.now() - self._llmWaitSince) / 1000);
      }, 1000);
    },

    /** Stop and reset the LLM-wait counter (called when tokens or tool events arrive). */
    _stopLlmWait: function() {
      if (this._llmWaitTimer) { clearInterval(this._llmWaitTimer); this._llmWaitTimer = null; }
      this._llmWaitSince = 0;
      this.llmWaitSeconds = 0;
    },

    get elapsedDisplay() {
      var s = this.elapsedSeconds;
      if (s < 60) return s + 's';
      var m = Math.floor(s / 60);
      return m + 'm ' + (s % 60) + 's';
    },

    // Clear any stuck typing indicator after 240s of complete silence.
    // This is deliberately generous: high-latency providers (e.g. reasoning models,
    // OpenRouter free tier, DeepSeek R1) can take 60-180s for a single LLM call with
    // no streaming tokens. The timeout is reset on EVERY meaningful WS event so it
    // only fires if the connection truly goes dark (no tool_start, tool_result, phase,
    // or text_delta for the full 4 minutes).
    _resetTypingTimeout: function() {
      var self = this;
      if (self._typingTimeout) clearTimeout(self._typingTimeout);
      self._typingTimeout = setTimeout(function() {
        self._stopElapsed();
        self.messages = self.messages.filter(function(m) { return !m.thinking && !m.streaming; });
        self.messages.push({ id: ++msgId, role: 'system', text: 'The agent stopped responding (timeout). The task may still be running — try sending a follow-up message to check status.', meta: '', tools: [], ts: Date.now() });
        self.sending = false;
        self.scrollToBottom(true);
      }, 240000);
    },

    _clearTypingTimeout: function() {
      if (this._typingTimeout) {
        clearTimeout(this._typingTimeout);
        this._typingTimeout = null;
      }
    },

    executeSlashCommand(cmd, cmdArgs) {
      this.showSlashMenu = false;
      this.inputText = '';
      var self = this;
      cmdArgs = cmdArgs || '';
      switch (cmd) {
        case '/help':
          self.messages.push({ id: ++msgId, role: 'system', text: self.slashCommands.map(function(c) { return '`' + c.cmd + '` — ' + c.desc; }).join('\n'), meta: '', tools: [] });
          self.scrollToBottom(true);
          break;
        case '/agents':
          location.hash = 'agents';
          break;
        case '/bookmarks':
          location.hash = 'bookmarks';
          break;
        case '/new':
          if (self.currentAgent) {
            OpenFangAPI.post('/api/agents/' + self.currentAgent.id + '/session/reset', {}).then(function() {
              self.messages = [];
              self.sessionPromptTokens = 0;
              self.sessionCompletionTokens = 0;
              self.lastTurnWallMs = null;
              self._clearStreamErrorTelemetry();
              OpenFangToast.success('Session reset');
            }).catch(function(e) { var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Reset failed: '+msg+d+h); });
          }
          break;
        case '/compact':
          if (self.currentAgent) {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Compacting session...', meta: '', tools: [] });
            OpenFangAPI.post('/api/agents/' + self.currentAgent.id + '/session/compact', {}).then(function(res) {
              self.messages.push({ id: ++msgId, role: 'system', text: res.message || 'Compaction complete', meta: '', tools: [] });
              self.scrollToBottom(true);
            }).catch(function(e) { var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Compaction failed: '+msg+d+h); });
          }
          break;
        case '/stop':
          if (self.currentAgent) {
            OpenFangAPI.post('/api/agents/' + self.currentAgent.id + '/stop', {}).then(function(res) {
              self.messages.push({ id: ++msgId, role: 'system', text: res.message || 'Run cancelled', meta: '', tools: [] });
              self.sending = false;
              self.scrollToBottom(true);
            }).catch(function(e) { var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Stop failed: '+msg+d+h); });
          }
          break;
        case '/btw':
          if (!cmdArgs.trim()) {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Usage: `/btw <context>` — Inject extra context or tasks into the running agent loop.', meta: '', tools: [] });
            self.scrollToBottom(true);
            break;
          }
          if (!self.currentAgent) {
            self.messages.push({ id: ++msgId, role: 'system', text: 'No agent selected.', meta: '', tools: [] });
            self.scrollToBottom(true);
            break;
          }
          // Show a local indicator immediately so the user sees the injection was sent
          self.messages.push({ id: ++msgId, role: 'system', text: '↩ **btw injected:** ' + cmdArgs, meta: '', tools: [] });
          self.scrollToBottom(true);
          OpenFangAPI.post('/api/agents/' + self.currentAgent.id + '/btw', { text: cmdArgs })
            .catch(function(e) {
              var msg = e.message || 'Unknown error';
              var isNotRunning = e.status === 409 || (msg && msg.toLowerCase().indexOf('not running') !== -1);
              if (isNotRunning) {
                self.messages.push({ id: ++msgId, role: 'system', text: '⚠ Agent is not currently running — btw context not injected. Send a message first, then /btw while it\'s working.', meta: '', tools: [] });
              } else {
                self.messages.push({ id: ++msgId, role: 'system', text: '⚠ Injection failed: ' + msg, meta: '', tools: [] });
              }
              self.scrollToBottom(true);
            });
          break;
        case '/redirect':
          if (!cmdArgs.trim()) {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Usage: `/redirect <new instruction>` — Override the running agent loop. Stops the current plan and redirects to a new directive.', meta: '', tools: [] });
            self.scrollToBottom(true);
            break;
          }
          if (!self.currentAgent) {
            self.messages.push({ id: ++msgId, role: 'system', text: 'No agent selected.', meta: '', tools: [] });
            self.scrollToBottom(true);
            break;
          }
          // Show a local indicator immediately so the user sees the redirect was sent
          self.messages.push({ id: ++msgId, role: 'system', text: '↩ **redirect sent:** ' + cmdArgs, meta: '', tools: [] });
          self.scrollToBottom(true);
          OpenFangAPI.post('/api/agents/' + self.currentAgent.id + '/redirect', { text: cmdArgs })
            .catch(function(e) {
              var msg = e.message || 'Unknown error';
              var isNotRunning = e.status === 409 || (msg && msg.toLowerCase().indexOf('not running') !== -1);
              if (isNotRunning) {
                self.messages.push({ id: ++msgId, role: 'system', text: '⚠ Agent is not currently running — redirect not sent. Send a message first, then /redirect while it\'s working.', meta: '', tools: [] });
              } else {
                self.messages.push({ id: ++msgId, role: 'system', text: '⚠ Redirect failed: ' + msg, meta: '', tools: [] });
              }
              self.scrollToBottom(true);
            });
          break;
        case '/usage':
          if (self.currentAgent) {
            var approxTokens = self.messages.reduce(function(sum, m) { return sum + Math.round((m.text || '').length / 4); }, 0);
            self.messages.push({ id: ++msgId, role: 'system', text: '**Session Usage**\n- Messages: ' + self.messages.length + '\n- Approx tokens: ~' + approxTokens, meta: '', tools: [] });
            self.scrollToBottom(true);
          }
          break;
        case '/think':
          if (cmdArgs === 'on') {
            self.thinkingMode = 'on';
          } else if (cmdArgs === 'off') {
            self.thinkingMode = 'off';
          } else if (cmdArgs === 'stream') {
            self.thinkingMode = 'stream';
          } else {
            // Cycle: off -> on -> stream -> off
            if (self.thinkingMode === 'off') self.thinkingMode = 'on';
            else if (self.thinkingMode === 'on') self.thinkingMode = 'stream';
            else self.thinkingMode = 'off';
          }
          var modeLabel = self.thinkingMode === 'stream' ? 'enabled (streaming reasoning)' : (self.thinkingMode === 'on' ? 'enabled' : 'disabled');
          self.messages.push({ id: ++msgId, role: 'system', text: 'Extended thinking **' + modeLabel + '**. ' +
            (self.thinkingMode === 'stream' ? 'Reasoning tokens will appear in a collapsible panel.' :
             self.thinkingMode === 'on' ? 'The agent will show its reasoning when supported by the model.' :
             'Normal response mode.'), meta: '', tools: [] });
          self.scrollToBottom(true);
          break;
        case '/context':
          // Send via WS command
          if (self.currentAgent && OpenFangAPI.isWsConnected()) {
            OpenFangAPI.wsSend({ type: 'command', command: 'context', args: '' });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Not connected. Connect to an agent first.', meta: '', tools: [] });
            self.scrollToBottom(true);
          }
          break;
        case '/verbose':
          if (self.currentAgent && OpenFangAPI.isWsConnected()) {
            OpenFangAPI.wsSend({ type: 'command', command: 'verbose', args: cmdArgs });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Not connected. Connect to an agent first.', meta: '', tools: [] });
            self.scrollToBottom(true);
          }
          break;
        case '/queue':
          if (self.currentAgent && OpenFangAPI.isWsConnected()) {
            OpenFangAPI.wsSend({ type: 'command', command: 'queue', args: '' });
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: 'Not connected.', meta: '', tools: [] });
            self.scrollToBottom(true);
          }
          break;
        case '/status':
          OpenFangAPI.get('/api/status').then(function(s) {
            self.messages.push({ id: ++msgId, role: 'system', text: '**System Status**\n- Agents: ' + (s.agent_count || 0) + '\n- Uptime: ' + (s.uptime_seconds || 0) + 's\n- Version: ' + (s.version || '?'), meta: '', tools: [] });
            self.scrollToBottom(true);
          }).catch(function() {});
          break;
        case '/model':
          if (self.currentAgent) {
            if (cmdArgs) {
              var arg = cmdArgs.trim();
              var ca = self.currentAgent;
              var curCombo = ((ca.model_provider || '').trim() + '/' + (ca.model_name || '').trim()).replace(/^\/+/, '');
              if (arg === curCombo || arg === (ca.model_name || '').trim()) {
                self.messages.push({ id: ++msgId, role: 'system', text: 'Already using that model (`' + (ca.model_name || '') + '`).', meta: '', tools: [] });
                self.scrollToBottom(true);
                break;
              }
              OpenFangToast.confirm(
                'Switch model?',
                OpenFangToast.modelProviderChangeWarningText(),
                function() {
                  OpenFangAPI.put('/api/agents/' + self.currentAgent.id + '/model', { model: arg }).then(function(resp) {
                    var resolvedModel = (resp && resp.model) || arg;
                    var resolvedProvider = (resp && resp.provider) || '';
                    self.currentAgent.model_name = resolvedModel;
                    if (resolvedProvider) { self.currentAgent.model_provider = resolvedProvider; }
                    self.messages.push({ id: ++msgId, role: 'system', text: 'Model switched to: `' + resolvedModel + '`' + (resolvedProvider ? ' (provider: `' + resolvedProvider + '`)' : '') + ' — canonical session reset.', meta: '', tools: [] });
                    self.scrollToBottom(true);
                  }).catch(function(e) { var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Model switch failed: '+msg+d+h); });
                },
                { danger: false, confirmLabel: 'Switch' }
              );
            } else {
              self.messages.push({ id: ++msgId, role: 'system', text: '**Current Model**\n- Provider: `' + (self.currentAgent.model_provider || '?') + '`\n- Model: `' + (self.currentAgent.model_name || '?') + '`', meta: '', tools: [] });
              self.scrollToBottom(true);
            }
          } else {
            self.messages.push({ id: ++msgId, role: 'system', text: 'No agent selected.', meta: '', tools: [] });
            self.scrollToBottom(true);
          }
          break;
        case '/clear':
          self.messages = [];
          break;
        case '/exit':
          OpenFangAPI.wsDisconnect();
          self._wsAgent = null;
          self.currentAgent = null;
          self.messages = [];
          window.dispatchEvent(new Event('close-chat'));
          break;
        case '/budget':
          OpenFangAPI.get('/api/budget').then(function(b) {
            var fmt = function(v) { return v > 0 ? '$' + v.toFixed(2) : 'unlimited'; };
            self.messages.push({ id: ++msgId, role: 'system', text: '**Budget Status**\n' +
              '- Hourly: $' + (b.hourly_spend||0).toFixed(4) + ' / ' + fmt(b.hourly_limit) + '\n' +
              '- Daily: $' + (b.daily_spend||0).toFixed(4) + ' / ' + fmt(b.daily_limit) + '\n' +
              '- Monthly: $' + (b.monthly_spend||0).toFixed(4) + ' / ' + fmt(b.monthly_limit), meta: '', tools: [] });
            self.scrollToBottom(true);
          }).catch(function() {});
          break;
        case '/peers':
          OpenFangAPI.get('/api/network/status').then(function(ns) {
            self.messages.push({ id: ++msgId, role: 'system', text: '**OFP Network**\n' +
              '- Status: ' + (ns.enabled ? 'Enabled' : 'Disabled') + '\n' +
              '- Connected peers: ' + (ns.connected_peers||0) + ' / ' + (ns.total_peers||0), meta: '', tools: [] });
            self.scrollToBottom(true);
          }).catch(function() {});
          break;
        case '/a2a':
          OpenFangAPI.get('/api/a2a/agents').then(function(res) {
            var agents = res.agents || [];
            if (!agents.length) {
              self.messages.push({ id: ++msgId, role: 'system', text: 'No external A2A agents discovered.', meta: '', tools: [] });
            } else {
              var lines = agents.map(function(a) { return '- **' + a.name + '** — ' + a.url; });
              self.messages.push({ id: ++msgId, role: 'system', text: '**A2A Agents (' + agents.length + ')**\n' + lines.join('\n'), meta: '', tools: [] });
            }
            self.scrollToBottom(true);
          }).catch(function() {});
          break;
        case '/t': {
          var tParts = cmdArgs.trim().split(/\s+/);
          var tSub = tParts[0] || '';
          var tName = tParts.slice(1).join(' ').trim();
          if (!tSub || tSub === 'list') {
            self._loadTemplates().then(function(tList) {
              if (!tList.length) {
                self.messages.push({ id: ++msgId, role: 'system', text: 'No templates saved yet.\nUse `/t save <name>` while text is in the input to save one.', meta: '', tools: [] });
              } else {
                var tLines = tList.map(function(t) { return '- **' + t.name + '** — ' + t.text.slice(0, 60) + (t.text.length > 60 ? '…' : ''); });
                self.messages.push({ id: ++msgId, role: 'system', text: '**Saved templates (' + tList.length + ')**\n' + tLines.join('\n') + '\n\nUse `/t <name>` to expand.', meta: '', tools: [] });
              }
              self.scrollToBottom(true);
            });
          } else if (tSub === 'save') {
            if (!tName) {
              self.messages.push({ id: ++msgId, role: 'system', text: 'Usage: `/t save <name>` — saves current input text as a template named <name>.', meta: '', tools: [] });
              self.scrollToBottom(true);
            } else {
              var textToSave = self.inputText.trim();
              if (!textToSave) {
                self.messages.push({ id: ++msgId, role: 'system', text: 'Input is empty — type your template text first, then run `/t save <name>`.', meta: '', tools: [] });
                self.scrollToBottom(true);
              } else {
                self.inputText = '';
                self._saveTemplate(tName, textToSave).then(function() {
                  self.messages.push({ id: ++msgId, role: 'system', text: '✓ Template **' + tName + '** saved.', meta: '', tools: [] });
                  self.scrollToBottom(true);
                });
              }
            }
          } else if (tSub === 'delete' || tSub === 'del' || tSub === 'rm') {
            if (!tName) {
              self.messages.push({ id: ++msgId, role: 'system', text: 'Usage: `/t delete <name>`', meta: '', tools: [] });
              self.scrollToBottom(true);
            } else {
              self._deleteTemplate(tName).then(function(deleted) {
                self.messages.push({ id: ++msgId, role: 'system', text: deleted ? '✓ Template **' + tName + '** deleted.' : 'No template named **' + tName + '** found.', meta: '', tools: [] });
                self.scrollToBottom(true);
              });
            }
          } else {
            // Expand template by name — entire cmdArgs is the name
            var expandName = cmdArgs.trim();
            self._findTemplate(expandName).then(function(found) {
              if (!found) {
                self.messages.push({ id: ++msgId, role: 'system', text: 'No template named **' + expandName + '**. Use `/t list` to see saved templates.', meta: '', tools: [] });
                self.scrollToBottom(true);
              } else {
                self.inputText = found.text;
                self.$nextTick(function() {
                  var el = document.getElementById('msg-input');
                  if (el) { el.focus(); el.setSelectionRange(el.value.length, el.value.length); }
                });
              }
            });
          }
          break;
        }
      }
    },

    /** In-memory template cache — loaded once from server, kept in sync. */
    _templateCache: null,

    /** Load templates from server (lazy, cached). Returns a Promise<Array>. */
    _loadTemplates() {
      var self = this;
      if (self._templateCache !== null) return Promise.resolve(self._templateCache);
      return OpenFangAPI.get('/api/slash-templates').then(function(res) {
        self._templateCache = res.templates || [];
        return self._templateCache;
      }).catch(function() { return []; });
    },

    /** Persist the full template list to server and update cache. */
    _persistTemplates(list) {
      var self = this;
      self._templateCache = list;
      return OpenFangAPI.put('/api/slash-templates', { templates: list }).catch(function(e) {
        OpenFangToast.error('Failed to save templates: ' + (e.message || 'Unknown error'));
      });
    },

    /** Save or overwrite a template by name. Returns a Promise. */
    _saveTemplate(name, text) {
      return this._loadTemplates().then(function(list) {
        var next = list.slice();
        var idx = next.findIndex(function(t) { return t.name.toLowerCase() === name.toLowerCase(); });
        if (idx >= 0) { next[idx] = { name: next[idx].name, text: text }; } else { next.push({ name: name, text: text }); }
        return this._persistTemplates(next);
      }.bind(this));
    },

    /** Find a template by name (case-insensitive). Returns a Promise<entry|null>. */
    _findTemplate(name) {
      return this._loadTemplates().then(function(list) {
        return list.find(function(t) { return t.name.toLowerCase() === name.toLowerCase(); }) || null;
      });
    },

    /** Delete a template by name. Returns a Promise<bool>. */
    _deleteTemplate(name) {
      return this._loadTemplates().then(function(list) {
        var next = list.filter(function(t) { return t.name.toLowerCase() !== name.toLowerCase(); });
        if (next.length === list.length) return false;
        return this._persistTemplates(next).then(function() { return true; });
      }.bind(this));
    },

    selectAgent(agent) {
      try {
        var appStore = Alpine.store('app');
        if (appStore && appStore.premiumWalletInteractionsBlocked && appStore.premiumWalletInteractionsBlocked(agent)) {
          if (appStore.ensurePremiumAgentAccess) appStore.ensurePremiumAgentAccess(agent);
          return;
        }
      } catch (eGate) { /* ignore */ }

      // Snapshot the current agent's messages before switching so we can restore
      // them instantly if the user comes back (avoids blank-screen round-trips).
      if (this.currentAgent && this.currentAgent.id && this.messages.length) {
        _agentMsgCache[this.currentAgent.id] = this.messages.slice();
      }

      // Reset in-flight UI so switching agents mid-stream cannot leave a stuck "Generating" state
      this.sending = false;
      this.messageQueue = [];
      this._wsInFlightMsg = null;
      if (this._wsReconnectTimer) { clearTimeout(this._wsReconnectTimer); this._wsReconnectTimer = null; }
      this._clearTypingTimeout();
      this._stopElapsed();
      this._chatPinnedToBottom = true;
      this._wsAgent = null;
      this.currentAgent = agent;
      this.sessionPromptTokens = 0;
      this.sessionCompletionTokens = 0;
      this.lastTurnWallMs = null;
      this._clearStreamErrorTelemetry();
      this._clearRuntimeStatusTelemetry();

      // Restore from cache immediately (synchronous) — loadSession will refresh in background
      var cached = _agentMsgCache[agent.id];
      this.messages = cached ? cached.slice() : [];

      this.connectWs(agent.id);
      this.refreshChatNetworkHints();
      // Show welcome tips on first use
      if (!localStorage.getItem('of-chat-tips-seen')) {
        var localMsgId = 0;
        this.messages.push({
          id: ++localMsgId,
          role: 'system',
          text: '**Welcome to ArmaraOS chat**\n\n' +
            '- Type `/` to see available commands\n' +
            '- `/help` shows all commands\n' +
            '- `/think on` enables extended reasoning\n' +
            '- `/context` shows context window usage\n' +
            '- `/verbose off` hides tool details\n' +
            '- `Ctrl+Shift+F` toggles focus mode\n' +
            '- Drag & drop files to attach them\n' +
            '- `Ctrl+/` opens the command palette',
          meta: '',
          tools: []
        });
        localStorage.setItem('of-chat-tips-seen', 'true');
      }
      // Focus input after agent selection
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('msg-input');
        if (el) el.focus();
      });
    },

    /**
     * Map a single server-shaped message (`{role, content, tools?, images?}`)
     * into the local UI shape the chat renderer expects. Shared by both the
     * initial latest-page load and the scroll-up prepend so historical
     * tool-cards and embedded images look identical regardless of how/when
     * they arrived.
     */
    _mapServerMessage(m) {
      var self = this;
      var role = m.role === 'User' ? 'user' : (m.role === 'System' ? 'system' : 'agent');
      var text = typeof m.content === 'string' ? m.content : JSON.stringify(m.content);
      text = self.sanitizeToolText(text);
      var tools = (m.tools || []).map(function(t, idx) {
        var card = {
          id: (t.name || 'tool') + '-hist-' + idx,
          name: t.name || 'unknown',
          running: false,
          expanded: false,
          input: t.input || '',
          result: t.result || '',
          is_error: !!t.is_error
        };
        card.expanded = self.shouldAutoExpandTool(card);
        return card;
      });
      var images = (m.images || []).map(function(img) {
        return { file_id: img.file_id, filename: img.filename || 'image' };
      });
      return { id: ++msgId, role: role, text: text, meta: '', tools: tools, images: images };
    },

    async loadSession(agentId) {
      var self = this;
      // Bump the per-agent version so any in-flight older-history fetches
      // for the *same* agent that complete after this call are discarded.
      var prevState = _agentSessionState[agentId];
      var version = ((prevState && prevState.version) || 0) + 1;
      _agentSessionState[agentId] = {
        oldestIndex: 0,
        hasMore: false,
        totalVisible: 0,
        loadingOlder: false,
        version: version
      };
      try {
        // Latest page only on first load. Server returns the tail of the
        // session when `before` is omitted; `limit` clamps the page size.
        var url = '/api/agents/' + agentId + '/session?limit=' + CHAT_HISTORY_PAGE_SIZE;
        var data = await OpenFangAPI.get(url);
        // Only replace messages from the server when the agent isn't currently
        // streaming — we don't want to clobber an in-progress live turn.
        if (self.sending) return;
        // Guard: don't apply stale load to a different agent (user switched while request was in flight)
        if (!self.currentAgent || self.currentAgent.id !== agentId) return;
        // Guard: another loadSession started after us (rapid agent toggle)
        var stateNow = _agentSessionState[agentId];
        if (!stateNow || stateNow.version !== version) return;

        var totalVisible = (typeof data.total_visible_messages === 'number')
          ? data.total_visible_messages
          : ((data.messages || []).length);
        var oldestIndex = (typeof data.oldest_index === 'number')
          ? data.oldest_index
          : 0;
        var hasMore = !!data.has_more;

        if (data.messages && data.messages.length) {
          var mapped = data.messages.map(function(m) { return self._mapServerMessage(m); });
          self.messages = mapped;
          _agentMsgCache[agentId] = mapped.slice();
          self.$nextTick(function() { self.scrollToBottom(true); });
        } else if (!self.messages.length) {
          delete _agentMsgCache[agentId];
        }

        _agentSessionState[agentId] = {
          oldestIndex: oldestIndex,
          hasMore: hasMore,
          totalVisible: totalVisible,
          loadingOlder: false,
          version: version
        };
      } catch(e) {
        if (self.currentAgent && self.currentAgent.id === agentId && !self.messages.length) {
          self.messages.push({ id: ++msgId, role: 'system', text: '⚠ Could not load chat history. ' + (e && e.message ? e.message : ''), meta: '', tools: [] });
        }
      }
    },

    /**
     * Lazy-load: prepend the next batch of older messages when the user
     * scrolls near the top of the chat scroll container. Idempotent + race-
     * safe: the version field on _agentSessionState ensures a stale fetch
     * (because the user switched agents mid-request) does not pollute the
     * new agent's view.
     *
     * Scroll preservation: we measure scrollHeight before the prepend and
     * restore (newScrollHeight - oldScrollHeight + oldScrollTop) afterwards
     * so the message the user was reading stays anchored under their cursor.
     */
    async loadOlderMessages() {
      var self = this;
      if (!self.currentAgent) return;
      var agentId = self.currentAgent.id;
      var state = _agentSessionState[agentId];
      if (!state) return;
      if (!state.hasMore) return;
      if (state.loadingOlder) return;
      if (state.oldestIndex <= 0) return;
      // Capture version up front; if it changes during the await, we drop the
      // result on the floor (user has switched agents or restarted history).
      var version = state.version;
      state.loadingOlder = true;
      var el = document.getElementById('messages');
      var prevScrollHeight = el ? el.scrollHeight : 0;
      var prevScrollTop = el ? el.scrollTop : 0;
      try {
        var url = '/api/agents/' + agentId + '/session?limit=' + CHAT_HISTORY_PAGE_SIZE
          + '&before=' + state.oldestIndex;
        var data = await OpenFangAPI.get(url);
        var stateAfter = _agentSessionState[agentId];
        if (!stateAfter || stateAfter.version !== version) return;
        if (!self.currentAgent || self.currentAgent.id !== agentId) return;
        var batch = (data.messages || []).map(function(m) { return self._mapServerMessage(m); });
        if (batch.length) {
          self.messages = batch.concat(self.messages);
          _agentMsgCache[agentId] = self.messages.slice();
          // Restore scroll anchor on the NEXT animation frame so Alpine has
          // rendered the prepended nodes and scrollHeight reflects them.
          self.$nextTick(function() {
            var el2 = document.getElementById('messages');
            if (!el2) return;
            var delta = el2.scrollHeight - prevScrollHeight;
            // Stay pinned to whatever the user was reading; don't jump to top.
            el2.scrollTop = prevScrollTop + delta;
            self._chatPinnedToBottom = false;
          });
        }
        var newOldest = (typeof data.oldest_index === 'number')
          ? data.oldest_index
          : Math.max(0, stateAfter.oldestIndex - batch.length);
        var hasMore = !!data.has_more;
        var totalVisible = (typeof data.total_visible_messages === 'number')
          ? data.total_visible_messages
          : stateAfter.totalVisible;
        _agentSessionState[agentId] = {
          oldestIndex: newOldest,
          hasMore: hasMore,
          totalVisible: totalVisible,
          loadingOlder: false,
          version: version
        };
      } catch(e) {
        var s = _agentSessionState[agentId];
        if (s && s.version === version) s.loadingOlder = false;
      }
    },

    // Multi-session: load session list for current agent
    async loadSessions(agentId) {
      try {
        var data = await OpenFangAPI.get('/api/agents/' + agentId + '/sessions');
        this.sessions = data.sessions || [];
      } catch(e) { this.sessions = []; }
    },

    // Multi-session: create a new session
    async createSession() {
      if (!this.currentAgent) return;
      var label = prompt('Session name (optional):');
      if (label === null) return; // cancelled
      try {
        await OpenFangAPI.post('/api/agents/' + this.currentAgent.id + '/sessions', {
          label: label.trim() || undefined
        });
        await this.loadSessions(this.currentAgent.id);
        await this.loadSession(this.currentAgent.id);
        this.messages = [];
        this.scrollToBottom(true);
        this._wsAgent = null;
        OpenFangAPI.wsDisconnect();
        this.connectWs(this.currentAgent.id);
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('New session created');
      } catch(e) {
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Failed to create session');
      }
    },

    // Multi-session: switch to an existing session
    async switchSession(sessionId) {
      if (!this.currentAgent) return;
      try {
        await OpenFangAPI.post('/api/agents/' + this.currentAgent.id + '/sessions/' + sessionId + '/switch', {});
        this.messages = [];
        this.sending = false;
        this._wsInFlightMsg = null;
        if (this._wsReconnectTimer) { clearTimeout(this._wsReconnectTimer); this._wsReconnectTimer = null; }
        await this.loadSession(this.currentAgent.id);
        await this.loadSessions(this.currentAgent.id);
        // Reconnect WebSocket for new session (cannot reuse socket: session binding on server)
        this._wsAgent = null;
        OpenFangAPI.wsDisconnect();
        this.connectWs(this.currentAgent.id);
      } catch(e) {
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Failed to switch session');
      }
    },

    connectWs(agentId) {
      var self = this;
      var idStr = String(agentId);
      this._wsAgent = idStr;
      if (self._wsReconnectTimer) { clearTimeout(self._wsReconnectTimer); self._wsReconnectTimer = null; }

      OpenFangAPI.wsConnect(idStr, {
        onOpen: function() {
          Alpine.store('app').wsConnected = true;
          // If we dropped mid-stream and have an in-flight message, retry it now
          if (self._wsInFlightMsg) {
            var pending = self._wsInFlightMsg;
            self._wsInFlightMsg = null;
            self.messages = self.messages.filter(function(m) { return !m.thinking && !m.streaming; });
            self.messages.push({ id: ++msgId, role: 'system', text: 'Reconnected — retrying your message...', meta: '', tools: [], ts: Date.now() });
            self.sending = false;
            self.$nextTick(function() { self._sendPayload(pending.text, pending.files, pending.images); });
          }
        },
        onMessage: function(data) { self.handleWsMessage(data); },
        onClose: function() {
          Alpine.store('app').wsConnected = false;
          // If the socket closed while we were waiting for a response, surface it
          if (self.sending) {
            self._clearTypingTimeout();
            self._stopElapsed();
            // Save in-flight message so onOpen can retry
            var lastUser = null;
            for (var i = self.messages.length - 1; i >= 0; i--) {
              if (self.messages[i].role === 'user') { lastUser = self.messages[i]; break; }
            }
            if (lastUser && !self._wsInFlightMsg) {
              self._wsInFlightMsg = { text: lastUser.text, files: [], images: lastUser.images || [] };
            }
            self.messages = self.messages.filter(function(m) { return !m.thinking && !m.streaming; });
            self.messages.push({ id: ++msgId, role: 'system', text: 'Connection lost while waiting for a response. Reconnecting...', meta: '', tools: [], ts: Date.now() });
            self.sending = false;
            self.scrollToBottom(true);
          }
          // Schedule auto-reconnect after a short delay
          if (self._wsAgent) {
            if (self._wsReconnectTimer) clearTimeout(self._wsReconnectTimer);
            self._wsReconnectTimer = setTimeout(function() {
              if (self._wsAgent) { self.connectWs(self._wsAgent); }
            }, 3000);
          } else {
            self._wsAgent = null;
          }
        },
        onError: function() {
          Alpine.store('app').wsConnected = false;
          if (self.sending) {
            self._clearTypingTimeout();
            self._stopElapsed();
            self.messages = self.messages.filter(function(m) { return !m.thinking && !m.streaming; });
            self.messages.push({ id: ++msgId, role: 'system', text: 'Connection error. Check that the ArmaraOS daemon is running, then try again.', meta: '', tools: [], ts: Date.now() });
            self.sending = false;
            self.scrollToBottom(true);
          }
        }
      });
    },

    handleWsMessage(data) {
      try {
      switch (data.type) {
        case 'connected': break;

        // Legacy thinking event (backward compat)
        case 'thinking':
          if (!this.messages.length || !this.messages[this.messages.length - 1].thinking) {
            var thinkLabel = data.level ? 'Thinking (' + data.level + ')...' : 'Processing...';
            this.messages.push({ id: ++msgId, role: 'agent', text: thinkLabel, meta: '', thinking: true, streaming: true, tools: [] });
            this.scrollToBottom();
            this._resetTypingTimeout();
          } else if (data.level) {
            var thinkIdx = this.messages.length - 1;
            var lastThink = thinkIdx >= 0 ? this.messages[thinkIdx] : null;
            if (lastThink && lastThink.thinking) {
              lastThink.text = 'Thinking (' + data.level + ')...';
              this.messages.splice(thinkIdx, 1, lastThink);
            }
          }
          break;

        // New typing lifecycle
        case 'typing':
          if (data.state === 'start') {
            if (!this.messages.length || !this.messages[this.messages.length - 1].thinking) {
              this.messages.push({ id: ++msgId, role: 'agent', text: 'Processing...', meta: '', thinking: true, streaming: true, tools: [] });
              this.scrollToBottom();
            }
            this._resetTypingTimeout();
            this._startLlmWait(); // LLM call starting — begin timing
          } else if (data.state === 'tool') {
            this._stopLlmWait(); // tool event = LLM responded; stop the wait counter
            var toolTypIdx = this.messages.length - 1;
            var typingMsg = toolTypIdx >= 0 ? this.messages[toolTypIdx] : null;
            if (typingMsg && (typingMsg.thinking || typingMsg.streaming)) {
              typingMsg.text = 'Using ' + (data.tool || 'tool') + '...';
              this.messages.splice(toolTypIdx, 1, typingMsg);
            }
            this._resetTypingTimeout();
          } else if (data.state === 'stop') {
            this._clearTypingTimeout();
            this._stopLlmWait();
          }
          break;

        case 'phase':
          // Any phase event means the server is still alive — reset the watchdog
          this._resetTypingTimeout();
          // iteration phase = agent loop entered a new LLM call; start the wait counter
          if (data.phase === 'iteration' || data.phase === 'thinking') {
            this._startLlmWait();
          }
          var phaseIdx = this.messages.length - 1;
          var phaseMsg = phaseIdx >= 0 ? this.messages[phaseIdx] : null;
          if (phaseMsg && (phaseMsg.thinking || phaseMsg.streaming)) {
            // Skip phases that have no user-meaningful display text — "streaming"
            // and "done" are lifecycle signals, not status to show in the chat bubble.
            if (data.phase === 'streaming' || data.phase === 'done') {
              break;
            }
            // Context warning: show prominently as a separate system message
            if (data.phase === 'context_warning') {
              var cwDetail = data.detail || 'Context limit reached.';
              this.messages.push({ id: ++msgId, role: 'system', text: cwDetail, meta: '', tools: [] });
            } else if (data.phase === 'iteration') {
              // Multi-step progress: extract step number and update the thinking bubble
              var iterMatch = (data.detail || '').match(/Step\s+(\d+)/i);
              if (iterMatch) this.currentIteration = parseInt(iterMatch[1], 10);
              var iterLine = data.detail || ('Step ' + this.currentIteration + ' — thinking…');
              if (phaseMsg.thinking) {
                phaseMsg.text = iterLine;
                this.messages.splice(phaseIdx, 1, phaseMsg);
              } else if (phaseMsg.streaming && phaseMsg.role === 'agent') {
                // After the first text token, thinking is false — keep status in phaseStatus so we do not wipe streamed body text.
                phaseMsg.phaseStatus = iterLine;
                this.messages.splice(phaseIdx, 1, phaseMsg);
              }
            } else if (data.phase === 'thinking' && this.thinkingMode === 'stream' && phaseMsg.thinking) {
              // Stream reasoning tokens to a collapsible panel
              if (!phaseMsg._reasoning) phaseMsg._reasoning = '';
              phaseMsg._reasoning += (data.detail || '') + '\n';
              phaseMsg.text = '<details><summary>Reasoning...</summary>\n\n' + phaseMsg._reasoning + '</details>';
              this.messages.splice(phaseIdx, 1, phaseMsg);
            } else {
              var phaseDetail;
              if (data.phase === 'tool_use') {
                phaseDetail = 'Using ' + (data.detail || 'tool') + '...';
              } else if (data.phase === 'thinking') {
                phaseDetail = 'Thinking...';
              } else {
                phaseDetail = data.detail || 'Working...';
              }
              if (phaseMsg.thinking) {
                phaseMsg.text = phaseDetail;
                this.messages.splice(phaseIdx, 1, phaseMsg);
              } else if (phaseMsg.streaming && phaseMsg.role === 'agent') {
                phaseMsg.phaseStatus = phaseDetail;
                this.messages.splice(phaseIdx, 1, phaseMsg);
              }
            }
          }
          this.scrollToBottom();
          break;

        case 'text_delta':
          // Streaming tokens are the most frequent proof of life — reset watchdog
          this._resetTypingTimeout();
          this._stopLlmWait(); // first token arrived — LLM responded
          var lastIdx = this.messages.length - 1;
          var last = lastIdx >= 0 ? this.messages[lastIdx] : null;
          if (last && last.streaming) {
            if (last.thinking) { last.text = ''; last.thinking = false; }
            // If we already detected a text-based tool call, skip further text
            if (last._toolTextDetected) break;
            last.text += data.content;
            // Detect function-call patterns streamed as text and convert to tool cards
            var fcIdx = last.text.search(/\w+<\/function[=,>]/);
            if (fcIdx === -1) fcIdx = last.text.search(/<function=\w+>/);
            if (fcIdx !== -1) {
              var fcPart = last.text.substring(fcIdx);
              var toolMatch = fcPart.match(/^(\w+)<\/function/) || fcPart.match(/^<function=(\w+)>/);
              last.text = last.text.substring(0, fcIdx).trim();
              last._toolTextDetected = true;
              if (toolMatch) {
                if (!last.tools) last.tools = [];
                var inputMatch = fcPart.match(/[=,>]\s*(\{[\s\S]*)/);
                last.tools.push({
                  id: toolMatch[1] + '-txt-' + Date.now(),
                  name: toolMatch[1],
                  running: true,
                  expanded: false,
                  input: inputMatch ? inputMatch[1].replace(/<\/function>?\s*$/, '').trim() : '',
                  result: '',
                  is_error: false
                });
              }
            }
            this.tokenCount = Math.round(last.text.length / 4);
            // Force Alpine reactivity: splice-in-place so x-for re-renders
            // this item. Direct property mutation on array elements may not
            // trigger DOM updates from async WebSocket callbacks.
            this.messages.splice(lastIdx, 1, last);
          } else {
            this.messages.push({ id: ++msgId, role: 'agent', text: data.content, meta: '', streaming: true, tools: [] });
          }
          this.scrollToBottom();
          break;

        case 'tool_start':
          this._resetTypingTimeout(); // LLM responded with a tool call — still live
          this._stopLlmWait(); // tool call = LLM answered; wait counter off until next iteration
          var tsIdx = this.messages.length - 1;
          var lastMsg = tsIdx >= 0 ? this.messages[tsIdx] : null;
          if (lastMsg && lastMsg.streaming) {
            if (!lastMsg.tools) lastMsg.tools = [];
            lastMsg.tools.push({ id: data.tool + '-' + Date.now(), name: data.tool, running: true, expanded: false, input: '', result: '', is_error: false });
            this.messages.splice(tsIdx, 1, lastMsg);
          }
          this.scrollToBottom();
          break;

        case 'tool_end':
          // Tool call parsed by LLM — update tool card with input params
          var teIdx = this.messages.length - 1;
          var lastMsg2 = teIdx >= 0 ? this.messages[teIdx] : null;
          if (lastMsg2 && lastMsg2.tools) {
            for (var ti = lastMsg2.tools.length - 1; ti >= 0; ti--) {
              if (lastMsg2.tools[ti].name === data.tool && lastMsg2.tools[ti].running) {
                lastMsg2.tools[ti].input = data.input || '';
                break;
              }
            }
            this.messages.splice(teIdx, 1, lastMsg2);
          }
          break;

        case 'tool_result':
          // Tool execution completed — reset watchdog and start timing the next LLM call
          this._resetTypingTimeout();
          this._startLlmWait();
          var trIdx = this.messages.length - 1;
          var lastMsg3 = trIdx >= 0 ? this.messages[trIdx] : null;
          if (lastMsg3 && lastMsg3.tools) {
            for (var ri = lastMsg3.tools.length - 1; ri >= 0; ri--) {
              if (lastMsg3.tools[ri].name === data.tool && lastMsg3.tools[ri].running) {
                lastMsg3.tools[ri].running = false;
                lastMsg3.tools[ri].result = data.result || '';
                lastMsg3.tools[ri].is_error = !!data.is_error;
                lastMsg3.tools[ri].expanded = this.shouldAutoExpandTool(lastMsg3.tools[ri]);
                // Extract image URLs from image_generate or browser_screenshot results
                if ((data.tool === 'image_generate' || data.tool === 'browser_screenshot') && !data.is_error) {
                  try {
                    var parsed = JSON.parse(data.result);
                    if (parsed.image_urls && parsed.image_urls.length) {
                      lastMsg3.tools[ri]._imageUrls = parsed.image_urls;
                    }
                  } catch(e) { /* not JSON */ }
                }
                // Extract audio file path and URL from text_to_speech results
                if (data.tool === 'text_to_speech' && !data.is_error) {
                  try {
                    var ttsResult = JSON.parse(data.result);
                    if (ttsResult.saved_to) {
                      lastMsg3.tools[ri]._audioFile = ttsResult.saved_to;
                      lastMsg3.tools[ri]._audioDuration = ttsResult.duration_estimate_ms;
                    }
                    if (ttsResult.audio_url) {
                      lastMsg3.tools[ri]._audioUrl = ttsResult.audio_url;
                    }
                  } catch(e) { /* not JSON */ }
                }
                break;
              }
            }
            this.messages.splice(trIdx, 1, lastMsg3);
          }
          // Cache after each tool result so mid-run state survives navigation
          if (this.currentAgent) _agentMsgCache[this.currentAgent.id] = this.messages.slice();
          this.scrollToBottom();
          break;

        case 'ainl_runtime_telemetry':
          this._applyAinlRuntimeTelemetry(data.telemetry, 'ws');
          break;

        case 'response':
          var responseSelf = this;
          this._clearTypingTimeout();
          this._stopElapsed();
          this._wsInFlightMsg = null;  // response received — no retry needed
          // Update context pressure from response
          if (data.context_pressure) {
            this.contextPressure = data.context_pressure;
          }
          // Collect streamed text before removing streaming messages
          var streamedText = '';
          var streamedTools = [];
          this.messages.forEach(function(m) {
            if (m.streaming && !m.thinking && m.role === 'agent') {
              streamedText += m.text || '';
              streamedTools = streamedTools.concat(m.tools || []);
            }
          });
          streamedTools.forEach(function(t) {
            t.running = false;
            // Text-detected tool calls (model leaked as text) — mark as not executed
            if (t.id && t.id.indexOf('-txt-') !== -1 && !t.result) {
              t.result = 'Model attempted this call as text (not executed via tool system)';
              t.is_error = true;
            }
            t.expanded = responseSelf.shouldAutoExpandTool(t);
          });
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          if (data.cost_usd != null) this.sessionCostUsd += data.cost_usd;
          if (data.turn_wall_ms != null) this.lastTurnWallMs = data.turn_wall_ms;
          if (data.input_tokens != null) this.sessionPromptTokens += data.input_tokens;
          if (data.output_tokens != null) this.sessionCompletionTokens += data.output_tokens;
          this._clearStreamErrorTelemetry();
          var meta = (data.input_tokens || 0) + ' in / ' + (data.output_tokens || 0) + ' out';
          if (data.cost_usd != null) meta += ' | $' + data.cost_usd.toFixed(4);
          if (data.iterations) meta += ' | ' + data.iterations + ' iter';
          if (data.fallback_model) meta += ' | fallback: ' + data.fallback_model;
          if (data.skill_draft_path) meta += ' | skill draft: ' + data.skill_draft_path;
          if (responseSelf.efficientMode && responseSelf.efficientMode !== 'off') {
            var pct = data.compression_savings_pct;
            meta += pct > 0 ? ' | ⚡ eco ↓' + pct + '%' : ' | ⚡ eco';
          }
          meta += this._buildEcoMetaSuffix(data);
          if (data.turn_outcome) meta += ' | outcome: ' + data.turn_outcome;
          var ecoTip = this._buildEcoMetaTooltip(data);
          // Use server response if non-empty, otherwise preserve accumulated streamed text
          var finalText = (data.content && data.content.trim()) ? data.content : streamedText;
          // Strip raw function-call JSON that some models leak as text
          finalText = this.sanitizeToolText(finalText);
          // If text is empty but tools ran, provide a clear summary rather than a blank bubble
          if (!finalText.trim() && streamedTools.length) {
            var toolNames = streamedTools.map(function(t) { return '`' + t.name + '`'; });
            var allOk = streamedTools.every(function(t) { return !t.is_error; });
            finalText = allOk
              ? 'Done. Ran ' + toolNames.join(', ') + '.'
              : 'Finished with errors. Ran ' + toolNames.join(', ') + '. See tool details above.';
          }
          // If text is still empty with no tools (e.g. model returned nothing useful), say so
          if (!finalText.trim()) {
            finalText = '*(The agent returned no text. If you were expecting a response, try again or check /status.)*';
          }
          var wsCompressedInput = (data.compressed_input && data.compression_savings_pct > 0) ? data.compressed_input : null;
          this._recordEcoSaving(data.compression_savings_pct);
          this.messages.push({ id: ++msgId, role: 'agent', text: finalText, meta: meta, tools: streamedTools, ts: Date.now(),
            compressedInput: wsCompressedInput, originalInput: wsCompressedInput ? (responseSelf._lastSentOriginal || '') : null,
            savingsPct: data.compression_savings_pct || 0, ecoMetaTooltip: ecoTip || null });
          this._attachVoiceReply(this.messages[this.messages.length - 1], data);
          this._updateMemoryAppliedIndicatorForTurn();
          // Snapshot to cache so switching away and back shows the complete turn instantly
          if (this.currentAgent) _agentMsgCache[this.currentAgent.id] = this.messages.slice();
          this.sending = false;
          this.tokenCount = 0;
          this.scrollToBottom();
          var self3 = this;
          this.$nextTick(function() {
            var el = document.getElementById('msg-input'); if (el) el.focus();
            self3._processQueue();
          });
          break;

        case 'silent_complete':
          // Agent intentionally chose not to reply (NO_REPLY / silent completion)
          this._clearTypingTimeout();
          this._stopElapsed();
          this._wsInFlightMsg = null;
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          this.sending = false;
          this.tokenCount = 0;
          // Show a subtle system note so the user knows something happened
          if (data.cost_usd != null) this.sessionCostUsd += data.cost_usd;
          if (data.turn_wall_ms != null) this.lastTurnWallMs = data.turn_wall_ms;
          if (data.input_tokens != null) this.sessionPromptTokens += data.input_tokens;
          if (data.output_tokens != null) this.sessionCompletionTokens += data.output_tokens;
          this._clearStreamErrorTelemetry();
          var silentMeta = (data.input_tokens || 0) + ' in / ' + (data.output_tokens || 0) + ' out';
          if (data.cost_usd != null) silentMeta += ' | $' + data.cost_usd.toFixed(4);
          if (self.efficientMode && self.efficientMode !== 'off') {
            var sPct = data.compression_savings_pct;
            silentMeta += sPct > 0 ? ' | ⚡ eco ↓' + sPct + '%' : ' | ⚡ eco';
          }
          silentMeta += this._buildEcoMetaSuffix(data);
          if (data.turn_outcome) silentMeta += ' | outcome: ' + data.turn_outcome;
          var silentEcoTip = this._buildEcoMetaTooltip(data);
          this.messages.push({ id: ++msgId, role: 'system', text: '*(No reply — agent processed the message but determined no response was needed.)*', meta: silentMeta, tools: [], ts: Date.now(), ecoMetaTooltip: silentEcoTip || null });
          if (this.currentAgent) _agentMsgCache[this.currentAgent.id] = this.messages.slice();
          this.scrollToBottom();
          if (data.skill_draft_path) {
            OpenFangToast.success('Skill draft saved: ' + data.skill_draft_path);
          }
          var selfSilent = this;
          this.$nextTick(function() { selfSilent._processQueue(); });
          break;

        case 'error':
          this._clearTypingTimeout();
          this._stopElapsed();
          this._wsInFlightMsg = null;  // don't retry on explicit error from server
          this.messages = this.messages.filter(function(m) { return !m.thinking && !m.streaming; });
          var rawErr = typeof data.content === 'string' ? data.content : JSON.stringify(data.content || '');
          var friendly = this._applyFriendlyError(rawErr);
          var errBody = friendly.text;
          if (this.networkHints && this.networkHints.likely_vpn) {
            errBody += '\n\nIf this keeps happening: a VPN or firewall may block the AI provider. Try split tunneling or allowlisting.';
          }
          this.messages.push({ id: ++msgId, role: 'system', text: errBody, meta: '', tools: [], ts: Date.now() });
          this.sending = false;
          this.tokenCount = 0;
          this.scrollToBottom(true);
          var self2 = this;
          this.$nextTick(function() {
            var el = document.getElementById('msg-input'); if (el) el.focus();
            self2._processQueue();
          });
          break;

        case 'agents_updated':
          if (data.agents) {
            Alpine.store('app').agents = data.agents;
            Alpine.store('app').agentCount = data.agents.length;
          }
          break;

        case 'command_result':
          // Update context pressure if included in command result
          if (data.context_pressure) {
            this.contextPressure = data.context_pressure;
          }
          this.messages.push({ id: ++msgId, role: 'system', text: data.message || 'Command executed.', meta: '', tools: [] });
          this.scrollToBottom(true);
          break;

        case 'canvas':
          // Agent presented an interactive canvas — render it in an iframe sandbox
          var canvasHtml = '<div class="canvas-panel" style="border:1px solid var(--border);border-radius:8px;margin:8px 0;overflow:hidden;">';
          canvasHtml += '<div style="padding:6px 12px;background:var(--surface);border-bottom:1px solid var(--border);font-size:0.85em;display:flex;justify-content:space-between;align-items:center;">';
          canvasHtml += '<span>' + (data.title || 'Canvas') + '</span>';
          canvasHtml += '<span style="opacity:0.5;font-size:0.8em;">' + (data.canvas_id || '').substring(0, 8) + '</span></div>';
          canvasHtml += '<iframe sandbox="allow-scripts" srcdoc="' + (data.html || '').replace(/"/g, '&quot;') + '" ';
          canvasHtml += 'style="width:100%;min-height:300px;border:none;background:#fff;" loading="lazy"></iframe></div>';
          this.messages.push({ id: ++msgId, role: 'agent', text: canvasHtml, meta: 'canvas', isHtml: true, tools: [] });
          this.scrollToBottom();
          break;

        case 'pong': break;
      }
      } catch (err) {
        console.warn('[ArmaraOS chat] WebSocket handler error:', err);
      }
    },

    // Format timestamp for display
    formatTime: function(ts) {
      if (!ts) return '';
      var d = new Date(ts);
      var h = d.getHours();
      var m = d.getMinutes();
      var ampm = h >= 12 ? 'PM' : 'AM';
      h = h % 12 || 12;
      return h + ':' + (m < 10 ? '0' : '') + m + ' ' + ampm;
    },

    /**
     * Parse scheduler inbox messages from the kernel:
     * - v2: `<<<ARMARAOS_SCHEDULER_V2>>>` + JSON meta line + `<<<SCHEDULER_OUTPUT>>>` + body
     * - v1: `[Scheduler] job_name @ ISO8601` then `\\n\\n` + body (legacy)
     * Returns null for normal chat. See `append_cron_output_to_agent_session` in the kernel.
     */
    parseSchedulerEnvelopeV2: function(text) {
      var prefix = '<<<ARMARAOS_SCHEDULER_V2>>>\n';
      if (!text || text.indexOf(prefix) !== 0) return null;
      var rest = text.slice(prefix.length);
      var outMark = '\n<<<SCHEDULER_OUTPUT>>>\n';
      var idx = rest.indexOf(outMark);
      if (idx === -1) return null;
      var metaLine = rest.slice(0, idx).trim();
      var output = rest.slice(idx + outMark.length);
      try {
        var meta = JSON.parse(metaLine);
        if (meta.v !== 2) return null;
        var om = meta.output_mode === 'json' ? 'json' : 'markdown';
        return {
          version: 2,
          jobName: (meta.job_name || '').trim() || 'Scheduled job',
          atIso: (meta.ran_at || '').trim(),
          body: output,
          program: (meta.program || '').trim(),
          jobId: (meta.job_id || '').trim(),
          outputMode: om,
        };
      } catch (e) {
        return null;
      }
    },

    inferSchedulerV1OutputMode: function(body) {
      var t = (body || '').trim();
      if (!t) return 'markdown';
      var last = t[t.length - 1];
      if ((t[0] === '{' && last === '}') || (t[0] === '[' && last === ']')) {
        try {
          JSON.parse(t);
          return 'json';
        } catch (e) {}
      }
      return 'markdown';
    },

    schedulerParts: function(msg) {
      if (!msg || msg.role !== 'agent' || msg.thinking || msg.isHtml || !msg.text) return null;
      var t = msg.text.trim();
      var v2 = this.parseSchedulerEnvelopeV2(t);
      if (v2) return v2;
      var sep = t.indexOf('\n\n');
      var head = sep === -1 ? t : t.slice(0, sep);
      var body = sep === -1 ? '' : t.slice(sep + 2);
      var hm = head.match(/^\[Scheduler\]\s+(.+)\s+@\s+(.+)$/);
      if (!hm) return null;
      var om = this.inferSchedulerV1OutputMode(body);
      return {
        version: 1,
        jobName: hm[1].trim(),
        atIso: hm[2].trim(),
        body: body,
        program: '',
        jobId: '',
        outputMode: om,
      };
    },

    formatSchedulerAt: function(iso) {
      if (!iso) return '';
      try {
        var d = new Date(iso);
        if (isNaN(d.getTime())) return iso;
        return d.toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
      } catch (e) {
        return iso;
      }
    },

    shortSchedulerJobId: function(id) {
      if (!id) return '';
      if (id.length <= 14) return id;
      return id.slice(0, 8) + '…';
    },

    schedulerBodyHtml: function(msg) {
      var p = this.schedulerParts(msg);
      if (!p || !p.body) return '';
      var mode = p.outputMode || 'markdown';
      if (mode === 'json') {
        var pretty = p.body;
        try {
          pretty = JSON.stringify(JSON.parse(p.body.trim()), null, 2);
        } catch (e) {}
        return '<pre class="message-scheduler-pre">' + escapeHtml(pretty) + '</pre>';
      }
      return '<div class="markdown-body message-scheduler-md">' + this.renderMarkdown(p.body) + '</div>';
    },

    /**
     * Strip server-injected voice/STT scaffolding from **display** only. The full string is still
     * stored on `msg.text` (canonical session / LLM turn); the UI hides `[ARMAVOS_*]` blocks so
     * users see their words (or a short “Recording...” line) instead of UUID / tool boilerplate.
     */
    stripVoiceAgentUiNoise: function(text) {
      if (!text) return '';
      var t = String(text);
      t = t.replace(/\[ARMAVOS_VOICE_CONTEXT\][\s\S]*?\[\/ARMAVOS_VOICE_CONTEXT\]/g, '');
      t = t.replace(/\[ARMAVOS_VOICE_POLICY\][\s\S]*?\[\/ARMAVOS_VOICE_POLICY\]/g, '');
      t = t.replace(
        /\n{1,}\*\*Voice\/audio attachments\*\*[\s\S]*?Do not ask the user to paste a UUID\./g,
        ''
      );
      return t.replace(/\n{3,}/g, '\n\n').trim();
    },

    /** True when the visible text is only the client/server “call media_transcribe” hint (no real words yet). */
    isVoiceTranscribePlaceholder: function(s) {
      var t = (s || '').trim();
      if (!t) return true;
      return /^\[Voice message/i.test(t) && /media_transcribe/i.test(t) && /file_id/i.test(t);
    },

    /** Main chat bubble HTML (shared so tool-call turns can render reply below tool cards without duplicating logic). */
    formatChatMessageHtml: function(msg) {
      if (!msg || !msg.text) return '';
      // User turns: hide voice pipeline markers by default; keep full text in a collapsed <details>
      // (same spirit as tool cards — technical payload available on demand).
      if (msg.role === 'user' && !msg.isHtml && !msg.thinking) {
        var raw = msg.text;
        var stripped = this.stripVoiceAgentUiNoise(raw);
        var visible = stripped;
        if (this.isVoiceTranscribePlaceholder(visible)) visible = '';
        var esc = this.escapeHtml(visible);
        if (this.searchQuery && this.searchQuery.trim()) esc = this.highlightSearch(esc);
        // Human-facing body: transcript or a tiny recording hint — never the UUID wall (styles: components.css).
        var bodyHtml = visible
          ? '<div class="chat-user-voice-body">' + esc + '</div>'
          : '<p class="chat-user-voice-recording">Recording...</p>';
        var showVoiceDetails =
          stripped.trim() !== String(raw).trim() || (!visible && String(raw).trim().length > 0);
        if (!showVoiceDetails) return bodyHtml;
        // Collapsed strip: same spirit as tool cards — technical payload on demand, tokenized chrome only.
        var detailsHtml =
          '<details class="chat-voice-ctx-details">' +
          '<summary>' +
          '<span class="chat-voice-ctx-chevron" aria-hidden="true">▸</span>' +
          '<span>Internal context</span>' +
          '</summary>' +
          '<pre class="tool-pre chat-voice-ctx-pre">' +
          this.escapeHtml(raw) +
          '</pre></details>';
        return '<div class="chat-user-voice-wrap">' + bodyHtml + detailsHtml + '</div>';
      }
      var inner = msg.isHtml ? msg.text : ((msg.role === 'agent' || msg.role === 'system') && !msg.thinking
        ? this.renderMarkdown(msg.text)
        : this.escapeHtml(msg.text));
      return this.highlightSearch(inner);
    },

    // Copy message text to clipboard (Clipboard API + execCommand fallback for desktop WebView)
    copyMessage: function(msg) {
      var self = this;
      var text = msg.text || '';
      if (msg.role === 'user' && text) {
        var origU = text;
        text = this.stripVoiceAgentUiNoise(text);
        if (!String(text).trim() && String(origU).trim()) text = 'Recording...';
      }
      if (msg.isHtml && text) {
        var div = document.createElement('div');
        div.innerHTML = text;
        text = div.textContent || div.innerText || text;
      }
      copyTextToClipboard(text).then(function() {
        msg._copied = true;
        var idx = self.messages.indexOf(msg);
        if (idx !== -1) self.messages.splice(idx, 1, msg);
        setTimeout(function() {
          msg._copied = false;
          var j = self.messages.indexOf(msg);
          if (j !== -1) self.messages.splice(j, 1, msg);
        }, 2000);
      }).catch(function() {
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.error('Could not copy — try selecting the message text');
        }
      });
    },

    bookmarkCategoriesSorted: function() {
      if (typeof ArmaraosBookmarks === 'undefined') return [];
      var st = ArmaraosBookmarks.load();
      return st.categories.slice().sort(function(a, b) { return a.order - b.order; });
    },

    openBookmarkModal: function(msg) {
      if (typeof ArmaraosBookmarks === 'undefined') {
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Bookmarks unavailable');
        return;
      }
      if (!msg || !msg.text || !msg.text.trim() || msg.thinking || msg.streaming) return;
      if (msg.role !== 'agent' && msg.role !== 'user') return;
      this.bookmarkMsg = msg;
      var sorted = this.bookmarkCategoriesSorted();
      this.bookmarkCategoryId = sorted[0] ? sorted[0].id : '';
      this.bookmarkNewCategory = '';
      var line = (msg.role === 'user' ? this.stripVoiceAgentUiNoise(msg.text || '') : (msg.text || ''))
        .split('\n')[0]
        .trim();
      if (!line && msg.role === 'user') line = 'Recording...';
      this.bookmarkTitle = line.length > 140 ? line.slice(0, 137) + '...' : line;
      this.bookmarkModalOpen = true;
    },

    closeBookmarkModal: function() {
      this.bookmarkModalOpen = false;
      this.bookmarkMsg = null;
    },

    confirmBookmark: function() {
      if (typeof ArmaraosBookmarks === 'undefined' || !this.bookmarkMsg) return;
      var catId = (this.bookmarkNewCategory && this.bookmarkNewCategory.trim())
        ? ArmaraosBookmarks.ensureCategory(this.bookmarkNewCategory.trim())
        : this.bookmarkCategoryId;
      if (!catId) {
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Choose or create a category');
        return;
      }
      var agent = this.currentAgent;
      var tools = (this.bookmarkMsg.tools || []).map(function(t) {
        return { name: t.name, input: t.input, result: t.result, is_error: t.is_error };
      });
      ArmaraosBookmarks.addItem({
        categoryId: catId,
        title: this.bookmarkTitle.trim() || 'Bookmark',
        text: this.bookmarkMsg.text,
        agentId: agent ? agent.id : null,
        agentName: agent ? agent.name : null,
        images: this.bookmarkMsg.images || [],
        tools: tools
      });
      this.closeBookmarkModal();
      if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Saved to Bookmarks');
    },

    // Process queued messages after current response completes
    _processQueue: function() {
      if (!this.messageQueue.length || this.sending) return;
      var next = this.messageQueue.shift();
      this._sendPayload(next.text, next.files, next.images);
    },

    async sendMessage() {
      if (!this.currentAgent || (!this.inputText.trim() && !this.attachments.length)) return;
      var text = this.inputText.trim();

      // Save to per-agent input history before dispatching
      if (text) {
        var deduped = (this._msgHistory || []).filter(function(h) { return h !== text; });
        this._msgHistory = [text].concat(deduped).slice(0, 50);
        this._msgHistoryIdx = -1;
        this._msgHistoryBuffer = '';
        try { localStorage.setItem('armaraos-chat-history-' + this.currentAgent.id, JSON.stringify(this._msgHistory)); } catch (_eH) { /* ignore */ }
      }

      // Handle slash commands
      if (text.startsWith('/') && !this.attachments.length) {
        var cmd = text.split(' ')[0].toLowerCase();
        var cmdArgs = text.substring(cmd.length).trim();
        var matched = this.slashCommands.find(function(c) { return c.cmd === cmd; });
        if (matched) {
          this.executeSlashCommand(matched.cmd, cmdArgs);
          return;
        }
      }

      this.inputText = '';

      // Reset textarea height to single line
      var ta = document.getElementById('msg-input');
      if (ta) ta.style.height = '';

      // Upload attachments first if any
      var fileRefs = [];
      var uploadedFiles = [];
      if (this.attachments.length) {
        for (var i = 0; i < this.attachments.length; i++) {
          var att = this.attachments[i];
          att.uploading = true;
          try {
            var uploadRes = await OpenFangAPI.upload(this.currentAgent.id, att.file);
            fileRefs.push('[File: ' + att.file.name + ']');
            uploadedFiles.push({ file_id: uploadRes.file_id, filename: uploadRes.filename, content_type: uploadRes.content_type });
          } catch(e) {
            OpenFangToast.error('Failed to upload ' + att.file.name);
            fileRefs.push('[File: ' + att.file.name + ' (upload failed)]');
          }
          att.uploading = false;
        }
        // Clean up previews
        for (var j = 0; j < this.attachments.length; j++) {
          if (this.attachments[j].preview) URL.revokeObjectURL(this.attachments[j].preview);
        }
        this.attachments = [];
      }

      // Build final message text
      var finalText = text;
      if (fileRefs.length) {
        finalText = (text ? text + '\n' : '') + fileRefs.join('\n');
      }

      // Collect image references for inline rendering
      var msgImages = uploadedFiles.filter(function(f) { return f.content_type && f.content_type.startsWith('image/'); });

      // Store original text for diff UI (before compression occurs on the server)
      this._lastSentOriginal = text;
      // Always show user message immediately and update cache so switching away preserves it
      this.messages.push({ id: ++msgId, role: 'user', text: finalText, meta: '', tools: [], images: msgImages, ts: Date.now() });
      if (this.currentAgent) _agentMsgCache[this.currentAgent.id] = this.messages.slice();
      this.scrollToBottom(true);

      // If already streaming, queue this message
      if (this.sending) {
        this.messageQueue.push({ text: finalText, files: uploadedFiles, images: msgImages });
        return;
      }

      this._sendPayload(finalText, uploadedFiles, msgImages);
    },

    // Split a large text into chunks just under the WS size limit.
    // Breaks on paragraph boundaries when possible.
    _splitLargeText: function(text, maxBytes) {
      var chunks = [];
      var remaining = text;
      while (remaining.length > 0) {
        if (new TextEncoder().encode(remaining).length <= maxBytes) {
          chunks.push(remaining);
          break;
        }
        // Try to break on a paragraph boundary near the limit
        var searchEnd = maxBytes;
        while (searchEnd > 0 && new TextEncoder().encode(remaining.slice(0, searchEnd)).length > maxBytes) {
          searchEnd = Math.floor(searchEnd * 0.9);
        }
        var breakAt = remaining.lastIndexOf('\n\n', searchEnd);
        if (breakAt <= 0) breakAt = remaining.lastIndexOf('\n', searchEnd);
        if (breakAt <= 0) breakAt = searchEnd;
        chunks.push(remaining.slice(0, breakAt).trim());
        remaining = remaining.slice(breakAt).trim();
      }
      return chunks;
    },

    async _sendPayload(finalText, uploadedFiles, msgImages) {
      this.sending = true;
      this._clearStreamErrorTelemetry();
      this._clearRuntimeStatusTelemetry();
      this._startElapsed();
      this._captureMemoryBaselineForTurn();

      // Detect input that would be rejected by the 64KB WS / HTTP limit.
      // Auto-split into chunks and queue them so the agent receives all content.
      var WS_LIMIT = 60 * 1024; // slightly under 64KB to leave room for JSON framing
      var encoder = typeof TextEncoder !== 'undefined' ? new TextEncoder() : null;
      var byteLen = encoder ? encoder.encode(finalText).length : finalText.length;
      if (byteLen > WS_LIMIT && !uploadedFiles.length) {
        var chunks = this._splitLargeText(finalText, WS_LIMIT);
        if (chunks.length > 1) {
          // Show the user what is happening
          this.messages.push({ id: ++msgId, role: 'system', text: 'Large input (' + Math.ceil(byteLen / 1024) + ' KB) — splitting into ' + chunks.length + ' parts and sending automatically.', meta: '', tools: [], ts: Date.now() });
          this.scrollToBottom(true);
          // Send first chunk now, queue the rest with continuation prefixes
          var self = this;
          for (var ci = 1; ci < chunks.length; ci++) {
            (function(i, total, chunk) {
              self.messageQueue.unshift({ text: '[Continued — part ' + (i + 1) + ' of ' + total + ']\n\n' + chunk, files: [], images: [] });
            })(ci, chunks.length, chunks[ci]);
          }
          finalText = '[Part 1 of ' + chunks.length + ' — please process all parts before responding]\n\n' + chunks[0];
        }
      }

      // Try WebSocket first — `voice_reply` when the user enabled spoken reply (Piper) in the UI
      var wsPayload = { type: 'message', content: finalText };
      if (uploadedFiles && uploadedFiles.length) wsPayload.attachments = uploadedFiles;
      if (this.voiceReplyEnabled) wsPayload.voice_reply = true;
      if (OpenFangAPI.wsSend(wsPayload)) {
        // Track in-flight so onClose can retry if the connection drops while waiting
        this._wsInFlightMsg = { text: finalText, files: uploadedFiles || [], images: msgImages || [] };
        this.messages.push({ id: ++msgId, role: 'agent', text: '', meta: '', thinking: true, streaming: true, tools: [], ts: Date.now() });
        this.scrollToBottom(true);
        return;
      }
      // WebSocket unavailable — clear any stale in-flight reference
      this._wsInFlightMsg = null;

      // HTTP fallback
      if (!OpenFangAPI.isWsConnected()) {
        OpenFangToast.info('Using HTTP mode (no streaming)');
      }
      this.messages.push({ id: ++msgId, role: 'agent', text: '', meta: '', thinking: true, tools: [], ts: Date.now() });
      this.scrollToBottom(true);

      try {
        var self = this;
        var httpBody = { message: finalText };
        if (uploadedFiles && uploadedFiles.length) httpBody.attachments = uploadedFiles;
        if (self.voiceReplyEnabled) httpBody.voice_reply = true;
        var res = await OpenFangAPI.post('/api/agents/' + this.currentAgent.id + '/message', httpBody);
        this.messages = this.messages.filter(function(m) { return !m.thinking; });
        if (res.turn_wall_ms != null) this.lastTurnWallMs = res.turn_wall_ms;
        if (res.input_tokens != null) this.sessionPromptTokens += res.input_tokens;
        if (res.output_tokens != null) this.sessionCompletionTokens += res.output_tokens;
        this._clearStreamErrorTelemetry();
        this._applyAinlRuntimeTelemetry(res.ainl_runtime_telemetry, 'http');
        var httpMeta = (res.input_tokens || 0) + ' in / ' + (res.output_tokens || 0) + ' out';
        if (res.cost_usd != null) httpMeta += ' | $' + res.cost_usd.toFixed(4);
        if (res.iterations) httpMeta += ' | ' + res.iterations + ' iter';
        if (res.skill_draft_path) httpMeta += ' | skill draft: ' + res.skill_draft_path;
        if (self.efficientMode && self.efficientMode !== 'off') {
          var hPct = res.compression_savings_pct;
          httpMeta += hPct > 0 ? ' | ⚡ eco ↓' + hPct + '%' : ' | ⚡ eco';
        }
        httpMeta += this._buildEcoMetaSuffix(res);
        if (res.turn_outcome) httpMeta += ' | outcome: ' + res.turn_outcome;
        var httpEcoTip = this._buildEcoMetaTooltip(res);
        var httpCompressedInput = (res.compressed_input && res.compression_savings_pct > 0) ? res.compressed_input : null;
        this._recordEcoSaving(res.compression_savings_pct);
        var httpTools = [];
        if (res.tools && res.tools.length) {
          var httpNow = Date.now();
          for (var hti = 0; hti < res.tools.length; hti++) {
            var ht = res.tools[hti];
            httpTools.push({
              id: (ht.name || 'tool') + '-http-' + hti + '-' + httpNow,
              name: ht.name || '',
              running: false,
              expanded: false,
              input: typeof ht.input === 'string' ? ht.input : JSON.stringify(ht.input != null ? ht.input : {}),
              result: ht.result || '',
              is_error: !!ht.is_error
            });
            httpTools[httpTools.length - 1].expanded = this.shouldAutoExpandTool(httpTools[httpTools.length - 1]);
          }
        }
        this.messages.push({ id: ++msgId, role: 'agent', text: res.response, meta: httpMeta, tools: httpTools, ts: Date.now(),
          compressedInput: httpCompressedInput, originalInput: httpCompressedInput ? (self._lastSentOriginal || '') : null,
          savingsPct: res.compression_savings_pct || 0, ecoMetaTooltip: httpEcoTip || null });
        this._attachVoiceReply(this.messages[this.messages.length - 1], res);
        this._updateMemoryAppliedIndicatorForTurn();
      } catch(e) {
        this.messages = this.messages.filter(function(m) { return !m.thinking; });
        var technical = e.message || 'Unknown error';
        if (e.detail && e.detail !== technical) technical += ' — ' + e.detail;
        if (e.hint) technical += ' Hint: ' + e.hint;
        var friendlyHttp = this._applyFriendlyError(technical);
        var httpUserText = friendlyHttp.text;
        if (this.networkHints && this.networkHints.likely_vpn) {
          httpUserText += '\n\nIf this keeps happening: a VPN or firewall may block the AI provider. Try split tunneling or allowlisting.';
        }
        this.messages.push({ id: ++msgId, role: 'system', text: httpUserText, meta: '', tools: [], ts: Date.now() });
      }
      this.sending = false;
      this.scrollToBottom(true);
      // Process next queued message
      var self = this;
      this.$nextTick(function() {
        var el = document.getElementById('msg-input'); if (el) el.focus();
        self._processQueue();
      });
    },

    _extractInjectedLinesTotal(status) {
      var m = (status && status.graph_memory_context_metrics) || {};
      if (typeof m.injected_lines_total === 'number') return m.injected_lines_total;
      return Number(m.injected_episodic_total || 0)
        + Number(m.injected_semantic_total || 0)
        + Number(m.injected_conflict_total || 0)
        + Number(m.injected_procedural_total || 0);
    },

    async _captureMemoryBaselineForTurn() {
      try {
        var status = await OpenFangAPI.get('/api/status');
        this._memoryInjectedBeforeTurn = this._extractInjectedLinesTotal(status);
      } catch (e) {
        this._memoryInjectedBeforeTurn = null;
      }
    },

    async _updateMemoryAppliedIndicatorForTurn() {
      try {
        var status = await OpenFangAPI.get('/api/status');
        var after = this._extractInjectedLinesTotal(status);
        if (typeof after !== 'number') return;
        var before = this._memoryInjectedBeforeTurn;
        this.memoryContextAppliedLastTurn = (typeof before === 'number') ? (after > before) : null;
        this.memoryContextAppliedAtMs = Date.now();
      } catch (e) {
        this.memoryContextAppliedLastTurn = null;
      } finally {
        this._memoryInjectedBeforeTurn = null;
      }
    },

    // Stop the current agent run
    stopAgent: function() {
      if (!this.currentAgent) return;
      var self = this;
      OpenFangAPI.post('/api/agents/' + this.currentAgent.id + '/stop', {}).then(function(res) {
        self.messages.push({ id: ++msgId, role: 'system', text: res.message || 'Run cancelled', meta: '', tools: [], ts: Date.now() });
        self.sending = false;
        self.scrollToBottom(true);
        self.$nextTick(function() { self._processQueue(); });
      }).catch(function(e) { var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Stop failed: '+msg+d+h); });
    },

    killAgent() {
      if (!this.currentAgent) return;
      var self = this;
      var name = this.currentAgent.name;
      OpenFangToast.confirm('Stop Agent', 'Stop agent "' + name + '"? The agent will be shut down.', async function() {
        try {
          await OpenFangAPI.del('/api/agents/' + self.currentAgent.id);
          OpenFangAPI.wsDisconnect();
          self._wsAgent = null;
          self.currentAgent = null;
          self.messages = [];
          OpenFangToast.success('Agent "' + name + '" stopped');
          Alpine.store('app').refreshAgents();
        } catch(e) {
          var msg = e.message||'Unknown error'; var d=e.detail&&e.detail!==msg?' — '+e.detail:''; var h=e.hint?' Hint: '+e.hint:''; OpenFangToast.error('Failed to stop agent: '+msg+d+h);
        }
      });
    },

    openAgentWorkspace() {
      if (!this.currentAgent) return;
      var rel = String(this.currentAgent.workspace_rel_home || '').trim();
      if (!rel) {
        OpenFangToast.warn('Workspace path is not browseable from Home folder.');
        return;
      }
      if (rel === '.') rel = '';
      try {
        sessionStorage.setItem(
          'armaraos-home-prefill-agent-id',
          String(this.currentAgent.id || '')
        );
      } catch (eWs) { /* ignore */ }
      window.location.hash = rel
        ? ('home-files?path=' + encodeURIComponent(rel))
        : 'home-files';
    },

    _latexTimer: null,

    onMessagesScroll() {
      var el = document.getElementById('messages');
      if (!el) return;
      var threshold = 72;
      var dist = el.scrollHeight - el.scrollTop - el.clientHeight;
      this._chatPinnedToBottom = dist <= threshold;
      // Lazy-load older history when the user is near the top of the chat
      // and the server reported `has_more: true` for this agent. Cheap and
      // idempotent (loadOlderMessages no-ops when nothing more is available
      // or a fetch is already in flight).
      if (el.scrollTop <= CHAT_HISTORY_TOP_THRESHOLD_PX && this.currentAgent) {
        var st = _agentSessionState[this.currentAgent.id];
        if (st && st.hasMore && !st.loadingOlder) {
          this.loadOlderMessages();
        }
      }
    },

    /** @param {boolean} [force] - If true, always scroll (user action or new session). If omitted, only scroll when user is already near the bottom (avoids fighting scroll while reading history during streaming). */
    scrollToBottom(force) {
      var self = this;
      if (force === true) {
        this._chatPinnedToBottom = true;
      } else if (!this._chatPinnedToBottom) {
        return;
      }
      var el = document.getElementById('messages');
      if (el) self.$nextTick(function() {
        el.scrollTop = el.scrollHeight;
        // Debounce LaTeX rendering to avoid running on every streaming token
        if (self._latexTimer) clearTimeout(self._latexTimer);
        self._latexTimer = setTimeout(function() { renderLatex(el); }, 150);
      });
    },

    addFiles(files) {
      var self = this;
      /** Must match `MAX_UPLOAD_SIZE` in `routes.rs` (chat attachments). */
      var MAX_CHAT_ATTACHMENT_BYTES = 128 * 1024 * 1024;
      var blockedExt = ['.exe', '.dll', '.bat', '.cmd', '.msi', '.scr', '.com', '.app', '.deb', '.rpm', '.dmg', '.pkg', '.iso'];
      var extOkList = ['.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.ico', '.tif', '.tiff', '.heic', '.heif', '.avif', '.svg',
        '.pdf', '.txt', '.md', '.markdown', '.json', '.jsonl', '.csv', '.tsv', '.tab', '.xml', '.xsl', '.html', '.htm', '.xhtml',
        '.css', '.js', '.jsx', '.mjs', '.cjs', '.ts', '.tsx', '.mts', '.cts', '.vue', '.svelte', '.php', '.phtml', '.py', '.pyw',
        '.rs', '.go', '.java', '.kt', '.cs', '.c', '.h', '.cpp', '.hpp', '.rb', '.sql', '.yaml', '.yml', '.toml', '.ini', '.cfg',
        '.ainl', '.lang', '.graphql', '.gql',         '.xlsx', '.xls', '.xlsm', '.ods', '.docx', '.doc', '.odt', '.rtf', '.pptx', '.ppt', '.odp',
        '.zip',
        '.woff', '.woff2', '.ttf', '.otf', '.eot', '.mp3', '.wav', '.ogg', '.oga', '.opus', '.flac', '.m4a', '.aac', '.mp4', '.webm', '.mov', '.mkv', '.m4v'];
      function attachmentAllowed(file) {
        var ext = file.name.lastIndexOf('.') !== -1 ? file.name.substring(file.name.lastIndexOf('.')).toLowerCase() : '';
        if (ext && blockedExt.indexOf(ext) !== -1) return false;
        var t = (file.type || '').toLowerCase();
        if (t.startsWith('image/') || t.startsWith('audio/') || t.startsWith('video/') || t.startsWith('text/') || t.startsWith('font/')) return true;
        if (t.startsWith('application/vnd.openxmlformats') || t.startsWith('application/vnd.oasis') || t.startsWith('application/vnd.ms-')) return true;
        var appExact = ['application/pdf', 'application/json', 'application/xml', 'application/javascript', 'application/typescript', 'application/rtf', 'application/sql', 'application/csv', 'application/graphql', 'application/xhtml+xml', 'application/msword', 'application/ld+json', 'application/x-httpd-php', 'application/x-yaml', 'application/x-sh', 'application/x-shellscript', 'application/toml', 'application/zip', 'application/x-zip-compressed'];
        if (appExact.indexOf(t) !== -1) return true;
        if (ext && extOkList.indexOf(ext) !== -1) return true;
        if (t === 'application/octet-stream' && ext && extOkList.indexOf(ext) !== -1) return true;
        return false;
      }
      for (var i = 0; i < files.length; i++) {
        var file = files[i];
        if (file.size > MAX_CHAT_ATTACHMENT_BYTES) {
          OpenFangToast.warn('File "' + file.name + '" exceeds 128MB limit');
          continue;
        }
        if (!attachmentAllowed(file)) {
          OpenFangToast.warn('File type not supported (or blocked): ' + file.name);
          continue;
        }
        var preview = null;
        if (file.type.startsWith('image/')) {
          preview = URL.createObjectURL(file);
        }
        self.attachments.push({ file: file, preview: preview, uploading: false });
      }
    },

    removeAttachment(idx) {
      var att = this.attachments[idx];
      if (att && att.preview) URL.revokeObjectURL(att.preview);
      this.attachments.splice(idx, 1);
    },

    handleDrop(e) {
      e.preventDefault();
      if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files.length) {
        this.addFiles(e.dataTransfer.files);
      }
    },

    isGrouped(idx) {
      if (idx === 0) return false;
      var prev = this.messages[idx - 1];
      var curr = this.messages[idx];
      return prev && curr && prev.role === curr.role && !curr.thinking && !prev.thinking;
    },

    // Strip raw function-call text that some models (Llama, Groq, etc.) leak into output.
    // These models don't use proper tool_use blocks — they output function calls as plain text.
    sanitizeToolText: function(text) {
      if (!text) return text;
      // Pattern: tool_name</function={"key":"value"} or tool_name</function,{...}
      text = text.replace(/\s*\w+<\/function[=,]?\s*\{[\s\S]*$/gm, '');
      // Pattern: <function=tool_name>{...}</function>
      text = text.replace(/<function=\w+>[\s\S]*?<\/function>/g, '');
      // Pattern: tool_name{"type":"function",...}
      text = text.replace(/\s*\w+\{"type"\s*:\s*"function"[\s\S]*$/gm, '');
      // Pattern: lone </function...> tags
      text = text.replace(/<\/function[^>]*>/g, '');
      // Pattern: <|python_tag|> or similar special tokens
      text = text.replace(/<\|[\w_]+\|>/g, '');
      return text.trim();
    },

    formatToolJson: function(text) {
      if (!text) return '';
      if (typeof text === 'object') {
        return JSON.stringify(text, null, 2);
      }
      try { return JSON.stringify(JSON.parse(text), null, 2); }
      catch(e) { return text; }
    },

    toolResultText: function(tool) {
      if (!tool || tool.result == null) return '';
      if (typeof tool.result === 'string') return tool.result;
      try { return JSON.stringify(tool.result, null, 2); } catch (e) { return String(tool.result); }
    },

    toolResultChars: function(tool) {
      return this.toolResultText(tool).length;
    },

    toolAuthUrl: function(tool) {
      var text = this.toolResultText(tool);
      if (!text) return '';
      var urls = text.match(/https?:\/\/[^\s<>"')]+/g) || [];
      for (var i = 0; i < urls.length; i++) {
        var u = String(urls[i] || '');
        var ul = u.toLowerCase();
        if (/\/oauth|\/authorize|oauth2|accounts\.google\.com|\/consent|client_id=|redirect_uri=|response_type=|scope=/.test(ul)) {
          return u;
        }
      }
      var head = text.slice(0, 500).toLowerCase();
      if (urls.length && /^(action required|authorization needed|authentication needed|requires authorization)\b/.test(head.trim())) {
        return urls[0];
      }
      return '';
    },

    /**
     * Errors the model cannot clear alone — expand tool + show user callout (UI only).
     */
    toolErrorNeedsUserAction: function(tool) {
      if (!tool || !tool.is_error) return false;
      if (this.toolAuthUrl(tool)) return true;
      var r = this.toolResultText(tool);
      var low = r.slice(0, 6000).toLowerCase();
      if (/allowlist|capability denied|capability deny|not permitted by policy|policy blocks|denied by capability/i.test(low)) return true;
      if (/permission denied|eacces|operation not permitted|requires elevation|must be run as|access denied\b/i.test(low)) return true;
      if (/microphone|camera|screen recording|not allowed by user|user denied|blocked by user|grant.*permission/i.test(low)) return true;
      if (/enable mcp|mcp server.*not|mcp.*disabled|no mcp connection|mcp.*unavailable|authenticate mcp|mcp.*oauth/i.test(low)) return true;
      if (/invalid api key|api key not|authentication failed|bad api key|401 unauthorized|sign-?in failed/i.test(low)) return true;
      if (/payment required|insufficient credits|billing|402|quota exceeded for|plan upgrade|subscription/i.test(low)) return true;
      if (/human approval|approval required|pending approval|user must|settings.*required|configure.*in settings/i.test(low)) return true;
      if (/blocked by|ssrf|forbidden host|not allowlisted url/i.test(low)) return true;
      return false;
    },

    /** Short, static guidance lines (never echo raw tool output — avoids XSS). */
    toolUserActionCalloutLines: function(tool) {
      if (!tool || !tool.is_error || !this.toolErrorNeedsUserAction(tool)) return [];
      if (this.toolAuthUrl(tool)) {
        return [
          'Complete authorization in your browser using the link in this card.',
          'When finished, send your message again so the agent can continue.'
        ];
      }
      var r = this.toolResultText(tool).slice(0, 4000).toLowerCase();
      if (/allowlist|capability denied|capability deny|not permitted by policy/i.test(r)) {
        return [
          'Open the agent or workspace tool policy and add this tool to the allowlist (or widen the rule), save, then retry.',
          'If you use profiles per agent, confirm the active profile includes this capability.'
        ];
      }
      if (/permission denied|eacces|operation not permitted|requires elevation/i.test(r)) {
        return [
          'Grant the permission the tool asked for (macOS: System Settings → Privacy & Security).',
          'Restart the agent if the kernel caches permissions, then retry the same step.'
        ];
      }
      if (/microphone|camera|screen recording|user denied|not allowed by user/i.test(r)) {
        return [
          'Allow the capability in System Settings (Privacy & Security) for ArmaraOS / your terminal host.',
          'Retry after granting access so the tool run can complete.'
        ];
      }
      if (/enable mcp|mcp server|mcp.*disabled|no mcp connection|mcp.*unavailable|authenticate mcp/i.test(r)) {
        return [
          'Open Settings → MCP: enable the server, confirm it is reachable, and finish OAuth if prompted.',
          'Save, wait for the green/connected state, then send your message again.'
        ];
      }
      // Do not conflate GitHub API auth with the chat LLM provider key (UI matches "401" broadly).
      if (tool && tool.name === 'github_subtree_download' && (/github\.com\/rest|api\.github\.com|bad credentials|git\/trees/i.test(r) || /401/.test(r))) {
        return [
          'Omit the `token` field for public repos, or set a real GitHub PAT if the repo is private or you hit rate limits. An empty `token` is invalid.',
          'This is not the StandardCompute / chat provider key — that key is only for the LLM request.',
          'To persist a PAT for tools and git, use Settings → Vault (GITHUB_TOKEN or GH_TOKEN), then retry.'
        ];
      }
      if (/invalid api key|api key not|authentication failed|401 unauthorized|bad api key/i.test(r)) {
        return [
          'Update the provider API key in Settings (and verify the model id is allowed for that key).',
          'Retry after saving — the agent cannot fix an invalid key on its own.'
        ];
      }
      if (/payment required|insufficient credits|billing|402|quota exceeded|plan upgrade|subscription/i.test(r)) {
        return [
          'Check your provider balance, plan limits, or subscription on the provider’s site.',
          'Switch model or top up credits, then retry.'
        ];
      }
      if (/human approval|approval required|pending approval|user must/i.test(r)) {
        return [
          'Complete the approval or confirmation step the workflow requested (dashboard, email, or VCS).',
          'Then rerun the tool or resend your last message.'
        ];
      }
      if (/blocked by|ssrf|forbidden host|not allowlisted url/i.test(r)) {
        return [
          'Adjust URL allowlists / SSRF policy in Settings or agent config if this host should be permitted.',
          'Otherwise use a different URL that is already allowlisted.'
        ];
      }
      return [
        'This failure needs a change outside the chat (settings, permissions, or provider account).',
        'Review the raw result below, fix the underlying issue, then retry.'
      ];
    },

    /** Extra classes on tool cards (visual only). */
    toolCardStatusClasses: function(tool) {
      var o = {};
      if (tool && tool.is_error) {
        o['tool-card-error'] = true;
        if (this.toolErrorNeedsUserAction(tool)) o['tool-card-error--user-action'] = true;
        else o['tool-card-error--agent'] = true;
      }
      return o;
    },

    /** System message row — subtle surface tiers (visual only; same text to model/history). */
    systemMessageSurfaceClass: function(msg) {
      if (!msg || msg.role !== 'system') return '';
      var t = (msg.text || '').toLowerCase();
      if (/allowlist|capabilit|permission denied|oauth|authorize|settings|api key|billing|402|401|403|forbidden|enable mcp|mcp server|user must|human approval|grant.*access/i.test(t)) {
        return ' message-system--action';
      }
      if (/tool|error|failed|warn|⚠|mcp|denied|timeout|unavailable/i.test(t)) {
        return ' message-system--diagnostic';
      }
      return ' message-system--ambient';
    },

    toolStatusText: function(tool) {
      if (!tool) return '';
      if (tool.running) return 'running...';
      if (this.toolAuthUrl(tool)) return 'auth needed';
      if (tool.is_error && this.toolErrorNeedsUserAction(tool)) return 'needs you';
      if (tool.is_error) return 'retry ok';
      var chars = this.toolResultChars(tool);
      if (chars > 500) return Math.max(1, Math.round(chars / 1024)) + 'KB';
      return 'done';
    },

    /** Parse tool `input` when the server stored JSON as a string. */
    _toolInputObject: function(tool) {
      if (!tool || tool.input == null) return null;
      if (typeof tool.input === 'object') return tool.input;
      var s = String(tool.input).trim();
      if (!s) return null;
      try {
        return JSON.parse(s);
      } catch (e) {
        return null;
      }
    },

    /** One-line summary for web tools from arguments (collapsed header; avoids dumping HTML/snippets). */
    _compactWebToolSummary: function(tool) {
      if (!tool || !tool.name) return '';
      var io = this._toolInputObject(tool);
      if (tool.name === 'web_fetch') {
        var url = io && typeof io.url === 'string' ? io.url.trim() : '';
        if (url) {
          try {
            var host = new URL(url).host || url;
            return 'Fetched · ' + host;
          } catch (e) {
            return 'Fetched · ' + (url.length > 56 ? url.slice(0, 53) + '…' : url);
          }
        }
        return 'Fetched page';
      }
      if (tool.name === 'web_search') {
        var q = io && typeof io.query === 'string' ? io.query.trim() : '';
        if (q) return 'Search · ' + (q.length > 64 ? q.slice(0, 61) + '…' : q);
        return 'Web search';
      }
      return '';
    },

    /** Short MCP summary: server/tool segments, not raw JSON. */
    _compactMcpToolSummary: function(tool) {
      if (!tool || !tool.name || tool.name.indexOf('mcp_') !== 0) return '';
      var rest = tool.name.slice(4);
      var parts = rest.split('_');
      if (parts.length >= 2) {
        var tail = parts.slice(1).join(' ').replace(/\s+/g, ' ');
        return parts[0] + ' · ' + (tail.length > 52 ? tail.slice(0, 49) + '…' : tail);
      }
      return rest.length > 64 ? rest.slice(0, 61) + '…' : rest;
    },

    /** Collapsed header for browser / Playwright tools from JSON input (no page dump). */
    _compactBrowserToolSummary: function(tool) {
      if (!tool || !tool.name) return '';
      if (tool.name.indexOf('browser_') !== 0 && tool.name.indexOf('playwright_') !== 0) return '';
      var io = this._toolInputObject(tool);
      if (!io || typeof io !== 'object') return 'Browser';
      var action = typeof io.action === 'string' ? io.action.trim() : '';
      var url = typeof io.url === 'string' ? io.url.trim() : (typeof io.target_url === 'string' ? io.target_url.trim() : '');
      if (url) {
        try {
          return (action ? action + ' · ' : 'Open · ') + (new URL(url).host || url);
        } catch (e) {
          return (action ? action + ' · ' : 'Open · ') + (url.length > 48 ? url.slice(0, 45) + '…' : url);
        }
      }
      if (action) return 'Browser · ' + (action.length > 56 ? action.slice(0, 53) + '…' : action);
      return 'Browser';
    },

    toolSummaryText: function(tool) {
      if (!tool) return '';
      if (tool.running) return 'Waiting for tool response...';
      if (this.toolAuthUrl(tool)) return 'Authorization required. Open the link below, then retry.';
      if (tool.is_error) {
        if (this.toolErrorNeedsUserAction(tool)) return 'Needs your attention';
        return 'Tool error · model can retry';
      }
      var compactWeb = this._compactWebToolSummary(tool);
      if (compactWeb) return compactWeb;
      var compactMcp = this._compactMcpToolSummary(tool);
      if (compactMcp) return compactMcp;
      var compactBrowser = this._compactBrowserToolSummary(tool);
      if (compactBrowser) return compactBrowser;
      var text = this.toolResultText(tool);
      if (!text) return 'Tool completed.';
      var oneLine = text
        .replace(/^Error calling tool[^:]*:\s*/i, '')
        .replace(/https?:\/\/[^\s<>"')]+/g, '[link]')
        .replace(/\s+/g, ' ')
        .trim();
      if (!oneLine) return 'Tool completed.';
      var max = (tool.name && tool.name.indexOf('mcp_') === 0) ? 72 : 120;
      return oneLine.length > max ? (oneLine.slice(0, max - 1) + '…') : oneLine;
    },

    shouldAutoExpandTool: function(tool) {
      if (!tool || tool.running) return false;
      if (this.toolAuthUrl(tool)) return true;
      if (tool.is_error && this.toolErrorNeedsUserAction(tool)) return true;
      return false;
    },

    // Prefer codecs whisper.cpp accepts natively (ogg/mp3/wav/flac) to avoid server-side ffmpeg;
    // fall back to WebM Opus (Chrome), then audio/mp4 (Safari often). See MediaRecorder MIME support per browser/OS.
    chooseVoiceRecordingMimeType: function() {
      var candidates = [
        'audio/ogg;codecs=opus',
        'audio/ogg',
        'audio/webm;codecs=opus',
        'audio/webm',
        'audio/mp4'
      ];
      for (var i = 0; i < candidates.length; i++) {
        if (typeof MediaRecorder !== 'undefined' && MediaRecorder.isTypeSupported(candidates[i])) {
          return candidates[i];
        }
      }
      return '';
    },

    // Voice: start recording
    startRecording: async function() {
      if (this.recording) return;
      try {
        var stream = await navigator.mediaDevices.getUserMedia({ audio: true });
        var mimeType = this.chooseVoiceRecordingMimeType();
        this._audioChunks = [];
        this._mediaRecorder = mimeType
          ? new MediaRecorder(stream, { mimeType: mimeType })
          : new MediaRecorder(stream);
        var self = this;
        this._mediaRecorder.ondataavailable = function(e) {
          if (e.data.size > 0) self._audioChunks.push(e.data);
        };
        this._mediaRecorder.onstop = function() {
          stream.getTracks().forEach(function(t) { t.stop(); });
          self._handleRecordingComplete();
        };
        this._mediaRecorder.start(250);
        this.recording = true;
        this.recordingTime = 0;
        this._recordingTimer = setInterval(function() { self.recordingTime++; }, 1000);
      } catch(e) {
        console.warn('[ArmaraOS chat] getUserMedia failed:', e);
        var msg = 'Could not use the microphone.';
        if (e && (e.name === 'NotAllowedError' || e.name === 'PermissionDeniedError')) {
          msg += ' Allow access in System Settings → Privacy & Security → Microphone (choose ArmaraOS). If ArmaraOS is not listed, install a desktop build that includes the microphone permission, then try again.';
        } else if (e && e.message) {
          msg += ' ' + e.message;
        }
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error(msg);
      }
    },

    // Voice: stop recording
    stopRecording: function() {
      if (!this.recording || !this._mediaRecorder) return;
      this._mediaRecorder.stop();
      this.recording = false;
      if (this._recordingTimer) { clearInterval(this._recordingTimer); this._recordingTimer = null; }
    },

    // Voice: handle completed recording — upload and transcribe
    _handleRecordingComplete: async function() {
      if (!this._audioChunks.length || !this.currentAgent) return;
      var blob = new Blob(this._audioChunks, { type: this._audioChunks[0].type || 'audio/webm' });
      this._audioChunks = [];
      if (blob.size < 100) return; // too small

      // Show a temporary "Transcribing..." message
      this.messages.push({ id: ++msgId, role: 'system', text: 'Transcribing audio...', thinking: true, ts: Date.now(), tools: [] });
      this.scrollToBottom(true);

      try {
        // Upload audio file — extension must match container (upload MIME normalization on server).
        var t = (blob.type || '').toLowerCase();
        var ext = 'webm';
        if (t.indexOf('ogg') !== -1) ext = 'ogg';
        else if (t.indexOf('webm') !== -1) ext = 'webm';
        else if (t.indexOf('mp4') !== -1 || t.indexOf('m4a') !== -1) ext = 'mp4';
        else if (t.indexOf('audio/mpeg') !== -1 || t.indexOf('mp3') !== -1) ext = 'mp3';
        var file = new File([blob], 'voice_' + Date.now() + '.' + ext, { type: blob.type });
        var upload = await OpenFangAPI.upload(this.currentAgent.id, file);

        // Remove the "Transcribing..." message
        this.messages = this.messages.filter(function(m) { return !m.thinking || m.role !== 'system'; });

        // Use server-side transcription if available, otherwise fall back to placeholder.
        // Include upload.file_id in the text so the model never mistakes the synthetic
        // filename (voice_*.webm) for the server file_id (UUID in temp openfang_uploads).
        var text = (upload.transcription && upload.transcription.trim())
          ? upload.transcription.trim()
          : ('[Voice message — transcribe with **media_transcribe** using file_id `' + upload.file_id +
            '` and content_type `' + upload.content_type + '` (display name: `' + upload.filename + '`). ' +
            'Do not use the display name as file_id.]');
        var voiceAttachments = [{
          file_id: upload.file_id,
          filename: upload.filename || '',
          content_type: upload.content_type || 'audio/webm'
        }];
        this._sendPayload(text, voiceAttachments, []);
      } catch(e) {
        this.messages = this.messages.filter(function(m) { return !m.thinking || m.role !== 'system'; });
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.error('Failed to upload audio: ' + openFangErrText(e));
      }
    },

    // Voice: format recording time as MM:SS
    formatRecordingTime: function() {
      var m = Math.floor(this.recordingTime / 60);
      var s = this.recordingTime % 60;
      return (m < 10 ? '0' : '') + m + ':' + (s < 10 ? '0' : '') + s;
    },

    // Search: toggle open/close
    toggleSearch: function() {
      this.searchOpen = !this.searchOpen;
      if (this.searchOpen) {
        var self = this;
        this.$nextTick(function() {
          var el = document.getElementById('chat-search-input');
          if (el) el.focus();
        });
      } else {
        this.searchQuery = '';
      }
    },

    // Search: filter messages by query
    get filteredMessages() {
      if (!this.searchQuery.trim()) return this.messages;
      var q = this.searchQuery.toLowerCase();
      return this.messages.filter(function(m) {
        return (m.text && m.text.toLowerCase().indexOf(q) !== -1) ||
               (m.tools && m.tools.some(function(t) { return t.name.toLowerCase().indexOf(q) !== -1; }));
      });
    },

    // Search: highlight matched text in a string
    highlightSearch: function(html) {
      if (!this.searchQuery.trim() || !html) return html;
      var q = this.searchQuery.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      var regex = new RegExp('(' + q + ')', 'gi');
      return html.replace(regex, '<mark style="background:var(--warning);color:var(--bg);border-radius:2px;padding:0 2px">$1</mark>');
    },

    renderMarkdown: renderMarkdown,
    escapeHtml: escapeHtml,

    /**
     * Attach a voice reply (or its failure reason) to the just-pushed agent message.
     *
     * The server cascades local TTS (Kokoro stub, then either **macOS `say` -> Piper** when
     * Settings → Voice enables “Prefer macOS say”, else **Piper -> macOS `say`**), so an audio URL
     * may arrive even when Piper is broken (the rhasspy/piper 2023.11.14-2 macOS aarch64 release
     * ships .dSYM debug symbols but omits the dylibs the binary links against). When neither
     * provider succeeds, the server now sends `voice_reply_error` so we can show the user a
     * real reason instead of silently producing nothing.
     *
     * Strategy:
     *   1. Always attempt autoplay (browsers may reject without user gesture).
     *   2. **Always** embed a markdown link into the message body so the inline `<audio controls>`
     *      from `markdownEmbedAudioUploadLinks` renders as a "tap to play" fallback that's robust
     *      to autoplay denial — this is the deterministic fix for "speaker says ready but I never
     *      hear anything" reports.
     */
    _attachVoiceReply: function(message, payload) {
      if (!message || !payload) return;
      if (payload.voice_reply_audio_url) {
        var url = payload.voice_reply_audio_url;
        var providerLabel = (payload.voice_reply_provider === 'macos_say')
          ? 'macOS say'
          : (payload.voice_reply_provider === 'piper' ? 'Piper' : 'voice reply');
        // Append a hidden (but renderable) markdown link so the audio embedder produces controls.
        var marker = '\n\n[' + providerLabel + ' audio reply](' + url + ')';
        if (message.text && message.text.indexOf(url) === -1) {
          message.text = message.text + marker;
        }
        if (typeof Audio !== 'undefined') {
          try {
            var au = new Audio(window.location.origin + url);
            au.play().catch(function() {
              if (typeof OpenFangToast !== 'undefined') {
                OpenFangToast.info('Voice reply ready — tap the inline player to listen (browser blocked autoplay).');
              }
            });
          } catch (e) { /* non-fatal */ }
        }
      } else if (payload.voice_reply_error) {
        // Server tried and failed (e.g. broken Piper bundle). Tell the user, don't pretend.
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.warn('Voice reply unavailable: ' + payload.voice_reply_error);
        }
        var note = '\n\n*(voice reply unavailable: ' + String(payload.voice_reply_error).replace(/[*_`]/g, '') + ')*';
        if (message.text && message.text.indexOf('voice reply unavailable') === -1) {
          message.text = message.text + note;
        }
      }
    },

    /**
     * Toggle whether to request a local TTS spoken reply from the daemon.
     * When turning on, preflight `GET /api/system/local-voice` so the UI only enables if some
     * provider (Piper OR macOS `say`) can actually synthesize; otherwise a toast explains the
     * specific failure (e.g. "Piper bundle missing dylibs and /usr/bin/say not found").
     */
    /** If localStorage had spoken-reply on but no TTS provider is ready, turn off and explain. */
    _syncVoiceReplyWithServer: async function() {
      if (!this.voiceReplyEnabled) return;
      if (typeof OpenFangAPI === 'undefined' || !OpenFangAPI.get) return;
      try {
        var s = await OpenFangAPI.get('/api/system/local-voice');
        if (s && (s.tts_ready || s.piper_ready)) return;
        this.voiceReplyEnabled = false;
        try { localStorage.setItem('armaraos-voice-reply', '0'); } catch (e) { /* non-fatal */ }
        this.voiceReplyHint = 'Spoken replies were saved as on, but no local TTS is ready — tap the speaker again for diagnostics.';
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.warn(this.voiceReplyHint);
        }
      } catch (err) {
        this.voiceReplyHint = '';
      }
    },

    toggleVoiceReply: async function() {
      if (this.voiceReplyEnabled) {
        this.voiceReplyEnabled = false;
        this.voiceReplyHint = '';
        try { localStorage.setItem('armaraos-voice-reply', '0'); } catch (e) { /* non-fatal */ }
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.info('Spoken replies off — assistant answers stay text-only unless you use cloud TTS tools.');
        }
        return;
      }
      if (typeof OpenFangAPI === 'undefined' || !OpenFangAPI.get) {
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.error('Spoken reply check unavailable: API client not loaded.');
        }
        return;
      }
      var self = this;
      // The toast points users at Settings → Voice on every enable (success or failure):
      // success → confirms which voice is active and how to change it;
      // failure → tells them what's missing AND where to fix / install / pick a voice.
      var goToVoiceSettings = function() {
        try {
          window.location.hash = 'settings';
          setTimeout(function() {
            try {
              var sp = (window.Alpine && Alpine.store && Alpine.store('settingsPage'));
              if (sp) {
                sp.tab = 'voice';
                if (typeof sp.loadVoiceSettings === 'function') sp.loadVoiceSettings();
              } else {
                window.dispatchEvent(new CustomEvent('armaraos:open-settings-tab', { detail: { tab: 'voice' } }));
              }
            } catch(e) { /* non-fatal */ }
          }, 120);
        } catch(e) { /* non-fatal */ }
      };
      try {
        var s = await OpenFangAPI.get('/api/system/local-voice');
        var ready = !!(s && (s.tts_ready || s.piper_ready));
        if (ready) {
          self.voiceReplyEnabled = true;
          self.voiceReplyHint = '';
          try { localStorage.setItem('armaraos-voice-reply', '1'); } catch (e2) { /* non-fatal */ }
          var providerName = (s && s.tts_provider === 'macos_say')
            ? ('macOS ' + (s.preferred_say_voice ? '"' + s.preferred_say_voice + '"' : 'system voice') + ' (say)')
            : (s && s.tts_provider === 'piper'
                ? ('Piper' + (s.custom_piper_voice ? ' / custom "' + s.custom_piper_voice + '"' : ' (en_US-lessac-medium)'))
                : 'local TTS');
          if (typeof OpenFangToast !== 'undefined') {
            OpenFangToast.success(
              'Spoken replies on — using ' + providerName + '. To change voice or upload your own, open Settings → Voice.',
              { actionLabel: 'Open Settings → Voice', onAction: goToVoiceSettings, duration: 8000 }
            );
          }
        } else {
          var notEnabled = s && s.enabled === false;
          var detail = (s && s.tts_error) ? (' (' + s.tts_error + ')') : '';
          var msg = notEnabled
            ? 'Spoken reply needs local TTS but [local_voice] is disabled. Enable it in Settings → Voice.'
            : 'Spoken reply not ready' + detail + '. Open Settings → Voice to see component status, install a voice, or pick a fallback.';
          self.voiceReplyHint = 'Local TTS not ready — open Settings → Voice for diagnostics and to pick / install a voice.';
          if (typeof OpenFangToast !== 'undefined') {
            OpenFangToast.warn(msg, { actionLabel: 'Open Settings → Voice', onAction: goToVoiceSettings, duration: 10000 });
          }
        }
      } catch (err) {
        var m = (err && err.message) ? err.message : 'request failed';
        self.voiceReplyHint = 'Could not verify local TTS — check connection to the daemon, then open Settings → Voice.';
        if (typeof OpenFangToast !== 'undefined') {
          OpenFangToast.error('Could not verify local voice: ' + m, { actionLabel: 'Open Settings → Voice', onAction: goToVoiceSettings });
        }
      }
    },

    // Cycle eco mode: Off → Balanced → Aggressive → Adaptive → Off.
    // Persists per-agent via ui-prefs and syncs the runtime global config so
    // the active chat always executes with the selected mode.
    cycleEcoMode: async function() {
      var next = this.efficientMode === 'off' ? 'balanced'
               : this.efficientMode === 'balanced' ? 'aggressive'
               : this.efficientMode === 'aggressive' ? 'adaptive'
               : 'off';
      this.efficientMode = next;
      // Persist instantly to localStorage so the pill survives page reload
      // without waiting for the async config fetch on next init.
      try { localStorage.setItem('armaraos-eco-mode', next); } catch(e) {}
      try {
        if (this.currentAgent && this.currentAgent.id) {
          Alpine.store('app').setAgentEcoMode(this.currentAgent.id, next);
        }
      } catch(e2) { /* non-fatal */ }
      await this._syncEcoModeToConfig(next);
    },

    // Open the eco diff modal for a message that has compressedInput set.
    openEcoDiff: function(msg) {
      if (typeof openEcoDiffModal === 'function') {
        openEcoDiffModal(msg.originalInput || '', msg.compressedInput || '', msg.savingsPct || 0);
      }
    }
  };
}
