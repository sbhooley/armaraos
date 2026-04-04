// ArmaraOS Bookmarks — saved chat outputs (localStorage) + Bookmarks page
'use strict';

(function () {
  var STORAGE_KEY = 'armaraos-bookmarks-v1';
  var MAX_TOOL_RESULT = 100000;

  function defaultState() {
    return {
      version: 1,
      categories: [{ id: 'cat-default', name: 'General', order: 0 }],
      items: [],
    };
  }

  function load() {
    try {
      var raw = localStorage.getItem(STORAGE_KEY);
      if (!raw) return defaultState();
      var d = JSON.parse(raw);
      if (!d || !Array.isArray(d.categories) || !Array.isArray(d.items)) return defaultState();
      if (!d.categories.length) {
        d.categories = defaultState().categories;
      }
      return d;
    } catch (e) {
      return defaultState();
    }
  }

  function save(state) {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
    } catch (e) {
      if (typeof OpenFangToast !== 'undefined') {
        OpenFangToast.error('Could not save bookmarks (storage full?)');
      }
    }
    try {
      window.dispatchEvent(new CustomEvent('armaraos-bookmarks-changed'));
    } catch (e2) { /* ignore */ }
  }

  function genId(prefix) {
    return prefix + '-' + Date.now().toString(36) + '-' + Math.random().toString(36).slice(2, 9);
  }

  function clampTool(t) {
    if (!t) return null;
    var input = typeof t.input === 'string' ? t.input : JSON.stringify(t.input || '');
    var result = typeof t.result === 'string' ? t.result : JSON.stringify(t.result || '');
    if (result.length > MAX_TOOL_RESULT) {
      result = result.slice(0, MAX_TOOL_RESULT) + '\n… [truncated]';
    }
    return {
      name: t.name || 'tool',
      input: input.length > 20000 ? input.slice(0, 20000) + '…' : input,
      result: result,
      is_error: !!t.is_error,
    };
  }

  window.ArmaraosBookmarks = {
    load: load,
    save: save,
    genId: genId,

    ensureCategory: function (name) {
      var st = load();
      var n = (name || '').trim();
      if (!n) return st.categories[0].id;
      for (var i = 0; i < st.categories.length; i++) {
        if (st.categories[i].name.toLowerCase() === n.toLowerCase()) {
          return st.categories[i].id;
        }
      }
      var maxOrder = 0;
      st.categories.forEach(function (c) {
        if (c.order > maxOrder) maxOrder = c.order;
      });
      var id = genId('cat');
      st.categories.push({ id: id, name: n, order: maxOrder + 1 });
      save(st);
      return id;
    },

    addItem: function (opts) {
      var st = load();
      var catId = opts.categoryId || (st.categories[0] && st.categories[0].id);
      if (!catId) {
        catId = genId('cat');
        st.categories.push({ id: catId, name: 'General', order: 0 });
      }
      var tools = null;
      if (opts.tools && opts.tools.length) {
        tools = opts.tools.map(clampTool).filter(Boolean);
      }
      var images = (opts.images || []).map(function (img) {
        return { file_id: img.file_id, filename: img.filename || 'image' };
      });
      var item = {
        id: genId('bm'),
        categoryId: catId,
        title: (opts.title || '').trim() || 'Bookmark',
        text: opts.text || '',
        createdAt: new Date().toISOString(),
        agentId: opts.agentId || null,
        agentName: opts.agentName || null,
        images: images,
        tools: tools,
      };
      st.items.push(item);
      save(st);
      return item;
    },

    deleteItem: function (id) {
      var st = load();
      st.items = st.items.filter(function (x) {
        return x.id !== id;
      });
      save(st);
    },

    moveItemToCategory: function (itemId, categoryId) {
      var st = load();
      st.items.forEach(function (it) {
        if (it.id === itemId) it.categoryId = categoryId;
      });
      save(st);
    },

    reorderItemInCategory: function (categoryId, itemId, delta) {
      var st = load();
      var inCat = st.items.filter(function (x) {
        return x.categoryId === categoryId;
      });
      var idx = -1;
      for (var i = 0; i < inCat.length; i++) {
        if (inCat[i].id === itemId) {
          idx = i;
          break;
        }
      }
      if (idx < 0) return;
      var j = idx + delta;
      if (j < 0 || j >= inCat.length) return;
      var a = inCat[idx];
      var b = inCat[j];
      var ai = st.items.indexOf(a);
      var bi = st.items.indexOf(b);
      st.items[ai] = b;
      st.items[bi] = a;
      save(st);
    },

    addCategory: function (name) {
      var n = (name || '').trim();
      if (!n) return null;
      var st = load();
      if (st.categories.some(function (c) { return c.name.toLowerCase() === n.toLowerCase(); })) {
        return null;
      }
      var maxOrder = 0;
      st.categories.forEach(function (c) {
        if (c.order > maxOrder) maxOrder = c.order;
      });
      var id = genId('cat');
      st.categories.push({ id: id, name: n, order: maxOrder + 1 });
      save(st);
      return id;
    },

    renameCategory: function (categoryId, name) {
      var n = (name || '').trim();
      if (!n) return;
      var st = load();
      st.categories.forEach(function (c) {
        if (c.id === categoryId) c.name = n;
      });
      save(st);
    },

    deleteCategory: function (categoryId) {
      var st = load();
      if (st.categories.length <= 1) return;
      var fallback = st.categories.find(function (c) { return c.id !== categoryId; });
      if (!fallback) return;
      st.items.forEach(function (it) {
        if (it.categoryId === categoryId) it.categoryId = fallback.id;
      });
      st.categories = st.categories.filter(function (c) {
        return c.id !== categoryId;
      });
      save(st);
    },

    reorderCategory: function (categoryId, delta) {
      var st = load();
      var sorted = st.categories.slice().sort(function (a, b) {
        return a.order - b.order;
      });
      var idx = sorted.findIndex(function (c) {
        return c.id === categoryId;
      });
      if (idx < 0) return;
      var j = idx + delta;
      if (j < 0 || j >= sorted.length) return;
      var t = sorted[idx];
      sorted[idx] = sorted[j];
      sorted[j] = t;
      sorted.forEach(function (c, i) {
        c.order = i;
      });
      st.categories = sorted;
      save(st);
    },
  };
})();

