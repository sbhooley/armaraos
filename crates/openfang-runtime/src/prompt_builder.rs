//! Centralized system prompt builder.
//!
//! Assembles a structured, multi-section system prompt from agent context.
//! Replaces the scattered `push_str` prompt injection throughout the codebase
//! with a single, testable, ordered prompt builder.
//!
//! ## Manifest `system_prompt` vs. what the LLM sees
//!
//! On disk / in the API, `[model].system_prompt` is **identity and user-authored
//! instructions only**. The kernel merges it as [`PromptContext::base_system_prompt`]
//! and replaces `manifest.model.system_prompt` with [`build_system_prompt`] output
//! before each LLM run (AINL awareness, host environment, tools, safety, etc.).
//! The registry keeps the **base** text; the expanded string is not persisted.

/// Metadata key: kernel set this to `true` after replacing the manifest's base
/// `system_prompt` with [`build_system_prompt`] output for the current request.
pub const KERNEL_EXPANDED_SYSTEM_PROMPT_META_KEY: &str = "kernel_expanded_system_prompt_v1";

/// All the context needed to build a system prompt for an agent.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Agent name (from manifest).
    pub agent_name: String,
    /// Agent description (from manifest).
    pub agent_description: String,
    /// Identity and user-authored instructions from `[model].system_prompt` in the manifest.
    ///
    /// This is **not** the full string sent to the LLM: [`build_system_prompt`] wraps it with
    /// host sections (AINL, ArmaraOS vs OpenClaw, tools, memory protocol, safety, …).
    pub base_system_prompt: String,
    /// Tool names this agent has access to.
    pub granted_tools: Vec<String>,
    /// Recalled memories as (key, content) pairs.
    pub recalled_memories: Vec<(String, String)>,
    /// Skill summary text (from kernel.build_skill_summary()).
    pub skill_summary: String,
    /// Prompt context from prompt-only skills.
    pub skill_prompt_context: String,
    /// MCP server/tool summary text.
    pub mcp_summary: String,
    /// Agent workspace path.
    pub workspace_path: Option<String>,
    /// SOUL.md content (persona).
    pub soul_md: Option<String>,
    /// USER.md content.
    pub user_md: Option<String>,
    /// MEMORY.md content.
    pub memory_md: Option<String>,
    /// Cross-channel canonical context summary.
    pub canonical_context: Option<String>,
    /// Known user name (from shared memory).
    pub user_name: Option<String>,
    /// Channel type (telegram, discord, web, etc.).
    pub channel_type: Option<String>,
    /// Whether this agent was spawned as a subagent.
    pub is_subagent: bool,
    /// Whether this agent has autonomous config.
    pub is_autonomous: bool,
    /// AGENTS.md content (behavioral guidance).
    pub agents_md: Option<String>,
    /// BOOTSTRAP.md content (first-run ritual).
    pub bootstrap_md: Option<String>,
    /// Workspace context section (project type, context files).
    pub workspace_context: Option<String>,
    /// IDENTITY.md content (visual identity + personality frontmatter).
    pub identity_md: Option<String>,
    /// HEARTBEAT.md content (autonomous agent checklist).
    pub heartbeat_md: Option<String>,
    /// Peer agents visible to this agent: (name, state, model).
    pub peer_agents: Vec<(String, String, String)>,
    /// Current date/time string for temporal awareness.
    pub current_date: Option<String>,
    /// Sender identity (e.g. WhatsApp phone number, Telegram user ID).
    pub sender_id: Option<String>,
    /// Sender display name.
    pub sender_name: Option<String>,
}

