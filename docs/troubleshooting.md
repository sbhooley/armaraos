# Troubleshooting & FAQ

Common issues, diagnostics, and answers to frequently asked questions about OpenFang.

Paths use the default data directory **`~/.armaraos/`** (see [data-directory.md](data-directory.md) for `ARMARAOS_HOME`, `OPENFANG_HOME`, and migration from `~/.openfang`).

## Table of Contents

- [Quick Diagnostics](#quick-diagnostics)
- [Config schema in the dashboard](#config-schema-in-the-dashboard-at-a-glance)
- [Installation Issues](#installation-issues)
- [Configuration Issues](#configuration-issues)
- [LLM Provider Issues](#llm-provider-issues)
- [Channel Issues](#channel-issues)
- [Agent Issues](#agent-issues)
- [Agent automation hardening](agent-automation-hardening.md) (tool args, phases, persist vs re-scrape)
- [API Issues](#api-issues)
- [Desktop App Issues](#desktop-app-issues)
- [Performance](#performance)
- [FAQ](#faq)

---

## Quick Diagnostics

Run the built-in diagnostic tool:

```bash
armaraos doctor
```

(`openfang doctor` is equivalent when you have the CLI from the same install.)

This checks:
- Configuration file exists and is valid TOML
- API keys are set in environment
- Database is accessible
- Daemon status (running or not)
- Port availability
- Tool dependencies (Python, signal-cli, etc.)

### Check Daemon Status

```bash
openfang status
```

### Check Health via API

```bash
curl http://127.0.0.1:4200/api/health
curl http://127.0.0.1:4200/api/health/detail  # Requires auth
```

### Config schema in the dashboard (at a glance)

To compare **effective** config file versioning with the **binary’s** built-in schema constant (useful after upgrades):

- **Settings** (`#settings`): Immediately **below the tab bar** (all Settings tabs), a summary line shows **Daemon** version, **Config schema** (formatted like `1 (binary 1)`), **API** listen address, **Log** level, and **Home** directory. **Settings → System** also has a **Config schema** stat card with the same value.
- **Monitor → Daemon & runtime** (`#runtime`): The system overview includes **Config schema** in the same `effective (binary N)` style.
- **API:** `GET /api/status` exposes `config_schema_version` (effective after load) and `config_schema_version_binary` (constant in the running binary). `GET /api/config` includes `config_schema_version`. See [data-directory.md](data-directory.md#config-schema-version) for what the numbers mean on disk.

### Dashboard support bundle (redacted `.zip`)

For bug reports, the API can generate a **redacted archive** under `~/.armaraos/support/`:

- **Create:** `POST /api/support/diagnostics` (returns JSON with `bundle_path`, `bundle_filename`, `relative_path`).
- **Download:** `GET /api/support/diagnostics/download?name=<bundle_filename>` streams the `.zip` (`Content-Disposition: attachment`). The `name` query must match the safe pattern `armaraos-diagnostics-YYYYMMDD-HHMMSS.zip` (no path segments).
- **Contents:** Start with **`README.txt`** (what each file is for) and **`diagnostics_snapshot.json`** (structured triage: daemon version, both config schema numbers, paths, `api_listen`, `log_level`, default model, network flag, SQLite `user_version` vs expected memory schema, OS/arch, whether `ARMARAOS_HOME` / `OPENFANG_HOME` are set — no secret values). Also: **`meta.json`** (compact superset of legacy fields plus schema versions and memory SQLite hints), `config.toml`, redacted `secrets.env`, `audit.json` (recent audit entries + tip hash), `data/openfang.db` + WAL/SHM when present, and recent files under `home/logs/…`.
- **Access:** From **loopback** (127.0.0.1 / ::1) both **POST** and **GET** above may be used **without** Bearer auth so the embedded dashboard can create and fetch the zip when an API key is set. Remote clients must use normal API authentication for both routes.

From the UI: **Generate + copy bundle** when disconnected, or **Settings → System Info → Support** (desktop Help menu uses the same flow). On **desktop**, the app copies the zip to **Downloads** via Tauri (`copy_diagnostics_to_downloads` with **`bundlePath`**); on failure, the UI may retry via the HTTP download.

**Home folder:** To grab the same file manually, open **Home folder** → `support/` and use **Download** (full file via `GET /api/armaraos-home/download`) — **View** only previews up to **512 KiB**, so large zips often show a preview error but **Download** still works.

### Remote access vs loopback (diagnostics)

**By design**, `POST /api/support/diagnostics` and **`GET /api/support/diagnostics/download`** are allowed **without** Bearer auth only from **loopback** (127.0.0.1 / ::1) so the embedded dashboard and desktop shell can create and download a bundle when an API key is configured. **`GET /api/armaraos-home/download`** follows the same loopback rule. **Non-loopback** callers must use the same authentication as the rest of the API.

If you ever need **remote** support bundles (e.g. field support over the internet), that is a separate product decision: require strong auth, rate limits, audit logging, and possibly a dedicated support role — do not widen loopback-only endpoints without a threat model.

### View Logs

OpenFang uses `tracing` for structured logging.

**CLI daemon (`openfang start`, `openfang gateway start`):** Logs go to **stderr** and to **`~/.armaraos/logs/daemon.log`** (under `ARMARAOS_HOME` when set). Open the dashboard **Logs → Daemon** to tail or filter without reading the file on disk. The **Live** tab is the **audit** trail (agent actions, tools, etc.), not the same as Rust tracing.

**TUI / `openfang chat`:** Tracing is written to **`~/.armaraos/tui.log`** so the terminal UI is not corrupted.

**Config:** `log_level` in `config.toml` (or dashboard **Logs → Daemon** → Save) sets default verbosity for the daemon process. **Restart the daemon** after changing it. `RUST_LOG` overrides the filter when set:

```bash
RUST_LOG=info openfang start          # Default-style filter
RUST_LOG=debug openfang start         # Verbose
RUST_LOG=openfang=debug openfang start  # Only OpenFang debug, deps at info
```

---

## Installation Issues

### `cargo install` fails with compilation errors

**Cause**: Rust toolchain too old or missing system dependencies.

**Fix**:
```bash
rustup update stable
rustup default stable
rustc --version  # Need 1.75+
```

On Linux, you may also need:
```bash
# Debian/Ubuntu
sudo apt install pkg-config libssl-dev libsqlite3-dev

# Fedora
sudo dnf install openssl-devel sqlite-devel
```

### `openfang` command not found after install

**Fix**: Ensure `~/.cargo/bin` is in your PATH:
```bash
export PATH="$HOME/.cargo/bin:$PATH"
# Add to ~/.bashrc or ~/.zshrc to persist
```

### Docker container won't start

**Common causes**:
- No API key provided: `docker run -e GROQ_API_KEY=... ghcr.io/RightNow-AI/openfang`
- Port already in use: change the port mapping `-p 3001:4200`
- Permission denied on volume mount: check directory permissions

---

## Configuration Issues

### "Config file not found"

**Fix**: Run `openfang init` to create the default config:
```bash
openfang init
```

This creates `~/.armaraos/config.toml` with sensible defaults.

### "Missing API key" warnings on start

**Cause**: No LLM provider API key found in environment.

**Fix**: Set at least one provider key:
```bash
export GROQ_API_KEY="gsk_..."     # Groq (free tier available)
# OR
export ANTHROPIC_API_KEY="sk-ant-..."
# OR
export OPENAI_API_KEY="sk-..."
```

Add to your shell profile to persist across sessions.

### Stale config after upgrading

**Symptom:** A problem survives reinstalling the app, or a **fresh install** on another machine behaves differently from your upgraded machine.

**Cause:** Installers update the binary, not your data directory. **`~/.armaraos/`** (config, SQLite, caches) persists until you move or delete it.

**Fix:** See [data-directory.md](data-directory.md) (config schema version, backup, and full reset). Quick isolation test: stop the daemon, rename `~/.armaraos` to `~/.armaraos.bak`, start again — a new profile is created. Restore from the backup if needed.

### Config validation errors

Run validation manually:
```bash
openfang config show
```

Common issues:
- Malformed TOML syntax (use a TOML validator)
- Invalid port numbers (must be 1-65535)
- Missing required fields in channel configs

### "Port already in use"

**Fix**: Change the port in config or kill the existing process:
```bash
# Change API port
# In config.toml:
# [api]
# listen_addr = "127.0.0.1:3001"

# Or find and kill the process using the port
# Linux/macOS:
lsof -i :4200
# Windows:
netstat -aon | findstr :4200
```

---

## LLM Provider Issues

### "Authentication failed" / 401 errors

**Causes**:
- API key not set or incorrect
- API key expired or revoked
- Wrong env var name

**Fix**: Verify your key:
```bash
# Check if the env var is set
echo $GROQ_API_KEY

# Test the provider
curl http://127.0.0.1:4200/api/providers/groq/test -X POST
```

### "Rate limited" / 429 errors

**Cause**: Too many requests to the LLM provider.

**Fix**:
- The driver automatically retries with exponential backoff
- Reduce `max_llm_tokens_per_hour` in agent capabilities
- Switch to a provider with higher rate limits
- Use multiple providers with model routing

### Slow responses

**Possible causes**:
- Provider API latency (try Groq for fast inference)
- Large context window (use `/compact` to shrink session)
- Complex tool chains (check iteration count in response)

**Fix**: Use per-agent model overrides to use faster models for simple agents:
```toml
[model]
provider = "groq"
model = "llama-3.1-8b-instant"  # Fast, small model
```

### "Model not found"

**Fix**: Check available models:
```bash
curl http://127.0.0.1:4200/api/models
```

Or use an alias:
```toml
[model]
model = "llama"  # Alias for llama-3.3-70b-versatile
```

See the full alias list:
```bash
curl http://127.0.0.1:4200/api/models/aliases
```

### Ollama / local models not connecting

**Fix**: Ensure the local server is running:
```bash
# Ollama
ollama serve  # Default: http://localhost:11434

# vLLM
python -m vllm.entrypoints.openai.api_server --model ...

# LM Studio
# Start from the LM Studio UI, enable API server
```

---

## Channel Issues

### Telegram bot not responding

**Checklist**:
1. Bot token is correct: `echo $TELEGRAM_BOT_TOKEN`
2. Bot has been started (send `/start` in Telegram)
3. If `allowed_users` is set, your Telegram user ID is in the list
4. Check logs for "Telegram adapter" messages

### Discord bot offline

**Checklist**:
1. Bot token is correct
2. **Message Content Intent** is enabled in Discord Developer Portal
3. Bot has been invited to the server with correct permissions
4. Check Gateway connection in logs

### Slack bot not receiving messages

**Checklist**:
1. Both `SLACK_BOT_TOKEN` (xoxb-) and `SLACK_APP_TOKEN` (xapp-) are set
2. Socket Mode is enabled in the Slack app settings
3. Bot has been added to the channels it should monitor
4. Required scopes: `chat:write`, `app_mentions:read`, `im:history`, `im:read`, `im:write`

### Webhook-based channels (WhatsApp, LINE, Viber, etc.)

**Checklist**:
1. Your server is publicly accessible (or use a tunnel like ngrok)
2. Webhook URL is correctly configured in the platform dashboard
3. Webhook port is open and not blocked by firewall
4. Verify token matches between config and platform dashboard

### "Channel adapter failed to start"

**Common causes**:
- Missing or invalid token
- Port already in use (for webhook-based channels)
- Network connectivity issues

Check logs for the specific error:
```bash
RUST_LOG=openfang_channels=debug openfang start
```

---

## Agent Issues

### `file_write` / `shell_exec`: "Missing 'path'" or "Missing 'command'"

**Cause:** The model issued the tool with an **empty** `{}` input or without required fields. This is a **malformed tool call**, not a filesystem denial.

**v0.6.5+:** The error message now lists every missing required field with its type and description, allowing the model to self-correct in one step.

**Fix:**

1. Retry with explicit JSON: `file_write` needs `path` + `content`; `shell_exec` needs `command`.
2. Do **not** redo expensive browser/API work unless you have verified the data never made it to disk or session storage.

**Details and workflow patterns:** [agent-automation-hardening.md](agent-automation-hardening.md).

### `file_write` / `apply_patch`: "Access denied: path resolves outside workspace"

**Cause:** The path points outside the agent's workspace directory. Both `file_write` and `apply_patch` are sandboxed to the agent's own workspace.

**Fix:** Use `shell_exec` for cross-workspace writes:

```json
{ "command": "python3", "args": ["-c", "open('/abs/path/file.py','w').write('content')"] }
```

See [agent-automation-hardening.md — Cross-workspace writes](agent-automation-hardening.md#cross-workspace-writes-access-denied-path-resolves-outside-workspace).

### Agent stuck in a loop

**Cause**: The agent is repeatedly calling the same tool with the same parameters.

**Automatic protection**: OpenFang has a built-in loop guard:
- **Warn** at 3 identical tool calls
- **Block** at 5 identical tool calls
- **Circuit breaker** at 30 total blocked calls (stops the agent)
- **All-blocked early exit** (v0.6.5+): if every tool call in an iteration is blocked, a counter increments; after **3 consecutive all-blocked iterations** the loop exits gracefully with a summary.

**Note:** Identical **empty** tool calls (e.g. repeated `file_write` with `{}`) count toward the same pattern and escalate quickly. Fix the **arguments** first; see [agent-automation-hardening.md](agent-automation-hardening.md).

**Manual fix**: Cancel the agent's current run:
```bash
curl -X POST http://127.0.0.1:4200/api/agents/{id}/stop
```

Or via chat command: `/stop`

### Agent claims process is running without calling `process_start`

**Cause:** The model called `process_list`, saw an empty result, then responded with "Starting it now — proc_1 is up" **without** actually calling `process_start`.

**v0.6.5+:** The runtime detects this phantom action and re-prompts the model to call the actual tool.

**If you see this in logs:** Look for `Process phantom detected` in the daemon log. If the model loops on this, check that `process_start` is granted in the agent manifest's `[capabilities].tools`.

**Always use `cwd` when starting scripts that load local files:**

```json
{ "command": "python3", "args": ["bot.py"], "cwd": "/path/to/workspace" }
```

See [agent-automation-hardening.md — Process management](agent-automation-hardening.md#process-management-process_start-process_kill-process_list).

### Agent running out of context

**Cause**: Conversation history is too long for the model's context window.

**Fix**: Compact the session:
```bash
curl -X POST http://127.0.0.1:4200/api/agents/{id}/session/compact
```

Or via chat command: `/compact`

Auto-compaction is enabled by default when the session reaches the threshold (configurable in `[compaction]`).

### Agent not using tools

**Cause**: Tools not granted in the agent's capabilities.

**Fix**: Check the agent's manifest:
```toml
[capabilities]
tools = ["file_read", "web_fetch", "shell_exec"]  # Must list each tool
# OR
# tools = ["*"]  # Grant all tools (use with caution)
```

### "Permission denied" errors in agent responses

**Cause**: The agent is trying to use a tool or access a resource not in its capabilities.

**Fix**: Add the required capability to the agent manifest. Common ones:
- `tools = [...]` for tool access
- `network = ["*"]` for network access
- `memory_write = ["self.*"]` for memory writes
- `shell = ["*"]` for shell commands (use with caution)

### Agent spawning fails

**Check**:
1. TOML manifest is valid: `openfang agent spawn --dry-run manifest.toml`
2. LLM provider is configured and has a valid key
3. Model specified in manifest exists in the catalog

---

## API Issues

### 401 Unauthorized

**Cause**: API key required but not provided.

**Fix**: Include the Bearer token:
```bash
curl -H "Authorization: Bearer your-api-key" http://127.0.0.1:4200/api/agents
```

### 429 Too Many Requests

**Cause**: GCRA rate limiter triggered.

**Fix**: Wait for the `Retry-After` period, or increase rate limits in config:
```toml
[api]
rate_limit_per_second = 20  # Increase if needed
```

### CORS errors from browser

**Cause**: Trying to access API from a different origin.

**Fix**: Add your origin to CORS config:
```toml
[api]
cors_origins = ["http://localhost:5173", "https://your-app.com"]
```

### WebSocket disconnects

**Possible causes**:
- Idle timeout (send periodic pings)
- Network interruption (reconnect automatically)
- Agent crashed (check logs)

**Client-side fix**: Implement reconnection logic with exponential backoff.

### OpenAI-compatible API not working with my tool

**Checklist**:
1. Use `POST /v1/chat/completions` (not `/api/agents/{id}/message`)
2. Set the model to `openfang:agent-name` (e.g., `openfang:coder`)
3. Streaming: set `"stream": true` for SSE responses
4. Images: use `image_url` with `data:image/png;base64,...` format

### Dashboard: “Check daemon vs GitHub” / version compare fails

**Cause (older builds):** The UI called **GitHub’s API from the browser**, which can fail (CORS, ad blockers, offline WebView).

**Fix:** Use a current daemon. **Settings → System Info** and **Monitor → Runtime** call **`GET /api/version/github-latest`**, which fetches the release **on the server**. Verify: `curl -sS http://127.0.0.1:4200/api/version/github-latest | head -c 300`

### Restart daemon or messaging bridges from the dashboard

**Hot reload:** **Settings → System Info → Daemon / API** or **Monitor → Runtime** — **Reload config**, **Reload channels** (restarts channel bridges / gateways from disk), **Reload integrations**. These use your normal API auth when `api_key` is set.

**Full stop:** **Shut down daemon** calls **`POST /api/shutdown`** (graceful exit). Restart via the **desktop app**, **`openfang start`**, or your **process supervisor**. Loopback clients may call shutdown without Bearer even when a key is set; remote clients must authenticate.

---

## Desktop App Issues

### App won't start

**Checklist**:
1. Only one instance can run at a time (single-instance enforcement)
2. Check if the daemon is already running on the same ports
3. Try deleting `~/.armaraos/daemon.json` and restarting

### White/blank screen in app

**Cause**: The embedded API server hasn't started yet.

**Fix**: Wait a few seconds. If persistent, check logs for server startup errors.

### System tray icon missing

**Platform-specific**:
- **Linux**: Requires a system tray (e.g., `libappindicator` on GNOME)
- **macOS**: Should work out of the box
- **Windows**: Check notification area settings, may need to show hidden icons

### Scheduled AINL: `adapter blocked by capability gate: web` (or similar)

**Typical expectation:** current ArmaraOS injects **`AINL_ALLOW_IR_DECLARED_ADAPTERS=1`** for scheduled **`ainl run`** and sets a desktop default after **`~/.armaraos/.env`**, so digest / web graphs should run without users exporting adapter env vars.

**If it still fails:** upgrade ArmaraOS and PyPI **`ainativelang`**, check **Agents → agent → Info** for **`scheduled_ainl_host_adapter`** (including **`ainl_allow_ir_declared_adapters`**), and see **[scheduled-ainl.md](scheduled-ainl.md)** for manifest opt-out and explicit allowlists.

---

## Performance

### High memory usage

**Tips**:
- Reduce the number of concurrent agents
- Use session compaction for long-running agents
- Use smaller models (Llama 8B instead of 70B for simple tasks)
- Clear old sessions: `DELETE /api/sessions/{id}`

### Slow startup

**Normal startup**: <200ms for the kernel, ~1-2s with channel adapters.

If slower:
- Check database size (`~/.armaraos/data/openfang.db`)
- Reduce the number of enabled channels
- Check network connectivity (MCP server connections happen at boot)

### High CPU usage

**Possible causes**:
- WASM sandbox execution (fuel-limited, should self-terminate)
- Multiple agents running simultaneously
- Channel adapters reconnecting (exponential backoff)

---

## FAQ

### How do I switch the default LLM provider?

Edit `~/.armaraos/config.toml`:
```toml
[default_model]
provider = "groq"
model = "llama-3.3-70b-versatile"
api_key_env = "GROQ_API_KEY"
```

### Can I use multiple providers at the same time?

Yes. Each agent can use a different provider via its manifest `[model]` section. The kernel creates a dedicated driver per unique provider configuration.

### How do I add a new channel?

1. Add the channel config to `~/.armaraos/config.toml` under `[channels]`
2. Set the required environment variables (tokens, secrets)
3. Restart the daemon

### How do I update OpenFang?

```bash
# From source
cd openfang && git pull && cargo install --path crates/openfang-cli

# Docker
docker pull ghcr.io/RightNow-AI/openfang:latest
```

### Can agents talk to each other?

Yes. Agents can use the `agent_send`, `agent_spawn`, `agent_find`, and `agent_list` tools to communicate. The orchestrator template is specifically designed for multi-agent delegation.

### Is my data sent to the cloud?

Only LLM API calls go to the provider's servers. All agent data, memory, sessions, and configuration are stored locally in SQLite (`~/.armaraos/data/openfang.db`). The OFP wire protocol uses HMAC-SHA256 mutual authentication for P2P communication.

### How do I back up my data?

Back up these files:
- `~/.armaraos/config.toml` (configuration)
- `~/.armaraos/data/openfang.db` (all agent data, memory, sessions)
- `~/.armaraos/skills/` (installed skills)

### How do I reset everything?

```bash
rm -rf ~/.armaraos
# If an old tree was never migrated, also: rm -rf ~/.openfang
armaraos init  # Start fresh
```

### Can I run OpenFang without an internet connection?

Yes, if you use a local LLM provider:
- **Ollama**: `ollama serve` + `ollama pull llama3.2`
- **vLLM**: Self-hosted model server
- **LM Studio**: GUI-based local model runner

Set the provider in config:
```toml
[default_model]
provider = "ollama"
model = "llama3.2"
```

### What's the difference between OpenFang and OpenClaw?

| Aspect | OpenFang | OpenClaw |
|--------|----------|----------|
| Language | Rust | Python |
| Channels | 40 | 38 |
| Skills | 60 | 57 |
| Providers | 20 | 3 |
| Security | 16 systems | Config-based |
| Binary size | ~30 MB | ~200 MB |
| Startup | <200 ms | ~3 s |

OpenFang can import OpenClaw configs: `openfang migrate --from openclaw`

### How do I report a bug or request a feature?

- Bugs: Open an issue on GitHub
- Security: See [SECURITY.md](../SECURITY.md) for responsible disclosure
- Features: Open a GitHub discussion or PR

### What are the system requirements?

| Resource | Minimum | Recommended |
|----------|---------|-------------|
| RAM | 128 MB | 512 MB |
| Disk | 50 MB (binary) | 500 MB (with data) |
| CPU | Any x86_64/ARM64 | 2+ cores |
| OS | Linux, macOS, Windows | Any |
| Rust | 1.75+ (build only) | Latest stable |

### How do I enable debug logging for a specific crate?

```bash
RUST_LOG=openfang_runtime=debug,openfang_channels=info openfang start
```

### Can I use OpenFang as a library?

Yes. Each crate is independently usable:
```toml
[dependencies]
openfang-runtime = { path = "crates/openfang-runtime" }
openfang-memory = { path = "crates/openfang-memory" }
```

The `openfang-kernel` crate assembles everything, but you can use individual crates for custom integrations.

---

## Common Community Questions

### How do I update OpenFang?

Re-run the install script to get the latest release:
```bash
curl -fsSL https://openfang.sh/install | sh
```
Or build from source:
```bash
git pull origin main
cargo build --release -p openfang-cli
```

### How do I run OpenFang in Docker?

```bash
docker run -d --name openfang \
  -e GROQ_API_KEY=your_key_here \
  -p 4200:4200 \
  ghcr.io/rightnow-ai/openfang:latest
```

### How do I protect the dashboard with a password?

OpenFang has built-in dashboard authentication. Enable it in `~/.armaraos/config.toml`:

```toml
[auth]
enabled = true
username = "admin"
password_hash = "$argon2id$..."  # see below
```

Generate the password hash:

```bash
openfang auth hash-password
```

Paste the output into the `password_hash` field and restart the daemon.

For public-facing deployments, you should also place a reverse proxy (Caddy, nginx) in front for TLS termination.

### Where are the Get started page and Quick actions documented?

The sidebar **Get started** entry opens hash **`#overview`**: **Quick actions** at the top (after the optional **Live** strip; includes **App Store** → `#ainl-library`), hero stats, setup checklist, **Setup Wizard** visibility (`openfang-onboarded`, sidebar **Get started** re-click), and panels. For layout, Alpine markup locations, CSS classes, and loading skeleton behavior, see **[dashboard-overview-ui.md](dashboard-overview-ui.md)**. For the **Setup Wizard** (`#wizard`) itself (provider step, agent creation manifest, static rebuild), see **[dashboard-setup-wizard.md](dashboard-setup-wizard.md)**. For manual QA (checklist, Quick action hash targets, wizard gating), see **[dashboard-testing.md](dashboard-testing.md)** (*Get started page*). **Settings** and **Runtime** UI polish: **[dashboard-settings-runtime-ui.md](dashboard-settings-runtime-ui.md)**.

### How do I configure the embedding model for memory?

In `~/.armaraos/config.toml`:
```toml
[memory]
embedding_provider = "openai"     # or "ollama", "gemini"
embedding_model = "text-embedding-3-small"
embedding_api_key_env = "OPENAI_API_KEY"
```

For local Ollama embeddings:
```toml
[memory]
embedding_provider = "ollama"
embedding_model = "nomic-embed-text"
```

### Email channel responds to ALL emails — how do I restrict it?

Add `allowed_senders` to your email config:
```toml
[channels.email]
allowed_senders = ["me@example.com", "boss@company.com"]
```
Empty list = responds to everyone. Always set this to avoid auto-replying to spam.

### How do I use Z.AI / GLM-5?

```toml
[default_model]
provider = "zai"
model = "glm-5-20250605"
api_key_env = "ZHIPU_API_KEY"
```

### How do I add Kimi 2.5?

Kimi models are built-in. Use alias `kimi` or the full model ID:
```toml
[default_model]
provider = "moonshot"
model = "kimi-k2.5"
api_key_env = "MOONSHOT_API_KEY"
```

### Can I use multiple Telegram bots?

Not yet — each channel type currently supports one bot. Multi-bot routing is tracked as a feature request (#586). As a workaround, run multiple OpenFang instances on different ports with different configs.

### Claude Code integration shows errors

Add to `~/.armaraos/config.toml`:
```toml
[claude_code]
skip_permissions = true
```
Then restart the daemon.

### Trader hand shell permissions

The trader hand needs shell access for executing trading scripts. In your agent's `agent.toml`:
```toml
[capabilities]
shell = ["python *", "node *"]
```

### OpenRouter free models don't work

OpenRouter free models have strict rate limits and may return empty responses. Use a paid model or try a different free provider like Groq (`GROQ_API_KEY`).
