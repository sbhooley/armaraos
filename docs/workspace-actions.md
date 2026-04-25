# Workspace actions (`armaraos.toml`)

Workspace actions give agents a deterministic contract for "run this project task" flows.

Instead of having the model compose shell like:

- `source .venv/bin/activate && python server.py`
- `nohup node gateway.js &`
- `export PORT=8080 && ...`

you declare named actions once in `armaraos.toml`, then call:

- `workspace_actions_list`
- `workspace_action`
- `schedule_action_create` (for recurring jobs)

The runtime executes through `script_run` (deterministic interpreter selection + managed daemons).

---

## File location

Put the contract at the workspace root:

- `<agent-workspace>/armaraos.toml`

The agent workspace is visible in the dashboard and API (`workspace` / `workspace_rel_home` on agent payloads).

---

## Contract schema

Each action lives under `[actions.<name>]`.

```toml
[actions.gateway]
description = "Start gateway server"
script = "src/gateway.ts"            # required
mode = "daemon"                      # optional: oneshot | daemon (default oneshot)
language = "typescript"              # optional hint: python|shell|bash|zsh|node|typescript|bun|deno
cwd = "."                            # optional
args = ["--port", "8080"]            # optional
timeout_seconds = 120                # optional (oneshot)

[actions.gateway.env]                # optional
PORT = "8080"
NODE_ENV = "development"

[actions.gateway.health_check]       # optional (daemon)
url = "http://127.0.0.1:8080/health"
timeout_seconds = 20
expect_status = 200
```

Validation rules:

- At least one action must exist.
- Every action needs non-empty `script`.
- `mode` (if set) must be `oneshot` or `daemon`.
- `health_check.url` (if set) must be non-empty.

---

## Runtime tools

### `workspace_actions_list`

Returns all declared actions with summary fields:

- `name`
- `description`
- `script`
- `mode`
- `language`

Use this first when the user asks "start the gateway/server" and you want deterministic action names instead of guessing paths.

### `workspace_action`

Runs a named action:

```json
{
  "action": "gateway"
}
```

Optional per-call overrides:

- `args` (appended to contract args)
- `env` (merged over contract env)
- `mode`
- `timeout_seconds`

### `schedule_action_create`

Creates a recurring kernel cron job that runs a workspace action by name:

```json
{
  "description": "Gateway heartbeat",
  "schedule": "every 5 minutes",
  "action": "gateway"
}
```

This wraps `schedule_create` with:

- `action.kind = "workspace_action"`
- `action.action_name = "<your action>"`

So scheduled runs stay deterministic and do not require LLM turns.

---

## Scheduling behavior

Kernel cron now supports `workspace_action` directly as a scheduler action kind.

When the job fires:

1. Kernel resolves the agent workspace.
2. Runtime loads `armaraos.toml`.
3. Named action executes through the same deterministic `script_run` path.
4. Delivery/audit/events follow normal cron behavior (`CronJobCompleted` / `CronJobFailed`).

---

## Recommended pattern

1. Define stable project entrypoints in `armaraos.toml`.
2. Use `workspace_action` for interactive "start/run" requests.
3. Use `schedule_action_create` for recurring automation.
4. Keep fallback tools (`script_detect`, `script_run`) for repos without contracts.

This keeps intent selection in the model, while execution strategy stays in deterministic runtime code.