/// Build the complete system prompt from a `PromptContext`.
///
/// Produces an ordered, multi-section prompt. Sections with no content are
/// omitted entirely (no empty headers). Subagent mode skips sections that
/// add unnecessary context overhead.
pub fn build_system_prompt(ctx: &PromptContext) -> String {
    let mut sections: Vec<String> = Vec::with_capacity(12);

    // Section 1 — Agent Identity (always present)
    sections.push(build_identity_section(ctx));

    // Section 1.5 — Current Date/Time (always present when set)
    if let Some(ref date) = ctx.current_date {
        sections.push(format!("## Current Date\nToday is {date}."));
    }

    // Section 1.6 — AINL product awareness (ArmaraOS hosts AINL graphs / tooling; include for subagents too)
    sections.push(AINL_AWARENESS_SECTION.to_string());

    // Section 1.65 — AINL authoring workflow (all agents; complements MCP tool list when present)
    sections.push(AINL_AUTHORING_WORKFLOW_SECTION.to_string());

    if granted_ainl_mcp_tools(&ctx.granted_tools) {
        sections.push(AINL_MCP_TOOLS_GRANTED_NOTE.to_string());
    }

    // Section 1.7 — Disambiguate ArmaraOS from OpenClaw (models often conflate the two)
    sections.push(ARMARAOS_NOT_OPENCLAW_SECTION.to_string());

    // Section 2 — Tool Call Behavior (skip for subagents)
    if !ctx.is_subagent {
        sections.push(TOOL_CALL_BEHAVIOR.to_string());
    }

    // Section 2.5 — Agent Behavioral Guidelines (skip for subagents)
    if !ctx.is_subagent {
        if let Some(ref agents) = ctx.agents_md {
            if !agents.trim().is_empty() {
                sections.push(cap_str(agents, 2000));
            }
        }
    }

    // Section 3 — Available Tools (always present if tools exist)
    let tools_section = build_tools_section(&ctx.granted_tools);
    if !tools_section.is_empty() {
        sections.push(tools_section);
    }

    // Section 4 — Memory Protocol (always present)
    let mem_section = build_memory_section(&ctx.recalled_memories);
    sections.push(mem_section);

    // Section 5 — Skills (only if skills available)
    if !ctx.skill_summary.is_empty() || !ctx.skill_prompt_context.is_empty() {
        sections.push(build_skills_section(
            &ctx.skill_summary,
            &ctx.skill_prompt_context,
        ));
    }

    // Section 6 — MCP Servers (only if summary present)
    if !ctx.mcp_summary.is_empty() {
        sections.push(build_mcp_section(&ctx.mcp_summary));
    }

    // Section 7 — Persona / Identity files (skip for subagents)
    if !ctx.is_subagent {
        let persona = build_persona_section(
            ctx.identity_md.as_deref(),
            ctx.soul_md.as_deref(),
            ctx.user_md.as_deref(),
            ctx.memory_md.as_deref(),
            ctx.workspace_path.as_deref(),
        );
        if !persona.is_empty() {
            sections.push(persona);
        }
    }

    // Section 7.5 — Heartbeat checklist (only for autonomous agents)
    if !ctx.is_subagent && ctx.is_autonomous {
        if let Some(ref heartbeat) = ctx.heartbeat_md {
            if !heartbeat.trim().is_empty() {
                sections.push(format!(
                    "## Heartbeat Checklist\n{}",
                    cap_str(heartbeat, 1000)
                ));
            }
        }
    }

    // Section 8 — User Personalization (skip for subagents)
    if !ctx.is_subagent {
        sections.push(build_user_section(ctx.user_name.as_deref()));
    }

    // Section 9 — Channel Awareness (skip for subagents)
    if !ctx.is_subagent {
        if let Some(ref channel) = ctx.channel_type {
            sections.push(build_channel_section(channel));
        }
    }

    // Section 9.1 — Sender Identity (skip for subagents)
    if !ctx.is_subagent {
        if let Some(sender_line) =
            build_sender_section(ctx.sender_name.as_deref(), ctx.sender_id.as_deref())
        {
            sections.push(sender_line);
        }
    }

    // Section 9.5 — Peer Agent Awareness (skip for subagents)
    if !ctx.is_subagent && !ctx.peer_agents.is_empty() {
        sections.push(build_peer_agents_section(&ctx.agent_name, &ctx.peer_agents));
    }

    // Section 10 — Safety & Oversight (skip for subagents)
    if !ctx.is_subagent {
        sections.push(SAFETY_SECTION.to_string());
    }

    // Section 11 — Operational Guidelines (always present)
    sections.push(OPERATIONAL_GUIDELINES.to_string());

    // Section 12 — Canonical Context moved to build_canonical_context_message()
    // to keep the system prompt stable across turns for provider prompt caching.

    // Section 13 — Bootstrap Protocol (only on first-run, skip for subagents)
    if !ctx.is_subagent {
        if let Some(ref bootstrap) = ctx.bootstrap_md {
            if !bootstrap.trim().is_empty() {
                // Only inject if no user_name memory exists (first-run heuristic)
                let has_user_name = ctx.recalled_memories.iter().any(|(k, _)| k == "user_name");
                if !has_user_name && ctx.user_name.is_none() {
                    sections.push(format!(
                        "## First-Run Protocol\n{}",
                        cap_str(bootstrap, 1500)
                    ));
                }
            }
        }
    }

    // Section 14 — Workspace Context (skip for subagents)
    if !ctx.is_subagent {
        if let Some(ref ws_ctx) = ctx.workspace_context {
            if !ws_ctx.trim().is_empty() {
                sections.push(cap_str(ws_ctx, 1000));
            }
        }
    }

    sections.join("\n\n")
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

fn build_identity_section(ctx: &PromptContext) -> String {
    if ctx.base_system_prompt.is_empty() {
        format!(
            "You are {}, an AI agent running inside the ArmaraOS Agent OS.\n{}",
            ctx.agent_name, ctx.agent_description
        )
    } else {
        ctx.base_system_prompt.clone()
    }
}

/// Canonical pointers for questions about AI Native Language (AINL) on ArmaraOS.
const AINL_AWARENESS_SECTION: &str = "\
## AI Native Language (AINL)
This environment integrates **AI Native Language (AINL)** — the same project may appear as **AINL**, **AINativeLang**, or **AI Native Language**; treat those names as equivalent.

- **Website (product & documentation):** https://ainativelang.com
- **Source, issues, and releases:** https://github.com/sbhooley/ainativelang

When users ask about AINL versions, adapters, syntax, or release notes, treat these URLs as authoritative. Prefer **web_fetch** or **web_search** against them (or GitHub Releases / `docs/CHANGELOG.md` in the repo) instead of inventing changelog details.

If a tool or capability reports an **embedded runtime version** (e.g. from a bundled interpreter), that number may differ from the latest **PyPI** package (`pip install ainativelang`) or **git** tag — say so briefly and still point to the links above for official release notes.";

/// Workflow for authoring `.ainl` graphs (validate-first, MCP feedback, avoid random example grepping).
const AINL_AUTHORING_WORKFLOW_SECTION: &str = "\
## AINL authoring (workflows)
For scrapers, scheduled jobs, HTTP/API glue, bots, CRM-style automation, or similar **repeatable workflows**, prefer **AINL** `.ainl` graphs over ad-hoc scripts unless the user clearly needs a general-purpose language.

- **Treat AINL as built-in in this host:** if `mcp_ainl_*` tools are in **Your Tools**, use them directly; do **not** pause to install `ainativelang` with `pip`/`python -m pip` as a prerequisite.
- **Validate before run:** after editing `.ainl`, call **`mcp_ainl_ainl_validate`** with `strict: true` before **`mcp_ainl_ainl_run`**. On failure, fix the graph using fields in the tool response (e.g. **`primary_diagnostic`**, **`source_context`**, **`llm_repair_hint`**, **`agent_repair_steps`**) — do **not** use broad **`file_search`** or repo-wide greps on random `.ainl` files as your primary syntax reference.
- **Tool-call shape matters:** for `mcp_ainl_*` tools, pass the graph text in the `code` field (legacy `ainl` may work but `code` is canonical), and call the MCP tool directly — do not wrap MCP tool names inside `shell_exec`.
- **Real verbs only:** before writing `R adapter.VERB` lines, call **`mcp_ainl_ainl_capabilities`** (or use its output) so the verb exists for strict graphs.
- **Starters:** for new graphs, consider **`mcp_ainl_ainl_list_ecosystem`** and **`mcp_ainl_ainl_import_*`** before writing large graphs from scratch.
- **`ainl_run` adapters:** graphs that use `http`, `fs`, `cache`, or `sqlite` must pass an **`adapters`** object in the run payload — the MCP server does not register those adapters by default (see tool schema / server instructions).";

/// Short reminder when this agent actually has `mcp_ainl_*` tools granted.
const AINL_MCP_TOOLS_GRANTED_NOTE: &str = "\
## AINL MCP tools (granted)
You have the **ainl** MCP namespace in **Your Tools**. Golden path: **`mcp_ainl_ainl_validate`** (strict) → **`mcp_ainl_ainl_compile`** (when IR is needed) → **`mcp_ainl_ainl_run`** with correct **`adapters`** when needed. Use **`mcp_ainl_ainl_list_ecosystem`** for importable starters.";

/// True when `granted_tools` includes at least one tool for MCP server `ainl` (`mcp_ainl_*`).
fn granted_ainl_mcp_tools(granted: &[String]) -> bool {
    granted.iter().any(|t| {
        let p: Vec<&str> = t.splitn(3, '_').collect();
        p.len() >= 3 && p[0] == "mcp" && p[1] == "ainl"
    })
}

/// Clarifies that this host is ArmaraOS, not the OpenClaw CLI/npm product (reduces spurious install attempts).
const ARMARAOS_NOT_OPENCLAW_SECTION: &str = "\
## Host environment (ArmaraOS, not OpenClaw)
You run inside **ArmaraOS** (OpenFang). This is **not** the OpenClaw agent framework or npm package.

- Do **not** install OpenClaw, run `npx openclaw`, or follow OpenClaw-only setup guides unless the user explicitly asked to **migrate from** OpenClaw.
- Config keys like `[openclaw_workspace]` / `OPENCLAW_WORKSPACE` are **legacy names** for a skills/capture folder path only — they do **not** mean you must install OpenClaw.
- Fixing AINL, MCP (`ainl_run`), HTTP adapters, or scheduled graphs does **not** require initializing OpenClaw.";

const PROMPT_ANCHOR_AINL: &str = "## AI Native Language (AINL)";
const PROMPT_ANCHOR_NOT_OPENCLAW: &str = "## Host environment (ArmaraOS, not OpenClaw)";

/// Append AINL + ArmaraOS/OpenClaw anchor sections if they are missing.
///
/// Full prompts should come from [`build_system_prompt`] (kernel path). This exists as a
/// safety net for callers that pass a bare identity prompt, so core host guidance is never
/// fully absent. It does **not** add tools, MCP, safety, or operational sections.
pub fn ensure_mandatory_host_anchor_sections(prompt: &mut String) {
    if !prompt.contains(PROMPT_ANCHOR_AINL) {
        prompt.push_str("\n\n");
        prompt.push_str(AINL_AWARENESS_SECTION);
    }
    if !prompt.contains(PROMPT_ANCHOR_NOT_OPENCLAW) {
        prompt.push_str("\n\n");
        prompt.push_str(ARMARAOS_NOT_OPENCLAW_SECTION);
    }
}

/// Static tool-call behavior directives.
const TOOL_CALL_BEHAVIOR: &str = "\
## Tool Call Behavior
- When you need to use a tool, call it immediately. Do not narrate or explain routine tool calls.
- Only explain tool calls when the action is destructive, unusual, or the user explicitly asked for an explanation.
- Prefer action over narration. If you can answer by using a tool, do it.
- When executing multiple sequential tool calls, batch them — don't output reasoning between each call.
- If a tool returns useful results, present the KEY information, not the raw output.
- When web_fetch or web_search returns content, you MUST include the relevant data in your response. \
Quote specific facts, numbers, or passages from the fetched content. Never say you fetched something \
without sharing what you found.
- Start with the answer, not meta-commentary about how you'll help.
- IMPORTANT: If your instructions or persona mention a shell command, script path, or code snippet, \
execute it via the appropriate tool call (shell_exec, file_write, etc.). Never output commands as \
code blocks — always call the tool instead.";

/// Build the grouped tools section (Section 3).
pub fn build_tools_section(granted_tools: &[String]) -> String {
    if granted_tools.is_empty() {
        return String::new();
    }

    // Group tools by category
    let mut groups: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for name in granted_tools {
        let cat = tool_category(name);
        let hint = tool_hint(name);
        groups.entry(cat).or_default().push((name.as_str(), hint));
    }

    let mut out = String::from("## Your Tools\nYou have access to these capabilities:\n");
    for (category, tools) in &groups {
        out.push_str(&format!("\n**{}**: ", capitalize(category)));
        let descs: Vec<String> = tools
            .iter()
            .map(|(name, hint)| {
                if hint.is_empty() {
                    (*name).to_string()
                } else {
                    format!("{name} ({hint})")
                }
            })
            .collect();
        out.push_str(&descs.join(", "));
    }
    out
}

/// Build canonical context as a standalone user message (instead of system prompt).
///
/// This keeps the system prompt stable across turns, enabling provider prompt caching
/// (Anthropic cache_control, etc.). The canonical context changes every turn, so
/// injecting it in the system prompt caused 82%+ cache misses.
///
/// Always prepends a cached **[Host AINL]** snapshot (`ainl --version`, `pip show ainativelang`)
/// so agents see local toolchain versions without a web round-trip.
pub fn build_canonical_context_message(ctx: &PromptContext) -> Option<String> {
    let host = crate::host_ainl_snapshot::host_ainl_snapshot_cached();

    if ctx.is_subagent {
        // Cross-session summary is omitted for subagents; still surface local AINL toolchain facts.
        return Some(host);
    }

    let session = ctx
        .canonical_context
        .as_ref()
        .filter(|c| !c.is_empty())
        .map(|c| format!("[Previous conversation context]\n{}", cap_str(c, 500)));

    match session {
        Some(s) => Some(format!("{host}\n\n{s}")),
        None => Some(host),
    }
}

/// Build the memory section (Section 4).
///
/// Also used by `agent_loop.rs` to append recalled memories after DB lookup.
pub fn build_memory_section(memories: &[(String, String)]) -> String {
    let mut out = String::from("## Memory\n");
    if memories.is_empty() {
        out.push_str(
            "- When the user asks about something from a previous conversation, use memory_recall first.\n\
             - Store important preferences, decisions, and context with memory_store for future use.",
        );
    } else {
        out.push_str(
            "- Use the recalled memories below to inform your responses.\n\
             - Only call memory_recall if you need information not already shown here.\n\
             - Store important preferences, decisions, and context with memory_store for future use.",
        );
        out.push_str("\n\nRecalled memories:\n");
        for (key, content) in memories.iter().take(5) {
            let capped = cap_str(content, 500);
            if key.is_empty() {
                out.push_str(&format!("- {capped}\n"));
            } else {
                out.push_str(&format!("- [{key}] {capped}\n"));
            }
        }
    }
    out
}

fn build_skills_section(skill_summary: &str, prompt_context: &str) -> String {
    let mut out = String::from("## Skills\n");
    if !skill_summary.is_empty() {
        out.push_str(
            "You have installed skills. If a request matches a skill, use its tools directly.\n",
        );
        out.push_str(skill_summary.trim());
    }
    if !prompt_context.is_empty() {
        out.push('\n');
        out.push_str(&cap_str(prompt_context, 2000));
    }
    out
}

fn build_mcp_section(mcp_summary: &str) -> String {
    format!("## Connected Tool Servers (MCP)\n{}", mcp_summary.trim())
}

fn build_persona_section(
    identity_md: Option<&str>,
    soul_md: Option<&str>,
    user_md: Option<&str>,
    memory_md: Option<&str>,
    workspace_path: Option<&str>,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(ws) = workspace_path {
        parts.push(format!("## Workspace\nWorkspace: {ws}"));
    }

    // Identity file (IDENTITY.md) — personality at a glance, before SOUL.md
    if let Some(identity) = identity_md {
        if !identity.trim().is_empty() {
            parts.push(format!("## Identity\n{}", cap_str(identity, 500)));
        }
    }

    if let Some(soul) = soul_md {
        if !soul.trim().is_empty() {
            let sanitized = strip_code_blocks(soul);
            parts.push(format!(
                "## Persona\nEmbody this identity in your tone and communication style. Be natural, not stiff or generic.\n{}",
                cap_str(&sanitized, 1000)
            ));
        }
    }

    if let Some(user) = user_md {
        if !user.trim().is_empty() {
            parts.push(format!("## User Context\n{}", cap_str(user, 500)));
        }
    }

    if let Some(memory) = memory_md {
        if !memory.trim().is_empty() {
            parts.push(format!("## Long-Term Memory\n{}", cap_str(memory, 500)));
        }
    }

    parts.join("\n\n")
}

fn build_user_section(user_name: Option<&str>) -> String {
    match user_name {
        Some(name) => {
            format!(
                "## User Profile\n\
                 The user's name is \"{name}\". Address them by name naturally \
                 when appropriate (greetings, farewells, etc.), but don't overuse it."
            )
        }
        None => "## User Profile\n\
             You don't know the user's name yet.\n\
             Your **only** public name to users is **Armara**. Never call yourself \"General Assistant\", \"AI assistant\", \
             \"virtual assistant\", or any other generic label — always **Armara**.\n\
             On your FIRST reply in this conversation, introduce yourself as **Armara**. \
             When the user's first message is a greeting, /start, hi/hello, or similarly empty of a concrete task, \
             use exactly this wording (only adjust line breaks for the channel):\n\n\
             Hello! I'm Armara, your personal assistant powered by AI Native Language. I'm here to help with everyday tasks, answer questions, search the web, build things, and more! To get started may I know your name and one thing you'd like help with today?\n\n\
             If their first message already contains a specific question or task, answer it helpfully and weave in a one-sentence version of the same introduction (name Armara, powered by AI Native Language, happy to help) before or after your answer — do not ignore their request.\n\
             Once they tell you their name, immediately use the `memory_store` tool with \
             key \"user_name\" and their name as the value so you remember it for future sessions."
            .to_string(),
    }
}

fn build_channel_section(channel: &str) -> String {
    let (limit, hints) = match channel {
        "telegram" => (
            "4096",
            "Use Telegram-compatible formatting (bold with *, code with `backticks`).",
        ),
        "discord" => (
            "2000",
            "Use Discord markdown. Split long responses across multiple messages if needed.",
        ),
        "slack" => (
            "4000",
            "Use Slack mrkdwn formatting (*bold*, _italic_, `code`).",
        ),
        "whatsapp" => (
            "4096",
            "Keep messages concise. WhatsApp has limited formatting.",
        ),
        "irc" => (
            "512",
            "Keep messages very short. No markdown — plain text only.",
        ),
        "matrix" => (
            "65535",
            "Matrix supports rich formatting. Use markdown freely.",
        ),
        "teams" => ("28000", "Use Teams-compatible markdown."),
        _ => ("4096", "Use markdown formatting where supported."),
    };
    format!(
        "## Channel\n\
         You are responding via {channel}. Keep messages under {limit} chars.\n\
         {hints}"
    )
}

fn build_sender_section(sender_name: Option<&str>, sender_id: Option<&str>) -> Option<String> {
    match (sender_name, sender_id) {
        (Some(name), Some(id)) => Some(format!("## Sender\nMessage from: {name} ({id})")),
        (Some(name), None) => Some(format!("## Sender\nMessage from: {name}")),
        (None, Some(id)) => Some(format!("## Sender\nMessage from: {id}")),
        (None, None) => None,
    }
}

fn build_peer_agents_section(self_name: &str, peers: &[(String, String, String)]) -> String {
    let mut out = String::from(
        "## Peer Agents\n\
         You are part of a multi-agent system. These agents are running alongside you:\n",
    );
    for (name, state, model) in peers {
        if name == self_name {
            continue; // Don't list yourself
        }
        out.push_str(&format!("- **{}** ({}) — model: {}\n", name, state, model));
    }
    out.push_str(
        "\nYou can communicate with them using `agent_send` (by name) and see all agents with `agent_list`. \
         Delegate tasks to specialized agents when appropriate.",
    );
    out
}

/// Static safety section.
const SAFETY_SECTION: &str = "\
## Safety
- Prioritize safety and human oversight over task completion.
- NEVER auto-execute purchases, payments, account deletions, or irreversible actions without explicit user confirmation.
- If a tool could cause data loss, explain what it will do and confirm first.
- If you cannot accomplish a task safely, explain the limitation.
- When in doubt, ask the user.";

/// Static operational guidelines (replaces STABILITY_GUIDELINES).
const OPERATIONAL_GUIDELINES: &str = "\
## Operational Guidelines
- Do NOT retry a tool call with identical parameters if it failed. Try a different approach.
- If a tool returns an error, analyze the error before calling it again.
- When editing AINL (`.ainl`), if **`mcp_ainl_ainl_validate`** fails, fix the graph from the structured fields in the response (`primary_diagnostic`, `agent_repair_steps`, etc.) and re-validate — do not rely on hunting random example `.ainl` files across the repo as your main syntax source.
- Prefer targeted, specific tool calls over broad ones.
- Plan your approach before executing multiple tool calls.
- If you cannot accomplish a task after a few attempts, explain what went wrong instead of looping.
- Never call the same tool more than 3 times with the same parameters.
- If a message requires no response (simple acknowledgments, reactions, messages not directed at you), respond with exactly NO_REPLY.";

// ---------------------------------------------------------------------------
// Tool metadata helpers
// ---------------------------------------------------------------------------

/// One-line hint for `mcp_ainl_*` tools (server key `ainl` from host MCP config).
fn mcp_ainl_tool_hint(full_name: &str) -> &'static str {
    let Some(rest) = full_name.strip_prefix("mcp_ainl_") else {
        return "AINL MCP toolchain";
    };
    match rest {
        "ainl_validate" => {
            "validate AINL source; fix using primary_diagnostic and agent_repair_steps"
        }
        "ainl_compile" => "compile AINL source to IR JSON",
        "ainl_capabilities" => {
            "list adapter verbs and strict contract (use before inventing R lines)"
        }
        "ainl_security_report" => "security / policy report for AINL source",
        "ainl_run" => "compile and run AINL; pass adapters for http/fs/cache/sqlite when needed",
        "ainl_list_ecosystem" => "list importable AINL starters from the ecosystem",
        "ainl_import_clawflow" => "import a graph from ClawFlow-style input",
        "ainl_import_agency_agent" => "import a graph from agency-agent style input",
        "ainl_import_markdown" => "import a graph from markdown description",
        "ainl_fitness_report" => "stability / fitness report for an AINL file",
        "ainl_ir_diff" => "diff IR between two AINL files",
        "ainl_ptc_signature_check" => "PTC signature check on AINL source",
        "ainl_trace_export" => "export trace JSONL to structured JSONL",
        "ainl_ptc_run" => "run PTC suite on AINL",
        "ainl_ptc_health_check" => "PTC health check",
        _ if rest.starts_with("ainl_import_") => "import a starter graph from the AINL ecosystem",
        _ => "AINL MCP tool (validate strict before run when editing graphs)",
    }
}

