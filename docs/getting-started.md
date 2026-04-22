# Getting Started with ArmaraOS

This guide walks you through installing ArmaraOS, configuring your first LLM provider, spawning an agent, and chatting with it.

Config and state default to **`~/.armaraos/`** (see [data-directory.md](data-directory.md) for env overrides and legacy **`~/.openfang`** migration).

## Table of Contents

- [Installation](#installation)
- [Configuration](#configuration)
- [Spawn Your First Agent](#spawn-your-first-agent)
- [Chat with an Agent](#chat-with-an-agent)
- [Start the Daemon](#start-the-daemon)
- [Using the WebChat UI](#using-the-webchat-ui)
- [Next Steps](#next-steps)

---

## Installation

### Option 1: Desktop App (Windows / macOS / Linux)

Download the installer for your platform from **[ainativelang.com](https://ainativelang.com)**:

| Platform | File |
|---|---|
| Windows | `.msi` installer |
| macOS | `.dmg` disk image |
| Linux | `.AppImage` or `.deb` |

The desktop app includes the full ArmaraOS system with a native window, system tray, auto-updates, and OS notifications. Updates are installed automatically in the background.

### Option 2: Cargo Install (Any Platform)

Requires Rust 1.75+:

```bash
cargo install --git https://github.com/sbhooley/armaraos openfang-cli
```

Or build from source:

```bash
git clone https://github.com/sbhooley/armaraos.git
cd armaraos
cargo install --path crates/openfang-cli
```

### Option 5: Docker

Prebuilt images: `ghcr.io/sbhooley/armaraos` (see releases). Build details, `OPENSSL_NO_VENDOR`, and faster local builds: [`docs/docker.md`](docker.md).

```bash
docker pull ghcr.io/sbhooley/armaraos:latest

docker run -d \
  --name openfang \
  -p 50051:50051 \
  -e ANTHROPIC_API_KEY=$ANTHROPIC_API_KEY \
  -v openfang-data:/data \
  ghcr.io/sbhooley/armaraos:latest
```

The image sets **`OPENFANG_LISTEN=0.0.0.0:50051`** so the API is reachable from the host (default **`api_listen`** is loopback-only and would not work with `-p`). Open **`http://localhost:50051/`** in your browser.

Or use Docker Compose:

```bash
git clone https://github.com/sbhooley/armaraos.git
cd armaraos
# Set your API keys in environment or .env file
docker compose up -d
```

### Verify Installation

```bash
openfang --version
```

---

## Configuration

### Initialize

Run the init command to create the `~/.armaraos/` directory and a default config file:

```bash
openfang init
```

This creates:

```
~/.armaraos/
  config.toml    # Main configuration
  data/          # Database and runtime data
  agents/        # Agent manifests (optional)
```

### Set Up an API Key

ArmaraOS needs at least one LLM provider API key. Set it as an environment variable:

```bash
# Anthropic (Claude)
export ANTHROPIC_API_KEY=sk-ant-...

# Or OpenAI
export OPENAI_API_KEY=sk-...

# Or Groq (free tier available)
export GROQ_API_KEY=gsk_...
```

Add the export to your shell profile (`~/.bashrc`, `~/.zshrc`, etc.) to persist it.

### Edit the Config

The default config uses Anthropic. To change the provider, edit `~/.armaraos/config.toml`:

```toml
[default_model]
provider = "groq"                      # anthropic, openai, groq, ollama, etc.
model = "llama-3.3-70b-versatile"      # Model identifier for the provider
api_key_env = "GROQ_API_KEY"           # Env var holding the API key

[memory]
decay_rate = 0.05                      # Memory confidence decay rate

[network]
listen_addr = "127.0.0.1:4200"        # OFP listen address
```

### Verify Your Setup

```bash
armaraos doctor
```

(`openfang doctor` is the same binary when both are on your PATH.)

This checks that your config exists, API keys are set, and the toolchain is available.

---

## Spawn Your First Agent

### Using a Built-in Template

ArmaraOS ships with 30 agent templates. Spawn the hello-world agent:

```bash
openfang agent spawn agents/hello-world/agent.toml
```

Output:

```
Agent spawned successfully!
  ID:   a1b2c3d4-e5f6-...
  Name: hello-world
```

### Using a Custom Manifest

Create your own `my-agent.toml`:

```toml
name = "my-assistant"
version = "0.1.0"
description = "A helpful assistant"
author = "you"
module = "builtin:chat"

[model]
provider = "groq"
model = "llama-3.3-70b-versatile"

[capabilities]
tools = ["file_read", "file_list", "web_fetch"]
memory_read = ["*"]
memory_write = ["self.*"]
```

Then spawn it:

```bash
openfang agent spawn my-agent.toml
```

### List Running Agents

```bash
openfang agent list
```

Output:

```
ID                                     NAME             STATE      PROVIDER     MODEL
-----------------------------------------------------------------------------------------------
a1b2c3d4-e5f6-...                     hello-world      Running    groq         llama-3.3-70b-versatile
```

---

## Chat with an Agent

Start an interactive chat session using the agent ID:

```bash
openfang agent chat a1b2c3d4-e5f6-...
```

Or use the quick chat command (picks the first available agent):

```bash
openfang chat
```

Or specify an agent by name:

```bash
openfang chat hello-world
```

Example session:

```
Chat session started (daemon mode). Type 'exit' or Ctrl+C to quit.

you> Hello! What can you do?

agent> I'm the hello-world agent running on ArmaraOS. I can:
- Read files from the filesystem
- List directory contents
- Fetch web pages

Try asking me to read a file or look up something on the web!

  [tokens: 142 in / 87 out | iterations: 1]

you> List the files in the current directory

agent> Here are the files in the current directory:
- Cargo.toml
- Cargo.lock
- README.md
- agents/
- crates/
- docs/
...

you> exit
Chat session ended.
```

---

## Start the Daemon

For persistent agents, multi-user access, and the WebChat UI, start the daemon:

```bash
openfang start
```

Output:

```
Starting ArmaraOS daemon...
ArmaraOS daemon running on http://127.0.0.1:4200
Press Ctrl+C to stop.
```

The daemon provides:
- **REST API** at `http://127.0.0.1:4200/api/`
- **WebSocket** endpoint at `ws://127.0.0.1:4200/api/agents/{id}/ws`
- **WebChat UI** at `http://127.0.0.1:4200/`
- **OFP networking** on port 4200

### Check Status

```bash
openfang status
```

### Stop the Daemon

Press `Ctrl+C` in the terminal running the daemon, or:

```bash
curl -X POST http://127.0.0.1:4200/api/shutdown
```

---

## Using the WebChat UI

With the daemon running, open your browser to:

```
http://127.0.0.1:4200/
```

The embedded WebChat UI allows you to:
- Open **Get started** from the sidebar (first section, above **Chat**; **Comms** is under **Monitor**) for the landing dashboard: **Quick actions** at the top (agents, **Skills/MCP**, **App Store**, channels, workflows, settings, **Daemon & runtime** → reload/shutdown controls on **Runtime**), system stats, provider status, optional **setup checklist**, and **Setup Wizard** entry in the header (after onboarding, use **Run setup again** or click **Get started** in the sidebar a second time to reopen the wizard — see [Dashboard Get started UI](dashboard-overview-ui.md)). The guided **Setup Wizard** (`#wizard`): steps, provider rules, and how agents are created — [dashboard-setup-wizard.md](dashboard-setup-wizard.md). Layout and source map: [dashboard-overview-ui.md](dashboard-overview-ui.md). Manual QA and `localStorage` notes: [Dashboard testing](dashboard-testing.md). **Settings** and **Runtime** pages use the same polished shell as other top-level routes, including **daemon lifecycle** actions — [Dashboard Settings & Runtime UI](dashboard-settings-runtime-ui.md).
- **Config schema at a glance:** Open **Settings** — the line under the tab bar shows **Config schema** as `effective (binary N)` (with **`⚠ mismatch`** when the file and binary disagree) plus API, log level, and home path; **System** repeats it as a stat tile, and **Daemon & runtime** shows the same schema line. Details: [Troubleshooting — Config schema in the dashboard](troubleshooting.md#config-schema-in-the-dashboard-at-a-glance).
- View all running agents
- Chat with any agent in real-time (via WebSocket)
- If the agent WebSocket is unavailable, the UI falls back to **`POST /api/agents/{id}/message`**; the JSON may include **`tools`** so tool runs still show as expandable cards (see [API Reference](api-reference.md), section **POST /api/agents/{id}/message**).
- See streaming responses as they are generated
- View token usage per message
- **Voice (mic):** Record a message with the mic; the server **transcribes** it to text for the agent. Replies are **text in the chat** unless you turn on the **speaker** button next to the mic (asks the daemon for a **Piper** audio reply when `[local_voice]` is set up). See [local-voice.md](local-voice.md#webchat-voice-in-text-out-by-default).

The default landing route in the SPA is still `#overview`; bookmarks and links using that hash continue to work.

---

## Next Steps

Now that you have ArmaraOS running:

- **Explore agent templates**: Browse the `agents/` directory for 30 pre-built agents (coder, researcher, writer, ops, analyst, security-auditor, and more).
- **Create custom agents**: Write your own `agent.toml` manifests. See the [Architecture guide](architecture) for details on capabilities and scheduling.
- **Set up channels**: Connect any of 40 messaging platforms (Telegram, Discord, Slack, WhatsApp, LINE, Mastodon, and 34 more). See [Channel Adapters](channel-adapters).
- **Use bundled skills**: 60 expert knowledge skills are pre-installed (GitHub, Docker, Kubernetes, security audit, prompt engineering, etc.). See [Skill Development](skill-development).
- **Build custom skills**: Extend agents with Python, WASM, or prompt-only skills. See [Skill Development](skill-development).
- **Use the API**: Broad REST/WS/SSE surface, including an OpenAI-compatible `/v1/chat/completions` and ArmaraOS Home browser routes. See [API Reference](api-reference).
- **Switch LLM providers**: 20 providers supported (Anthropic, OpenAI, Gemini, Groq, DeepSeek, xAI, Ollama, and more). Per-agent model overrides.
- **Set up workflows**: Chain multiple agents together. Use `openfang workflow create` with a TOML workflow definition.
- **Use MCP**: Connect to external tools via Model Context Protocol. Configure in `config.toml` under `[[mcp_servers]]`.
- **Migrate from OpenClaw**: Run `openfang migrate --from openclaw`. See [MIGRATION.md](../MIGRATION.md).
- **Desktop app**: Run `cargo tauri dev` for a native desktop experience with system tray.
- **Run diagnostics**: `armaraos doctor` checks your entire setup.
- **Local voice (optional)**: On first daemon start, `[local_voice]` can download Whisper + Piper into `~/.armaraos/voice/` (see [local-voice.md](local-voice.md)). On macOS/Linux you may still need **`whisper-cli`** (e.g. `brew install whisper-cpp`). Voice **input** uses STT; assistant **output** is normally **text** unless you opt into a voice reply as documented in [local-voice.md](local-voice.md#webchat-voice-in-text-out-by-default).

### Useful Commands Reference

```bash
openfang init                          # Initialize ~/.armaraos/
openfang start                         # Start the daemon
openfang status                        # Check daemon status
armaraos doctor                        # Run diagnostic checks

openfang agent spawn <manifest.toml>   # Spawn an agent
openfang agent list                    # List all agents
openfang agent chat <id>               # Chat with an agent
openfang agent kill <id>               # Kill an agent

openfang workflow list                 # List workflows
openfang workflow create <file.json>   # Create a workflow
openfang workflow run <id> <input>     # Run a workflow

openfang trigger list                  # List event triggers
openfang trigger create <args>         # Create a trigger
openfang trigger delete <id>           # Delete a trigger

openfang skill install <source>        # Install a skill
openfang skill list                    # List installed skills
openfang skill search <query>          # Search FangHub
openfang skill create                  # Scaffold a new skill

openfang channel list                  # List channel status
openfang channel setup <channel>       # Interactive setup wizard

openfang config show                   # Show current config
openfang config edit                   # Open config in editor

openfang chat [agent]                  # Quick chat (alias)
openfang migrate --from openclaw       # Migrate from OpenClaw
openfang mcp                           # Start MCP server (stdio)
```
