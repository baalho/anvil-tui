//! Token usage tracking and cost estimation.
//!
//! Accumulates prompt/completion token counts across turns. Optional
//! pricing config enables cost estimation for remote/paid APIs.

use anvil_config::PricingConfig;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
    pub estimated_cost_usd: Option<f64>,
    pub last_request_at: Option<DateTime<Utc>>,
}

impl TokenUsage {
    pub fn record(&mut self, prompt: u64, completion: u64, pricing: Option<&PricingConfig>) {
        self.prompt_tokens += prompt;
        self.completion_tokens += completion;
        self.total_tokens += prompt + completion;
        self.request_count += 1;
        self.last_request_at = Some(Utc::now());

        if let Some(p) = pricing {
            let cost = (prompt as f64 * p.input_per_million / 1_000_000.0)
                + (completion as f64 * p.output_per_million / 1_000_000.0);
            *self.estimated_cost_usd.get_or_insert(0.0) += cost;
        }
    }
}