/// Map a tool name to its category for grouping.
pub fn tool_category(name: &str) -> &'static str {
    match name {
        "file_read" | "file_write" | "file_list" | "file_delete" | "file_move" | "file_copy"
        | "file_search" => "Files",

        "web_search" | "web_fetch" => "Web",

        "browser_navigate" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_read_page" | "browser_close" | "browser_scroll" | "browser_wait"
        | "browser_evaluate" | "browser_select" | "browser_back" => "Browser",

        "shell_exec" | "shell_background" => "Shell",

        "memory_store" | "memory_recall" | "memory_delete" | "memory_list" => "Memory",

        "agent_send" | "agent_spawn" | "agent_list" | "agent_kill" => "Agents",

        "image_describe" | "image_generate" | "audio_transcribe" | "tts_speak" => "Media",

        "docker_exec" | "docker_build" | "docker_run" => "Docker",

        "cron_create" | "cron_list" | "cron_cancel" | "schedule_create" | "schedule_list"
        | "schedule_delete" | "channels_list" => "Scheduling & channels",

        "process_start" | "process_poll" | "process_write" | "process_kill" | "process_list" => {
            "Processes"
        }

        _ if name.starts_with("mcp_") => "MCP",
        _ if name.starts_with("skill_") => "Skills",
        _ => "Other",
    }
}

