//! Heuristic compression primitives for AINL hosts.
//!
//! This crate currently exposes the ArmaraOS "Ultra Cost-Efficient Mode"
//! input compressor. It is intentionally embedding-free and dependency-light
//! so it can be reused across hosts without shipping local ML models.
//!
//! Companion modules (no I/O; host wiring stays out-of-crate):
//! - [`profiles`] — built-in **compression profiles** and project→profile hints
//! - [`adaptive`] — **content-shaped** `EfficientMode` recommendations
//! - [`cache`] — **TTL hysteresis** for cache-aware coordination
//!
//! Set `RUST_LOG=ainl_compression=debug` to enable full before/after text
//! logging per call (useful for tuning preserve lists and retention ratios).

use std::collections::HashSet;
use std::time::Instant;
use tracing::debug;

pub mod adaptive;
pub mod cache;
pub mod profiles;

pub use adaptive::{recommend_mode_for_content, AdaptiveRecommendation};
pub use cache::{cache_policy_summary, effective_ttl_with_hysteresis, CacheTtlResult};
pub use profiles::{
    list_builtin_profiles, resolve_builtin_profile, suggest_profile_id_for_project,
    CompressionProfile, BUILTIN_PROFILES,
};

/// Input compression aggressiveness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(
    feature = "graph-telemetry",
    derive(serde::Serialize, serde::Deserialize)
)]
pub enum EfficientMode {
    /// Pass through without modification.
    #[default]
    Off,
    /// ~55 % token retention — sweet-spot 50–60 % reduction. (default)
    Balanced,
    /// ~40 % token retention — opt-in for high-volume / cost-sensitive paths.
    Aggressive,
}

impl EfficientMode {
    /// Parse from a config string; unknown values → `Off`.
    pub fn parse_config(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "balanced" => Self::Balanced,
            "aggressive" => Self::Aggressive,
            _ => Self::Off,
        }
    }

    /// Parse from free-form natural language intent.
    ///
    /// Examples:
    /// - "use aggressive eco mode" -> `Aggressive`
    /// - "balanced mode please" -> `Balanced`
    /// - "disable compression" -> `Off`
    pub fn parse_natural_language(s: &str) -> Self {
        let lo = s.to_ascii_lowercase();
        let has = |needle: &str| lo.contains(needle);
        if has("disable compression")
            || has("no compression")
            || has("compression off")
            || has("eco off")
            || has("turn off eco")
            || has("off mode")
        {
            return Self::Off;
        }
        if has("aggressive")
            || has("max savings")
            || has("highest savings")
            || has("ultra eco")
            || has("eco aggressive")
        {
            return Self::Aggressive;
        }
        if has("balanced")
            || has("default eco")
            || has("eco balanced")
            || has("enable eco")
            || has("compression on")
        {
            return Self::Balanced;
        }
        Self::parse_config(&lo)
    }

    /// Token retention ratio.
    ///
    /// `Balanced` targets ~55 % retention (40–50 % reduction) — sweet-spot for most prompts.
    /// `Aggressive` targets ~35 % retention (55–70 % reduction) — meaningfully wider gap vs
    /// Balanced; soft-preserve terms become score-boosts rather than force-keeps, and
    /// trailing-explanation sentences get a score penalty to prune meta-commentary.
    fn retain(self) -> f32 {
        match self {
            Self::Balanced => 0.55,
            Self::Aggressive => 0.35,
            Self::Off => 1.0,
        }
    }
}

/// Structured telemetry emitted for each compression operation.
#[derive(Debug, Clone)]
#[cfg_attr(
    feature = "graph-telemetry",
    derive(serde::Serialize, serde::Deserialize)
)]
pub struct CompressionMetrics {
    pub mode: EfficientMode,
    pub original_chars: usize,
    pub compressed_chars: usize,
    pub original_tokens: usize,
    pub compressed_tokens: usize,
    pub tokens_saved: usize,
    /// Percentage saved, range 0.0..100.0.
    pub savings_ratio_pct: f32,
    /// Optional caller-provided semantic preservation score.
    pub semantic_preservation_score: Option<f32>,
    pub elapsed_ms: u64,
}

