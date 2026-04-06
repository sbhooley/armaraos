# Dashboard: Bookmarks

The **Bookmarks** route (`#bookmarks`) lists messages and media you save from **Chat**. It is implemented in the embedded Alpine dashboard alongside **All Agents**, **Sessions**, and **Comms**.

## Source files

| Piece | Location |
|-------|----------|
| Nav + page shell | `crates/openfang-api/static/index_body.html` — `page === 'bookmarks'` |
| Page logic | `crates/openfang-api/static/js/pages/bookmarks.js` — `bookmarksPage()` |
| Styles | `crates/openfang-api/static/css/components.css` — classes prefixed `bookmarks-` |

## Saving from chat

In an agent thread, use the **bookmark** control on a message (alongside copy). A modal collects a **title**, optional **category** (existing or new), and stores the item.

## Storage: desktop vs browser

| Environment | Where data lives |
|-------------|------------------|
| **Tauri desktop app** | Synced under the app **data directory** (alongside other ArmaraOS prefs) so bookmarks survive app updates. |
| **Plain browser tab** | **`localStorage` only** for that origin (e.g. `http://127.0.0.1:4200`). Clearing site data removes bookmarks. |

The in-page subtitle reminds users of this split.

## Categories and ordering

- Categories are user-defined; each holds ordered bookmark items.
- The sidebar supports **reorder** (↑/↓) for categories and for items within the feed.

## Images and uploads

Bookmarked **images** render correctly while the daemon serves **upload** assets from the usual chat upload path. If the daemon is offline or paths are invalid, thumbnails may fail to load.

## Related

- [Dashboard Get started UI](dashboard-overview-ui.md) — default `#overview` landing (sidebar **Get started**), Quick actions, Setup Wizard gating  
- [Dashboard Settings & Runtime UI](dashboard-settings-runtime-ui.md) — `#settings` / `#runtime` shell polish  
- [Getting started](getting-started.md) — hash routing and `#overview` compatibility  
- [Desktop app](desktop.md) — Tauri shell vs browser  