/// Map a tool name to a one-line description hint.
pub fn tool_hint(name: &str) -> &'static str {
    match name {
        // Files
        "file_read" => "read file contents",
        "file_write" => "create or overwrite a file",
        "file_list" => "list directory contents",
        "file_delete" => "delete a file",
        "file_move" => "move or rename a file",
        "file_copy" => "copy a file",
        "file_search" => "search files by name pattern",

        // Web
        "web_search" => "search the web for information",
        "web_fetch" => "fetch a URL and get its content as markdown",

        // Browser
        "browser_navigate" => "open a URL in the browser",
        "browser_click" => "click an element on the page",
        "browser_type" => "type text into an input field",
        "browser_screenshot" => "capture a screenshot",
        "browser_read_page" => "extract page content as text",
        "browser_close" => "close the browser session",
        "browser_scroll" => "scroll the page",
        "browser_wait" => "wait for an element or condition",
        "browser_evaluate" => "run JavaScript on the page",
        "browser_select" => "select a dropdown option",
        "browser_back" => "go back to the previous page",

        // Shell
        "shell_exec" => "execute a shell command",
        "shell_background" => "run a command in the background",

        // Memory
        "memory_store" => "save a key-value pair to memory",
        "memory_recall" => "search memory for relevant context",
        "memory_delete" => "delete a memory entry",
        "memory_list" => "list stored memory keys",

        // Agents
        "agent_send" => "send a message to another agent",
        "agent_spawn" => "create a new agent",
        "agent_list" => "list running agents",
        "agent_kill" => "terminate an agent",

        // Media
        "image_describe" => "describe an image",
        "image_generate" => "generate an image from a prompt",
        "audio_transcribe" => "transcribe audio to text",
        "tts_speak" => "convert text to speech",

        // Docker
        "docker_exec" => "run a command in a container",
        "docker_build" => "build a Docker image",
        "docker_run" => "start a Docker container",

        // Scheduling & outbound channels
        "cron_create" => "create a kernel cron job (ArmaraOS scheduler)",
        "cron_list" => "list kernel cron jobs for this agent",
        "cron_cancel" => "remove a kernel cron job by id",
        "schedule_create" => "create a cron job (friendly alias of cron_create)",
        "schedule_list" => "list cron jobs for this agent",
        "schedule_delete" => "delete a cron job by id",
        "channels_list" => "list registered channel adapters for alerts",

        // Processes
        "process_start" => "start a long-running process (REPL, server)",
        "process_poll" => "read stdout/stderr from a running process",
        "process_write" => "write to a process's stdin",
        "process_kill" => "terminate a running process",
        "process_list" => "list active processes",

        _ if name.starts_with("mcp_ainl_") => mcp_ainl_tool_hint(name),

        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Cap a string to `max_chars`, appending "..." if truncated.
/// Strip markdown triple-backtick code blocks from content.
///
/// Prevents LLMs from copying code blocks as text output instead of making
/// tool calls when SOUL.md contains command examples.
fn strip_code_blocks(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut in_block = false;
    for line in content.lines() {
        if line.trim_start().starts_with("```") {
            in_block = !in_block;
            continue;
        }
        if !in_block {
            result.push_str(line);
            result.push('\n');
        }
    }
    // Collapse multiple blank lines left by stripped blocks
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}

fn cap_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(max_chars)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        format!("{}...", &s[..end])
    }
}

