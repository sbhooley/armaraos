# armaraos_automation_stub

**Naming:** “stub” means a **curated starting template** (real `http.GET`, real graph)—not an empty stand-in. Replace URLs and enable the cron when the environment is trusted.

Copy this folder as a starting point for **adapter-based** automation (HTTP, then LLM, DB, etc.).

- Default graph performs **`http.GET`** on `https://example.com/` — replace `probe_url` with your endpoint.
- **Outbound HTTPS** is required when you run or schedule this graph; air-gapped hosts should not enable the curated cron until URLs are internal.
- Curated job **`armaraos-automation-stub-weekly`** ships **paused**; enable in the Scheduler when you trust the environment.

See [docs/ootb-ainl.md](../../docs/ootb-ainl.md).
