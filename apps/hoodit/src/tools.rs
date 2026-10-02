use crate::app::{Call, Runtime};
use crate::model;
use crate::providers::{ProviderError, dex::Dex};
use serde_json::Value;

mod chart;
mod discover;
mod exit;
mod search;
mod token;

pub use chart::{ChartArgs, GetChart};
pub use discover::{Discover, DiscoverArgs};
pub use exit::{CheckExit, ExitArgs};
pub use search::{Search, SearchArgs};
pub use token::{GetToken, TokenArgs};

macro_rules! arg {
    ($value:expr) => {
        match $value {
            Ok(value) => value,
            Err(message) => return Ok($crate::model::error("INVALID_ARGUMENT", &message, false)),
        }
    };
}
pub(crate) use arg;

pub(crate) fn failure(error: ProviderError) -> Value {
    model::error(error.code, &error.message, error.retryable())
}

pub(crate) fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// ETH in USD from the deepest WETH pool, for pools DexScreener doesn't price.
pub(crate) fn eth_usd(rt: &Runtime, call: &Call) -> Option<f64> {
    let pools = Dex::new(rt).token_pools(call, model::WETH).ok()?;
    pools
        .iter()
        .filter(|s| s.token == model::WETH && s.price_usd.is_some())
        .max_by(|a, b| {
            a.liquidity_usd
                .unwrap_or(0.0)
                .total_cmp(&b.liquidity_usd.unwrap_or(0.0))
        })
        .and_then(|s| s.price_usd)
}
