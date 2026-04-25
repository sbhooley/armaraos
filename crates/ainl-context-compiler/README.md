# ainl-context-compiler

Multi-segment, role-aware **LLM context assembly** for AINL hosts: system/user/history/tool blocks, optional graph-memory vitals, trajectory recap, and failure warnings.

- **`mcp_ainl_prompt`:** short, static **`mcp_ainl_run` / `adapters`** cheatsheet text (`MCP_AINL_RUN_ADAPTERS_CHEATSHEET`, `mcp_ainl_run_adapters_cheatsheet_segment()`) for optional inclusion in whole-prompt compose when a host wants a compressed reminder alongside **`ainl_context_compiler`**.

- **Repository:** <https://github.com/sbhooley/armaraos>
- **API reference:** <https://docs.rs/ainl-context-compiler>

Feature flags pull in `ainl-memory`, `ainl-failure-learning`, `ainl-context-freshness`, `ainl-semantic-tagger`, and related crates. Distinct from `ainl-context-freshness`, which gates execution when context is stale.
