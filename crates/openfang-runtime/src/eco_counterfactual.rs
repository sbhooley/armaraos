//! Counterfactual compression comparisons for adaptive eco receipts.

use crate::eco_mode_resolver::normalize_efficient_mode;
use crate::prompt_compressor::{compress_with_metrics, Compressed, EfficientMode};
use openfang_types::adaptive_eco::{AdaptiveEcoTurnSnapshot, EcoCounterfactualReceipt};

fn mode_label(m: EfficientMode) -> &'static str {
    match m {
        EfficientMode::Off => "off",
        EfficientMode::Balanced => "balanced",
        EfficientMode::Aggressive => "aggressive",
    }
}

/// Build optional receipt: compares applied compression vs off baseline, optional recommended/balanced.
#[must_use]
pub fn build_eco_counterfactual_receipt(
    user_message: &str,
    applied_mode: EfficientMode,
    applied: &Compressed,
    savings_pct: u8,
    adaptive: Option<&AdaptiveEcoTurnSnapshot>,
) -> Option<EcoCounterfactualReceipt> {
    if applied_mode == EfficientMode::Off && adaptive.is_none() {
        return None;
    }
    let orig = applied.original_tokens as u64;
    let app_tok = applied.compressed_tokens as u64;
    let vs_off = applied.tokens_saved() as u64;
    let mut receipt = EcoCounterfactualReceipt {
        applied_mode: mode_label(applied_mode).to_string(),
        original_tokens_est: orig,
        applied_compressed_tokens_est: app_tok,
        vs_off_tokens_saved: vs_off,
        vs_off_savings_pct: savings_pct,
        recommended_mode: None,
        recommended_compressed_tokens_est: None,
        tokens_saved_delta_recommended_minus_applied: None,
        balanced_compressed_tokens_est: None,
        aggressive_extra_tokens_saved_vs_balanced: None,
    };
    if let Some(snap) = adaptive {
        let eff = normalize_efficient_mode(snap.effective_mode.as_str());
        let recm = normalize_efficient_mode(snap.recommended_mode.as_str());
        if recm != eff {
            let rm = EfficientMode::parse_config(snap.recommended_mode.as_str());
            let (c_rec, _) = compress_with_metrics(user_message, rm, None);
            receipt.recommended_mode = Some(snap.recommended_mode.clone());
            receipt.recommended_compressed_tokens_est = Some(c_rec.compressed_tokens as u64);
            let delta = c_rec.tokens_saved() as i64 - applied.tokens_saved() as i64;
            receipt.tokens_saved_delta_recommended_minus_applied = Some(delta);
        }
        if eff == "aggressive" {
            let (c_bal, _) = compress_with_metrics(user_message, EfficientMode::Balanced, None);
            receipt.balanced_compressed_tokens_est = Some(c_bal.compressed_tokens as u64);
            receipt.aggressive_extra_tokens_saved_vs_balanced = Some(
                c_bal
                    .compressed_tokens
                    .saturating_sub(applied.compressed_tokens) as u64,
            );
        }
    } else if applied_mode == EfficientMode::Aggressive {
        let (c_bal, _) = compress_with_metrics(user_message, EfficientMode::Balanced, None);
        receipt.balanced_compressed_tokens_est = Some(c_bal.compressed_tokens as u64);
        receipt.aggressive_extra_tokens_saved_vs_balanced = Some(
            c_bal
                .compressed_tokens
                .saturating_sub(applied.compressed_tokens) as u64,
        );
    }
    Some(receipt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt_compressor::Compressed;
    use openfang_types::adaptive_eco::AdaptiveEcoTurnSnapshot;

    #[test]
    fn receipt_fields_match_balanced_mode_without_adaptive() {
        let msg = "hello world this is a moderately long user message for eco receipt testing";
        let applied = Compressed {
            text: msg.to_string(),
            original_tokens: 100,
            compressed_tokens: 72,
        };
        let r = build_eco_counterfactual_receipt(msg, EfficientMode::Balanced, &applied, 28, None)
            .expect("receipt");
        assert_eq!(r.applied_mode, "balanced");
        assert_eq!(r.original_tokens_est, 100);
        assert_eq!(r.applied_compressed_tokens_est, 72);
        assert_eq!(r.vs_off_tokens_saved, 28);
        assert_eq!(r.vs_off_savings_pct, 28);
        assert!(r.recommended_mode.is_none());
    }

    #[test]
    fn returns_none_when_off_and_no_adaptive_snapshot() {
        let msg = "x";
        let applied = Compressed {
            text: msg.to_string(),
            original_tokens: 10,
            compressed_tokens: 10,
        };
        assert!(
            build_eco_counterfactual_receipt(msg, EfficientMode::Off, &applied, 0, None).is_none()
        );
    }

    #[test]
    fn adaptive_snapshot_sets_recommended_fields_when_modes_differ() {
        let msg = "token-heavy prompt ".repeat(20);
        let applied = Compressed {
            text: msg.clone(),
            original_tokens: 200,
            compressed_tokens: 120,
        };
        let snap = AdaptiveEcoTurnSnapshot {
            effective_mode: "balanced".to_string(),
            recommended_mode: "aggressive".to_string(),
            base_mode_before_circuit: None,
            circuit_breaker_tripped: false,
            hysteresis_blocked: false,
            reason_codes: vec![],
            provider: "ollama".to_string(),
            model: "test-model".to_string(),
            cache_capability: "none".to_string(),
            input_price_per_million: None,
            shadow_only: true,
            enforce: false,
        };
        let r = build_eco_counterfactual_receipt(
            &msg,
            EfficientMode::Balanced,
            &applied,
            15,
            Some(&snap),
        )
        .expect("receipt");
        assert_eq!(r.recommended_mode.as_deref(), Some("aggressive"));
        assert!(r.recommended_compressed_tokens_est.is_some());
        assert!(r.tokens_saved_delta_recommended_minus_applied.is_some());
    }
}
