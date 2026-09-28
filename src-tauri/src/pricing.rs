//! Lightweight token → cost pricing module (inspired by tokscale's `pricing/`).
//!
//! Provides an in-repo price table for common models (USD per 1M tokens,
//! matching the LiteLLM convention tokscale uses), a model-alias normalizer,
//! and `compute_cost` which turns a [`crate::providers::TokenBreakdown`] into an
//! estimated USD cost.
//!
//! Design notes (mirroring tokscale's safeguards):
//! - Prices are keyed by a *prefix* so a model runs e.g. `"claude-3-5-sonnet"` match.
//! - Bare brand tokens (`claude`, `gemini`) and generic words are NOT matched —
//!   they carry no model identity and would mis-price.
//! - Unknown models return `None` (unpriced) rather than guessing, so callers
//!   can fall back to "cost unknown" instead of a wrong number.
//! - Subscriptions / token plans are intentionally absent: they bill a fixed
//!   quota, not per token, so pay-per-token estimation must not apply.
//!
//! Prices here are reasonable public list rates (USD / 1M tokens) and should be
//! treated as an approximation (`cost_source = Estimated`); they are not
//! guaranteed current. A full multi-source lookup (LiteLLM / OpenRouter) is a
//! future extension.
//!
//! ## Status: estimation disabled
//!
//! Because the table has to be hand-updated against vendors who constantly ship
//! new models and reprice old ones, [`COST_ESTIMATION_ENABLED`] is `false` and
//! [`compute_cost`] always returns `None`. The table and its logic are kept
//! intact so it can be revived later; only the estimates are suppressed.

/// A single pricing row: whether the model's input or output costs.
#[derive(Debug, Clone, Copy)]
struct Tier {
    input_per_1m: f64,
    output_per_1m: f64,
}

/// Master switch for price-based cost estimation.
///
/// Estimation is **off** by default. The price table has to be hand-maintained
/// against vendors who ship new models and change rates constantly, so an
/// estimate drifts out of date silently and a stale number is worse than no
/// number — it looks authoritative while being wrong.
///
/// This gates *estimation only*. Costs a tool or API reports about itself
/// (Hermes / Cherry Studio / MiniMax logs, provider balance endpoints) are real
/// billing data that does not depend on this table, and remain available.
///
/// Re-enabling means flipping this to `true` and refreshing `PRICE_TABLE`.
pub const COST_ESTIMATION_ENABLED: bool = false;

/// Cache-read tokens bill at a fraction of the input rate.
///
/// 0.1x is the prevailing public-list convention (Anthropic, OpenAI and
/// DeepSeek all sell cache reads around a tenth of input). It matters a lot in
/// practice: measured locally, cache reads are 80-98% of all tokens for dsh and
/// 93% for Claude Code, so billing them at full input rate inflates the total
/// by roughly 86%.
///
/// We keep this as one named constant rather than a per-model column because a
/// per-model table would need real published rates for every row in
/// `PRICE_TABLE`, and a plausible-looking but invented number is worse than an
/// honestly uniform approximation on an already-estimated figure.
const CACHE_READ_DISCOUNT: f64 = 0.1;

