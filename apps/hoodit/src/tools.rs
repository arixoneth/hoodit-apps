use crate::{model, providers::ProviderError};
use serde_json::Value;

mod markets;
mod portfolio;

pub use markets::*;
pub use portfolio::*;

fn provider_error(error: ProviderError) -> Value {
    let code = match error.code {
        "INVALID_CURSOR" => "INVALID_ARGUMENT",
        "NO_INDEXED_POOL" | "UPSTREAM_NOT_FOUND" => "POOL_NOT_FOUND",
        "NOT_INDEXED" => "TOKEN_NOT_INDEXED",
        "QUOTE_BUDGET_EXHAUSTED" | "PROVIDER_BUDGET_EXHAUSTED" => "RATE_LIMITED",
        "DEADLINE_EXCEEDED" => "UPSTREAM_UNAVAILABLE",
        other => other,
    };
    model::error(code, &error.message, error.retryable)
}
