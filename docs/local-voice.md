# Local voice (Whisper.cpp + Piper)

ArmaraOS can run **local speech-to-text** via [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (`whisper-cli`) and **local text-to-speech** via [Piper](https://github.com/rhasspy/piper), without cloud STT/TTS API keys.

Configuration lives under **`[local_voice]`** in `config.toml`. See **[configuration.md](configuration.md#local_voice)** for every field.

## First launch auto-download

When **`enabled = true`** and **`auto_download = true`** (both default in new installs), the daemon **downloads bundled assets on first boot** into:

```
~/.armaraos/voice/
```

(or **`$ARMARAOS_HOME/voice/`** when the home directory is overridden)

| Subpath | Contents |
|---------|----------|
| `models/ggml-base.bin` | Whisper **base** GGML model (from Hugging Face `ggerganov/whisper.cpp`). |
| `piper_bundle/piper/` | Piper runtime for your OS/arch (from Rhasspy GitHub releases), including the `piper` / `piper.exe` binary and dependencies. |
| `voices/en_US-lessac-medium.onnx` (+ `.json`) | Default English Piper voice (from Hugging Face `rhasspy/piper-voices`). |

- **Windows (x64):** `whisper-cli.exe` and DLLs are also downloaded from the official **whisper.cpp** release zip into `voice/whisper_cpp_win/`.
- **macOS / Linux:** The upstream project does not ship a standalone `whisper-cli` zip for Unix on GitHub releases. After models and Piper are in place, the bootstrap **looks for** `whisper-cli` on **`PATH`**, then **`/opt/homebrew/bin/whisper-cli`**, then **`/usr/local/bin/whisper-cli`**. If none exist, check the daemon log for a short hint (commonly **`brew install whisper-cpp`** on macOS [Homebrew `whisper-cpp`](https://formulae.brew.sh/formula/whisper-cpp)).

**Tests / CI:** Asset download runs in **production daemon builds only**. It is **not** run when the kernel is compiled with `cfg(test)` (for example `cargo test`), so unit tests do not pull large files.

## Opting out

Disable downloads or the whole pipeline:

```toml
[local_voice]
enabled = false
```

or keep local voice but manage files yourself:

```toml
[local_voice]
auto_download = false
whisper_cli = "/absolute/path/to/whisper-cli"
whisper_model = "/absolute/path/to/ggml-base.bin"
piper_binary = "/absolute/path/to/piper"
piper_voice = "/absolute/path/to/en_US-lessac-medium.onnx"
```

## Media routing

The media engine selects STT in this general order unless **`[media] audio_provider`** overrides it: local Whisper when ready, then optional Parakeet/MLX env, then cloud keys (**`GROQ_API_KEY`**, **`OPENAI_API_KEY`**). See **`openfang-runtime`** `media_understanding.rs` and **[configuration.md](configuration.md)** for **`[media]`**.

## Related docs

- **[data-directory.md](data-directory.md)** — `voice/` under the ArmaraOS home directory.
- **[configuration.md](configuration.md#local_voice)** — `config.toml` reference for `[local_voice]`.