/// (model-prefix, tier). Ordered longest-prefix-ish; the first prefix a model
/// starts with wins. Keep affordable defaults; adjust as models change.
const PRICE_TABLE: &[(&str, Tier)] = &[
    // Claude
    (
        "claude-3-7-sonnet",
        Tier {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
    ),
    (
        "claude-3-5-sonnet",
        Tier {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
    ),
    (
        "claude-3-5-haiku",
        Tier {
            input_per_1m: 0.8,
            output_per_1m: 4.0,
        },
    ),
    (
        "claude-3-opus",
        Tier {
            input_per_1m: 15.0,
            output_per_1m: 75.0,
        },
    ),
    (
        "claude-sonnet",
        Tier {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
    ),
    (
        "claude-haiku",
        Tier {
            input_per_1m: 0.8,
            output_per_1m: 4.0,
        },
    ),
    // OpenAI
    (
        "gpt-5",
        Tier {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
    ),
    (
        "gpt-4o",
        Tier {
            input_per_1m: 2.5,
            output_per_1m: 10.0,
        },
    ),
    (
        "gpt-4.1",
        Tier {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
        },
    ),
    (
        "gpt-4",
        Tier {
            input_per_1m: 30.0,
            output_per_1m: 60.0,
        },
    ),
    (
        "o3",
        Tier {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
        },
    ),
    (
        "o1",
        Tier {
            input_per_1m: 15.0,
            output_per_1m: 60.0,
        },
    ),
    // DeepSeek
    (
        "deepseek-reasoner",
        Tier {
            input_per_1m: 0.55,
            output_per_1m: 2.19,
        },
    ),
    (
        "deepseek-chat",
        Tier {
            input_per_1m: 0.27,
            output_per_1m: 1.10,
        },
    ),
    (
        "deepseek",
        Tier {
            input_per_1m: 0.27,
            output_per_1m: 1.10,
        },
    ),
    // MiniMax
    (
        "minimax-m3",
        Tier {
            input_per_1m: 5.0,
            output_per_1m: 20.0,
        },
    ),
    (
        "minimax",
        Tier {
            input_per_1m: 2.0,
            output_per_1m: 8.0,
        },
    ),
    // Qwen
    (
        "qwen3",
        Tier {
            input_per_1m: 0.6,
            output_per_1m: 2.4,
        },
    ),
    (
        "qwen",
        Tier {
            input_per_1m: 0.5,
            output_per_1m: 2.0,
        },
    ),
    // Alibaba / Doubao
    (
        "doubao",
        Tier {
            input_per_1m: 0.3,
            output_per_1m: 1.2,
        },
    ),
    // xAI
    (
        "grok-4",
        Tier {
            input_per_1m: 2.0,
            output_per_1m: 12.0,
        },
    ),
    (
        "grok-3",
        Tier {
            input_per_1m: 3.0,
            output_per_1m: 15.0,
        },
    ),
    // MiniMax MiMo (Xiaomi)
    (
        "mimo-v2.6-pro",
        Tier {
            input_per_1m: 0.43,
            output_per_1m: 0.86,
        },
    ),
    (
        "mimo-v2.6-flash",
        Tier {
            input_per_1m: 0.14,
            output_per_1m: 0.29,
        },
    ),
    (
        "mimo",
        Tier {
            input_per_1m: 0.3,
            output_per_1m: 0.6,
        },
    ),
    // Gemini
    (
        "gemini-2.5-pro",
        Tier {
            input_per_1m: 1.25,
            output_per_1m: 10.0,
        },
    ),
    (
        "gemini-2.5-flash",
        Tier {
            input_per_1m: 0.30,
            output_per_1m: 2.50,
        },
    ),
    (
        "gemini",
        Tier {
            input_per_1m: 0.5,
            output_per_1m: 1.5,
        },
    ),
];

/// Model aliases: normalize vendor-specific spellings to a shared prefix that
/// appears in [`PRICE_TABLE`]. These run *before* prefix matching.
const MODEL_ALIASES: &[(&str, &str)] = &[
    // Anthropic aliases used by proxies / routers
    ("anthropic/claude", "claude-"),
    ("anthropic/claude-3", "claude-3"),
    // OpenAI router-style prefixes
    ("openai/gpt-4", "gpt-4"),
    ("openai/gpt-4o", "gpt-4o"),
    ("openai/gpt-5", "gpt-5"),
    // DeepSeek openrouter style
    ("deepseek/deepseek-chat", "deepseek-chat"),
    ("deepseek/deepseek-reasoner", "deepseek-reasoner"),
    // Moonshot / Kimi (commonly mislabeled)
    ("moonshotai/kimi", "kimi"),
    ("moonshot/kimi", "kimi"),
];

/// Strip a decision-affecting suffix that carries no price info (reasoning
/// tier, provider namespace tail, etc.). Kept deliberately conservative.
fn strip_noise(model: &str) -> &str {
    // CLIProxy-style `(level)` reasoning tier, e.g. `gpt-5(high)`.
    if let Some(i) = model.find('(') {
        return &model[..i];
    }
    model
}

/// Normalize the model id through the alias table, lowercasing for matching.
fn normalize(model: &str) -> String {
    let stripped = strip_noise(model);
    let lower = stripped.trim().to_ascii_lowercase();
    for (alias, canonical) in MODEL_ALIASES {
        if lower == *alias || lower.starts_with(alias) {
            return canonical.to_string();
        }
    }
    lower
}

/// Look up the price tier for a model, or `None` if unknown/unmatchable.
///
/// Guards:
/// - Generic / bare tokens never match (avoids mis-pricing on an eroding id).
fn tier_for(model: &str) -> Option<Tier> {
    let normalized = normalize(model);
    // Refuse bare brand words and generic tokens — no model identity.
    let is_generic = matches!(
        normalized.as_str(),
        "claude"
            | "gpt"
            | "gemini"
            | "deepseek"
            | "minimax"
            | "qwen"
            | "mimo"
            | "grok"
            | "kimi"
            | "model"
            | "default"
            | "router"
    );
    if normalized.is_empty() || is_generic {
        return None;
    }
    PRICE_TABLE
        .iter()
        .find(|(prefix, _)| normalized.starts_with(prefix))
        .map(|(_, tier)| *tier)
}

/// Estimate the USD cost of a token breakdown. Returns `None` when the model is
/// unknown/unpriced.
///
/// `cache_read` is billed at [`CACHE_READ_DISCOUNT`] × the input rate. Callers
/// must pass the cache portion in `cache_read` rather than pre-merging it into
/// `input`, otherwise it is charged at full price and the discount silently
/// never applies.
pub fn compute_cost(breakdown: &crate::providers::TokenBreakdown) -> Option<f64> {
    // Estimation disabled: report "unpriced" so every caller's existing
    // `if cost > 0.0` guard leaves the model without a fabricated figure.
    if !COST_ESTIMATION_ENABLED {
        return None;
    }
    let model = breakdown.model_id.as_deref()?;
    let tier = tier_for(model)?;
    let input_cost = breakdown.input / 1_000_000.0 * tier.input_per_1m;
    let cache_cost = breakdown.cache_read / 1_000_000.0 * tier.input_per_1m * CACHE_READ_DISCOUNT;
    let output_cost = breakdown.output / 1_000_000.0 * tier.output_per_1m;
    Some(input_cost + cache_cost + output_cost)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::TokenBreakdown;

    fn bd(model: &str, input: f64, output: f64) -> TokenBreakdown {
        TokenBreakdown {
            input,
            cache_read: 0.0,
            output,
            model_id: Some(model.to_string()),
        }
    }

    /// Skip a pricing assertion while estimation is switched off.
    ///
    /// The price table is retained behind [`COST_ESTIMATION_ENABLED`], so its
    /// arithmetic still needs coverage for the day it is turned back on. These
    /// tests are gated rather than deleted on purpose — deleting them would
    /// leave the table silently rotting.
    macro_rules! require_estimation {
        () => {
            if !COST_ESTIMATION_ENABLED {
                eprintln!("skipped: cost estimation disabled");
                return;
            }
        };
    }

    /// The switch itself must always be covered: while off, nothing is priced.
    #[test]
    fn switch_off_suppresses_estimation() {
        let priced = compute_cost(&bd("claude-3-5-sonnet", 1_000_000.0, 1_000_000.0));
        if COST_ESTIMATION_ENABLED {
            assert!(priced.is_some(), "switch on => known models must price");
        } else {
            assert!(
                priced.is_none(),
                "switch off => no estimate may be produced, got {priced:?}"
            );
        }
    }

    #[test]
    fn costs_known_models() {
        require_estimation!();
        let c = compute_cost(&bd("claude-3-5-sonnet", 1_000_000.0, 500_000.0)).unwrap();
        // 1M input @ 3.0 + 0.5M output @ 15.0 = 3.0 + 7.5 = 10.5
        assert!((c - 10.5).abs() < 1e-6);
    }

    #[test]
    fn unknown_model_is_none() {
        assert!(compute_cost(&bd("totally-unknown-model-xyz", 100.0, 100.0)).is_none());
    }

    #[test]
    fn bare_brand_does_not_price() {
        // "claude" alone must not hit a tier.
        assert!(tier_for("claude").is_none());
        assert!(tier_for("gemini").is_none());
    }

    #[test]
    fn alias_normalizes_openrouter_style() {
        assert!(tier_for("openai/gpt-4o").is_some());
        assert!(tier_for("deepseek/deepseek-chat").is_some());
    }

    /// Cache reads must bill at the discounted rate, not the input rate. This
    /// is the regression that mattered: cache is 80-98% of real traffic, so
    /// charging it at full price inflated totals by ~86%.
    #[test]
    fn cache_read_bills_at_discount_rate() {
        require_estimation!();
        let full = compute_cost(&TokenBreakdown {
            input: 0.0,
            cache_read: 1_000_000.0,
            output: 0.0,
            model_id: Some("claude-3-5-sonnet".to_string()),
        })
        .unwrap();
        // 1M cache @ 3.0 × 0.1 = 0.3
        assert!((full - 0.3).abs() < 1e-6, "got {full}");
    }

    /// Routing the cache portion through `cache_read` must cost strictly less
    /// than pre-merging it into `input` — which is what every caller used to do.
    #[test]
    fn pre_merged_cache_would_overcharge() {
        require_estimation!();
        let discounted = compute_cost(&TokenBreakdown {
            input: 0.0,
            cache_read: 1_000_000.0,
            output: 0.0,
            model_id: Some("claude-3-5-sonnet".to_string()),
        })
        .unwrap();
        let overcharged = compute_cost(&TokenBreakdown {
            input: 1_000_000.0,
            cache_read: 0.0,
            output: 0.0,
            model_id: Some("claude-3-5-sonnet".to_string()),
        })
        .unwrap();
        assert!(
            discounted < overcharged,
            "discounted {discounted} must beat pre-merged {overcharged}"
        );
        assert!((overcharged - 3.0).abs() < 1e-6);
    }

    /// A zero cache portion must not change the result — guards the new term
    /// against perturbing models that never report cache reads.
    #[test]
    fn zero_cache_is_neutral() {
        require_estimation!();
        let a = compute_cost(&bd("gpt-4o", 1_000_000.0, 0.0)).unwrap();
        let b = compute_cost(&TokenBreakdown {
            input: 1_000_000.0,
            cache_read: 0.0,
            output: 0.0,
            model_id: Some("gpt-4o".to_string()),
        })
        .unwrap();
        assert!((a - b).abs() < 1e-9);
    }

    #[test]
    fn output_only_is_billed_at_output_rate() {
        require_estimation!();
        let c = compute_cost(&bd("gpt-4o", 0.0, 1_000_000.0)).unwrap();
        assert!((c - 10.0).abs() < 1e-6);
    }
}
