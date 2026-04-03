// Shared page-load error fields, Alpine store sync, and clipboard copy for dashboard pages.
'use strict';

function clearPageLoadError(ctx) {
  ctx.loadError = '';
  ctx.loadErrorDetail = '';
  ctx.loadErrorHint = '';
  ctx.loadErrorRequestId = '';
  ctx.loadErrorWhere = '';
  ctx.loadErrorServerPath = '';
}

function applyPageLoadError(ctx, e, defaultMsg) {
  ctx.loadError = e && e.message ? e.message : (defaultMsg || 'Request failed.');
  ctx.loadErrorDetail = '';
  ctx.loadErrorHint = '';
  ctx.loadErrorRequestId = '';
  ctx.loadErrorWhere = '';
  ctx.loadErrorServerPath = '';
  if (e && (e.name === 'OpenFangAPIError' || e.detail || e.hint)) {
    ctx.loadErrorDetail = e.detail || '';
    ctx.loadErrorHint = e.hint || '';
    ctx.loadErrorRequestId = e.requestId || '';
    ctx.loadErrorWhere = e.where || '';
    ctx.loadErrorServerPath = e.serverPath || '';
  }
  try {
    var st = Alpine.store('app');
    if (st) {
      st.lastError = ctx.loadError;
      st.lastErrorHint = ctx.loadErrorHint;
      st.lastErrorDetail = ctx.loadErrorDetail;
      st.lastErrorWhere = ctx.loadErrorWhere;
      st.lastErrorServerPath = ctx.loadErrorServerPath;
      st.lastErrorRequestId = ctx.loadErrorRequestId;
    }
  } catch (_) { /* ignore */ }
}

function copyPageLoadErrorDebug(ctx, title) {
  var lines = [
    title || 'ArmaraOS page load error',
    'Primary: ' + (ctx.loadError || ''),
    'Detail: ' + (ctx.loadErrorDetail || ''),
    'Hint: ' + (ctx.loadErrorHint || ''),
    'Client request: ' + (ctx.loadErrorWhere || ''),
    'API path: ' + (ctx.loadErrorServerPath || ''),
    'Request ID: ' + (ctx.loadErrorRequestId || ''),
    'Time: ' + new Date().toISOString()
  ];
  var text = lines.join('\n');
  if (navigator.clipboard && navigator.clipboard.writeText) {
    navigator.clipboard.writeText(text).then(function() {
      if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Copied debug info');
    }).catch(function() {});
  }
}
