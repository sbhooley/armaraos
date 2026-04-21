// ArmaraOS dashboard — PostHog product analytics. Privacy: no prompts, keys, or chat.
// Events: session_started, daily_active (DAU-style), nav, usage_snapshot, command_palette; autocapture + replay when enabled.
'use strict';

(function (global) {
  var LS_ALLOW = 'armaraos-dashboard-analytics-v1';
  var LS_REPLAY = 'armaraos-dashboard-analytics-replay-v1';
  var LS_DISTINCT = 'armaraos-dashboard-analytics-distinct-id-v1';
  var SESSION_OK = 'armaraos-dashboard-analytics-session-sent-v1';
  /** YYYY-MM-DD — last calendar day we sent `armaraos_dashboard_daily_active`. */
  var LS_DAILY = 'armaraos-dashboard-daily-ping-date-v1';
  var _dailyScheduled = false;

  /** @typedef {{ configured?: boolean, apiKey?: string, api_host?: string }} PostHogDashboardCfg */
  function cfg() {
    /** @type {PostHogDashboardCfg} */
    var c = global.__ARMARAOS_POSTHOG__ || {};
    return c;
  }

  function randomId() {
    try {
      if (global.crypto && global.crypto.randomUUID) return global.crypto.randomUUID();
    } catch (e0) { /* ignore */ }
    return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, function (c) {
      var r = (Math.random() * 16) | 0;
      var v = c === 'x' ? r : (r & 0x3) | 0x8;
      return v.toString(16);
    });
  }

  function getAgentCountBestEffort() {
    try {
      var st = global.Alpine && global.Alpine.store && global.Alpine.store('app');
      if (!st) return null;
      if (typeof st.agentCount === 'number') return st.agentCount;
      if (Array.isArray(st.agents)) return st.agents.length;
    } catch (e) { /* ignore */ }
    return null;
  }

  /**
   * One automatic event per calendar day for DAU-style charts.
   * @param {boolean} [force] — if true, send with agent_count 0 when agents are not available yet (end of session fallback).
   */
  function maybeDailyPing(ph, force) {
    if (!allowMain() || !ph || typeof ph.capture !== 'function') return;
    var today = new Date().toISOString().slice(0, 10);
    try {
      if (global.localStorage.getItem(LS_DAILY) === today) return;
    } catch (e) {
      return;
    }
    var nc = getAgentCountBestEffort();
    if (nc == null && !force) return;
    var props = {
      calendar_date: today,
      shell: desktopShell() ? 'desktop' : 'browser',
      agent_count: nc != null ? nc : 0,
    };
    try {
      ph.capture('armaraos_dashboard_daily_active', props);
      global.localStorage.setItem(LS_DAILY, today);
    } catch (e2) { /* ignore */ }
  }

  function scheduleDailyPingDeferred(ph) {
    if (_dailyScheduled) return;
    _dailyScheduled = true;
    try {
      var api = ph || getPosthogApi();
      setTimeout(function () {
        maybeDailyPing(api, false);
      }, 4500);
      setTimeout(function () {
        maybeDailyPing(api, true);
      }, 22000);
    } catch (e) {
      _dailyScheduled = false;
    }
  }

  /** Call after daemon status / agents refresh so the daily event picks up `agent_count` as soon as agents load. */
  function armaraosAnalyticsRefreshEngagement() {
    if (!allowMain()) return;
    maybeDailyPing(getPosthogApi(), false);
  }

  function getDistinctId() {
    try {
      var x = global.localStorage.getItem(LS_DISTINCT);
      if (x) return x;
      x = randomId();
      global.localStorage.setItem(LS_DISTINCT, x);
      return x;
    } catch (e) {
      return 'anon-' + String(Date.now());
    }
  }

  /** Opt-out only when explicitly '0'. Missing key = opted in (default). */
  function allowMain() {
    try {
      if (!cfg().configured || !cfg().apiKey) return false;
      var v = global.localStorage.getItem(LS_ALLOW);
      if (v === '0') return false;
      return true;
    } catch (e) {
      return false;
    }
  }

  /** Opt-out only when explicitly '0'. Missing key = replay on when analytics on. */
  function allowReplay() {
    if (!allowMain()) return false;
    try {
      var v = global.localStorage.getItem(LS_REPLAY);
      if (v === '0') return false;
      return true;
    } catch (e) {
      return false;
    }
  }

  function desktopShell() {
    try {
      if (typeof global.isArmaraosDesktopShell === 'function') return !!global.isArmaraosDesktopShell();
    } catch (e) { /* ignore */ }
    return false;
  }

  var _sdkInitialized = false;
  var _posthog = null;

  function getPosthogApi() {
    if (_posthog) return _posthog;
    try {
      if (typeof global.posthog !== 'undefined') _posthog = global.posthog;
      else if (global.window && global.window.posthog) _posthog = global.window.posthog;
    } catch (e) { /* ignore */ }
    return _posthog;
  }

  function armaraosAnalyticsConfigured() {
    return !!cfg().configured && !!cfg().apiKey;
  }

  function armaraosAnalyticsAllow() {
    return allowMain();
  }

  function armaraosAnalyticsReplayAllow() {
    return allowReplay();
  }

  /**
   * Persist user consent for UI analytics (Settings or wizard).
   * @param {boolean} allow
   * @param {boolean} [replay]
   */
  function armaraosAnalyticsSetConsent(allow, replay) {
    try {
      global.localStorage.setItem(LS_ALLOW, allow ? '1' : '0');
      if (typeof replay === 'boolean') {
        global.localStorage.setItem(LS_REPLAY, replay ? '1' : '0');
      }
    } catch (e) { /* ignore */ }
    var ph = getPosthogApi();
    if (!ph) {
      queueMicrotask(function () {
        armaraosAnalyticsInit();
      });
      return;
    }
    if (!allow) {
      try {
        if (typeof ph.opt_out_capturing === 'function') ph.opt_out_capturing();
      } catch (e2) { /* ignore */ }
      return;
    }
    try {
      if (_sdkInitialized && typeof ph.opt_in_capturing === 'function') ph.opt_in_capturing();
      if (typeof ph.set_config === 'function') {
        ph.set_config({ disable_session_recording: !allowReplay() });
      }
    } catch (e3) { /* ignore */ }
    armaraosAnalyticsInit();
  }

  /**
   * Call from wizard when desktop install ping prefs load (mirrors allow checkbox).
   * @param {boolean} installPingAllowed
   */
  /**
   * Desktop wizard: when the user has never toggled dashboard prefs, mirror install-ping checkbox once.
   * If unset, default opt-in already applies — only write when mirroring explicit desktop opt-out.
   */
  function armaraosAnalyticsSyncWizardDesktopDefault(installPingAllowed) {
    try {
      if (!desktopShell()) return;
      if (global.localStorage.getItem(LS_ALLOW) != null) return;
      if (!installPingAllowed) global.localStorage.setItem(LS_ALLOW, '0');
      if (!installPingAllowed) global.localStorage.setItem(LS_REPLAY, '0');
    } catch (e) { /* ignore */ }
    armaraosAnalyticsInit();
  }

  function armaraosAnalyticsInit() {
    var c = cfg();
    if (!c.configured || !c.apiKey) return;
    if (!allowMain()) return;
    var ph = getPosthogApi();
    if (!ph || typeof ph.init !== 'function') return;
    if (_sdkInitialized) {
      try {
        if (typeof ph.opt_in_capturing === 'function') ph.opt_in_capturing();
        if (typeof ph.set_config === 'function') {
          ph.set_config({ disable_session_recording: !allowReplay() });
        }
      } catch (e0) { /* ignore */ }
      return;
    }
    var host = c.api_host || 'https://us.i.posthog.com';
    try {
      ph.init(c.apiKey, {
        api_host: host,
        persistence: 'localStorage',
        autocapture: true,
        capture_pageview: false,
        disable_session_recording: !allowReplay(),
      });
      try {
        ph.register({
          app: 'armaraos-dashboard',
          shell: desktopShell() ? 'desktop' : 'browser',
        });
        ph.identify(getDistinctId());
      } catch (e1) { /* ignore */ }
    } catch (e2) { /* ignore */ }
    _sdkInitialized = true;
    try {
      _maybeSessionStart(ph);
    } catch (e3) { /* ignore */ }
    try {
      scheduleDailyPingDeferred(ph);
    } catch (e4) { /* ignore */ }
  }

  function _maybeSessionStart(ph) {
    try {
      var k = SESSION_OK + '-' + new Date().toDateString();
      if (global.sessionStorage.getItem(k)) return;
      global.sessionStorage.setItem(k, '1');
      var props = {
        shell: desktopShell() ? 'desktop' : 'browser',
      };
      try {
        var st = global.Alpine && global.Alpine.store && global.Alpine.store('app');
        if (st && st.version) props.daemon_version_reported = String(st.version);
      } catch (e0) { /* ignore */ }
      props.session_replay = allowReplay();
      ph.capture('armaraos_dashboard_session_started', props);
    } catch (e1) { /* ignore */ }
  }

  /**
   * @param {string} event
   * @param {Record<string, unknown>} [props]
   */
  function armaraosTrack(event, props) {
    if (!allowMain()) return;
    var ph = getPosthogApi();
    if (!ph || typeof ph.capture !== 'function') return;
    try {
      ph.capture(event, props || {});
    } catch (e) { /* ignore */ }
  }

  /**
   * @param {string} page
   * @param {string} [query]
   */
  function armaraosAnalyticsNav(page, query) {
    var safe = { page: String(page || '') };
    if (query && String(page) === 'home-files') {
      try {
        var p = new URLSearchParams(query);
        if (p.has('path')) safe.has_path_query = true;
      } catch (e) { /* ignore */ }
    } else if (query && String(page) === 'scheduler') {
      try {
        var p2 = new URLSearchParams(query);
        if (p2.has('ainl')) safe.scheduler_ainl_prefill = true;
      } catch (e2) { /* ignore */ }
    }
    armaraosTrack('armaraos_dashboard_nav', safe);
  }

  /**
   * Aggregate diagnostics only (no model ids, no keys).
   * @param {{ agent_count?: number, providers_configured?: number, default_provider_kind?: string }} snap
   */
  function armaraosAnalyticsUsageSnapshot(snap) {
    var o = {};
    if (snap && typeof snap.agent_count === 'number') o.agent_count = snap.agent_count;
    if (snap && typeof snap.providers_configured === 'number') o.providers_configured = snap.providers_configured;
    if (snap && snap.default_provider_kind)
      o.default_provider_kind = String(snap.default_provider_kind).slice(0, 64);
    if (Object.keys(o).length === 0) return;
    armaraosTrack('armaraos_dashboard_usage_snapshot', o);
  }

  global.__ARMARAOS_ANALYTICS__ = {
    configured: armaraosAnalyticsConfigured,
    allow: armaraosAnalyticsAllow,
    replayAllow: armaraosAnalyticsReplayAllow,
    setConsent: armaraosAnalyticsSetConsent,
    init: armaraosAnalyticsInit,
    track: armaraosTrack,
    nav: armaraosAnalyticsNav,
    usageSnapshot: armaraosAnalyticsUsageSnapshot,
    syncWizardDesktopDefault: armaraosAnalyticsSyncWizardDesktopDefault,
    refreshEngagement: armaraosAnalyticsRefreshEngagement,
  };

  global.armaraosTrack = armaraosTrack;
  global.armaraosAnalyticsInit = armaraosAnalyticsInit;

  try {
    queueMicrotask(function () {
      armaraosAnalyticsInit();
    });
  } catch (e) {
    armaraosAnalyticsInit();
  }
})(typeof window !== 'undefined' ? window : globalThis);