function bookmarksPage() {
  return {
    categories: [],
    items: [],
    selectedCategoryId: '',
    newCategoryName: '',
    editingCategoryId: '',
    editingCategoryName: '',
    expandedTools: {},

    init() {
      var self = this;
      this.reload();
      window.addEventListener('armaraos-bookmarks-changed', function () {
        self.reload();
      });
    },

    reload() {
      var st = ArmaraosBookmarks.load();
      this.categories = st.categories.slice().sort(function (a, b) {
        return a.order - b.order;
      });
      this.items = st.items;
      var sel = this.selectedCategoryId;
      if (!sel || !this.categories.some(function (c) { return c.id === sel; })) {
        this.selectedCategoryId = this.categories[0] ? this.categories[0].id : '';
      }
    },

    get itemsInCategory() {
      var cid = this.selectedCategoryId;
      if (!cid) return [];
      return this.items.filter(function (x) {
        return x.categoryId === cid;
      });
    },

    addCategory() {
      var id = ArmaraosBookmarks.addCategory(this.newCategoryName);
      if (id) {
        this.newCategoryName = '';
        this.reload();
        this.selectedCategoryId = id;
        if (typeof OpenFangToast !== 'undefined') OpenFangToast.success('Category added');
      }
    },

    startEditCategory(c) {
      this.editingCategoryId = c.id;
      this.editingCategoryName = c.name;
    },

    commitEditCategory() {
      ArmaraosBookmarks.renameCategory(this.editingCategoryId, this.editingCategoryName);
      this.editingCategoryId = '';
      this.reload();
    },

    removeCategory(c) {
      if (this.categories.length <= 1) return;
      var self = this;
      var run = function () {
        ArmaraosBookmarks.deleteCategory(c.id);
        self.reload();
      };
      if (typeof OpenFangToast !== 'undefined') {
        OpenFangToast.confirm('Delete category', 'Bookmarks in this category move to another.', run);
      } else if (typeof window.confirm === 'function' && window.confirm('Delete category? Items will move to another.')) {
        run();
      }
    },

    removeSelectedCategory() {
      var c = this.categories.find(function (x) {
        return x.id === this.selectedCategoryId;
      }.bind(this));
      if (c) this.removeCategory(c);
    },

    moveCat(c, delta) {
      ArmaraosBookmarks.reorderCategory(c.id, delta);
      this.reload();
    },

    deleteItem(it) {
      var self = this;
      var run = function () {
        ArmaraosBookmarks.deleteItem(it.id);
        self.reload();
      };
      if (typeof OpenFangToast !== 'undefined') {
        OpenFangToast.confirm('Remove bookmark', 'Delete this saved message?', run);
      } else if (typeof window.confirm === 'function' && window.confirm('Delete this bookmark?')) {
        run();
      }
    },

    moveItem(it, ev) {
      ArmaraosBookmarks.moveItemToCategory(it.id, ev.target.value);
      this.reload();
    },

    itemUp(it) {
      ArmaraosBookmarks.reorderItemInCategory(this.selectedCategoryId, it.id, -1);
      this.reload();
    },

    itemDown(it) {
      ArmaraosBookmarks.reorderItemInCategory(this.selectedCategoryId, it.id, 1);
      this.reload();
    },

    formatDate(iso) {
      if (!iso) return '';
      try {
        return new Date(iso).toLocaleString();
      } catch (e) {
        return iso;
      }
    },

    renderMarkdown: renderMarkdown,

    toggleToolExpand(itemId, toolIdx) {
      var k = itemId + '-' + toolIdx;
      this.expandedTools[k] = !this.expandedTools[k];
    },

    isToolExpanded(itemId, toolIdx) {
      return !!this.expandedTools[itemId + '-' + toolIdx];
    },
  };
}
