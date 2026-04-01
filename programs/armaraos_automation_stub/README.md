# armaraos_automation_stub

Copy this folder as a starting point for **adapter-based** automation (HTTP, then LLM, DB, etc.).

- Default graph performs **`http.GET`** on `https://example.com/` — replace `probe_url` with your endpoint.
- **Outbound HTTPS** is required when you run or schedule this graph; air-gapped hosts should not enable the curated cron until URLs are internal.
- Curated job **`armaraos-automation-stub-weekly`** ships **paused**; enable in the Scheduler when you trust the environment.

See [docs/ootb-ainl.md](../../docs/ootb-ainl.md).
