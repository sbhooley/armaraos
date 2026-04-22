//! Optional **response-side** compression for assistant text (parity with Python `AINL_OUTPUT_COMPRESSION`).
//!
//! Input prompt compression stays in [`crate::prompt_compressor`]. This path runs **after** the model
//! produces final text, before it is stored in the session and returned to clients.

use crate::prompt_compressor::{compress_with_metrics, EfficientMode};

fn env_output_compression_enabled() -> bool {
    std::env::var("AINL_OUTPUT_COMPRESSION")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn output_compression_mode() -> EfficientMode {
    std::env::var("AINL_OUTPUT_COMPRESSION_MODE")
        .ok()
        .map(|s| EfficientMode::parse_natural_language(s.trim()))
        .unwrap_or(EfficientMode::Balanced)
}

/// When `AINL_OUTPUT_COMPRESSION` is truthy, compress `text` with `AINL_OUTPUT_COMPRESSION_MODE`
/// (default: balanced). Empty/whitespace-only text is returned unchanged.
#[must_use]
pub fn apply_if_env(text: String) -> String {
    if !env_output_compression_enabled() {
        return text;
    }
    if text.trim().is_empty() {
        return text;
    }
    let mode = output_compression_mode();
    if matches!(mode, EfficientMode::Off) {
        return text;
    }
    let (c, _) = compress_with_metrics(&text, mode, None);
    c.text
}
