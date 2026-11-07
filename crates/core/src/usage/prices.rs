//! USD per million tokens. Prices as of 2026-10-26 (Anthropic API price list; OpenAI gpt-5 family list
//! prices, not re-verified), edit here. A model with no row below is reported in tokens only.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
}

/// Claude: cache writes cost 1.25x input (5 min TTL) or 2x input (1 h TTL).
const fn claude(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
    }
}

/// OpenAI: caching is automatic, writes are free.
const fn openai(input: f64, output: f64, cache_read: f64) -> Price {
    Price {
        input,
        output,
        cache_read,
        cache_write_5m: 0.0,
        cache_write_1h: 0.0,
    }
}

/// First matching model-id prefix wins: keep specific ids before their family.
const PRICES: &[(&str, Price)] = &[
    ("claude-fable-5-1", claude(10.0, 50.0, 0.25)),
    ("claude-fable-5", claude(10.0, 50.0, 1.0)),
    ("claude-opus-5-5", claude(4.0, 20.0, 0.20)),
    ("claude-opus-5", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-8", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-7", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-6", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4-5", claude(5.0, 25.0, 0.50)),
    ("claude-opus-4", claude(15.0, 75.0, 1.50)),
    ("claude-sonnet-5", claude(2.0, 10.0, 0.20)),
    ("claude-sonnet-4", claude(3.0, 15.0, 0.30)),
    ("claude-haiku-4", claude(1.0, 5.0, 0.10)),
    ("gpt-5-mini", openai(0.25, 2.0, 0.025)),
    ("gpt-5-nano", openai(0.05, 0.40, 0.005)),
    ("gpt-5", openai(1.25, 10.0, 0.125)),
];

pub fn price(model: &str) -> Option<Price> {
    PRICES
        .iter()
        .find(|(prefix, _)| model.starts_with(prefix))
        .map(|(_, p)| *p)
}

#[cfg(test)]
mod tests {
    use super::price;

    #[test]
    fn specific_ids_win_over_family() {
        assert_eq!(price("claude-opus-5-5").unwrap().input, 4.0);
        assert_eq!(price("claude-opus-5").unwrap().input, 5.0);
        assert_eq!(price("claude-fable-5-1").unwrap().cache_read, 0.25);
        assert_eq!(price("gpt-5-codex").unwrap().output, 10.0);
        assert_eq!(price("gpt-5-mini").unwrap().input, 0.25);
        assert!(price("gpt-6-astra").is_none());
        assert!(price("<synthetic>").is_none());
    }
}
