//! API-equivalent prices in USD per million tokens.
//! Source: Anthropic pricing, checked 2026-11-22. Subscription (Pro/Max) users don't pay
//! these; the UI labels the result as an estimate.

pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
}

/// (model-id fragment, input, output, cache read). More specific fragments first,
/// because matching is by substring ("opus-5" would also match "opus-5-5").
const TABLE: &[(&str, f64, f64, f64)] = &[
    ("fable-5-2", 8.0, 40.0, 0.20),
    ("mythos-5-2", 8.0, 40.0, 0.20),
    ("fable-5-1", 10.0, 50.0, 0.25),
    ("mythos-5-1", 10.0, 50.0, 0.25),
    ("fable-5", 10.0, 50.0, 1.00),
    ("opus-5-5", 4.0, 20.0, 0.20),
    ("opus-5", 5.0, 25.0, 0.50),
    ("opus-4-8", 5.0, 25.0, 0.50),
    ("opus-4-7", 5.0, 25.0, 0.50),
    ("opus-4-6", 5.0, 25.0, 0.50),
    ("sonnet-5-5", 2.0, 10.0, 0.10),
    ("sonnet-5", 2.0, 10.0, 0.20),
    ("sonnet-4-6", 3.0, 15.0, 0.30),
    ("haiku-4-5", 1.0, 5.0, 0.10),
];

/// Haiku 5.5 has two rate cards, chosen by prompt length (input + cache tokens).
const HAIKU_5_5_SMALL: Price = Price { input: 0.10, output: 0.50, cache_read: 0.01 };
const HAIKU_5_5_LARGE: Price = Price { input: 0.50, output: 2.50, cache_read: 0.05 };

pub fn price_for(model: &str, prompt_tokens: u64) -> Option<Price> {
    if model.contains("haiku-5-5") {
        return Some(if prompt_tokens > 100_000 { HAIKU_5_5_LARGE } else { HAIKU_5_5_SMALL });
    }
    TABLE
        .iter()
        .find(|(id, ..)| model.contains(id))
        .map(|&(_, input, output, cache_read)| Price { input, output, cache_read })
}

pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
}

/// Estimated USD cost of one request, or None for models we don't know.
pub fn cost(model: &str, t: &Tokens, fast: bool) -> Option<f64> {
    let prompt = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h;
    let p = price_for(model, prompt)?;
    let usd = (t.input as f64 * p.input
        + t.cache_write_5m as f64 * p.input * 1.25
        + t.cache_write_1h as f64 * p.input * 2.0
        + t.cache_read as f64 * p.cache_read
        + t.output as f64 * p.output)
        / 1_000_000.0;
    Some(if fast { usd * 2.0 } else { usd })
}

/// USD saved because cache reads were billed below the full input price.
pub fn cache_savings(model: &str, t: &Tokens, fast: bool) -> f64 {
    let prompt = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h;
    let Some(p) = price_for(model, prompt) else { return 0.0 };
    let usd = t.cache_read as f64 * (p.input - p.cache_read) / 1_000_000.0;
    if fast { usd * 2.0 } else { usd }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(input: u64, output: u64, cache_read: u64, w5: u64, w1h: u64) -> Tokens {
        Tokens { input, output, cache_read, cache_write_5m: w5, cache_write_1h: w1h }
    }

    #[test]
    fn specific_models_win_over_prefixes() {
        assert_eq!(price_for("claude-opus-5-5", 0).unwrap().input, 4.0);
        assert_eq!(price_for("claude-opus-5", 0).unwrap().input, 5.0);
        assert_eq!(price_for("claude-fable-5-2", 0).unwrap().cache_read, 0.20);
        assert!(price_for("<synthetic>", 0).is_none());
    }

    #[test]
    fn cost_math() {
        // 1M of each bucket on Opus 5.5: 4 + 20 + 0.20 + 5 (1.25x) + 8 (2x) = 37.20
        let c = cost("claude-opus-5-5", &t(1_000_000, 1_000_000, 1_000_000, 1_000_000, 1_000_000), false).unwrap();
        assert!((c - 37.20).abs() < 1e-9, "{c}");
        let fast = cost("claude-opus-5-5", &t(1_000_000, 0, 0, 0, 0), true).unwrap();
        assert!((fast - 8.0).abs() < 1e-9);
    }

    #[test]
    fn cache_savings_math() {
        // 1M cache reads on Opus 5.5: would be $4 at input price, cost $0.20 → saved $3.80
        let s = cache_savings("claude-opus-5-5", &t(0, 0, 1_000_000, 0, 0), false);
        assert!((s - 3.80).abs() < 1e-9);
        assert_eq!(cache_savings("<synthetic>", &t(0, 0, 1_000_000, 0, 0), false), 0.0);
    }

    #[test]
    fn haiku_5_5_rate_card_switches_at_100k() {
        let small = cost("claude-haiku-5-5", &t(100_000, 0, 0, 0, 0), false).unwrap();
        let large = cost("claude-haiku-5-5", &t(100_001, 0, 0, 0, 0), false).unwrap();
        assert!((small - 0.01).abs() < 1e-9);
        assert!((large - 0.0500005).abs() < 1e-9);
    }
}