impl CompressionMetrics {
    #[must_use]
    pub fn from_result(
        mode: EfficientMode,
        original_text: &str,
        compressed: &Compressed,
        semantic_preservation_score: Option<f32>,
        elapsed_ms: u64,
    ) -> Self {
        let tokens_saved = compressed.tokens_saved();
        let savings_ratio_pct = if compressed.original_tokens == 0 {
            0.0
        } else {
            (tokens_saved as f32 * 100.0) / compressed.original_tokens as f32
        };
        Self {
            mode,
            original_chars: original_text.len(),
            compressed_chars: compressed.text.len(),
            original_tokens: compressed.original_tokens,
            compressed_tokens: compressed.compressed_tokens,
            tokens_saved,
            savings_ratio_pct,
            semantic_preservation_score,
            elapsed_ms,
        }
    }
}

/// Optional telemetry sink for compression metrics.
pub trait CompressionTelemetrySink: Send + Sync {
    fn emit(&self, metrics: CompressionMetrics);
}

/// Standalone input prompt compressor.
///
/// This is the intended public API for external agents that want to adopt
/// AINL eco-mode compression without pulling runtime-specific crates.
pub struct PromptCompressor {
    mode: EfficientMode,
    emit_telemetry: Option<Box<dyn Fn(CompressionMetrics) + Send + Sync>>,
}

impl PromptCompressor {
    #[must_use]
    pub fn new(mode: EfficientMode) -> Self {
        Self {
            mode,
            emit_telemetry: None,
        }
    }

    #[must_use]
    pub fn from_natural_language(mode_hint: &str) -> Self {
        Self::new(EfficientMode::parse_natural_language(mode_hint))
    }

    #[must_use]
    pub fn with_telemetry_callback(
        mode: EfficientMode,
        emit_telemetry: Option<Box<dyn Fn(CompressionMetrics) + Send + Sync>>,
    ) -> Self {
        Self {
            mode,
            emit_telemetry,
        }
    }

    pub fn compress(&self, text: &str) -> Compressed {
        self.compress_with_semantic_score(text, None)
    }

    pub fn compress_with_semantic_score(
        &self,
        text: &str,
        semantic_preservation_score: Option<f32>,
    ) -> Compressed {
        let t0 = Instant::now();
        let result = compress(text, self.mode);
        if let Some(cb) = &self.emit_telemetry {
            cb(CompressionMetrics::from_result(
                self.mode,
                text,
                &result,
                semantic_preservation_score,
                t0.elapsed().as_millis() as u64,
            ));
        }
        result
    }
}

/// Result of a compression pass.
pub struct Compressed {
    /// Compressed (or original, on no-op) text.
    pub text: String,
    /// Estimated original token count (chars/4).
    pub original_tokens: usize,
    /// Estimated compressed token count.
    pub compressed_tokens: usize,
}

impl Compressed {
    /// Tokens saved; 0 when compression was a no-op.
    pub fn tokens_saved(&self) -> usize {
        self.original_tokens.saturating_sub(self.compressed_tokens)
    }
}

fn tok(s: &str) -> usize {
    s.len() / 4 + 1
}

const FILLERS: &[&str] = &[
    "I think ",
    "I believe ",
    "Basically, ",
    "Essentially, ",
    "Of course, ",
    "Please note that ",
    "It is worth noting that ",
    "It's worth noting that ",
    "I would like to ",
    "I'd like to ",
    "Don't hesitate to ",
    "Feel free to ",
    "As you know, ",
    "As mentioned earlier, ",
    "That being said, ",
    "To be honest, ",
    "Needless to say, ",
    // Mid-sentence hedging words (always safe to strip)
    " basically ",
    " essentially ",
    " simply ",
    " just ",
    " very ",
    " really ",
];

/// Hard-preserve: force-keep in **both** Balanced and Aggressive.
/// Irreplaceable content — actual opcodes, URLs, diagnostic history, user-intent markers.
const HARD_PRESERVE: &[&str] = &[
    "exact",
    "steps",
    "already tried",
    "already restarted",
    "already checked",
    "restart",
    "daemon",
    "error",
    "http://",
    "https://",
    "R http",
    "R web",
    "L_",
    "->",
    "::",
    ".ainl",
    "opcode",
    "R queue",
    "R llm",
    "R core",
    "R solana",
    "R postgres",
    "R redis",
    "```",
];

/// Soft-preserve: force-keep in Balanced; **score-boost only** in Aggressive.
/// These identifiers/units are important but the LLM can reconstruct context without them
/// when the budget is tight.  Freeing them lets Aggressive prune changelog-dense text
/// where these terms would otherwise lock in nearly every sentence.
const SOFT_PRESERVE: &[&str] = &[
    "##", " ms", " kb", " mb", " gb", " %", "openfang", "armaraos", "manifest",
];

