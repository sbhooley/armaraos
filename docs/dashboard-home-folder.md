# Dashboard: Home folder browser

The embedded dashboard includes a **Home folder** page (`#home-files`) for read-only browsing of the daemon’s ArmaraOS data root (`config.home_dir`, usually `~/.armaraos`). Paths are sandboxed the same way as other file APIs: **no `..` traversal**, all paths are relative to that root.

## What you can do

- **List directories** and see file metadata (name, kind, size, mtime).
- **Read** text files up to **512 KiB** (`GET /api/armaraos-home/read`). Binary files are returned as **base64** in JSON (not editable from the UI).
- **Download** larger files (e.g. diagnostics zips) via **`GET /api/armaraos-home/download`** (cap **256 MiB**).
- **Write** UTF-8 text only when you opt in with **`[dashboard]`** glob allowlists (see below). Writes use a temp file + rename; optional **`.bak`** backup.

## Dashboard UI (View vs Download)

- **View** opens a **large, near full-viewport** modal (fills the embedded window) and loads the file through **`/read`** — fine for small text; **large or binary files** (especially **`support/*.zip`**) often hit the **512 KiB** limit and show an error in the modal.
- **Download** (green button on each **file** row, and **symlink** rows that point at files) saves the **full** object via **`/download`** (or on **desktop**, Tauri **`copy_home_file_to_downloads`** with `relativePath`).
- The modal **header** always includes **Download** once a path is known, even while loading or after a preview error; the error panel repeats **Download file** with a short note about large zips.

See **[dashboard-testing.md](dashboard-testing.md#home-folder-browser--preview-vs-download)** for manual QA steps.

## Configuration: `[dashboard]`

Add a **`[dashboard]`** section to `config.toml` (same file as `[auth]`, `[web]`, etc.):

```toml
[dashboard]
# Glob patterns relative to home_dir, forward slashes. Empty = read-only Home folder.
home_editable_globs = ["notes/**", "scratch.txt"]
home_edit_backup = true
home_edit_max_bytes = 524288
```

| Field | Default | Purpose |
|-------|---------|---------|
| `home_editable_globs` | `[]` | `globset` patterns for paths that may be **saved** from the dashboard. Empty disables all writes. |
| `home_edit_backup` | `true` | If true, before overwrite the server copies the previous file to `*.bak` once. |
| `home_edit_max_bytes` | `524288` (512 KiB) | Max UTF-8 byte length for `POST /api/armaraos-home/write`. |

**Hot reload:** Changing `[dashboard]` is picked up on config reload without a full kernel restart (same as other reloadable fields — see `config_reload`).

## Paths that are never writable

Even if a path matches `home_editable_globs`, the API **refuses writes** to:

- Anything under **`data/`**
- **`.env`**, **`secrets.env`**, **`vault.enc`**, **`config.toml`**, **`daemon.json`**
- Paths under **`.env/`** or files named **`.env`** in a subdirectory

Use a normal editor or SSH for those files.

## API reference

See **[api-reference.md](api-reference.md#armaraos-home-browser-endpoints)** for query/body shapes, response fields (`editable`, `home_edit`, `allowlist_error`), and status codes.

## Testing

- **Smoke script:** `scripts/verify-dashboard-smoke.sh` exercises **`armaraos-home/download`** for support zips. Extend or curl **`list` / `read` / `write`** when testing allowlists.
- **Manual:** Open **Home folder** in the dashboard, drill into a directory, open a file; with `home_editable_globs` set, confirm **Save** only on allowed paths and errors on blocked paths.

## Related docs

- [Configuration reference](configuration.md#dashboard) — **`[dashboard]`** table (`home_editable_globs`, backup, max bytes)
- [Dashboard testing](dashboard-testing.md)
- [Data directory](data-directory.md) — `home_dir`, `ARMARAOS_HOME`
