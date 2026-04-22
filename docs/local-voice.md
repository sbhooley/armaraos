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

## WebChat: voice in, text out (by default)

- **User → agent (STT):** The dashboard **mic** records audio, uploads it, and the API runs **speech-to-text** (upload-time and/or on the message path). The transcribed text is what the agent and **ainl-runtime** prelude see—same as a typed user message, so memory / persona / tagger paths stay consistent when STT succeeds.
- **Agent → user (replies):** The assistant’s answer is shown as **text in the chat** by default. The model does **not** speak back automatically in the browser.
- **Optional spoken reply (Piper):** In the **WebChat** input row, the **speaker** button (next to the mic) toggles **“spoken assistant reply”** on or off. When you turn it **on**, the dashboard first asks **`GET /api/system/local-voice`**; if **Piper is not ready**, the toggle stays off and a **toast** explains the usual fix (go online for first-time auto-download, check **`[local_voice]`**, or see this doc). When **on** and ready, the client sends **`voice_reply: true`** on the next message (typed or with voice) so the API may synthesize the reply with **Piper** and return a short-lived audio URL. The preference is stored in the browser as **`localStorage` `armaraos-voice-reply`**. When the toggle is **off** (default), answers stay **text-only** even if you use the mic.

## Related docs

- **[data-directory.md](data-directory.md)** — `voice/` under the ArmaraOS home directory.
- **[configuration.md](configuration.md#local_voice)** — `config.toml` reference for `[local_voice]`.
