# API Reference

ArmaraOS exposes a REST API, WebSocket endpoints, and SSE streaming when the daemon is running. The default listen address is `http://127.0.0.1:4200`.

All responses include security headers (CSP, X-Frame-Options, X-Content-Type-Options, HSTS) and are protected by a GCRA cost-aware rate limiter with per-IP token bucket tracking and automatic stale entry cleanup. ArmaraOS implements 16 security systems including Merkle audit trails, taint tracking, WASM dual metering, Ed25519 manifest signing, SSRF protection, subprocess sandboxing, and secret zeroization.

## Table of Contents

- [Authentication](#authentication)
- [Agent Endpoints](#agent-endpoints)
- [Workflow Endpoints](#workflow-endpoints)
- [Trigger Endpoints](#trigger-endpoints)
- [Memory Endpoints](#memory-endpoints)
- [Channel Endpoints](#channel-endpoints)
- [Template Endpoints](#template-endpoints)
- [System Endpoints](#system-endpoints)
- [Model Catalog Endpoints](#model-catalog-endpoints)
- [Provider Configuration Endpoints](#provider-configuration-endpoints)
- [Skills & Marketplace Endpoints](#skills--marketplace-endpoints)
- [ClawHub Endpoints](#clawhub-endpoints)
- [MCP & A2A Protocol Endpoints](#mcp--a2a-protocol-endpoints)
- [Audit & Security Endpoints](#audit--security-endpoints)
- [Usage & Analytics Endpoints](#usage--analytics-endpoints)
- [Migration Endpoints](#migration-endpoints)
- [Session Management Endpoints](#session-management-endpoints)
- [Slash Templates Endpoints](#slash-templates-endpoints)
- [UI Preferences Endpoints](#ui-preferences-endpoints)
- [Cron/Scheduler Endpoints](#cronscheduler-endpoints)
- [Support diagnostics bundle](#support-diagnostics-redacted-bundle)
- [ArmaraOS Home Browser Endpoints](#armaraos-home-browser-endpoints)
- [WebSocket Protocol](#websocket-protocol)
- [SSE Streaming](#sse-streaming)
- [OpenAI-Compatible API](#openai-compatible-api)
- [Error Responses](#error-responses)

---

## Authentication

When an API key is configured in `config.toml`, all endpoints (except `/api/health` and `/`) require a Bearer token:

```
Authorization: Bearer <your-api-key>
```

### Setting the API Key

Add to `~/.armaraos/config.toml`:

```toml
api_key = "your-secret-api-key"
```

### No Authentication

If `api_key` is empty or not set, the API is accessible without authentication. CORS is restricted to localhost origins in this mode.

### Public Endpoints (No Auth Required)

- `GET /api/health`
- `GET /` (WebChat UI)

---

## Agent Endpoints

### GET /api/agents

List all running agents.

Each object includes **`system_prompt`** and full **`identity`** (`emoji`, `avatar_url`, `color`, `archetype`, `vibe`, `greeting_style`) so dashboards can populate edit forms without a second round-trip. Other fields (`model_tier`, `auth_status`, `ready`, `last_active`, `mode`, `profile`) reflect runtime and catalog state.

**Response** `200 OK`:

```json
[
  {
    "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "name": "hello-world",
    "state": "Running",
    "mode": "Normal",
    "created_at": "2025-01-15T10:30:00Z",
    "last_active": "2025-01-15T11:00:00Z",
    "model_provider": "groq",
    "model_name": "llama-3.3-70b-versatile",
    "model_tier": "free",
    "auth_status": "configured",
    "ready": true,
    "profile": "full",
    "system_prompt": "You are a helpful assistant.",
    "identity": {
      "emoji": "🤖",
      "avatar_url": null,
      "color": "#FF5C00",
      "archetype": "assistant",
      "vibe": "professional",
      "greeting_style": null
    }
  }
]
```

### GET /api/agents/{id}

Returns detailed information about a single agent.

Adds **`system_prompt`**, full **`identity`**, per-agent **`tool_allowlist`** / **`tool_blocklist`**, **`fallback_models`**, and related fields on top of the list payload shape (without `model_tier` / `ready` enrichment).

**Response** `200 OK`:

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "hello-world",
  "state": "Running",
  "mode": "Normal",
  "profile": "full",
  "created_at": "2025-01-15T10:30:00Z",
  "session_id": "s1b2c3d4-...",
  "model": {
    "provider": "groq",
    "model": "llama-3.3-70b-versatile"
  },
  "capabilities": {
    "tools": ["file_read", "file_list", "web_fetch"],
    "network": []
  },
  "description": "A friendly greeting agent",
  "tags": [],
  "system_prompt": "You are a helpful assistant.",
  "identity": {
    "emoji": "🤖",
    "avatar_url": null,
    "color": "#FF5C00",
    "archetype": "assistant",
    "vibe": "professional",
    "greeting_style": null
  },
  "skills": [],
  "skills_mode": "all",
  "mcp_servers": [],
  "mcp_servers_mode": "all",
  "fallback_models": [],
  "tool_allowlist": [],
  "tool_blocklist": [],
  "scheduled_ainl_host_adapter": {
    "source": "default_online",
    "summary": "Default full host-adapter allowlist (agent has network, tools, shell, spawn, or OFP).",
    "adapter_count": 31,
    "ainl_allow_ir_declared_adapters": "1"
  }
}
```

`scheduled_ainl_host_adapter` describes scheduled **`ainl run`** env for this agent: **`AINL_HOST_ADAPTER_ALLOWLIST`** (`source` is `none`, `metadata` with `allowlist`, or `default_online` with `adapter_count`) and **`AINL_ALLOW_IR_DECLARED_ADAPTERS`** via **`ainl_allow_ir_declared_adapters`** (`"1"` = ignore env allowlist in AINL Python, `"0"` = do not). See **`docs/scheduled-ainl.md`**.

### POST /api/agents

Spawn a new agent from a TOML manifest.

The body must deserialize to **`AgentManifest`**: use **top-level** fields such as `name`, `description`, `profile`, and `[model]`. Do **not** nest those keys under an `[agent]` table; nested tables do not map to the manifest root and can yield a missing or default agent name. On-disk `agent.toml` files and the dashboard **Setup Wizard** use this flat shape — see [dashboard-setup-wizard.md](dashboard-setup-wizard.md).

**Request Body** (JSON):

```json
{
  "manifest_toml": "name = \"my-agent\"\nversion = \"0.1.0\"\ndescription = \"Test agent\"\nauthor = \"me\"\nmodule = \"builtin:chat\"\n\n[model]\nprovider = \"groq\"\nmodel = \"llama-3.3-70b-versatile\"\n\n[capabilities]\ntools = [\"file_read\", \"web_fetch\"]\nmemory_read = [\"*\"]\nmemory_write = [\"self.*\"]\n"
}
```

**Response** `201 Created`:

```json
{
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "my-agent"
}
```

### PUT /api/agents/{id}/update

Update an agent's configuration at runtime.

**Request Body**:

```json
{
  "description": "Updated description",
  "system_prompt": "You are a specialized assistant.",
  "tags": ["updated", "v2"]
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "agent_id": "a1b2c3d4-..."
}
```

### PUT /api/agents/{id}/mode

Set an agent's operating mode. `Stable` mode pins the current model and freezes the skill registry. `Normal` mode restores default behavior.

**Request Body**:

```json
{
  "mode": "Stable"
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "mode": "Stable",
  "agent_id": "a1b2c3d4-..."
}
```

### PATCH /api/agents/{id}/config

Hot-update name, description, system prompt, visual identity, model, provider, and fallback model chain. Omitted JSON keys leave those fields unchanged.

**Merge semantics (important for API clients):**

- **`description`**, **`system_prompt`**: if the key is present but the string is **empty**, the server **does not** apply the update (avoids accidental wipes from clients that used to send `""` when they did not have the real value).
- **Identity** (`emoji`, `avatar_url`, `archetype`, `vibe`, `greeting_style`): **absent** key → keep current; **empty string** → clear to `null`; **non-empty** → set. **`color`**: empty string keeps the current color (invalid payloads are ignored).

The updated agent row is persisted to SQLite after a successful patch.

**Request body** (all fields optional):

```json
{
  "name": "my-agent",
  "description": "Updated description",
  "system_prompt": "You are a specialized assistant.",
  "emoji": "🤖",
  "avatar_url": "https://example.com/avatar.png",
  "color": "#FF5C00",
  "archetype": "coder",
  "vibe": "technical",
  "greeting_style": "warm",
  "model": "llama-3.3-70b-versatile",
  "provider": "groq",
  "fallback_models": [{ "provider": "groq", "model": "llama-3.1-8b-instant" }]
}
```

**Response** `200 OK`:

```json
{
  "status": "ok",
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

### PATCH /api/agents/{id}/identity

Update visual / personality identity only. Uses the **same merge rules** as the identity fields on `PATCH …/config`: omitted keys keep existing values; empty strings clear optional strings; empty `color` keeps the previous color.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

### GET /api/agents/{id}/tools

Returns the agent’s explicit tool **allowlist** and **blocklist** (manifest fields). An empty allowlist means “no extra restriction” — effective tools come from the agent’s named **profile** and capabilities.

**Response** `200 OK`:

```json
{
  "tool_allowlist": ["file_read", "channel_send"],
  "tool_blocklist": []
}
```

### PUT /api/agents/{id}/tools

Replace allowlist and/or blocklist. Supply at least one of `tool_allowlist` or `tool_blocklist` (arrays of tool name strings). Omitted key means that list is left unchanged on the server. Changes are persisted to SQLite.

**Request body**:

```json
{
  "tool_allowlist": ["file_read", "web_fetch", "channel_send", "event_publish"],
  "tool_blocklist": []
}
```

**Response** `200 OK`:

```json
{
  "status": "ok"
}
```

### POST /api/agents/{id}/message

Send a message to an agent and receive the complete response.

**Request Body**:

```json
{
  "message": "What files are in the current directory?"
}
```

**Response** `200 OK`:

```json
{
  "response": "Here are the files in the current directory:\n- Cargo.toml\n- README.md\n...",
  "input_tokens": 142,
  "output_tokens": 87,
  "iterations": 1,
  "compression_savings_pct": 34,
  "compressed_input": "Understand why the dashboard…"
}
```

When **Ultra Cost-Efficient Mode** is active and the prompt compressor saves tokens, **`compression_savings_pct`** (1–100) and **`compressed_input`** (the text sent to the LLM) may be present. Omitted when there is no compression or zero savings. See [prompt-compression-efficient-mode.md](prompt-compression-efficient-mode.md).

### GET /api/agents/{id}/session

Returns the agent's conversation history.

**Response** `200 OK`:

```json
{
  "session_id": "s1b2c3d4-...",
  "agent_id": "a1b2c3d4-...",
  "message_count": 4,
  "context_window_tokens": 1250,
  "messages": [
    {
      "role": "User",
      "content": "Hello"
    },
    {
      "role": "Assistant",
      "content": "Hello! How can I help you?"
    }
  ]
}
```

### DELETE /api/agents/{id}

Terminate an agent and remove it from the registry.

**Response** `200 OK`:

```json
{
  "status": "killed",
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}
```

---

## Workflow Endpoints

### GET /api/workflows

List all registered workflows.

**Response** `200 OK`:

```json
[
  {
    "id": "w1b2c3d4-...",
    "name": "code-review-pipeline",
    "description": "Automated code review workflow",
    "steps": 3,
    "created_at": "2025-01-15T10:30:00Z"
  }
]
```

### POST /api/workflows

Create a new workflow definition.

**Request Body** (JSON):

```json
{
  "name": "code-review-pipeline",
  "description": "Review code changes with multiple agents",
  "steps": [
    {
      "name": "analyze",
      "agent_name": "coder",
      "prompt": "Analyze this code for potential issues: {{input}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "fail",
      "output_var": "analysis"
    },
    {
      "name": "security-check",
      "agent_name": "security-auditor",
      "prompt": "Review this code analysis for security vulnerabilities: {{analysis}}",
      "mode": "sequential",
      "timeout_secs": 120,
      "error_mode": "skip"
    },
    {
      "name": "summarize",
      "agent_name": "writer",
      "prompt": "Write a concise code review summary based on: {{analysis}}",
      "mode": "sequential",
      "timeout_secs": 60,
      "error_mode": "fail"
    }
  ]
}
```

**Step configuration options:**

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Step name |
| `agent_id` | string | Agent UUID (use either this or `agent_name`) |
| `agent_name` | string | Agent name (use either this or `agent_id`) |
| `prompt` | string | Prompt template with `{{input}}` and `{{output_var}}` placeholders |
| `mode` | string | `"sequential"`, `"fan_out"`, `"collect"`, `"conditional"`, `"loop"` |
| `timeout_secs` | integer | Timeout per step (default: 120) |
| `error_mode` | string | `"fail"`, `"skip"`, `"retry"` |
| `max_retries` | integer | For `"retry"` error mode (default: 3) |
| `output_var` | string | Variable name to store output for later steps |
| `condition` | string | For `"conditional"` mode |
| `max_iterations` | integer | For `"loop"` mode (default: 5) |
| `until` | string | For `"loop"` mode: stop condition |

**Response** `201 Created`:

```json
{
  "workflow_id": "w1b2c3d4-..."
}
```

### POST /api/workflows/{id}/run

Execute a workflow.

**Request Body**:

```json
{
  "input": "Review this pull request: ..."
}
```

**Response** `200 OK`:

```json
{
  "run_id": "r1b2c3d4-...",
  "output": "Code review summary:\n- No critical issues found\n...",
  "status": "completed"
}
```

### GET /api/workflows/{id}/runs

List execution history for a workflow.

**Response** `200 OK`:

```json
[
  {
    "id": "r1b2c3d4-...",
    "workflow_name": "code-review-pipeline",
    "state": "Completed",
    "steps_completed": 3,
    "started_at": "2025-01-15T10:30:00Z",
    "completed_at": "2025-01-15T10:32:15Z"
  }
]
```

---

## Trigger Endpoints

### GET /api/triggers

List all triggers. Optionally filter by agent.

**Query Parameters:**
- `agent_id` (optional): Filter by agent UUID

**Response** `200 OK`:

```json
[
  {
    "id": "t1b2c3d4-...",
    "agent_id": "a1b2c3d4-...",
    "pattern": {"lifecycle": {}},
    "prompt_template": "Event: {{event}}",
    "enabled": true,
    "fire_count": 5,
    "max_fires": 0,
    "created_at": "2025-01-15T10:30:00Z"
  }
]
```

### POST /api/triggers

Create a new event trigger.

**Request Body**:

```json
{
  "agent_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "pattern": {
    "agent_spawned": {
      "name_pattern": "*"
    }
  },
  "prompt_template": "A new agent was spawned: {{event}}. Review its capabilities.",
  "max_fires": 0
}
```

**Supported pattern types:**

| Pattern | Description |
|---------|-------------|
| `{"lifecycle": {}}` | All lifecycle events |
| `{"agent_spawned": {"name_pattern": "*"}}` | Agent spawn events |
| `{"agent_terminated": {}}` | Agent termination events |
| `{"all": {}}` | All events |

**Response** `201 Created`:

```json
{
  "trigger_id": "t1b2c3d4-...",
  "agent_id": "a1b2c3d4-..."
}
```

### PUT /api/triggers/{id}

Update an existing trigger's configuration.

**Request Body**:

```json
{
  "prompt_template": "Updated template: {{event}}",
  "enabled": false,
  "max_fires": 10
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "trigger_id": "t1b2c3d4-..."
}
```

### DELETE /api/triggers/{id}

Remove a trigger.

**Response** `200 OK`:

```json
{
  "status": "removed",
  "trigger_id": "t1b2c3d4-..."
}
```

---

## Memory Endpoints

### GET /api/memory/agents/{id}/kv

List all key-value pairs for an agent.

**Response** `200 OK`:

```json
{
  "kv_pairs": [
    {"key": "preferences", "value": {"theme": "dark"}},
    {"key": "state", "value": {"step": 3}}
  ]
}
```

### GET /api/memory/agents/{id}/kv/{key}

Get a specific key-value pair.

**Response** `200 OK`:

```json
{
  "key": "preferences",
  "value": {"theme": "dark"}
}
```

**Response** `404 Not Found` (key does not exist):

```json
{
  "error": "Key 'preferences' not found"
}
```

### PUT /api/memory/agents/{id}/kv/{key}

Set a key-value pair. Creates or overwrites.

**Request Body**:

```json
{
  "value": {"theme": "dark", "language": "en"}
}
```

**Response** `200 OK`:

```json
{
  "status": "stored",
  "key": "preferences"
}
```

### DELETE /api/memory/agents/{id}/kv/{key}

Delete a key-value pair.

**Response** `200 OK`:

```json
{
  "status": "deleted",
  "key": "preferences"
}
```

---

## Channel Endpoints

### GET /api/channels

List configured channel adapters and their status. Supports 40 channel adapters including Telegram, Discord, Slack, WhatsApp, Matrix, Email, Teams, Mattermost, IRC, Google Chat, Twitch, Rocket.Chat, Zulip, XMPP, LINE, Viber, Messenger, Reddit, Mastodon, Bluesky, and more.

**Response** `200 OK`:

```json
{
  "channels": [
    {
      "name": "telegram",
      "enabled": true,
      "has_token": true
    },
    {
      "name": "discord",
      "enabled": true,
      "has_token": false
    }
  ],
  "total": 2
}
```

### POST /api/channels/reload

Stop the current channel bridge, re-read config and **`secrets.env`** from disk, and start bridges again. Use after editing channel definitions or tokens.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "started": ["discord", "telegram"]
}
```

`started` is the list of channel names that were started. **Dashboard:** **Reload channels** (same locations as config reload). **Auth:** same as other POST routes when `api_key` is set.

---

## Template Endpoints

### GET /api/templates

List available agent templates from the agents directory.

**Response** `200 OK`:

```json
{
  "templates": [
    {
      "name": "hello-world",
      "description": "A friendly greeting agent",
      "path": "/home/user/.armaraos/agents/hello-world/agent.toml"
    },
    {
      "name": "coder",
      "description": "Expert coding assistant",
      "path": "/home/user/.armaraos/agents/coder/agent.toml"
    }
  ],
  "total": 30
}
```

### GET /api/templates/{name}

Get a specific template's manifest and raw TOML.

**Response** `200 OK`:

```json
{
  "name": "hello-world",
  "manifest": {
    "name": "hello-world",
    "description": "A friendly greeting agent",
    "module": "builtin:chat",
    "tags": [],
    "model": {
      "provider": "groq",
      "model": "llama-3.3-70b-versatile"
    },
    "capabilities": {
      "tools": ["file_read", "file_list", "web_fetch"],
      "network": []
    }
  },
  "manifest_toml": "name = \"hello-world\"\nversion = \"0.1.0\"\n..."
}
```

---

## System Endpoints

### Support diagnostics (redacted bundle)

Generate and download a **redacted** support archive under **`support/`** in the ArmaraOS home directory (same tree as [ArmaraOS Home Browser](#armaraos-home-browser-endpoints)).

**Loopback:** When the client address is **127.0.0.1** / **::1**, both routes below may be used **without** `Authorization: Bearer` even if `api_key` is set — so the embedded dashboard and desktop shell can create and save zips locally. **Non-loopback** callers must use normal API authentication. See [troubleshooting.md](troubleshooting.md#remote-access-vs-loopback-diagnostics).

#### POST /api/support/diagnostics

**Body:** JSON object (may be `{}`).

**Response** `200 OK` (illustrative):

```json
{
  "status": "ok",
  "bundle_path": "/Users/you/.armaraos/support/armaraos-diagnostics-20260405-120000.zip",
  "bundle_filename": "armaraos-diagnostics-20260405-120000.zip",
  "relative_path": "support/armaraos-diagnostics-20260405-120000.zip"
}
```

**Zip layout (order matters for support):** `README.txt` → `diagnostics_snapshot.json` → `config.toml` → `secrets.env` (if present) → `meta.json` → `audit.json` → `data/openfang.db*` → `home/logs/…`.

- **`README.txt`** — Index of bundle files; points triage at `diagnostics_snapshot.json` first.
- **`diagnostics_snapshot.json`** — Full support snapshot (config schema effective + binary, runtime fields, paths, memory SQLite `user_version` vs expected, env-var **presence** flags only).
- **`meta.json`** — Compact metadata (overlaps the snapshot; retained for older tooling).

Also: redacted `secrets.env`, `audit.json`, SQLite + WAL/SHM, recent logs.

#### GET /api/support/diagnostics/download

**Query:** `name` — must be exactly `armaraos-diagnostics-YYYYMMDD-HHMMSS.zip` (no `/`, `\\`, or `..`).

Streams the zip with `Content-Disposition: attachment`. Use `bundle_filename` from the POST response.

---

### ArmaraOS Home Browser Endpoints

Browse files under the configured ArmaraOS **`home_dir`** (typically `~/.armaraos`). All `path` values are **relative** to that root; `..` and absolute paths are rejected. Requires the same authentication as other API routes when `api_key` is set (except **loopback** may omit Bearer on **`/download`** when `api_key` is set — same policy as [support diagnostics](#support-diagnostics-redacted-bundle)).

Dashboard UI: **Home folder** (`#home-files`) — preview vs full-file **Download** behavior is documented in [dashboard-home-folder.md](dashboard-home-folder.md) and [dashboard-testing.md](dashboard-testing.md#home-folder-browser--preview-vs-download). Configuration: **`[dashboard]`** in `config.toml` — see [dashboard-home-folder.md](dashboard-home-folder.md) and [configuration.md](configuration.md#dashboard).

#### GET /api/armaraos-home/list

List a single directory.

**Query:** `path` — directory relative to home (empty = home root).

**Response** `200 OK` (illustrative):

```json
{
  "path": "",
  "root": "/Users/you/.armaraos",
  "entries": [
    {
      "name": "config.toml",
      "kind": "file",
      "size": 1204,
      "mtime_ms": 1710000000000,
      "editable": false
    }
  ],
  "truncated": false,
  "home_edit": {
    "allowlist_enabled": false,
    "allowlist_error": null,
    "max_bytes": 524288,
    "backup": true
  }
}
```

`kind` is `dir`, `file`, or `symlink`. `editable` is true only for files/symlinks that match **`home_editable_globs`** and are not **blocklisted** (see dashboard doc). If glob patterns in config are invalid, `allowlist_error` is a string and the list still returns.

Large directories are capped at **4000** entries; `truncated` is true if rows were cut.

#### GET /api/armaraos-home/read

Read a **file** (not a directory). Max **512 KiB** per file.

**Query:** `path` — file relative to home (required).

**Response** `200 OK`:

- `encoding`: `"utf8"` with string `content`, or `"base64"` for binary.
- `editable`: whether the dashboard may offer save (UTF-8 only, allowlist + not blocklisted).
- `allowlist_error`, `home_edit_max_bytes`, `home_edit_backup` mirror config / validation state.

#### GET /api/armaraos-home/download

Stream a file with a **larger** limit (**256 MiB**) for artifacts such as diagnostics zips under `support/`. Same sandbox rules as `read`.

**Query:** `path` — file relative to home (required).

Returns raw bytes with `Content-Type: application/octet-stream` and `Content-Disposition: attachment` when successful.

**Loopback:** Same rule as [support diagnostics](#support-diagnostics-redacted-bundle) — from **127.0.0.1** / **::1**, the request may succeed without Bearer when `api_key` is set (embedded UI). Remote clients must authenticate.

#### POST /api/armaraos-home/write

Write **UTF-8** body to a file when **`home_editable_globs`** is non-empty and the path matches and is not blocklisted.

**Body** (JSON):

```json
{
  "path": "notes/readme.txt",
  "content": "hello"
}
```

Errors include **403** when editing is disabled, path blocked, or not matched by globs; **413** when content exceeds `home_edit_max_bytes`.

---

### GET /api/health

Public health check. Does not require authentication. Returns a redacted subset of system status (no database or agent_count details).

**Response** `200 OK`:

```json
{
  "status": "ok",
  "uptime_seconds": 3600,
  "panic_count": 0,
  "restart_count": 0
}
```

The `status` field is `"ok"` when all systems are healthy, or `"degraded"` when the database is unreachable.

### GET /api/health/detail

Full health check with all dependency status. Requires authentication. Unlike the public `/api/health`, this endpoint includes database connectivity and agent counts.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "uptime_seconds": 3600,
  "panic_count": 0,
  "restart_count": 0,
  "agent_count": 3,
  "database": "connected",
  "config_warnings": []
}
```

### GET /api/status

Detailed kernel status including all agents. Includes `config_schema_version` (effective after load / migration) and `config_schema_version_binary` (the `CONFIG_SCHEMA_VERSION` constant compiled into this daemon). Also returns `version` (daemon package version), `api_listen`, `home_dir`, `log_level`, `network_enabled`, `default_provider`, `default_model`, and `uptime_seconds`.

**Response** `200 OK`:

```json
{
  "status": "running",
  "version": "0.7.1",
  "agent_count": 2,
  "default_provider": "groq",
  "default_model": "llama-3.3-70b-versatile",
  "uptime_seconds": 3600,
  "api_listen": "127.0.0.1:4200",
  "home_dir": "/home/user/.armaraos",
  "log_level": "info",
  "network_enabled": false,
  "config_schema_version": 1,
  "config_schema_version_binary": 1,
  "agents": [
    {
      "id": "a1b2c3d4-...",
      "name": "hello-world",
      "state": "Running",
      "created_at": "2025-01-15T10:30:00Z",
      "model_provider": "groq",
      "model_name": "llama-3.3-70b-versatile"
    }
  ]
}
```

### GET /api/version

Build and version information.

**Response** `200 OK`:

```json
{
  "name": "openfang",
  "version": "0.1.0",
  "build_date": "2025-01-15",
  "git_sha": "abc1234",
  "rust_version": "1.82.0",
  "platform": "linux",
  "arch": "x86_64"
}
```

### GET /api/version/github-latest

**Public read (GET-only):** Latest **GitHub release** metadata for the ArmaraOS repo, fetched **server-side** (so the dashboard does not call `api.github.com` from the browser/WebView).

**Response** `200 OK` (shape follows GitHub’s releases API; commonly includes `tag_name`, `html_url`, `published_at`):

```json
{
  "tag_name": "v0.7.1",
  "html_url": "https://github.com/sbhooley/armaraos/releases/tag/v0.7.1",
  "published_at": "2026-01-01T12:00:00Z"
}
```

Errors from GitHub are surfaced as non-200 JSON from this route (dashboard shows a toast / inline error).

### POST /api/shutdown

Initiate **graceful shutdown** of the HTTP server and kernel. Agent states are preserved to SQLite for restore on next boot.

**Security:** Requests from **loopback** (127.0.0.1 / ::1) may reach this handler **without** `Authorization: Bearer` even when `api_key` is set (same pattern as diagnostics loopback bypass). **Non-loopback** clients must authenticate like other POST routes.

**Response** `200 OK`:

```json
{
  "status": "shutting_down"
}
```

The connection may drop before the client parses JSON; that is expected.

### GET /api/profiles

List available agent profiles (predefined configurations for common use cases).

**Response** `200 OK`:

```json
{
  "profiles": [
    {
      "name": "coder",
      "tier": "smart",
      "description": "Expert coding assistant"
    },
    {
      "name": "researcher",
      "tier": "frontier",
      "description": "Deep research and analysis"
    }
  ]
}
```

### GET /api/tools

List all available tools that agents can use.

**Response** `200 OK`:

```json
{
  "tools": [
    "file_read",
    "file_write",
    "file_list",
    "web_fetch",
    "web_search",
    "shell_exec",
    "kv_get",
    "kv_set",
    "agent_call"
  ],
  "total": 23
}
```

### GET /api/config

Retrieve current kernel configuration (secrets are redacted). Includes `config_schema_version` (effective loaded value). Shape is a subset of the full `KernelConfig` — not every TOML field is mirrored here.

**Response** `200 OK`:

```json
{
  "home_dir": "/home/user/.armaraos",
  "data_dir": "/home/user/.armaraos/data",
  "config_schema_version": 1,
  "api_key": "***",
  "efficient_mode": "balanced",
  "default_model": {
    "provider": "groq",
    "model": "llama-3.3-70b-versatile",
    "api_key_env": "GROQ_API_KEY"
  },
  "memory": {
    "decay_rate": 0.1
  }
}
```

To change **one** field and persist it without hand-editing the file, use **`POST /api/config/set`** below.

### POST /api/config/set

Set a **single** configuration value, write it to **`{home_dir}/config.toml`**, and trigger an in-process **`reload_config()`**.

**Request body** (JSON):

| Field | Type | Description |
|-------|------|-------------|
| `path` | string (required) | Dot-separated TOML path. **One** segment = top-level key (e.g. `efficient_mode`). **Two** segments = `[first].second` (e.g. `memory.decay_rate`). **Three** segments = nested table (max depth **3** levels — deeper paths return `400`). |
| `value` | string / number / boolean (required) | Serialized into TOML. Other JSON types are stringified. |

**Example — Ultra Cost-Efficient Mode:**

```json
{
  "path": "efficient_mode",
  "value": "balanced"
}
```

**Response** `200 OK`:

```json
{
  "status": "applied",
  "path": "efficient_mode"
}
```

`status` is one of:

| Value | Meaning |
|-------|---------|
| `applied` | Written and reload succeeded without requiring full process restart. |
| `applied_partial` | Written; reload reported `restart_required` (some changes may need a daemon restart). |
| `saved_reload_failed` | File written but `reload_config()` failed — verify daemon logs. |

**Errors:**

- **`400`** — missing `path` or `value`; or `path` has more than three dot-separated segments (`path too deep (max 3 levels)`).
- **`500`** — TOML serialize failure or filesystem write error.

**Auth:** Same as other POST routes when `api_key` is set (Bearer), unless your deployment allows open access.

**Audit:** A config-change audit entry is recorded with the path.

**Dashboard:** **Settings → Budget** (eco mode dropdown) and the chat header **⚡ eco** button use this endpoint (see [prompt-compression-efficient-mode.md](prompt-compression-efficient-mode.md)).

### POST /api/config/reload

Reload **`config.toml`** from disk, validate, and apply **hot-reloadable** changes. Audit log records the request.

**Auth:** Required when a non-empty `api_key` is configured (unless your deployment treats unauthenticated access as open).

**Response** `200 OK` (representative):

```json
{
  "status": "applied",
  "restart_required": false,
  "restart_reasons": [],
  "hot_actions_applied": ["ApprovalPolicy"],
  "noop_changes": []
}
```

`status` may be `no_changes`, `applied`, or `partial` when `restart_required` is true (some edits still need a full process restart). **Dashboard:** **Settings → System Info → Daemon / API** or **Monitor → Runtime** → **Reload config**.

### POST /api/integrations/reload

Hot-reload integration / extension MCP configs and reconnect.

**Response** `200 OK`:

```json
{
  "status": "reloaded",
  "new_connections": 2
}
```

**Dashboard:** **Reload integrations**. **Auth:** same as other POST routes when `api_key` is set.

### GET /api/peers

List OFP (OpenFang Protocol) wire peers and their connection status.

**Response** `200 OK`:

```json
{
  "peers": [
    {
      "node_id": "peer-1",
      "address": "192.168.1.100:4000",
      "state": "connected",
      "authenticated": true,
      "last_seen": "2025-01-15T10:30:00Z"
    }
  ]
}
```

### GET /api/sessions

List all active sessions across agents.

**Response** `200 OK`:

```json
{
  "sessions": [
    {
      "id": "s1b2c3d4-...",
      "agent_id": "a1b2c3d4-...",
      "agent_name": "coder",
      "message_count": 12,
      "created_at": "2025-01-15T10:30:00Z"
    }
  ]
}
```

### DELETE /api/sessions/{id}

Delete a specific session and its conversation history.

**Response** `200 OK`:

```json
{
  "status": "deleted",
  "session_id": "s1b2c3d4-..."
}
```

---

## Model Catalog Endpoints

ArmaraOS maintains a built-in catalog of 51+ models across 20 providers. These endpoints allow you to browse available models, check provider authentication status, and resolve model aliases.

### GET /api/models

List the full model catalog. Returns all known models with their provider, tier, context window, and pricing information.

**Response** `200 OK`:

```json
{
  "models": [
    {
      "id": "claude-sonnet-4-20250514",
      "provider": "anthropic",
      "display_name": "Claude Sonnet 4",
      "tier": "frontier",
      "context_window": 200000,
      "input_cost_per_1m": 3.0,
      "output_cost_per_1m": 15.0,
      "supports_tools": true,
      "supports_vision": true,
      "supports_streaming": true
    },
    {
      "id": "gemini-2.5-flash",
      "provider": "gemini",
      "display_name": "Gemini 2.5 Flash",
      "tier": "smart",
      "context_window": 1048576,
      "input_cost_per_1m": 0.15,
      "output_cost_per_1m": 0.6,
      "supports_tools": true,
      "supports_vision": true,
      "supports_streaming": true
    }
  ],
  "total": 51
}
```

### GET /api/models/{id}

Get detailed information about a specific model.

**Response** `200 OK`:

```json
{
  "id": "llama-3.3-70b-versatile",
  "provider": "groq",
  "display_name": "Llama 3.3 70B",
  "tier": "fast",
  "context_window": 131072,
  "input_cost_per_1m": 0.59,
  "output_cost_per_1m": 0.79,
  "supports_tools": true,
  "supports_vision": false,
  "supports_streaming": true
}
```

**Response** `404 Not Found`:

```json
{
  "error": "Model 'unknown-model' not found in catalog"
}
```

### GET /api/models/aliases

List all model aliases. Aliases provide short names that resolve to full model IDs (e.g., `sonnet` resolves to `claude-sonnet-4-20250514`).

**Response** `200 OK`:

```json
{
  "aliases": {
    "sonnet": "claude-sonnet-4-20250514",
    "opus": "claude-opus-4-20250514",
    "haiku": "claude-3-5-haiku-20241022",
    "flash": "gemini-2.5-flash",
    "gpt4": "gpt-4o",
    "llama": "llama-3.3-70b-versatile",
    "deepseek": "deepseek-chat",
    "grok": "grok-2",
    "jamba": "jamba-1.5-large"
  },
  "total": 23
}
```

### GET /api/providers

List all known LLM providers and their authentication status. Auth status is detected by checking environment variable presence (never reads secret values).

**Response** `200 OK`:

```json
{
  "providers": [
    {
      "name": "anthropic",
      "display_name": "Anthropic",
      "auth_status": "configured",
      "env_var": "ANTHROPIC_API_KEY",
      "base_url": "https://api.anthropic.com",
      "model_count": 3
    },
    {
      "name": "groq",
      "display_name": "Groq",
      "auth_status": "configured",
      "env_var": "GROQ_API_KEY",
      "base_url": "https://api.groq.com/openai",
      "model_count": 4
    },
    {
      "name": "ollama",
      "display_name": "Ollama",
      "auth_status": "no_key_needed",
      "base_url": "http://localhost:11434",
      "model_count": 0
    }
  ],
  "total": 20
}
```

---

## Provider Configuration Endpoints

Manage LLM provider API keys at runtime without editing config files or restarting the daemon.

### POST /api/providers/{name}/key

Set an API key for a provider. The key is stored securely and takes effect immediately.

**Request Body**:

```json
{
  "api_key": "sk-..."
}
```

**Response** `200 OK`:

```json
{
  "status": "configured",
  "provider": "anthropic"
}
```

### DELETE /api/providers/{name}/key

Remove the API key for a provider. Agents using this provider will fall back to the FallbackDriver or fail.

**Response** `200 OK`:

```json
{
  "status": "removed",
  "provider": "anthropic"
}
```

### POST /api/providers/{name}/test

Test provider connectivity by making a minimal API call. Verifies that the configured API key is valid and the provider endpoint is reachable.

**Response** `200 OK`:

```json
{
  "status": "ok",
  "provider": "anthropic",
  "latency_ms": 245,
  "model_tested": "claude-sonnet-4-20250514"
}
```

**Response** `401 Unauthorized`:

```json
{
  "status": "failed",
  "provider": "anthropic",
  "error": "Invalid API key"
}
```

---

## Skills & Marketplace Endpoints

Manage the skill registry. Skills extend agent capabilities with Python, Node.js, WASM, or prompt-only modules. All skill installations go through SHA256 verification and prompt injection scanning.

### GET /api/skills

List all installed skills.

**Response** `200 OK`:

```json
{
  "skills": [
    {
      "name": "github",
      "version": "1.0.0",
      "runtime": "prompt_only",
      "description": "GitHub integration for issues, PRs, and repos",
      "bundled": true
    },
    {
      "name": "docker",
      "version": "1.0.0",
      "runtime": "prompt_only",
      "description": "Docker container management",
      "bundled": true
    }
  ],
  "total": 60
}
```

### POST /api/skills/install

Install a skill from a local path or URL. The skill manifest is verified (SHA256 checksum) and scanned for prompt injection before installation.

**Request Body**:

```json
{
  "source": "/path/to/skill",
  "verify": true
}
```

**Response** `201 Created`:

```json
{
  "status": "installed",
  "skill": "my-custom-skill",
  "version": "1.0.0"
}
```

### POST /api/skills/uninstall

Remove an installed skill. Bundled skills cannot be uninstalled.

**Request Body**:

```json
{
  "name": "my-custom-skill"
}
```

**Response** `200 OK`:

```json
{
  "status": "uninstalled",
  "skill": "my-custom-skill"
}
```

### POST /api/skills/create

Create a new skill from a template.

**Request Body**:

```json
{
  "name": "my-skill",
  "runtime": "python",
  "description": "A custom skill"
}
```

**Response** `201 Created`:

```json
{
  "status": "created",
  "skill": "my-skill",
  "path": "/home/user/.armaraos/skills/my-skill"
}
```

### GET /api/marketplace/search

Search the FangHub marketplace for community skills.

**Query Parameters:**
- `q` (required): Search query string
- `page` (optional): Page number (default: 1)

**Response** `200 OK`:

```json
{
  "results": [
    {
      "name": "weather-api",
      "author": "community",
      "description": "Real-time weather data integration",
      "downloads": 1250,
      "version": "2.1.0"
    }
  ],
  "total": 1,
  "page": 1
}
```

---

## ClawHub Endpoints

Browse and install skills from ClawHub (OpenClaw ecosystem compatibility). All installations go through the full security pipeline: SHA256 verification, SKILL.md security scanning, and trust boundary enforcement.

### GET /api/clawhub/search

Search ClawHub for compatible skills.

**Query Parameters:**
- `q` (required): Search query

**Response** `200 OK`:

```json
{
  "results": [
    {
      "slug": "data-pipeline",
      "name": "Data Pipeline",
      "description": "ETL data pipeline automation",
      "author": "clawhub-community",
      "version": "1.2.0"
    }
  ],
  "total": 1
}
```

### GET /api/clawhub/browse

Browse ClawHub categories.

**Query Parameters:**
- `category` (optional): Filter by category
- `page` (optional): Page number (default: 1)

**Response** `200 OK`:

```json
{
  "skills": [
    {
      "slug": "data-pipeline",
      "name": "Data Pipeline",
      "category": "data",
      "description": "ETL data pipeline automation"
    }
  ],
  "total": 15,
  "page": 1
}
```

### GET /api/clawhub/skill/{slug}

Get detailed information about a specific ClawHub skill.

**Response** `200 OK`:

```json
{
  "slug": "data-pipeline",
  "name": "Data Pipeline",
  "description": "ETL data pipeline automation",
  "author": "clawhub-community",
  "version": "1.2.0",
  "runtime": "python",
  "readme": "# Data Pipeline\n\nAutomated ETL...",
  "sha256": "a1b2c3d4..."
}
```

### POST /api/clawhub/install

Install a skill from ClawHub. Downloads, verifies SHA256 checksum, scans for prompt injection, and converts SKILL.md format to ArmaraOS skill.toml automatically.

**Request Body**:

```json
{
  "slug": "data-pipeline"
}
```

**Response** `201 Created`:

```json
{
  "status": "installed",
  "skill": "data-pipeline",
  "version": "1.2.0",
  "converted_from": "SKILL.md"
}
```

---

## MCP & A2A Protocol Endpoints

ArmaraOS supports both Model Context Protocol (MCP) for tool interoperability and Agent-to-Agent (A2A) protocol for cross-system agent communication.

### GET /api/mcp/servers

List configured and connected MCP servers with their available tools.

**Response** `200 OK`:

```json
{
  "servers": [
    {
      "name": "filesystem",
      "transport": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"],
      "connected": true,
      "tools": [
        {
          "name": "mcp_filesystem_read_file",
          "description": "Read a file from the filesystem"
        },
        {
          "name": "mcp_filesystem_write_file",
          "description": "Write content to a file"
        }
      ]
    }
  ],
  "total": 1
}
```

### POST /mcp

MCP HTTP transport endpoint. Accepts JSON-RPC 2.0 requests and exposes ArmaraOS tools via the MCP protocol to external clients.

**Request Body** (JSON-RPC 2.0):

```json
{
  "jsonrpc": "2.0",
  "method": "tools/list",
  "id": 1
}
```

**Response** `200 OK`:

```json
{
  "jsonrpc": "2.0",
  "result": {
    "tools": [
      {
        "name": "file_read",
        "description": "Read a file's contents",
        "inputSchema": {
          "type": "object",
          "properties": {
            "path": {"type": "string"}
          }
        }
      }
    ]
  },
  "id": 1
}
```

### GET /.well-known/agent.json

A2A agent card discovery endpoint. Returns the server's A2A agent card, which describes its capabilities, supported protocols, and available agents.

**Response** `200 OK`:

```json
{
  "name": "ArmaraOS",
  "description": "ArmaraOS Agent Operating System",
  "url": "http://127.0.0.1:4200",
  "version": "0.1.0",
  "capabilities": {
    "streaming": true,
    "pushNotifications": false
  },
  "skills": [
    {
      "id": "chat",
      "name": "Chat",
      "description": "General-purpose chat with any agent"
    }
  ]
}
```

### GET /a2a/agents

List agents available via A2A protocol.

**Response** `200 OK`:

```json
{
  "agents": [
    {
      "id": "a1b2c3d4-...",
      "name": "coder",
      "description": "Expert coding assistant",
      "skills": ["code-review", "debugging", "refactoring"]
    }
  ]
}
```

### POST /a2a/tasks/send

Send a task to an agent via A2A protocol. Follows the Google A2A specification for inter-agent task delegation.

**Request Body**:

```json
{
  "agent_id": "a1b2c3d4-...",
  "message": {
    "role": "user",
    "parts": [
      {"text": "Review this code for security issues"}
    ]
  }
}
```

**Response** `200 OK`:

```json
{
  "task_id": "task-1234-...",
  "status": "completed",
  "result": {
    "role": "agent",
    "parts": [
      {"text": "I found 2 potential security issues..."}
    ]
  }
}
```

### GET /a2a/tasks/{id}

Get the status and result of an A2A task.

**Response** `200 OK`:

```json
{
  "task_id": "task-1234-...",
  "status": "completed",
  "created_at": "2025-01-15T10:30:00Z",
  "completed_at": "2025-01-15T10:30:05Z",
  "result": {
    "role": "agent",
    "parts": [
      {"text": "Analysis complete..."}
    ]
  }
}
```

### POST /a2a/tasks/{id}/cancel

Cancel a running A2A task.

**Response** `200 OK`:

```json
{
  "task_id": "task-1234-...",
  "status": "cancelled"
}
```

---

## Audit & Security Endpoints

ArmaraOS maintains a Merkle hash chain audit trail for all security-relevant operations. These endpoints allow inspection and verification of the audit log integrity.

### GET /api/audit/recent

Retrieve recent audit log entries.

**Query Parameters:**
- `limit` (optional): Number of entries to return (default: 50, max: 500)

**Response** `200 OK`:

```json
{
  "entries": [
    {
      "id": 1042,
      "timestamp": "2025-01-15T10:30:00Z",
      "event_type": "agent_spawned",
      "agent_id": "a1b2c3d4-...",
      "details": "Agent 'coder' spawned with model groq/llama-3.3-70b-versatile",
      "hash": "a1b2c3d4e5f6...",
      "prev_hash": "f6e5d4c3b2a1..."
    }
  ],
  "total": 1042
}
```

### GET /api/audit/verify

Verify the integrity of the Merkle hash chain audit trail. Walks the entire chain and reports any broken links.

**Response** `200 OK`:

```json
{
  "status": "valid",
  "chain_length": 1042,
  "first_entry": "2025-01-10T08:00:00Z",
  "last_entry": "2025-01-15T10:30:00Z"
}
```

**Response** `200 OK` (chain broken):

```json
{
  "status": "broken",
  "chain_length": 1042,
  "break_at": 847,
  "error": "Hash mismatch at entry 847"
}
```

### GET /api/logs/daemon/recent

Returns recent **lines** from the CLI daemon tracing file: prefers **`{home}/logs/daemon.log`**, otherwise **`{home}/tui.log`** if that file exists (`home` is the kernel’s ArmaraOS/Armaraos data directory, same as `ARMARAOS_HOME` / `~/.armaraos`).

**Query parameters:**

| Name | Description |
|------|-------------|
| `lines` | Max lines after filtering (default `200`, clamped 1–2000). |
| `level` | Optional minimum severity: `trace`, `debug`, `info`, `warn`, `error` (matches `tracing` default format substrings on each line). |
| `filter` | Optional case-insensitive substring; line must contain it. |

**Response** `200 OK`:

```json
{
  "path": "logs/daemon.log",
  "lines": [
    { "seq": 1, "line": "2026-04-05T12:00:00.123456Z  INFO openfang_api: listening on 127.0.0.1:4200" }
  ]
}
```

`path` is `null` when no log file is present; `lines` is then empty.

---

### GET /api/security

Security status overview showing the state of all 16 security systems.

**Response** `200 OK`:

```json
{
  "security_systems": {
    "merkle_audit_trail": "active",
    "taint_tracking": "active",
    "wasm_dual_metering": "active",
    "security_headers": "active",
    "health_redaction": "active",
    "subprocess_sandbox": "active",
    "manifest_signing": "active",
    "gcra_rate_limiter": "active",
    "secret_zeroization": "active",
    "path_traversal_prevention": "active",
    "ssrf_protection": "active",
    "capability_inheritance_validation": "active",
    "ofp_hmac_auth": "active",
    "prompt_injection_scanning": "active",
    "loop_guard": "active",
    "session_repair": "active"
  },
  "total_systems": 16,
  "all_active": true
}
```

---

## Usage & Analytics Endpoints

Track token usage, costs, and model utilization across all agents. Powered by the metering engine with cost estimation from the model catalog.

### GET /api/usage

Get overall usage statistics.

**Query Parameters:**
- `period` (optional): Time period (`hour`, `day`, `week`, `month`; default: `day`)

**Response** `200 OK`:

```json
{
  "period": "day",
  "total_input_tokens": 125000,
  "total_output_tokens": 87000,
  "total_cost_usd": 0.42,
  "request_count": 156,
  "active_agents": 5
}
```

### GET /api/usage/summary

Get a high-level usage summary with quota information.

**Response** `200 OK`:

```json
{
  "today": {
    "input_tokens": 125000,
    "output_tokens": 87000,
    "cost_usd": 0.42,
    "requests": 156
  },
  "quota": {
    "hourly_token_limit": 1000000,
    "hourly_tokens_used": 45000,
    "hourly_reset_at": "2025-01-15T11:00:00Z"
  }
}
```

### GET /api/usage/by-model

Get usage breakdown by model.

**Response** `200 OK`:

```json
{
  "models": [
    {
      "model": "llama-3.3-70b-versatile",
      "provider": "groq",
      "input_tokens": 80000,
      "output_tokens": 55000,
      "cost_usd": 0.09,
      "request_count": 120
    },
    {
      "model": "gemini-2.5-flash",
      "provider": "gemini",
      "input_tokens": 45000,
      "output_tokens": 32000,
      "cost_usd": 0.33,
      "request_count": 36
    }
  ]
}
```

---

## Migration Endpoints

Import data from OpenClaw or other agent frameworks. The migration engine handles YAML-to-TOML manifest conversion, SKILL.md parsing, and session history import.

### GET /api/migrate/detect

Auto-detect migration sources on the system. Scans common locations for OpenClaw installations, config files, and agent data.

**Response** `200 OK`:

```json
{
  "sources": [
    {
      "type": "openclaw",
      "path": "/home/user/.openclaw",
      "version": "2.1.0",
      "agents_found": 12,
      "skills_found": 8
    }
  ]
}
```

### POST /api/migrate/scan

Scan a specific path for importable data.

**Request Body**:

```json
{
  "path": "/home/user/.openclaw"
}
```

**Response** `200 OK`:

```json
{
  "agents": [
    {
      "name": "my-agent",
      "format": "yaml",
      "convertible": true
    }
  ],
  "skills": [
    {
      "name": "custom-skill",
      "format": "SKILL.md",
      "convertible": true
    }
  ],
  "sessions": 45
}
```

### POST /api/migrate

Run the migration. Converts manifests, imports skills, and optionally imports session history.

**Request Body**:

```json
{
  "source": "/home/user/.openclaw",
  "import_agents": true,
  "import_skills": true,
  "import_sessions": false
}
```

**Response** `200 OK`:

```json
{
  "status": "completed",
  "agents_imported": 12,
  "skills_imported": 8,
  "sessions_imported": 0,
  "warnings": [
    "Skill 'legacy-plugin' uses unsupported runtime 'ruby', skipped"
  ]
}
```

---

## Session Management Endpoints

### POST /api/agents/{id}/session/reset

Reset an agent's session, clearing all conversation history.

**Response** `200 OK`:

```json
{
  "status": "reset",
  "agent_id": "a1b2c3d4-...",
  "new_session_id": "s5e6f7g8-..."
}
```

### POST /api/agents/{id}/session/compact

Trigger LLM-based session compaction. The agent's conversation is summarized by an LLM, keeping only the most recent messages plus a generated summary.

**Response** `200 OK`:

```json
{
  "status": "compacted",
  "message": "Session compacted: 80 messages summarized, 20 kept"
}
```

**Response** `200 OK` (no compaction needed):

```json
{
  "status": "ok",
  "message": "Session does not need compaction (below threshold)"
}
```

### POST /api/agents/{id}/stop

Cancel the agent's current LLM run. Aborts any in-progress generation.

**Response** `200 OK`:

```json
{
  "status": "stopped",
  "message": "Agent run cancelled"
}
```

### POST /api/agents/{id}/btw

Inject extra context into an agent loop **while it is already running**. The text is queued via an in-memory channel and drained at the start of the next iteration, appended to the conversation as a `[btw] …` user message before the next LLM call.

Use this to steer a running agent mid-turn — add tasks, share new information, or correct its direction without waiting for the current turn to finish.

Returns `409 Conflict` when the agent has no active loop (i.e. it is idle or already finished).

**Request Body**:

```json
{
  "text": "Also check whether the Cargo.lock is up to date before finishing."
}
```

**Response** `200 OK` (injection queued):

```json
{
  "status": "injected"
}
```

**Response** `409 Conflict` (no active loop):

```json
{
  "error": "agent not running"
}
```

**Dashboard:** Type `/btw <text>` in the chat input while an agent is working. The message is sent immediately and appears as a local confirmation in the chat timeline; the agent picks it up on its next iteration.

### PUT /api/agents/{id}/model

Switch an agent's LLM model at runtime.

**Request Body**:

```json
{
  "model": "claude-sonnet-4-20250514"
}
```

**Response** `200 OK`:

```json
{
  "status": "updated",
  "model": "claude-sonnet-4-20250514"
}
```

---

## Slash Templates Endpoints

Slash templates let users define reusable message shortcuts (e.g. `/t standup` expands to a full standup prompt). Templates are stored server-side in `~/.armaraos/slash-templates.json` so they survive app upgrades and browser data clears.

### GET /api/slash-templates

Load the full list of saved slash templates.

**Response** `200 OK`:

```json
{
  "templates": [
    {
      "name": "standup",
      "text": "Give me a morning standup summary: what did you do yesterday, what's planned today, any blockers?"
    },
    {
      "name": "debug",
      "text": "Walk me through debugging this error step by step."
    }
  ]
}
```

Returns `{ "templates": [] }` when no templates have been saved yet.

### PUT /api/slash-templates

Save the full list of slash templates, replacing any existing file. The body must contain a `"templates"` array. Writes are atomic (write to `.json.tmp`, then rename) to prevent corruption.

**Request Body**:

```json
{
  "templates": [
    { "name": "standup", "text": "Give me a morning standup summary…" },
    { "name": "review",  "text": "Review this code for bugs and style issues." }
  ]
}
```

**Response** `200 OK`:

```json
{
  "status": "saved",
  "count": 2
}
```

**Dashboard:** Use `/t save <name>` in any chat input to create a template, `/t <name>` to expand it, and `/t list` to browse. Templates are loaded from the server at first use and cached in memory for the session.

---

## UI Preferences Endpoints

Arbitrary dashboard UI state that must survive **desktop reinstalls** (which can clear embedded WebView `localStorage`) is stored in **`~/.armaraos/ui-prefs.json`**. The file is a single JSON object; the dashboard currently uses:

| Key | Type | Purpose |
|-----|------|---------|
| `pinned_agents` | array of strings (agent IDs) | Order of pinned rows in the sidebar **Quick open** list |

Writes use the same atomic **`.json.tmp` → rename** pattern as slash templates.

### GET /api/ui-prefs

Load persisted UI preferences. Returns **`{}`** when the file does not exist yet.

**Response** `200 OK`:

```json
{
  "pinned_agents": ["550e8400-e29b-41d4-a716-446655440000"]
}
```

### PUT /api/ui-prefs

Replace the entire preferences object on disk. The body **must** be a JSON **object** (not an array or scalar).

**Request body** (example):

```json
{
  "pinned_agents": ["550e8400-e29b-41d4-a716-446655440000"]
}
```

**Response** `200 OK`:

```json
{ "status": "ok" }
```

**Dashboard:** On load, the app merges server `pinned_agents` into `localStorage` (`armaraos-pinned-agents`). Each pin/unpin updates both `localStorage` and `PUT /api/ui-prefs`.

---

## Cron/Scheduler Endpoints

Manage recurring and one-shot scheduled jobs. Jobs can trigger agent turns, system events, or workflow runs on a schedule.

### GET /api/cron/jobs

List all cron jobs. Optionally filter by agent with `?agent_id=<uuid>`.

**Response** `200 OK`:

```json
{
  "jobs": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "agent_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
      "name": "daily-report",
      "enabled": true,
      "schedule": { "kind": "every", "every_secs": 3600 },
      "action": {
        "kind": "agent_turn",
        "message": "Generate the daily report",
        "timeout_secs": 120
      },
      "delivery": {
        "kind": "channel",
        "channel": "slack",
        "to": "#reports"
      },
      "created_at": "2026-03-15T10:30:00Z",
      "last_run": "2026-03-16T09:00:00Z",
      "next_run": "2026-03-16T10:00:00Z"
    }
  ],
  "total": 1
}
```

### POST /api/cron/jobs

Create a new cron job.

**Request Body**:

```json
{
  "agent_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
  "name": "daily-report",
  "schedule": { "kind": "every", "every_secs": 3600 },
  "action": {
    "kind": "agent_turn",
    "message": "Generate the daily report",
    "timeout_secs": 120
  },
  "delivery": {
    "kind": "channel",
    "channel": "slack",
    "to": "#reports"
  }
}
```

**Response** `201 Created`:

```json
{
  "result": "{\"job_id\":\"550e8400-e29b-41d4-a716-446655440000\",\"status\":\"created\"}"
}
```

### PUT /api/cron/jobs/{id}

Update an existing cron job in place. Request body matches **POST /api/cron/jobs** (same fields, including `agent_id`, `name`, `schedule`, `action`, `delivery`, `enabled`, and optional `one_shot`). The job ID in the path is preserved.

**Response** `200 OK`:

```json
{ "status": "updated", "job_id": "550e8400-e29b-41d4-a716-446655440000" }
```

### DELETE /api/cron/jobs/{id}

Delete a cron job by ID.

**Response** `200 OK`:

```json
{ "status": "deleted" }
```

### PUT /api/cron/jobs/{id}/enable

Enable or disable a cron job.

**Request Body**:

```json
{ "enabled": false }
```

**Response** `200 OK`:

```json
{ "status": "updated", "enabled": false }
```

### GET /api/cron/jobs/{id}/status

Get job metadata including last run time, status, and error history.

**Response** `200 OK`:

```json
{
  "job": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "agent_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7",
    "name": "daily-report",
    "enabled": true,
    "schedule": { "kind": "every", "every_secs": 3600 },
    "action": {
      "kind": "agent_turn",
      "message": "Generate the daily report",
      "timeout_secs": 120
    },
    "delivery": { "kind": "none" },
    "created_at": "2026-03-15T10:30:00Z",
    "last_run": "2026-03-16T09:00:00Z",
    "next_run": "2026-03-16T10:00:00Z"
  },
  "one_shot": false,
  "last_status": "ok",
  "consecutive_errors": 0
}

### POST /api/cron/jobs/{id}/run

Trigger a cron job immediately. The job executes asynchronously in the background — this endpoint returns immediately without waiting for completion. Poll `GET /api/cron/jobs/{id}/status` to check the result.

**Response** `200 OK`:

```json
{
  "status": "triggered",
  "job_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Error Responses**:

- `400 Bad Request` — Invalid job ID or job is disabled
- `404 Not Found` — Job not found

---

## WebSocket Protocol

### Connecting

```
GET /api/agents/{id}/ws
```

Upgrades to a WebSocket connection for real-time bidirectional chat with an agent. Returns `400` if the agent ID is invalid, or `404` if the agent does not exist.

### Message Format

All messages are JSON-encoded strings.

### Client to Server

**Send a message:**

```json
{
  "type": "message",
  "content": "What is the weather like?"
}
```

Plain text (non-JSON) is also accepted and treated as a message.

**Chat commands** (sent as messages with `/` prefix):

| Command | Description |
|---------|-------------|
| `/new` | Start a new session (clear history) |
| `/compact` | Trigger LLM session compaction |
| `/model <name>` | Switch the agent's model |
| `/stop` | Cancel current LLM run |
| `/usage` | Show token usage and cost |
| `/think` | Toggle extended thinking mode |
| `/models` | List available models |
| `/providers` | List LLM providers and auth status |

**Ping:**

```json
{
  "type": "ping"
}
```

### Server to Client

**Connection confirmed** (sent immediately on connect):

```json
{
  "type": "connected",
  "agent_id": "a1b2c3d4-..."
}
```

**Thinking indicator** (sent when agent starts processing):

```json
{
  "type": "thinking"
}
```

**Text delta** (streaming token, sent as the LLM generates output):

```json
{
  "type": "text_delta",
  "content": "The weather"
}
```

**Tool use started** (sent when the agent invokes a tool):

```json
{
  "type": "tool_start",
  "tool": "web_fetch"
}
```

**Complete response** (sent when agent finishes, contains final aggregated response):

```json
{
  "type": "response",
  "content": "The weather today is sunny with a high of 72F.",
  "input_tokens": 245,
  "output_tokens": 32,
  "iterations": 2,
  "cost_usd": 0.0012,
  "compression_savings_pct": 34,
  "compressed_input": "…"
}
```

**Compression:** When input compression ran with non-zero savings, **`compression_savings_pct`** and **`compressed_input`** may be included (same semantics as **`POST /api/agents/{id}/message`**). The stream may also emit internal compression stats before LLM tokens for dashboard telemetry.

**Error:**

```json
{
  "type": "error",
  "content": "Agent not found"
}
```

**Agent list update** (sent every 5 seconds with current agent states):

```json
{
  "type": "agents_updated",
  "agents": [
    {
      "id": "a1b2c3d4-...",
      "name": "hello-world",
      "state": "Running",
      "model_provider": "groq",
      "model_name": "llama-3.3-70b-versatile"
    }
  ]
}
```

**Pong** (response to ping):

```json
{
  "type": "pong"
}
```

### Connection Lifecycle

1. Client connects to `ws://host:port/api/agents/{id}/ws`.
2. Server sends `{"type": "connected"}`.
3. Client sends `{"type": "message", "content": "..."}`.
4. Server sends `{"type": "thinking"}`, then zero or more `{"type": "text_delta"}` events, then `{"type": "response"}`.
5. Server periodically sends `{"type": "agents_updated"}` every 5 seconds.
6. Client sends a Close frame or disconnects to end the session.

---

## SSE Streaming

### POST /api/agents/{id}/message/stream

Send a message and receive the response as a Server-Sent Events stream. This enables real-time token-by-token streaming.

**Request Body** (JSON):

```json
{
  "message": "Explain quantum computing"
}
```

**SSE Event Stream:**

```
event: chunk
data: {"content":"Quantum","done":false}

event: chunk
data: {"content":" computing","done":false}

event: chunk
data: {"content":" is a type","done":false}

event: tool_use
data: {"tool":"web_search"}

event: tool_result
data: {"tool":"web_search","input":{"query":"quantum computing basics"}}

event: done
data: {"done":true,"usage":{"input_tokens":150,"output_tokens":340}}
```

### SSE Event Types

| Event Name | Description |
|------------|-------------|
| `chunk` | Text delta from the LLM. `"done": false` indicates more tokens are coming. |
| `tool_use` | The agent is invoking a tool. Contains the tool name. |
| `tool_result` | A tool invocation has completed. Contains the tool name and input. |
| `done` | Final event. Contains `"done": true` and token usage statistics. |

### GET /api/logs/stream (audit, SSE)

Server-Sent Events stream of **new audit log entries** (same underlying store as `GET /api/audit/recent`). Each `data:` line is JSON with fields such as `seq`, `timestamp`, `agent_id`, `action`, `detail`, `outcome`, `hash`.

**Query parameters:**

| Name | Description |
|------|-------------|
| `level` | Optional: `info`, `warn`, or `error` — derived from a coarse classification of the audit `action` string. |
| `filter` | Optional case-insensitive substring match across `action`, `detail`, and `agent_id`. |
| `token` | When `api_key` is set and the client cannot send headers (e.g. `EventSource`), pass the same value as `Authorization: Bearer`. |

On first poll after connect, the server **backfills** recent entries; then it sends only new rows (poll interval ~1s). Heartbeat comments keep the connection alive.

**Auth:** Loopback clients may omit credentials; non-loopback requires Bearer or `token` (same rule as `/api/events/stream` and `/api/logs/daemon/stream`).

### GET /api/logs/daemon/stream (SSE)

SSE tail of the **daemon tracing log file** (same path rules as `GET /api/logs/daemon/recent`). Each `data:` payload is JSON: `{ "line": "..." }`.

**Query parameters:** `level`, `filter`, and `token` — same meaning as **`/api/logs/daemon/recent`**. On connect, up to ~300 recent matching lines are sent, then new file growth is read incrementally (~1s poll).

### GET /api/events/stream (kernel, SSE)

SSE stream of kernel **`Event`** values (JSON per message): live bus traffic plus a short history on connect. Uses the same **loopback / Bearer / `token=`** auth rules as the log streams above.

---

## OpenAI-Compatible API

ArmaraOS exposes an OpenAI-compatible API for drop-in integration with tools that support the OpenAI API format (Cursor, Continue, Open WebUI, etc.).

### POST /v1/chat/completions

Send a chat completion request using the OpenAI message format.

**Request Body**:

```json
{
  "model": "openfang:coder",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Hello!"}
  ],
  "stream": false,
  "temperature": 0.7,
  "max_tokens": 1024
}
```

**Model resolution** (the `model` field maps to an ArmaraOS agent):

| Format | Example | Behavior |
|--------|---------|----------|
| `openfang:<name>` | `openfang:coder` | Find agent by name |
| UUID | `a1b2c3d4-...` | Find agent by ID |
| Plain string | `coder` | Try as agent name |
| Any other | `gpt-4o` | Falls back to first registered agent |

**Image support** --- messages can include image content parts:

```json
{
  "model": "openfang:analyst",
  "messages": [
    {
      "role": "user",
      "content": [
        {"type": "text", "text": "Describe this image"},
        {"type": "image_url", "image_url": {"url": "data:image/png;base64,iVBOR..."}}
      ]
    }
  ]
}
```

**Response (non-streaming)** `200 OK`:

```json
{
  "id": "chatcmpl-a1b2c3d4-...",
  "object": "chat.completion",
  "created": 1708617600,
  "model": "coder",
  "choices": [
    {
      "index": 0,
      "message": {
        "role": "assistant",
        "content": "Hello! How can I help you today?"
      },
      "finish_reason": "stop"
    }
  ],
  "usage": {
    "prompt_tokens": 25,
    "completion_tokens": 12,
    "total_tokens": 37
  }
}
```

**Streaming** --- Set `"stream": true` for SSE:

```
data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"Hello"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"!"},"finish_reason":null}]}

data: {"id":"chatcmpl-...","object":"chat.completion.chunk","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":25,"completion_tokens":12,"total_tokens":37}}

data: [DONE]
```

### GET /v1/models

List available models (agents) in OpenAI format.

**Response** `200 OK`:

```json
{
  "object": "list",
  "data": [
    {
      "id": "openfang:coder",
      "object": "model",
      "created": 1708617600,
      "owned_by": "openfang"
    },
    {
      "id": "openfang:researcher",
      "object": "model",
      "created": 1708617600,
      "owned_by": "openfang"
    }
  ]
}
```

---

## Error Responses

All error responses use a consistent JSON format:

```json
{
  "error": "Description of what went wrong"
}
```

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| `200` | Success |
| `201` | Created (spawn agent, create workflow, create trigger, install skill) |
| `400` | Bad request (invalid UUID, missing required fields, malformed TOML/JSON) |
| `401` | Unauthorized (missing or invalid `Authorization: Bearer` header) |
| `404` | Not found (agent, workflow, trigger, template, model, skill, or KV key does not exist) |
| `429` | Too many requests (GCRA rate limit exceeded) |
| `500` | Internal server error (agent loop failure, database error, driver error) |

### Request IDs

Every response includes an `x-request-id` header with a UUID for tracing:

```
x-request-id: 550e8400-e29b-41d4-a716-446655440000
```

Use this value when reporting issues or correlating requests in logs.

### Security Headers

Every response includes security headers:

| Header | Value |
|--------|-------|
| `Content-Security-Policy` | `default-src 'self'` (with appropriate directives) |
| `X-Frame-Options` | `DENY` |
| `X-Content-Type-Options` | `nosniff` |
| `Strict-Transport-Security` | `max-age=63072000; includeSubDomains` |
| `X-Request-Id` | Unique UUID per request |

### Rate Limiting

The GCRA (Generic Cell Rate Algorithm) rate limiter provides cost-aware token bucket rate limiting with per-IP tracking and automatic stale entry cleanup. Different endpoints consume different token costs (e.g., `/api/agents/{id}/message` costs more than `/api/health`). When the limit is exceeded, the server returns `429 Too Many Requests`:

```
HTTP/1.1 429 Too Many Requests
Retry-After: 60

{"error": "Rate limit exceeded"}
```

The `Retry-After` header indicates the window duration in seconds.

---

## Endpoint Summary

**83+ endpoints total** across 15 groups (approximate; generated list may lag new routes).

| Method | Path | Description |
|--------|------|-------------|
| **System** | | |
| GET | `/` | WebChat UI |
| GET | `/api/health` | Health check (no auth, redacted) |
| GET | `/api/health/detail` | Full health check (auth required) |
| GET | `/api/status` | Kernel status |
| GET | `/api/version` | Version info |
| GET | `/api/version/github-latest` | Latest GitHub release (server-side fetch for dashboard) |
| POST | `/api/shutdown` | Graceful shutdown (loopback may omit Bearer; see route doc) |
| GET | `/api/profiles` | List agent profiles |
| GET | `/api/tools` | List available tools |
| GET | `/api/config` | Configuration (secrets redacted) |
| POST | `/api/config/reload` | Reload config from disk (hot reload + restart hints) |
| GET | `/api/ui-prefs` | Dashboard UI preferences (`~/.armaraos/ui-prefs.json`; e.g. pinned agents) |
| PUT | `/api/ui-prefs` | Save UI preferences (full JSON object replace; atomic write) |
| GET | `/api/peers` | List OFP wire peers |
| **Agents** | | |
| GET | `/api/agents` | List agents (includes `system_prompt`, `identity`) |
| POST | `/api/agents` | Spawn agent |
| GET | `/api/agents/{id}` | Get agent details (+ `tool_allowlist` / `tool_blocklist`) |
| PUT | `/api/agents/{id}/update` | Update agent config |
| PATCH | `/api/agents/{id}/config` | Hot-update name, prompt, identity, model, fallbacks |
| PATCH | `/api/agents/{id}/identity` | Update identity only (merged) |
| GET | `/api/agents/{id}/tools` | Get tool allowlist / blocklist |
| PUT | `/api/agents/{id}/tools` | Set tool allowlist / blocklist |
| PUT | `/api/agents/{id}/mode` | Set agent mode (Stable/Normal) |
| DELETE | `/api/agents/{id}` | Kill agent |
| POST | `/api/agents/{id}/message` | Send message (blocking) |
| POST | `/api/agents/{id}/message/stream` | Send message (SSE stream) |
| GET | `/api/agents/{id}/session` | Get conversation history |
| GET | `/api/agents/{id}/ws` | WebSocket chat |
| POST | `/api/agents/{id}/session/reset` | Reset session |
| POST | `/api/agents/{id}/session/compact` | LLM-based compaction |
| POST | `/api/agents/{id}/stop` | Cancel current run |
| PUT | `/api/agents/{id}/model` | Switch model |
| **Workflows** | | |
| GET | `/api/workflows` | List workflows |
| POST | `/api/workflows` | Create workflow |
| POST | `/api/workflows/{id}/run` | Run workflow |
| GET | `/api/workflows/{id}/runs` | List workflow runs |
| **Triggers** | | |
| GET | `/api/triggers` | List triggers |
| POST | `/api/triggers` | Create trigger |
| PUT | `/api/triggers/{id}` | Update trigger |
| DELETE | `/api/triggers/{id}` | Delete trigger |
| **Memory** | | |
| GET | `/api/memory/agents/{id}/kv` | List KV pairs |
| GET | `/api/memory/agents/{id}/kv/{key}` | Get KV value |
| PUT | `/api/memory/agents/{id}/kv/{key}` | Set KV value |
| DELETE | `/api/memory/agents/{id}/kv/{key}` | Delete KV value |
| **Channels** | | |
| GET | `/api/channels` | List channels (40 adapters) |
| POST | `/api/channels/reload` | Reload channel bridges from disk |
| **Templates** | | |
| GET | `/api/templates` | List templates |
| GET | `/api/templates/{name}` | Get template |
| **Sessions** | | |
| GET | `/api/sessions` | List sessions |
| DELETE | `/api/sessions/{id}` | Delete session |
| **Model Catalog** | | |
| GET | `/api/models` | Full model catalog (51+ models) |
| GET | `/api/models/{id}` | Model details |
| GET | `/api/models/aliases` | List 23 model aliases |
| GET | `/api/providers` | Provider list with auth status |
| **Provider Config** | | |
| POST | `/api/providers/{name}/key` | Set provider API key |
| DELETE | `/api/providers/{name}/key` | Remove provider API key |
| POST | `/api/providers/{name}/test` | Test provider connectivity |
| **Skills & Marketplace** | | |
| GET | `/api/skills` | List installed skills (60 bundled) |
| POST | `/api/skills/install` | Install skill |
| POST | `/api/skills/uninstall` | Uninstall skill |
| POST | `/api/skills/create` | Create new skill |
| GET | `/api/marketplace/search` | Search FangHub |
| **ClawHub** | | |
| GET | `/api/clawhub/search` | Search ClawHub |
| GET | `/api/clawhub/browse` | Browse ClawHub |
| GET | `/api/clawhub/skill/{slug}` | Skill details |
| POST | `/api/clawhub/install` | Install from ClawHub |
| **MCP & A2A** | | |
| POST | `/api/integrations/reload` | Hot-reload extension MCP integrations |
| GET | `/api/mcp/servers` | MCP server connections |
| POST | `/mcp` | MCP HTTP transport (JSON-RPC 2.0) |
| GET | `/.well-known/agent.json` | A2A agent card |
| GET | `/a2a/agents` | A2A agent list |
| POST | `/a2a/tasks/send` | Send A2A task |
| GET | `/a2a/tasks/{id}` | Get A2A task status |
| POST | `/a2a/tasks/{id}/cancel` | Cancel A2A task |
| **Audit & Security** | | |
| GET | `/api/audit/recent` | Recent audit logs |
| GET | `/api/audit/verify` | Verify Merkle chain integrity |
| GET | `/api/logs/daemon/recent` | Recent daemon tracing lines (file tail) |
| GET | `/api/security` | Security status (16 systems) |
| **SSE (see [SSE Streaming](#sse-streaming))** | | |
| GET | `/api/logs/stream` | SSE: live audit entries (`level`, `filter`, `token`) |
| GET | `/api/logs/daemon/stream` | SSE: tail daemon/tui tracing log |
| GET | `/api/events/stream` | SSE: kernel event bus |
| **Usage & Analytics** | | |
| GET | `/api/usage` | Usage statistics |
| GET | `/api/usage/summary` | Usage summary with quota |
| GET | `/api/usage/by-model` | Usage by model breakdown |
| **Migration** | | |
| GET | `/api/migrate/detect` | Detect migration sources |
| POST | `/api/migrate/scan` | Scan for importable data |
| POST | `/api/migrate` | Run migration |
| **OpenAI Compatible** | | |
| POST | `/v1/chat/completions` | OpenAI-compatible chat |
| GET | `/v1/models` | OpenAI-compatible model list |