/// Capitalize the first letter of a string.
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_ctx() -> PromptContext {
        PromptContext {
            agent_name: "researcher".to_string(),
            agent_description: "Research agent".to_string(),
            base_system_prompt: "You are Researcher, a research agent.".to_string(),
            granted_tools: vec![
                "web_search".to_string(),
                "web_fetch".to_string(),
                "file_read".to_string(),
                "file_write".to_string(),
                "memory_store".to_string(),
                "memory_recall".to_string(),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn test_full_prompt_has_all_sections() {
        let prompt = build_system_prompt(&basic_ctx());
        assert!(prompt.contains("You are Researcher"));
        assert!(prompt.contains("## AI Native Language (AINL)"));
        assert!(prompt.contains("## AINL authoring (workflows)"));
        assert!(!prompt.contains("## AINL MCP tools (granted)"));
        assert!(prompt.contains("## Host environment (ArmaraOS, not OpenClaw)"));
        assert!(prompt.contains("https://ainativelang.com"));
        assert!(prompt.contains("https://github.com/sbhooley/ainativelang"));
        assert!(prompt.contains("## Tool Call Behavior"));
        assert!(prompt.contains("## Your Tools"));
        assert!(prompt.contains("## Memory"));
        assert!(prompt.contains("## User Profile"));
        assert!(prompt.contains("## Safety"));
        assert!(prompt.contains("## Operational Guidelines"));
    }

    #[test]
    fn test_section_ordering() {
        let prompt = build_system_prompt(&basic_ctx());
        let tool_behavior_pos = prompt.find("## Tool Call Behavior").unwrap();
        let tools_pos = prompt.find("## Your Tools").unwrap();
        let memory_pos = prompt.find("## Memory").unwrap();
        let safety_pos = prompt.find("## Safety").unwrap();
        let guidelines_pos = prompt.find("## Operational Guidelines").unwrap();

        assert!(tool_behavior_pos < tools_pos);
        assert!(tools_pos < memory_pos);
        assert!(memory_pos < safety_pos);
        assert!(safety_pos < guidelines_pos);
    }

    #[test]
    fn test_subagent_omits_sections() {
        let mut ctx = basic_ctx();
        ctx.is_subagent = true;
        let prompt = build_system_prompt(&ctx);

        assert!(prompt.contains("## AI Native Language (AINL)"));
        assert!(prompt.contains("## AINL authoring (workflows)"));
        assert!(prompt.contains("## Host environment (ArmaraOS, not OpenClaw)"));
        assert!(!prompt.contains("## Tool Call Behavior"));
        assert!(!prompt.contains("## User Profile"));
        assert!(!prompt.contains("## Channel"));
        assert!(!prompt.contains("## Safety"));
        // Subagents still get tools and guidelines
        assert!(prompt.contains("## Your Tools"));
        assert!(prompt.contains("## Operational Guidelines"));
        assert!(prompt.contains("## Memory"));
    }

    #[test]
    fn test_empty_tools_no_section() {
        let ctx = PromptContext {
            agent_name: "test".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Your Tools"));
    }

    #[test]
    fn test_tool_grouping() {
        let tools = vec![
            "web_search".to_string(),
            "web_fetch".to_string(),
            "file_read".to_string(),
            "browser_navigate".to_string(),
        ];
        let section = build_tools_section(&tools);
        assert!(section.contains("**Browser**"));
        assert!(section.contains("**Files**"));
        assert!(section.contains("**Web**"));
    }

    #[test]
    fn test_tool_categories() {
        assert_eq!(tool_category("file_read"), "Files");
        assert_eq!(tool_category("web_search"), "Web");
        assert_eq!(tool_category("browser_navigate"), "Browser");
        assert_eq!(tool_category("shell_exec"), "Shell");
        assert_eq!(tool_category("memory_store"), "Memory");
        assert_eq!(tool_category("agent_send"), "Agents");
        assert_eq!(tool_category("mcp_github_search"), "MCP");
        assert_eq!(tool_category("unknown_tool"), "Other");
    }

    #[test]
    fn test_tool_hints() {
        assert!(!tool_hint("web_search").is_empty());
        assert!(!tool_hint("file_read").is_empty());
        assert!(!tool_hint("browser_navigate").is_empty());
        assert!(tool_hint("some_unknown_tool").is_empty());
    }

    #[test]
    fn test_mcp_ainl_tool_hints() {
        assert!(!tool_hint("mcp_ainl_ainl_validate").is_empty());
        assert!(!tool_hint("mcp_ainl_ainl_run").is_empty());
        assert!(tool_hint("mcp_ainl_ainl_validate").contains("validate"));
    }

    #[test]
    fn test_granted_ainl_mcp_adds_connected_note() {
        let mut ctx = basic_ctx();
        ctx.granted_tools.push("mcp_ainl_ainl_validate".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## AINL MCP tools (granted)"));
        assert!(prompt.contains("Golden path"));
    }

    #[test]
    fn test_memory_section_empty() {
        let section = build_memory_section(&[]);
        assert!(section.contains("## Memory"));
        assert!(section.contains("use memory_recall first"));
        assert!(!section.contains("Recalled memories"));
    }

    #[test]
    fn test_memory_section_with_items() {
        let memories = vec![
            ("pref".to_string(), "User likes dark mode".to_string()),
            ("ctx".to_string(), "Working on Rust project".to_string()),
        ];
        let section = build_memory_section(&memories);
        assert!(section.contains("Recalled memories"));
        assert!(section.contains("[pref] User likes dark mode"));
        assert!(section.contains("[ctx] Working on Rust project"));
        assert!(section.contains("Use the recalled memories below"));
        assert!(!section.contains("use memory_recall first"));
    }

    #[test]
    fn test_memory_cap_at_5() {
        let memories: Vec<(String, String)> = (0..10)
            .map(|i| (format!("k{i}"), format!("value {i}")))
            .collect();
        let section = build_memory_section(&memories);
        assert!(section.contains("[k0]"));
        assert!(section.contains("[k4]"));
        assert!(!section.contains("[k5]"));
    }

    #[test]
    fn test_memory_content_capped() {
        let long_content = "x".repeat(1000);
        let memories = vec![("k".to_string(), long_content)];
        let section = build_memory_section(&memories);
        // Should be capped at 500 + "..."
        assert!(section.contains("..."));
        assert!(section.len() < 1200);
    }

    #[test]
    fn test_skills_section_omitted_when_empty() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Skills"));
    }

    #[test]
    fn test_skills_section_present() {
        let mut ctx = basic_ctx();
        ctx.skill_summary = "- web-search: Search the web\n- git-expert: Git commands".to_string();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Skills"));
        assert!(prompt.contains("web-search"));
    }

    #[test]
    fn test_mcp_section_omitted_when_empty() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("## Connected Tool Servers"));
    }

    #[test]
    fn test_mcp_section_present() {
        let mut ctx = basic_ctx();
        ctx.mcp_summary = "- github: 5 tools (search, create_issue, ...)".to_string();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Connected Tool Servers (MCP)"));
        assert!(prompt.contains("github"));
    }

    #[test]
    fn test_persona_section_with_soul() {
        let mut ctx = basic_ctx();
        ctx.soul_md = Some("You are a pirate. Arr!".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Persona"));
        assert!(prompt.contains("pirate"));
    }

    #[test]
    fn test_persona_soul_capped_at_1000() {
        let long_soul = "x".repeat(2000);
        let section = build_persona_section(None, Some(&long_soul), None, None, None);
        assert!(section.contains("..."));
        // The raw soul content in the section should be at most 1003 chars (1000 + "...")
        assert!(section.len() < 1200);
    }

    #[test]
    fn test_channel_telegram() {
        let section = build_channel_section("telegram");
        assert!(section.contains("4096"));
        assert!(section.contains("Telegram"));
    }

    #[test]
    fn test_channel_discord() {
        let section = build_channel_section("discord");
        assert!(section.contains("2000"));
        assert!(section.contains("Discord"));
    }

    #[test]
    fn test_channel_irc() {
        let section = build_channel_section("irc");
        assert!(section.contains("512"));
        assert!(section.contains("plain text"));
    }

    #[test]
    fn test_channel_unknown_gets_default() {
        let section = build_channel_section("smoke_signal");
        assert!(section.contains("4096"));
        assert!(section.contains("smoke_signal"));
    }

    #[test]
    fn test_user_name_known() {
        let mut ctx = basic_ctx();
        ctx.user_name = Some("Alice".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("Alice"));
        assert!(!prompt.contains("don't know the user's name"));
    }

    #[test]
    fn test_user_name_unknown() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("don't know the user's name"));
        assert!(prompt.contains("General Assistant"));
        assert!(prompt.contains("Armara"));
    }

    #[test]
    fn test_prompt_disambiguates_armaraos_from_openclaw() {
        let ctx = basic_ctx();
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("ArmaraOS, not OpenClaw"));
        assert!(prompt.contains("Do **not** install OpenClaw"));
    }

    #[test]
    fn test_ensure_mandatory_host_anchor_sections_appends_when_missing() {
        let mut p = "Hello.".to_string();
        ensure_mandatory_host_anchor_sections(&mut p);
        assert!(p.contains("## AI Native Language (AINL)"));
        assert!(p.contains("## Host environment (ArmaraOS, not OpenClaw)"));
        let before = p.len();
        ensure_mandatory_host_anchor_sections(&mut p);
        assert_eq!(p.len(), before, "second call should not duplicate sections");
    }

    #[test]
    fn test_canonical_context_not_in_system_prompt() {
        let mut ctx = basic_ctx();
        ctx.canonical_context =
            Some("User was discussing Rust async patterns last time.".to_string());
        let prompt = build_system_prompt(&ctx);
        // Canonical context should NOT be in system prompt (moved to user message)
        assert!(!prompt.contains("## Previous Conversation Context"));
        assert!(!prompt.contains("Rust async patterns"));
        // But should be available via build_canonical_context_message
        let msg = build_canonical_context_message(&ctx).expect("canonical msg");
        assert!(msg.contains("[Host AINL"));
        assert!(msg.contains("Rust async patterns"));
    }

    #[test]
    fn test_canonical_context_host_snapshot_without_session() {
        let ctx = basic_ctx();
        assert!(ctx.canonical_context.is_none());
        let msg = build_canonical_context_message(&ctx).expect("host snapshot");
        assert!(msg.contains("[Host AINL"));
    }

    #[test]
    fn test_canonical_context_subagent_gets_host_only() {
        let mut ctx = basic_ctx();
        ctx.is_subagent = true;
        ctx.canonical_context = Some("Previous context here.".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(!prompt.contains("Previous Conversation Context"));
        let msg = build_canonical_context_message(&ctx).expect("host snapshot for subagent");
        assert!(msg.contains("[Host AINL"));
        assert!(!msg.contains("Previous context here."));
    }

    #[test]
    fn test_empty_base_prompt_generates_default_identity() {
        let ctx = PromptContext {
            agent_name: "helper".to_string(),
            agent_description: "A helpful agent".to_string(),
            ..Default::default()
        };
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("You are helper"));
        assert!(prompt.contains("A helpful agent"));
    }

    #[test]
    fn test_workspace_in_persona() {
        let mut ctx = basic_ctx();
        ctx.workspace_path = Some("/home/user/project".to_string());
        let prompt = build_system_prompt(&ctx);
        assert!(prompt.contains("## Workspace"));
        assert!(prompt.contains("/home/user/project"));
    }

    #[test]
    fn test_cap_str_short() {
        assert_eq!(cap_str("hello", 10), "hello");
    }

    #[test]
    fn test_cap_str_long() {
        let result = cap_str("hello world", 5);
        assert_eq!(result, "hello...");
    }

    #[test]
    fn test_cap_str_multibyte_utf8() {
        // This was panicking with "byte index is not a char boundary" (#38)
        let chinese = "你好世界这是一个测试字符串";
        let result = cap_str(chinese, 4);
        assert_eq!(result, "你好世界...");
        // Exact boundary
        assert_eq!(cap_str(chinese, 100), chinese);
    }

    #[test]
    fn test_cap_str_emoji() {
        let emoji = "👋🌍🚀✨💯";
        let result = cap_str(emoji, 3);
        assert_eq!(result, "👋🌍🚀...");
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("files"), "Files");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("MCP"), "MCP");
    }
}
