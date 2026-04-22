# Development: building and testing

## Workspace build and checks

From the repository root:

```bash
cargo build --workspace --lib
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

`cargo test --workspace` exercises unit tests, integration tests, and several HTTP/API suites. A full run can take **on the order of 15–25+ minutes** on a typical laptop, depending on CPU and I/O.

## Integration tests and local services

- **OpenFang API** (`crates/openfang-api/tests/`): Many tests spin up a real Axum server and in-process kernel with temporary home directories. They are written to avoid depending on a live GPU or a real Ollama model where possible (e.g. **planner** e2e tests use **Wiremock** for `ainl-inference-server`-shaped URLs and set `ARMARA_NATIVE_INFER_URL` accordingly).
- **Legacy LLM fallback paths** that use the OpenAI-compatible surface expect a base URL that includes **`/v1`** (same shape as the Ollama default, `http://localhost:11434/v1`). Tests that mock `POST /v1/chat/completions` configure the kernel `default_model.base_url` to match.
- **Local voice bootstrap** (`local_voice` auto-download) uses blocking HTTP on a **dedicated OS thread** so it is safe to call during kernel boot from async contexts (for example `#[tokio::test]`), without nesting `reqwest::blocking` on the Tokio runtime worker.
- **Voice / STT (API + runtime):** Unit tests in **`openfang-api`** cover upload-registry behavior, temp expiry sweep, per-agent “latest voice” replacement, **merge** of client text with server transcripts (`routes::voice_message_tests`, including `merge_client_with_voice_transcripts`), and tool-placeholder detection. Run a focused check with:
  ```bash
  cargo test -p openfang-api --lib voice_message
  ```
  **`openfang-runtime`** tests cover **`MediaEngine::transcribe_audio`** error paths and **`tts`** (`TtsEngine` validation, **`synthesize_piper_local`** when Piper is not configured). A full **successful** STT/TTS round-trip is not in the default suite (requires providers or local binaries); use manual QA with **[local-voice.md](local-voice.md)**.
- Some tests may **spawn subprocesses** (for example MCP/npm tooling). Benign **`npm`** log lines during a run do not necessarily indicate a Rust test failure; use the final **`cargo test` summary** (pass/fail per suite) as the source of truth.

## Optional / ignored tests

Individual crates may expose **`#[ignore]`** tests (live network, optional keys, or manual workflows). Run them only when you intend to, with the environment documented on that test or crate.

## Related docs

- Manual dashboard checks: [`dashboard-testing.md`](dashboard-testing.md)
- Data directory and env overrides: [`data-directory.md`](data-directory.md)
