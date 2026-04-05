// ArmaraOS Event Timeline — filtered audit trail (GET /api/audit/recent)
'use strict';

function timelinePage() {
  return {
    entries: [],
    loading: false,
    loadError: '',
    filterQ: '',
    limitN: 200,
    tipHash: '',
    totalEntries: 0,

    async loadTimeline() {
      this.loading = true;
      this.loadError = '';
      try {
        var q = (this.filterQ || '').trim();
        var url = '/api/audit/recent?n=' + encodeURIComponent(String(this.limitN || 200));
        if (q) url += '&q=' + encodeURIComponent(q);
        var data = await OpenFangAPI.get(url);
        this.entries = data.entries || [];
        this.tipHash = data.tip_hash || '';
        this.totalEntries = typeof data.total === 'number' ? data.total : 0;
      } catch(e) {
        this.entries = [];
        this.loadError = e.message || 'Could not load audit timeline.';
      }
      this.loading = false;
    },

    async loadData() {
      return this.loadTimeline();
    },

    formatTime(ts) {
      if (!ts) return '—';
      try {
        var d = new Date(ts);
        return isNaN(d.getTime()) ? '—' : d.toLocaleString();
      } catch(e) { return '—'; }
    },

    truncate(s, maxLen) {
      if (s == null || s === '') return '—';
      var t = String(s);
      if (t.length <= maxLen) return t;
      return t.slice(0, maxLen) + '…';
    }
  };
}