fn hard_keep(s: &str) -> bool {
    let lo = s.to_lowercase();
    HARD_PRESERVE.iter().any(|p| lo.contains(&p.to_lowercase()))
}

fn soft_match(s: &str) -> bool {
    let lo = s.to_lowercase();
    SOFT_PRESERVE.iter().any(|p| lo.contains(&p.to_lowercase()))
}

/// Returns `true` when `s` must be included regardless of budget.
fn must_keep(s: &str, mode: EfficientMode) -> bool {
    hard_keep(s) || (mode != EfficientMode::Aggressive && soft_match(s))
}

/// Compress `text` toward `mode.retain()` of its original token budget.
///
/// Prompts shorter than 80 tokens, or `Off` mode, pass through unchanged.
/// Code fences (` ``` `) are extracted and re-inserted verbatim.
pub fn compress(text: &str, mode: EfficientMode) -> Compressed {
    let orig = tok(text);
    if mode == EfficientMode::Off || orig < 80 {
        return Compressed {
            text: text.to_string(),
            original_tokens: orig,
            compressed_tokens: orig,
        };
    }
    // Floor: never go below 25 % of original (prevents total context loss on short messages),
    // but keep it relative so both modes stay distinct on moderate-length inputs.
    // The old fixed `.max(80)` floor was equalising Balanced and Aggressive on ~100–200 token
    // messages because both natural budgets fell below 80, producing identical outputs.
    let budget = ((orig as f32 * mode.retain()) as usize).max(orig / 4);

    // Split at code fences; preserve code blocks verbatim.
    let mut blocks: Vec<(bool, String)> = Vec::new();
    let mut rest = text;
    while let Some(f) = rest.find("```") {
        if f > 0 {
            blocks.push((false, rest[..f].to_string()));
        }
        rest = &rest[f + 3..];
        if let Some(e) = rest.find("```") {
            blocks.push((true, format!("```{}```", &rest[..e])));
            rest = &rest[e + 3..];
        } else {
            blocks.push((true, format!("```{rest}")));
            rest = "";
            break;
        }
    }
    if !rest.is_empty() {
        blocks.push((false, rest.to_string()));
    }

    let code_tok: usize = blocks.iter().filter(|(c, _)| *c).map(|(_, t)| tok(t)).sum();
    let mut prose_budget = budget.saturating_sub(code_tok);
    let mut out: Vec<String> = Vec::new();

    for (is_code, block) in &blocks {
        if *is_code {
            out.push(block.clone());
            continue;
        }
        let prose = compress_prose(block, prose_budget, mode);
        prose_budget = prose_budget.saturating_sub(tok(&prose));
        out.push(prose);
    }

    let result = out.join("\n\n").trim().to_string();
    let c = tok(&result);
    // Safety: never return longer than original.
    if c >= orig {
        debug!(orig_tok = orig, "prompt_compressor: no gain — passthrough");
        Compressed {
            text: text.to_string(),
            original_tokens: orig,
            compressed_tokens: orig,
        }
    } else {
        debug!(
            orig_tok = orig,
            compressed_tok = c,
            savings_pct = 100u64.saturating_sub((c as u64 * 100) / orig.max(1) as u64),
            original_text = %text,
            compressed_text = %result,
            "prompt_compressor: compressed"
        );
        Compressed {
            text: result,
            original_tokens: orig,
            compressed_tokens: c,
        }
    }
}

/// Compress and return structured telemetry metrics in one call.
pub fn compress_with_metrics(
    text: &str,
    mode: EfficientMode,
    semantic_preservation_score: Option<f32>,
) -> (Compressed, CompressionMetrics) {
    let t0 = Instant::now();
    let result = compress(text, mode);
    let semantic_preservation_score = semantic_preservation_score
        .or_else(|| Some(estimate_semantic_preservation_score(text, &result.text)));
    let metrics = CompressionMetrics::from_result(
        mode,
        text,
        &result,
        semantic_preservation_score,
        t0.elapsed().as_millis() as u64,
    );
    (result, metrics)
}

