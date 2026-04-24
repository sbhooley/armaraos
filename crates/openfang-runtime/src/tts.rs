//! Text-to-speech engine — synthesize text to audio.
//!
//! Auto-cascades through available providers based on configured API keys.

use openfang_types::config::TtsConfig;
use tokio::io::AsyncWriteExt;

/// Maximum audio response size (10MB).
const MAX_AUDIO_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Result of TTS synthesis.
#[derive(Debug)]
pub struct TtsResult {
    pub audio_data: Vec<u8>,
    pub format: String,
    pub provider: String,
    pub duration_estimate_ms: u64,
}

/// Text-to-speech engine.
pub struct TtsEngine {
    config: TtsConfig,
}

impl TtsEngine {
    pub fn new(config: TtsConfig) -> Self {
        Self { config }
    }

    /// Detect which TTS provider is available based on environment variables.
    fn detect_provider() -> Option<&'static str> {
        if std::env::var("OPENAI_API_KEY").is_ok() {
            return Some("openai");
        }
        if std::env::var("ELEVENLABS_API_KEY").is_ok() {
            return Some("elevenlabs");
        }
        None
    }

    /// Synthesize text to audio bytes.
    /// Auto-cascade: configured provider -> OpenAI -> ElevenLabs.
    /// Optional overrides for voice and format (per-request, from tool input).
    pub async fn synthesize(
        &self,
        text: &str,
        voice_override: Option<&str>,
        format_override: Option<&str>,
    ) -> Result<TtsResult, String> {
        if !self.config.enabled {
            return Err("TTS is disabled in configuration".into());
        }

        // Validate text length
        if text.is_empty() {
            return Err("Text cannot be empty".into());
        }
        if text.len() > self.config.max_text_length {
            return Err(format!(
                "Text too long: {} chars (max {})",
                text.len(),
                self.config.max_text_length
            ));
        }

        let provider = self
            .config
            .provider
            .as_deref()
            .or_else(|| Self::detect_provider())
            .ok_or("No TTS provider configured. Set OPENAI_API_KEY or ELEVENLABS_API_KEY")?;

        match provider {
            "openai" => {
                self.synthesize_openai(text, voice_override, format_override)
                    .await
            }
            "elevenlabs" => self.synthesize_elevenlabs(text, voice_override).await,
            other => Err(format!("Unknown TTS provider: {other}")),
        }
    }

    /// Synthesize via OpenAI TTS API.
    async fn synthesize_openai(
        &self,
        text: &str,
        voice_override: Option<&str>,
        format_override: Option<&str>,
    ) -> Result<TtsResult, String> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| "OPENAI_API_KEY not set")?;

        // Apply per-request overrides or fall back to config defaults
        let voice = voice_override.unwrap_or(&self.config.openai.voice);
        let format = format_override.unwrap_or(&self.config.openai.format);

        let body = serde_json::json!({
            "model": self.config.openai.model,
            "input": text,
            "voice": voice,
            "response_format": format,
            "speed": self.config.openai.speed,
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(self.config.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("OpenAI TTS request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::str_utils::safe_truncate_str(&err, 500);
            return Err(format!("OpenAI TTS failed (HTTP {status}): {truncated}"));
        }

        // Check content length before downloading
        if let Some(len) = response.content_length() {
            if len as usize > MAX_AUDIO_RESPONSE_BYTES {
                return Err(format!(
                    "Audio response too large: {len} bytes (max {MAX_AUDIO_RESPONSE_BYTES})"
                ));
            }
        }

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read audio response: {e}"))?;

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            ));
        }

        // Rough duration estimate: ~150 words/min at ~12 bytes/ms for MP3
        let word_count = text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500); // ~400ms per word, min 500ms

        Ok(TtsResult {
            audio_data: audio_data.to_vec(),
            format: format.to_string(),
            provider: "openai".to_string(),
            duration_estimate_ms: duration_ms,
        })
    }

    /// Synthesize via ElevenLabs TTS API.
    async fn synthesize_elevenlabs(
        &self,
        text: &str,
        voice_override: Option<&str>,
    ) -> Result<TtsResult, String> {
        let api_key =
            std::env::var("ELEVENLABS_API_KEY").map_err(|_| "ELEVENLABS_API_KEY not set")?;

        let voice_id = voice_override.unwrap_or(&self.config.elevenlabs.voice_id);
        let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{}", voice_id);

        let body = serde_json::json!({
            "text": text,
            "model_id": self.config.elevenlabs.model_id,
            "voice_settings": {
                "stability": self.config.elevenlabs.stability,
                "similarity_boost": self.config.elevenlabs.similarity_boost,
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("xi-api-key", &api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(self.config.timeout_secs))
            .send()
            .await
            .map_err(|e| format!("ElevenLabs TTS request failed: {e}"))?;

        if !response.status().is_success() {
            let status = response.status();
            let err = response.text().await.unwrap_or_default();
            let truncated = crate::str_utils::safe_truncate_str(&err, 500);
            return Err(format!(
                "ElevenLabs TTS failed (HTTP {status}): {truncated}"
            ));
        }

        if let Some(len) = response.content_length() {
            if len as usize > MAX_AUDIO_RESPONSE_BYTES {
                return Err(format!(
                    "Audio response too large: {len} bytes (max {MAX_AUDIO_RESPONSE_BYTES})"
                ));
            }
        }

        let audio_data = response
            .bytes()
            .await
            .map_err(|e| format!("Failed to read audio response: {e}"))?;

        if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
            return Err(format!(
                "Audio data exceeds {}MB limit",
                MAX_AUDIO_RESPONSE_BYTES / 1024 / 1024
            ));
        }

        let word_count = text.split_whitespace().count();
        let duration_ms = (word_count as u64 * 400).max(500);

        Ok(TtsResult {
            audio_data: audio_data.to_vec(),
            format: "mp3".to_string(),
            provider: "elevenlabs".to_string(),
            duration_estimate_ms: duration_ms,
        })
    }
}

/// Local Piper TTS for `[local_voice]` (voice replies without cloud STT/TTS APIs).
///
/// Note: prefer [`synthesize_local_tts`] for the dashboard speaker-reply path — it cascades
/// Piper → macOS `say`, which is essential because the upstream rhasspy/piper macOS aarch64
/// release ships `.dSYM` debug symbols but **omits** `libonnxruntime.1.14.1.dylib`,
/// `libespeak-ng.1.dylib`, and `libpiper_phonemize.1.dylib` — the binary file_exists() check
/// passes, but `dyld` aborts at first invocation.
pub async fn synthesize_piper_local(
    text: &str,
    local_voice: &openfang_types::config::LocalVoiceConfig,
) -> Result<TtsResult, String> {
    if text.trim().is_empty() {
        return Err("Text cannot be empty".into());
    }
    if !local_voice.piper_ready_with_active_voice() {
        return Err(
            "Piper is not configured: set [local_voice] enabled=true, piper_binary, and piper_voice (or upload one in Settings → Voice)."
                .into(),
        );
    }
    let bin = local_voice.piper_binary.as_ref().unwrap();
    let model_owned = local_voice
        .active_piper_voice()
        .ok_or("Piper voice path missing after readiness check")?;
    let model = &model_owned;
    let out = std::env::temp_dir().join(format!("openfang_piper_{}.wav", uuid::Uuid::new_v4()));
    let mut cmd = tokio::process::Command::new(bin);
    cmd.arg("--model")
        .arg(model.as_os_str())
        .arg("--output_file")
        .arg(out.as_os_str())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Piper's release binaries `dlopen` sibling dylibs via `@rpath` — set DYLD/LD_LIBRARY_PATH
    // to the binary's directory so its bundled deps resolve even when launched outside that dir.
    if let Some(dir) = bin.parent() {
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", dir);
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", dir);
        // Piper also looks here for `espeak-ng-data` — explicit env is more reliable than CWD.
        let espeak_data = dir.join("espeak-ng-data");
        if espeak_data.is_dir() {
            cmd.env("ESPEAK_DATA_PATH", &espeak_data);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("failed to spawn piper: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .await
            .map_err(|e| format!("piper stdin: {e}"))?;
    }
    let status = child.wait_with_output().await.map_err(|e| e.to_string())?;
    if !status.status.success() {
        // Best-effort cleanup if piper partially wrote a file before crashing.
        let _ = tokio::fs::remove_file(&out).await;
        let stderr = String::from_utf8_lossy(&status.stderr);
        let truncated = crate::str_utils::safe_truncate_str(stderr.as_ref(), 400);
        return Err(format!(
            "piper exited with {}: {}",
            status.status, truncated
        ));
    }
    let audio_data = tokio::fs::read(&out)
        .await
        .map_err(|e| format!("read piper wav: {e}"))?;
    let _ = tokio::fs::remove_file(&out).await;
    if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
        return Err("Piper output too large".into());
    }
    if audio_data.len() < 44 {
        // 44 = minimum WAVE/RIFF header. Piper exited 0 but produced no audio — bail.
        return Err("piper produced empty / truncated audio".into());
    }
    let word_count = text.split_whitespace().count();
    let duration_ms = (word_count as u64 * 400).max(500);
    Ok(TtsResult {
        audio_data,
        format: "wav".into(),
        provider: "piper".into(),
        duration_estimate_ms: duration_ms,
    })
}

/// macOS built-in `/usr/bin/say` TTS. Always available on macOS, no install required.
/// Used as the deterministic fallback for [`synthesize_local_tts`] when Piper is broken or absent.
///
/// `voice` selects a `say -v <voice>` option (e.g. `"Susan"`, `"Susan (Enhanced)"`,
/// `"Ava (Premium)"`); `None` uses the system default voice.
#[cfg(target_os = "macos")]
pub async fn synthesize_macos_say(text: &str, voice: Option<&str>) -> Result<TtsResult, String> {
    if text.trim().is_empty() {
        return Err("Text cannot be empty".into());
    }
    let out = std::env::temp_dir().join(format!("openfang_say_{}.wav", uuid::Uuid::new_v4()));
    let mut cmd = tokio::process::Command::new("/usr/bin/say");
    if let Some(v) = voice.map(str::trim).filter(|v| !v.is_empty()) {
        cmd.arg("-v").arg(v);
    }
    // `--data-format=LEI16@22050` produces 16-bit little-endian PCM at 22050 Hz, wrapped in
    // a WAVE container — directly playable by <audio> in every browser.
    let status = cmd
        .arg("--file-format=WAVE")
        .arg("--data-format=LEI16@22050")
        .arg("-o")
        .arg(out.as_os_str())
        .arg(text)
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|e| format!("failed to spawn /usr/bin/say: {e}"))?;
    if !status.status.success() {
        let _ = tokio::fs::remove_file(&out).await;
        let stderr = String::from_utf8_lossy(&status.stderr);
        return Err(format!("say exited with {}: {}", status.status, stderr));
    }
    let audio_data = tokio::fs::read(&out)
        .await
        .map_err(|e| format!("read say wav: {e}"))?;
    let _ = tokio::fs::remove_file(&out).await;
    if audio_data.len() > MAX_AUDIO_RESPONSE_BYTES {
        return Err("say output too large".into());
    }
    if audio_data.len() < 44 {
        return Err("say produced empty audio".into());
    }
    let word_count = text.split_whitespace().count();
    let duration_ms = (word_count as u64 * 400).max(500);
    Ok(TtsResult {
        audio_data,
        format: "wav".into(),
        provider: "macos_say".into(),
        duration_estimate_ms: duration_ms,
    })
}

/// Cascading local TTS for the WebChat speaker reply path:
///   1. **Kokoro** (when assets ready *and* inference is implemented — currently a scaffolding
///      stub that always errors so we don't ship fake audio).
///   2. **macOS `say` (optional first)** — when [`LocalVoiceConfig::prefer_macos_say`] is true and
///      `/usr/bin/say` exists, try `say` with [`LocalVoiceConfig::preferred_say_voice`] **before**
///      Piper so System Settings voices are honored while Piper stays a fallback.
///   3. **Piper** — neural cross-platform voice (bundled or custom `.onnx`).
///   4. **macOS `say` (fallback)** — when `prefer_macos_say` is false (or the first `say` attempt
///      was skipped), use `say` after Piper so the feature works when Piper is missing or broken.
///
/// Returns the **first** successful synthesis, or — if all attempts fail — a single concatenated
/// error containing every provider's failure reason (so we can surface it to the chat instead of
/// only to logs).
pub async fn synthesize_local_tts(
    text: &str,
    local_voice: &openfang_types::config::LocalVoiceConfig,
) -> Result<TtsResult, String> {
    if text.trim().is_empty() {
        return Err("Text cannot be empty".into());
    }
    let mut errors: Vec<String> = Vec::new();

    if local_voice.kokoro.assets_ready() {
        match synthesize_kokoro(text, &local_voice.kokoro).await {
            Ok(r) => return Ok(r),
            Err(e) => errors.push(format!("kokoro: {e}")),
        }
    }

    #[cfg(target_os = "macos")]
    {
        if local_voice.prefer_macos_say && local_voice.macos_say_binary_present() {
            let voice = local_voice.preferred_say_voice.as_deref();
            match synthesize_macos_say(text, voice).await {
                Ok(r) => return Ok(r),
                Err(e) => errors.push(format!("macos_say: {e}")),
            }
        }
    }

    if local_voice.piper_ready_with_active_voice() {
        match synthesize_piper_local(text, local_voice).await {
            Ok(r) => return Ok(r),
            Err(e) => errors.push(format!("piper: {e}")),
        }
    } else {
        errors.push("piper: not configured (missing piper_binary or piper_voice)".into());
    }

    #[cfg(target_os = "macos")]
    {
        // When `prefer_macos_say` is on, `/usr/bin/say` was already attempted above — do not repeat.
        if local_voice.macos_say_binary_present() && !local_voice.prefer_macos_say {
            let voice = local_voice.preferred_say_voice.as_deref();
            match synthesize_macos_say(text, voice).await {
                Ok(r) => return Ok(r),
                Err(e) => errors.push(format!("macos_say: {e}")),
            }
        }
    }

    Err(format!("local TTS unavailable — {}", errors.join("; ")))
}

/// Kokoro-82M synthesis — **scaffolding only**.
///
/// The runtime currently ships the auto-download path (`ensure_local_voice` will pull the
/// 310 MiB `kokoro-v1.0.onnx` and a default voice embedding into
/// `~/.armaraos/voice/kokoro/` when `[local_voice.kokoro] enabled = true`) but does **not**
/// yet ship an inference path: a real Rust implementation needs an ONNX runtime + an
/// espeak-ng phonemizer + a token vocab loader, which is a large dependency to land
/// reliably across macOS / Linux / Windows. Until that lands, this function returns a
/// clear error so the dashboard can surface "downloading…/coming soon" without producing
/// fake audio.
pub async fn synthesize_kokoro(
    _text: &str,
    cfg: &openfang_types::config::KokoroConfig,
) -> Result<TtsResult, String> {
    if !cfg.assets_ready() {
        return Err("kokoro assets not yet downloaded".into());
    }
    Err(
        "Kokoro inference is not yet wired in this build (assets are downloaded — falling back to Piper / say). \
         Track armaraos#kokoro-tts for the inference rollout."
            .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> TtsConfig {
        TtsConfig::default()
    }

    #[test]
    fn test_engine_creation() {
        let engine = TtsEngine::new(default_config());
        assert!(!engine.config.enabled);
    }

    #[test]
    fn test_config_defaults() {
        let config = TtsConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.max_text_length, 4096);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.openai.voice, "alloy");
        assert_eq!(config.openai.model, "tts-1");
        assert_eq!(config.openai.format, "mp3");
        assert_eq!(config.openai.speed, 1.0);
        assert_eq!(config.elevenlabs.voice_id, "21m00Tcm4TlvDq8ikWAM");
        assert_eq!(config.elevenlabs.model_id, "eleven_monolingual_v1");
    }

    #[tokio::test]
    async fn test_synthesize_disabled() {
        let engine = TtsEngine::new(default_config());
        let result = engine.synthesize("Hello", None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("disabled"));
    }

    #[tokio::test]
    async fn test_synthesize_empty_text() {
        let mut config = default_config();
        config.enabled = true;
        let engine = TtsEngine::new(config);
        let result = engine.synthesize("", None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[tokio::test]
    async fn test_synthesize_text_too_long() {
        let mut config = default_config();
        config.enabled = true;
        config.max_text_length = 10;
        let engine = TtsEngine::new(config);
        let result = engine
            .synthesize("This text is definitely longer than ten chars", None, None)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too long"));
    }

    #[test]
    fn test_detect_provider_none() {
        // In test env, likely no API keys set
        let _ = TtsEngine::detect_provider(); // Just verify no panic
    }

    #[tokio::test]
    async fn test_synthesize_no_provider() {
        let mut config = default_config();
        config.enabled = true;
        let engine = TtsEngine::new(config);
        // This may or may not error depending on env vars
        let result = engine.synthesize("Hello world", None, None).await;
        // If no API keys are set, should error
        if let Err(err) = result {
            assert!(err.contains("No TTS provider") || err.contains("not set"));
        }
    }

    #[test]
    fn test_max_audio_constant() {
        assert_eq!(MAX_AUDIO_RESPONSE_BYTES, 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_synthesize_piper_local_rejects_empty() {
        let c = openfang_types::config::LocalVoiceConfig::default();
        let e = super::synthesize_piper_local("  ", &c).await.unwrap_err();
        assert!(e.contains("empty") || e.contains("Empty"), "{e}");
    }

    /// Local Piper: without `[local_voice]` / piper on PATH, the fast path is "not ready".
    #[tokio::test]
    async fn test_synthesize_piper_local_not_configured() {
        let c = openfang_types::config::LocalVoiceConfig::default();
        let e = super::synthesize_piper_local("Hello, world.", &c)
            .await
            .unwrap_err();
        assert!(e.contains("Piper") || e.contains("not configured"), "{e}");
    }

    /// Cascading entrypoint rejects empty input before consulting any provider.
    #[tokio::test]
    async fn test_synthesize_local_tts_rejects_empty() {
        let c = openfang_types::config::LocalVoiceConfig::default();
        let e = super::synthesize_local_tts("   ", &c).await.unwrap_err();
        assert!(e.contains("empty") || e.contains("Empty"), "{e}");
    }

    /// On macOS, `synthesize_local_tts` must succeed via the built-in `/usr/bin/say` fallback
    /// even when no Piper is configured — this is the deterministic path that fixes the bug
    /// where the dashboard speaker toggle reported "ready" but the agent never sent audio
    /// (because the upstream Piper macOS bundle was missing dylibs).
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_synthesize_local_tts_falls_back_to_macos_say() {
        if !std::path::Path::new("/usr/bin/say").is_file() {
            return; // not on a real Mac (rare in CI)
        }
        let c = openfang_types::config::LocalVoiceConfig::default();
        let r = super::synthesize_local_tts("hello", &c)
            .await
            .expect("macOS say fallback should produce audio");
        assert_eq!(r.provider, "macos_say");
        assert!(r.audio_data.len() > 44, "expected real WAV bytes");
        assert_eq!(r.format, "wav");
    }

    /// Picking an unknown `say` voice must not panic. macOS `say` is permissive and
    /// silently falls back to the default voice for unknown ids on most versions, so
    /// the contract is "return *something* coherent" — either an Err with a useful
    /// message, or an Ok WAV synthesized using the default voice. Callers (the cascade
    /// in `synthesize_local_tts`) only need a deterministic, non-panicking result.
    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_macos_say_invalid_voice_does_not_panic() {
        if !std::path::Path::new("/usr/bin/say").is_file() {
            return;
        }
        let r = super::synthesize_macos_say("hello", Some("DefinitelyNotAnInstalledVoice42")).await;
        match r {
            Ok(out) => {
                assert_eq!(out.provider, "macos_say");
                assert!(
                    out.audio_data.len() > 44,
                    "expected wav bytes when say falls back"
                );
            }
            Err(msg) => {
                assert!(!msg.is_empty(), "error message should be informative");
            }
        }
    }

    /// Custom Piper voice override: when `custom_piper_voice` points at a stem under
    /// `custom_voices_dir`, `active_piper_voice()` returns that path; the synth refuses
    /// because the binary still doesn't exist (this exercises the resolution path only).
    #[tokio::test]
    async fn test_active_piper_voice_prefers_custom_when_present() {
        let tmp = std::env::temp_dir().join(format!("ainl_voice_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let onnx = tmp.join("custom_x.onnx");
        std::fs::write(&onnx, b"not really onnx but a file").unwrap();
        let c = openfang_types::config::LocalVoiceConfig {
            custom_voices_dir: Some(tmp.clone()),
            custom_piper_voice: Some("custom_x".into()),
            piper_voice: Some(tmp.join("default.onnx")),
            ..Default::default()
        };
        let active = c.active_piper_voice().expect("custom path should resolve");
        assert_eq!(active, onnx);
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// Kokoro stub: assets-not-ready yields a clear error and never produces fake audio.
    #[tokio::test]
    async fn test_synthesize_kokoro_scaffolding_errors_clearly() {
        let cfg = openfang_types::config::KokoroConfig::default();
        let e = super::synthesize_kokoro("hi", &cfg).await.unwrap_err();
        assert!(
            e.contains("not yet") || e.contains("assets"),
            "expected scaffolding-only error, got: {e}"
        );
    }
}
