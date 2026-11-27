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

/// OpenAI models used by Codex: (model id, input, cached input, output), standard tier,
/// prompts up to 272K tokens. Source: developers.openai.com/api/docs/pricing, checked 2026-11-27
/// (gpt-5-codex / gpt-5.1-codex from their model pages: same rates as gpt-5).
const OPENAI: &[(&str, f64, f64, f64)] = &[
    ("gpt-5.6-sol", 4.0, 0.40, 20.0),
    ("gpt-5.6-terra", 2.0, 0.20, 12.0),
    ("gpt-5.6-luna", 0.20, 0.02, 1.20),
    ("gpt-5.5", 5.0, 0.50, 30.0),
    ("gpt-5.4", 2.50, 0.25, 15.0),
    ("gpt-5.4-mini", 0.75, 0.075, 4.50),
    ("gpt-5.4-nano", 0.20, 0.02, 1.25),
    ("gpt-5.3-codex", 1.75, 0.175, 14.0),
    ("gpt-5.2", 1.75, 0.175, 14.0),
    ("gpt-5.1", 1.25, 0.125, 10.0),
    ("gpt-5.1-codex", 1.25, 0.125, 10.0),
    ("gpt-5", 1.25, 0.125, 10.0),
    ("gpt-5-codex", 1.25, 0.125, 10.0),
    ("gpt-5-mini", 0.25, 0.025, 2.0),
    ("gpt-5-nano", 0.05, 0.005, 0.40),
];
/// These models bill prompts over 272K tokens at 2x input and 1.5x output (same source).
const OPENAI_LONG: &[&str] = &["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna", "gpt-5.5", "gpt-5.4"];

/// Exact id or a dated snapshot ("gpt-5-2025-08-07"); never a different variant like "-mini".
fn openai_price(model: &str, prompt: u64) -> Option<Price> {
    let (id, input, cached, output) = *OPENAI.iter().find(|(id, ..)| model == *id || model.strip_prefix(id).is_some_and(|rest| rest.starts_with("-20")))?;
    let long = prompt > 272_000 && OPENAI_LONG.contains(&id);
    Some(if long { Price { input: input * 2.0, output: output * 1.5, cache_read: cached * 2.0 } } else { Price { input, output, cache_read: cached } })
}

/// Estimated USD for one Codex response. OpenAI counts cached tokens inside `input`.
pub fn openai_cost(model: &str, input: u64, cached: u64, output: u64) -> Option<f64> {
    let p = openai_price(model, input)?;
    let fresh = input.saturating_sub(cached);
    Some((fresh as f64 * p.input + cached as f64 * p.cache_read + output as f64 * p.output) / 1_000_000.0)
}

pub fn openai_cache_savings(model: &str, input: u64, cached: u64) -> f64 {
    openai_price(model, input).map_or(0.0, |p| cached.min(input) as f64 * (p.input - p.cache_read) / 1_000_000.0)
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
    fn openai_prices_match_exact_ids_and_snapshots_only() {
        // 1M fresh input + 1M cached (inside input) + 1M output on gpt-5: 1.25 + 0.125 + 10
        let c = openai_cost("gpt-5", 2_000_000 - 1, 1_000_000 - 1, 1_000_000).unwrap();
        assert!((c - 11.375).abs() < 1e-4, "{c}");
        assert!(openai_cost("gpt-5-2025-08-07", 1, 0, 0).is_some());
        assert!(openai_cost("gpt-5.1-codex-mini", 1, 0, 0).is_none()); // not priced as gpt-5.1-codex
        assert!(openai_cost("claude-opus-5-5", 1, 0, 0).is_none());
        // long prompts on gpt-5.5: 2x input
        let long = openai_cost("gpt-5.5", 300_000, 0, 0).unwrap();
        assert!((long - 3.0).abs() < 1e-9, "{long}");
        assert!((openai_cache_savings("gpt-5", 1_000_000, 1_000_000) - 1.125).abs() < 1e-9);
    }

    #[test]
    fn haiku_5_5_rate_card_switches_at_100k() {
        let small = cost("claude-haiku-5-5", &t(100_000, 0, 0, 0, 0), false).unwrap();
        let large = cost("claude-haiku-5-5", &t(100_001, 0, 0, 0, 0), false).unwrap();
        assert!((small - 0.01).abs() < 1e-9);
        assert!((large - 0.0500005).abs() < 1e-9);
    }
}