/// Lightweight lexical semantic-preservation heuristic in range 0.0..1.0.
#[must_use]
pub fn estimate_semantic_preservation_score(original: &str, compressed: &str) -> f32 {
    fn terms(s: &str) -> std::collections::HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| t.len() >= 4)
            .collect()
    }
    let a = terms(original);
    if a.is_empty() {
        return 1.0;
    }
    let b = terms(compressed);
    let overlap = a.iter().filter(|t| b.contains(*t)).count();
    (overlap as f32 / a.len() as f32).clamp(0.0, 1.0)
}

fn compress_prose(text: &str, budget: usize, mode: EfficientMode) -> String {
    let sents: Vec<&str> = text
        .split(". ")
        .flat_map(|l| l.split('\n'))
        .filter(|s| !s.trim().is_empty())
        .collect();
    if sents.len() <= 2 {
        return text.to_string();
    }

    // Intent vocabulary from the first two sentences (position-biased TF-IDF proxy).
    let intent: HashSet<&str> = sents
        .iter()
        .take(2)
        .flat_map(|s| s.split_whitespace())
        .filter(|w| w.len() > 3)
        .collect();
    let n = sents.len();

    let mut scored: Vec<(usize, f32)> = sents
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            if must_keep(s, mode) {
                return (i, f32::MAX);
            }
            let words: Vec<&str> = s.split_whitespace().collect();
            let wc = words.len().max(1) as f32;
            let overlap = words.iter().filter(|w| intent.contains(*w)).count() as f32;
            let pos = if i == 0 {
                2.5
            } else if i < n / 4 {
                1.5
            } else if i > n * 4 / 5 {
                1.2
            } else {
                1.0
            };
            let ent = if words
                .iter()
                .any(|w| w.parse::<f64>().is_ok() || w.starts_with("http"))
            {
                1.4
            } else {
                1.0
            };
            // Aggressive-only modifiers: boost soft-preserve sentences; penalise trailing-explanation
            // clauses that typically start with "This ", "These ", "It " or "Which " and carry
            // low new information (they rephrase or justify what came before).
            let (soft_boost, trailing_pen) = if mode == EfficientMode::Aggressive {
                let boost = if soft_match(s) { 1.3 } else { 1.0 };
                let t = s.trim();
                let pen = if t.starts_with("This ")
                    || t.starts_with("These ")
                    || t.starts_with("It ")
                    || t.starts_with("Which ")
                {
                    0.65
                } else {
                    1.0
                };
                (boost, pen)
            } else {
                (1.0, 1.0)
            };
            (
                i,
                (overlap / wc + 0.2) * pos * ent * soft_boost * trailing_pen,
            )
        })
        .collect();

    scored.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: HashSet<usize> = HashSet::new();
    let mut used = 0usize;
    for &(idx, score) in &scored {
        let s = sents[idx];
        if score == f32::MAX || used + tok(s) <= budget {
            kept.insert(idx);
            used += tok(s);
        }
        if used >= budget && score != f32::MAX {
            break;
        }
    }

    let mut joined: String = (0..n)
        .filter(|i| kept.contains(i))
        .map(|i| sents[i].trim())
        .collect::<Vec<_>>()
        .join(". ");
    // Replace fillers with a single space so adjacent words don't fuse,
    // then collapse any resulting double-spaces.
    for filler in FILLERS {
        joined = joined.replace(filler, " ");
    }
    // Collapse "word  word" → "word word" after filler removal.
    while joined.contains("  ") {
        joined = joined.replace("  ", " ");
    }
    // Re-capitalize the first character in case filler stripping left a lowercase fragment.
    let joined = joined.trim();
    let mut chars = joined.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_prompt_passthrough() {
        let short = "Hello, please help me.";
        let r = compress(short, EfficientMode::Balanced);
        assert_eq!(r.text, short);
        assert_eq!(r.tokens_saved(), 0);
    }

    #[test]
    fn code_block_preserved_verbatim() {
        let msg = "Fix my code:\n```rust\nfn add(a: i32, b: i32) -> i32 { a + b }\n```\nIt panics.";
        let r = compress(msg, EfficientMode::Balanced);
        assert!(
            r.text.contains("fn add(a: i32"),
            "code block must survive compression"
        );
    }

    #[test]
    fn off_mode_is_identity() {
        let text = "word ".repeat(100);
        let r = compress(&text, EfficientMode::Off);
        assert_eq!(r.text, text);
        assert_eq!(r.tokens_saved(), 0);
    }

    #[test]
    fn balanced_reduces_long_prose() {
        let msg =
            "I am working on a React component and experiencing a problem with state management. \
            The component re-renders multiple times when it should only render once. \
            I have tried using useMemo but it does not seem to work as expected. \
            Basically the error says too many re-renders and I believe the issue might be related \
            to the useEffect dependency array. \
            I think I need help understanding what is going wrong and how to resolve the problem. \
            I would like to know if there is a standard approach for fixing infinite render loops. \
            Please provide a clear explanation and I'd like step-by-step guidance if possible.";
        let r = compress(msg, EfficientMode::Balanced);
        let ratio = r.compressed_tokens as f32 / r.original_tokens as f32;
        assert!(
            ratio < 0.85,
            "expected >15 % compression on long prose, got {ratio:.2}"
        );
        assert!(r.text.contains("React"), "intent keywords must survive");
    }

    #[test]
    fn parse_config_roundtrip() {
        assert_eq!(
            EfficientMode::parse_config("balanced"),
            EfficientMode::Balanced
        );
        assert_eq!(
            EfficientMode::parse_config("AGGRESSIVE"),
            EfficientMode::Aggressive
        );
        assert_eq!(EfficientMode::parse_config("off"), EfficientMode::Off);
        assert_eq!(EfficientMode::parse_config("unknown"), EfficientMode::Off);
    }

    #[test]
    fn parse_natural_language_roundtrip() {
        assert_eq!(
            EfficientMode::parse_natural_language("use aggressive eco mode for max savings"),
            EfficientMode::Aggressive
        );
        assert_eq!(
            EfficientMode::parse_natural_language("balanced mode please"),
            EfficientMode::Balanced
        );
        assert_eq!(
            EfficientMode::parse_natural_language("disable compression for this turn"),
            EfficientMode::Off
        );
    }

    #[test]
    fn telemetry_callback_emits_metrics() {
        use std::sync::{Arc, Mutex};
        let captured: Arc<Mutex<Vec<CompressionMetrics>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        let compressor = PromptCompressor::with_telemetry_callback(
            EfficientMode::Balanced,
            Some(Box::new(move |m| {
                sink.lock().expect("lock").push(m);
            })),
        );
        let _ = compressor.compress_with_semantic_score(
            "I think I would like to understand basically why the dashboard is showing a red error badge. \
            Please note that I already restarted the daemon and still see the issue.",
            Some(0.91),
        );
        let rows = captured.lock().expect("lock");
        assert_eq!(rows.len(), 1);
        let m = &rows[0];
        assert_eq!(m.mode, EfficientMode::Balanced);
        assert!(m.original_tokens >= m.compressed_tokens);
        assert!(m.savings_ratio_pct >= 0.0);
        assert_eq!(m.semantic_preservation_score, Some(0.91));
    }

    #[test]
    fn semantic_preservation_score_reasonable_range() {
        let original = "Please restart the daemon and check the red error badge in dashboard logs";
        let compressed = "Restart daemon; check red error badge in dashboard logs";
        let score = estimate_semantic_preservation_score(original, compressed);
        assert!((0.0..=1.0).contains(&score));
        assert!(score > 0.5, "expected high overlap score, got {score}");
    }

    #[test]
    fn smoke_complex_ainl_workflow_question() {
        let input = "\
            I am really trying to understand basically why my AINL workflow is failing at the R http.GET step. \
            I think the issue might be related to the timeout setting or the URL format that I am passing to the adapter. \
            Essentially, the workflow looks like this: I start with L_start, then I call R http.GET https://api.example.com/data?key=abc&region=us-east-1 ->result, \
            and after that I do R core.GET result body ->body. \
            I have already tried increasing the timeout to 30 seconds by passing a third positional argument, but it does not seem to help. \
            To be honest, I am not really sure whether the problem is the URL query string encoding, \
            or whether the -> result binding is somehow not resolving the value correctly in the next step. \
            Please note that I have already checked the adapter docs and the http adapter section of AGENTS.md. \
            I would really appreciate a step-by-step explanation of what might be going wrong and what exact steps I should take to debug this. \
            It would also be helpful if you could show me the correct opcode syntax for a GET request with headers and timeout.";
        let r = compress(input, EfficientMode::Balanced);
        let savings =
            100usize.saturating_sub((r.compressed_tokens * 100) / r.original_tokens.max(1));
        assert!(
            r.text.contains("R http.GET") || r.text.contains("http.GET"),
            "http.GET must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("https://") || r.text.contains("api.example.com"),
            "URL must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("->"),
            "-> binding must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("steps") || r.text.contains("step"),
            "steps/step must survive: got: {}",
            r.text
        );
        assert!(
            savings >= 10,
            "expected ≥10 % savings on complex AINL question ({}→{} tok), got {}%: [{}]",
            r.original_tokens,
            r.compressed_tokens,
            savings,
            r.text
        );
    }

    #[test]
    fn aggressive_vs_balanced_gap() {
        let everyday =
            "I am working on a React component and experiencing a problem with state management. \
            The component re-renders multiple times when it should only render once. \
            I have tried using useMemo but it does not seem to work as expected. \
            Basically the error says too many re-renders and I believe the issue might be related \
            to the useEffect dependency array. \
            I think I need help understanding what is going wrong and how to resolve the problem. \
            I would like to know if there is a standard approach for fixing infinite render loops. \
            Please provide a clear explanation and I'd like step-by-step guidance if possible.";
        let bal = compress(everyday, EfficientMode::Balanced);
        let agg = compress(everyday, EfficientMode::Aggressive);
        let bal_pct =
            100usize.saturating_sub((bal.compressed_tokens * 100) / bal.original_tokens.max(1));
        let agg_pct =
            100usize.saturating_sub((agg.compressed_tokens * 100) / agg.original_tokens.max(1));

        let changelog = "The ArmaraOS kernel now injects efficient_mode into each scheduled run. \
            This makes the list self-documenting and more robust for real dashboard status messages. \
            The openfang runtime resolves the manifest field at startup. \
            It is worth noting that the latency is under 30 ms for most prompts. \
            These changes improve the armaraos agent scheduling pipeline significantly. \
            Which means users can expect 20 % fewer API calls on high-volume deployments. \
            The openfang kernel also now exposes a new manifest key for efficient_mode override. \
            This ensures per-agent configuration always wins over the global config value.";
        let bal_cl = compress(changelog, EfficientMode::Balanced);
        let agg_cl = compress(changelog, EfficientMode::Aggressive);
        let bal_cl_pct = 100usize
            .saturating_sub((bal_cl.compressed_tokens * 100) / bal_cl.original_tokens.max(1));
        let agg_cl_pct = 100usize
            .saturating_sub((agg_cl.compressed_tokens * 100) / agg_cl.original_tokens.max(1));

        assert!(
            agg_pct > bal_pct + 10,
            "Aggressive should beat Balanced by >10% on everyday prose; Bal={}% Agg={}%",
            bal_pct,
            agg_pct
        );
        assert!(
            agg_cl_pct > bal_cl_pct + 8,
            "Aggressive should beat Balanced by >8% on soft-identifier changelog; Bal={}% Agg={}%",
            bal_cl_pct,
            agg_cl_pct
        );
    }

    #[test]
    fn preserve_marker_forces_keep() {
        let msg = "I want help. Please do not drop the exact steps required for this. ".repeat(20);
        let r = compress(&msg, EfficientMode::Aggressive);
        assert!(
            r.text.contains("exact steps"),
            "preserve marker must survive aggressive mode"
        );
    }

    #[test]
    fn readme_dashboard_example_ratio() {
        let input = "I think I would like to understand basically why the dashboard is showing me \
            a red error badge on the agents page. Essentially, it seems like the agent is not \
            responding and I am not sure what steps I should take to investigate this issue. \
            Please note that I have already tried restarting the daemon. To be honest, I am not \
            really sure where to look next.";
        let r = compress(input, EfficientMode::Balanced);
        let savings =
            100usize.saturating_sub((r.compressed_tokens * 100) / r.original_tokens.max(1));
        assert!(
            r.text.contains("red error badge") || r.text.contains("error badge"),
            "error badge context must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("daemon"),
            "daemon restart context must survive"
        );
        assert!(
            savings >= 30,
            "expected ≥30 % savings on verbose dashboard question, got {}%: [{}]",
            savings,
            r.text
        );
    }

    #[test]
    fn http_adapter_prompt_preserves_technical_terms() {
        let input =
            "Can you help me understand why the R http.GET call is failing with a timeout? \
            I am using the URL https://example.com/api?key=abc and getting a connection error. \
            The adapter seems to not be working and I am not sure if it is the timeout setting \
            or the URL format that is causing issues with the -> result binding.";
        let r = compress(input, EfficientMode::Balanced);
        assert!(
            r.text.contains("R http.GET") || r.text.contains("http.GET"),
            "R http.GET must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("https://") || r.text.contains("http"),
            "URL must survive: got: {}",
            r.text
        );
        assert!(
            r.text.contains("->"),
            "-> binding must survive: got: {}",
            r.text
        );
    }

    #[test]
    fn benchmark_mode_savings_corpus() {
        let corpus = vec![
            (
                "dashboard-verbose",
                "I think I would like to understand basically why the dashboard is showing me \
                a red error badge on the agents page. Essentially, it seems like the agent is not \
                responding and I am not sure what steps I should take to investigate this issue. \
                Please note that I have already tried restarting the daemon. To be honest, I am not \
                really sure where to look next.",
            ),
            (
                "ainl-http-technical",
                "I am really trying to understand basically why my AINL workflow is failing at the R http.GET step. \
                I think the issue might be related to the timeout setting or the URL format that I am passing to the adapter. \
                Essentially, the workflow looks like this: I start with L_start, then I call R http.GET https://api.example.com/data?key=abc&region=us-east-1 ->result, \
                and after that I do R core.GET result body ->body. \
                I have already tried increasing the timeout to 30 seconds by passing a third positional argument, but it does not seem to help. \
                To be honest, I am not really sure whether the problem is the URL query string encoding, \
                or whether the -> result binding is somehow not resolving the value correctly in the next step.",
            ),
            (
                "everyday-prose",
                "I am working on a React component and experiencing a problem with state management. \
                The component re-renders multiple times when it should only render once. \
                I have tried using useMemo but it does not seem to work as expected. \
                Basically the error says too many re-renders and I believe the issue might be related \
                to the useEffect dependency array. \
                I think I need help understanding what is going wrong and how to resolve the problem. \
                I would like to know if there is a standard approach for fixing infinite render loops. \
                Please provide a clear explanation and I'd like step-by-step guidance if possible.",
            ),
            (
                "changelog-soft-identifiers",
                "The ArmaraOS kernel now injects efficient_mode into each scheduled run. \
                This makes the list self-documenting and more robust for real dashboard status messages. \
                The openfang runtime resolves the manifest field at startup. \
                It is worth noting that the latency is under 30 ms for most prompts. \
                These changes improve the armaraos agent scheduling pipeline significantly. \
                Which means users can expect 20 % fewer API calls on high-volume deployments. \
                The openfang kernel also now exposes a new manifest key for efficient_mode override. \
                This ensures per-agent configuration always wins over the global config value.",
            ),
        ];

        let mut balanced_pcts: Vec<u64> = Vec::new();
        let mut aggressive_pcts: Vec<u64> = Vec::new();

        for (name, input) in corpus {
            let off = compress(input, EfficientMode::Off);
            let bal = compress(input, EfficientMode::Balanced);
            let agg = compress(input, EfficientMode::Aggressive);

            let bal_pct = 100u64.saturating_sub(
                (bal.compressed_tokens as u64 * 100) / bal.original_tokens.max(1) as u64,
            );
            let agg_pct = 100u64.saturating_sub(
                (agg.compressed_tokens as u64 * 100) / agg.original_tokens.max(1) as u64,
            );

            balanced_pcts.push(bal_pct);
            aggressive_pcts.push(agg_pct);

            eprintln!(
                "[bench] {name}: off={}tok, balanced={}tok (↓{}%), aggressive={}tok (↓{}%), delta=+{}%",
                off.compressed_tokens,
                bal.compressed_tokens,
                bal_pct,
                agg.compressed_tokens,
                agg_pct,
                agg_pct.saturating_sub(bal_pct)
            );
        }

        balanced_pcts.sort_unstable();
        aggressive_pcts.sort_unstable();
        let mid = balanced_pcts.len() / 2;
        let bal_median = balanced_pcts[mid];
        let agg_median = aggressive_pcts[mid];
        let bal_mean = balanced_pcts.iter().sum::<u64>() as f64 / balanced_pcts.len() as f64;
        let agg_mean = aggressive_pcts.iter().sum::<u64>() as f64 / aggressive_pcts.len() as f64;

        eprintln!(
            "[bench-summary] balanced median={}%, mean={:.1}% | aggressive median={}%, mean={:.1}% | delta median=+{}%",
            bal_median,
            bal_mean,
            agg_median,
            agg_mean,
            agg_median.saturating_sub(bal_median)
        );

        assert!(
            agg_median >= bal_median,
            "aggressive should not underperform balanced median"
        );
    }
}
