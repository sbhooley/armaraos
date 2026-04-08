// ArmaraOS Channels Page — OpenClaw-style setup UX with QR code support
'use strict';

function channelsPage() {
  return {
    allChannels: [],
    categoryFilter: 'all',
    searchQuery: '',
    setupModal: null,
    configuring: false,
    testing: {},
    formValues: {},
    /** Optional Discord channel ID / Slack channel / Telegram chat_id for live test messages */
    testTargetId: '',
    showAdvanced: false,
    showBusinessApi: false,
    loading: true,
    loadError: '',
    pollTimer: null,

    bindings: [],
    bindingsLoading: false,
    bindingForm: { agent: '', channel: '', peer_id: '', guild_id: '', account_id: '' },
    bindingSaving: false,

    // Setup flow step tracking
    setupStep: 1, // 1=Configure, 2=Verify, 3=Ready
    testPassed: false,

    // WhatsApp QR state
    qr: {
      loading: false,
      available: false,
      dataUrl: '',
      sessionId: '',
      message: '',
      help: '',
      connected: false,
      expired: false,
      error: ''
    },
    qrPollTimer: null,

    categories: [
      { key: 'all', label: 'All' },
      { key: 'messaging', label: 'Messaging' },
      { key: 'social', label: 'Social' },
      { key: 'enterprise', label: 'Enterprise' },
      { key: 'developer', label: 'Developer' },
      { key: 'notifications', label: 'Notifications' }
    ],

    get filteredChannels() {
      var self = this;
      return this.allChannels.filter(function(ch) {
        if (self.categoryFilter !== 'all' && ch.category !== self.categoryFilter) return false;
        if (self.searchQuery) {
          var q = self.searchQuery.toLowerCase();
          return ch.name.toLowerCase().indexOf(q) !== -1 ||
                 ch.display_name.toLowerCase().indexOf(q) !== -1 ||
                 ch.description.toLowerCase().indexOf(q) !== -1;
        }
        return true;
      });
    },

    get configuredCount() {
      return this.allChannels.filter(function(ch) { return ch.configured; }).length;
    },

    categoryCount(cat) {
      var all = this.allChannels.filter(function(ch) { return cat === 'all' || ch.category === cat; });
      var configured = all.filter(function(ch) { return ch.configured; });
      return configured.length + '/' + all.length;
    },

    basicFields() {
      if (!this.setupModal || !this.setupModal.fields) return [];
      return this.setupModal.fields.filter(function(f) { return !f.advanced; });
    },

    advancedFields() {
      if (!this.setupModal || !this.setupModal.fields) return [];
      return this.setupModal.fields.filter(function(f) { return f.advanced; });
    },

    hasAdvanced() {
      return this.advancedFields().length > 0;
    },

    isQrChannel() {
      return this.setupModal && this.setupModal.setup_type === 'qr';
    },

    async loadChannels() {
      this.loading = true;
      this.loadError = '';
      try {
        var data = await OpenFangAPI.get('/api/channels');
        this.allChannels = (data.channels || []).map(function(ch) {
          ch.connected = ch.configured && ch.has_token;
          return ch;
        });
      } catch(e) {
        this.loadError = e.message || 'Could not load channels.';
      }
      this.loading = false;
      this.startPolling();
    },

    async loadBindings() {
      this.bindingsLoading = true;
      try {
        var data = await OpenFangAPI.get('/api/bindings');
        this.bindings = data.bindings || [];
      } catch(e) {
        this.bindings = [];
      }
      this.bindingsLoading = false;
    },

    async addBinding() {
      var agent = (this.bindingForm.agent || '').trim();
      if (!agent) {
        OpenFangToast.warn('Enter target agent name or UUID');
        return;
      }
      var ch = (this.bindingForm.channel || '').trim();
      var body = {
        agent: agent,
        match_rule: {
          channel: ch || null,
          peer_id: (this.bindingForm.peer_id || '').trim() || null,
          guild_id: (this.bindingForm.guild_id || '').trim() || null,
          account_id: (this.bindingForm.account_id || '').trim() || null,
          roles: []
        }
      };
      this.bindingSaving = true;
      try {
        await OpenFangAPI.post('/api/bindings', body);
        OpenFangToast.success('Binding added; channels reloaded');
        this.bindingForm = { agent: '', channel: '', peer_id: '', guild_id: '', account_id: '' };
        await this.loadBindings();
        await this.refreshStatus();
      } catch(e) {
        OpenFangToast.error(openFangErrText(e));
      }
      this.bindingSaving = false;
    },

    async removeBinding(idx) {
      var self = this;
      OpenFangToast.confirm('Remove binding', 'Remove routing rule at index ' + idx + '?', async function() {
        try {
          await OpenFangAPI.delete('/api/bindings/' + idx);
          OpenFangToast.success('Binding removed');
          await self.loadBindings();
          await self.refreshStatus();
        } catch(e) {
          OpenFangToast.error(openFangErrText(e));
        }
      });
    },

    testMessagePayload() {
      var id = (this.testTargetId || '').trim();
      if (!id) return {};
      var n = this.setupModal && this.setupModal.name;
      if (n === 'telegram') return { chat_id: id };
      return { channel_id: id };
    },

    async loadData() {
      await this.loadChannels();
      await this.loadBindings();
    },

    startPolling() {
      var self = this;
      if (this.pollTimer) clearInterval(this.pollTimer);
      this.pollTimer = setInterval(function() { self.refreshStatus(); }, 15000);
    },

    async refreshStatus() {
      try {
        var data = await OpenFangAPI.get('/api/channels');
        var byName = {};
        (data.channels || []).forEach(function(ch) { byName[ch.name] = ch; });
        this.allChannels.forEach(function(c) {
          var fresh = byName[c.name];
          if (fresh) {
            c.configured = fresh.configured;
            c.has_token = fresh.has_token;
            c.connected = fresh.configured && fresh.has_token;
            c.fields = fresh.fields;
          }
        });
      } catch(e) { console.warn('Channel refresh failed:', e.message); }
    },

    statusBadge(ch) {
      if (!ch.configured) return { text: 'Not Configured', cls: 'badge-muted' };
      if (!ch.has_token) return { text: 'Missing Token', cls: 'badge-warn' };
      if (ch.connected) return { text: 'Ready', cls: 'badge-success' };
      return { text: 'Configured', cls: 'badge-info' };
    },

    difficultyClass(d) {
      if (d === 'Easy') return 'difficulty-easy';
      if (d === 'Hard') return 'difficulty-hard';
      return 'difficulty-medium';
    },

    openSetup(ch) {
      this.setupModal = ch;
      this.testTargetId = '';
      // Pre-populate form values from saved config (non-secret fields).
      var vals = {};
      if (ch.fields) {
        ch.fields.forEach(function(f) {
          if (f.value !== undefined && f.value !== null && f.type !== 'secret') {
            vals[f.key] = String(f.value);
          }
        });
      }
      this.formValues = vals;
      this.showAdvanced = false;
      this.showBusinessApi = false;
      this.setupStep = ch.configured ? 3 : 1;
      this.testPassed = !!ch.configured;
      this.resetQR();
      // Auto-start QR flow for QR-type channels
      if (ch.setup_type === 'qr') {
        this.startQR();
      }
    },

    // ── QR Code Flow (WhatsApp Web style) ──────────────────────────

    resetQR() {
      this.qr = {
        loading: false, available: false, dataUrl: '', sessionId: '',
        message: '', help: '', connected: false, expired: false, error: ''
      };
      if (this.qrPollTimer) { clearInterval(this.qrPollTimer); this.qrPollTimer = null; }
    },

    async startQR() {
      this.qr.loading = true;
      this.qr.error = '';
      this.qr.connected = false;
      this.qr.expired = false;
      try {
        var result = await OpenFangAPI.post('/api/channels/whatsapp/qr/start', {});
        this.qr.available = result.available || false;
        this.qr.dataUrl = result.qr_data_url || '';
        this.qr.sessionId = result.session_id || '';
        this.qr.message = result.message || '';
        this.qr.help = result.help || '';
        this.qr.connected = result.connected || false;
        if (this.qr.available && this.qr.dataUrl && !this.qr.connected) {
          this.pollQR();
        }
        if (this.qr.connected) {
          OpenFangToast.success('WhatsApp connected!');
          await this.refreshStatus();
        }
      } catch(e) {
        this.qr.error = e.message || 'Could not start QR login';
      }
      this.qr.loading = false;
    },

    pollQR() {
      var self = this;
      if (this.qrPollTimer) clearInterval(this.qrPollTimer);
      this.qrPollTimer = setInterval(async function() {
        try {
          var result = await OpenFangAPI.get('/api/channels/whatsapp/qr/status?session_id=' + encodeURIComponent(self.qr.sessionId));
          if (result.connected) {
            clearInterval(self.qrPollTimer);
            self.qrPollTimer = null;
            self.qr.connected = true;
            self.qr.message = result.message || 'Connected!';
            OpenFangToast.success('WhatsApp linked successfully!');
            await self.refreshStatus();
          } else if (result.expired) {
            clearInterval(self.qrPollTimer);
            self.qrPollTimer = null;
            self.qr.expired = true;
            self.qr.message = 'QR code expired. Click to generate a new one.';
          } else {
            self.qr.message = result.message || 'Waiting for scan...';
          }
        } catch(e) { /* silent retry */ }
      }, 3000);
    },

    // ── Standard Form Flow ─────────────────────────────────────────

    async saveChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;

      // Validate required fields before hitting the server
      if (this.setupModal.fields) {
        var missing = [];
        this.setupModal.fields.forEach(function(f) {
          if (!f.required) return;
          // A field is satisfied if it already has a saved value OR the user has typed something
          var hasSaved = f.has_value;
          var hasTyped = (this.formValues[f.key] || '').trim().length > 0;
          if (!hasSaved && !hasTyped) missing.push(f.label || f.key);
        }.bind(this));
        if (missing.length > 0) {
          OpenFangToast.error('Required: ' + missing.join(', '));
          return;
        }
      }

      this.configuring = true;
      try {
        var configResult = await OpenFangAPI.post('/api/channels/' + name + '/configure', {
          fields: this.formValues
        });
        this.setupStep = 2;

        // If the server saved the config but could not activate the channel (e.g. bad token),
        // surface the reason immediately rather than hiding it behind the test step.
        if (configResult && configResult.activated === false && configResult.note) {
          OpenFangToast.warn(this.setupModal.display_name + ' saved but not activated — ' + configResult.note);
          await this.refreshStatus();
          this.configuring = false;
          return;
        }

        // Auto-test after save
        try {
          var testResult = await OpenFangAPI.post('/api/channels/' + name + '/test', this.testMessagePayload());
          if (testResult.status === 'ok') {
            this.testPassed = true;
            this.setupStep = 3;
            OpenFangToast.success(this.setupModal.display_name + ' activated!');
          } else {
            OpenFangToast.warn(this.setupModal.display_name + ' saved. ' + (testResult.message || 'Test to verify connection.'));
          }
        } catch(te) {
          OpenFangToast.warn(this.setupModal.display_name + ' saved. Test to verify connection.');
        }
        await this.refreshStatus();
      } catch(e) {
        var msg = e.message || 'Unknown error';
        var detail = e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
        var hint = e.hint ? (' Hint: ' + e.hint) : '';
        OpenFangToast.error('Failed: ' + msg + detail + hint);
      }
      this.configuring = false;
    },

    async removeChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;
      var displayName = this.setupModal.display_name;
      var self = this;
      OpenFangToast.confirm('Remove Channel', 'Remove ' + displayName + ' configuration? This will deactivate the channel.', async function() {
        try {
          await OpenFangAPI.delete('/api/channels/' + name + '/configure');
          OpenFangToast.success(displayName + ' removed and deactivated.');
          await self.refreshStatus();
          self.setupModal = null;
        } catch(e) {
          var msg = e.message || 'Unknown error';
          var detail = e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
          OpenFangToast.error('Failed: ' + msg + detail);
        }
      });
    },

    async testChannel() {
      if (!this.setupModal) return;
      var name = this.setupModal.name;
      this.testing[name] = true;
      try {
        var result = await OpenFangAPI.post('/api/channels/' + name + '/test', this.testMessagePayload());
        if (result.status === 'ok') {
          this.testPassed = true;
          this.setupStep = 3;
          OpenFangToast.success(result.message);
        } else {
          OpenFangToast.error(result.message);
        }
      } catch(e) {
        var msg = e.message || 'Unknown error';
        var detail = e.detail && e.detail !== msg ? (' — ' + e.detail) : '';
        var hint = e.hint ? (' Hint: ' + e.hint) : '';
        OpenFangToast.error('Test failed: ' + msg + detail + hint);
      }
      this.testing[name] = false;
    },

    async copyConfig(ch) {
      var tpl = ch ? ch.config_template : (this.setupModal ? this.setupModal.config_template : '');
      if (!tpl) return;
      try {
        await navigator.clipboard.writeText(tpl);
        OpenFangToast.success('Copied to clipboard');
      } catch(e) {
        OpenFangToast.error('Copy failed');
      }
    },

    destroy() {
      if (this.pollTimer) { clearInterval(this.pollTimer); this.pollTimer = null; }
      if (this.qrPollTimer) { clearInterval(this.qrPollTimer); this.qrPollTimer = null; }
    }
  };
}
