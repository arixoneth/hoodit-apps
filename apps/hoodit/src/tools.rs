use crate::app::{Call, HooditApp, Runtime};
use aomi_sdk::DynToolCallCtx;
use serde_json::Value;
use std::future::Future;
use std::sync::Arc;

mod chart;
mod check;
mod exit;
mod find;
mod holders;
mod scan;
mod token;
mod trades;
mod wallet;

pub use chart::Chart;
pub use check::Check;
pub use exit::Exit;
pub use find::Find;
pub use holders::Holders;
pub use scan::Scan;
pub use token::Token;
pub use trades::Trades;
pub use wallet::Wallet;

/// Runs an async tool body on the app's runtime and returns its JSON.
pub(crate) fn exec<F, Fut>(app: &HooditApp, ctx: &DynToolCallCtx, body: F) -> Result<Value, String>
where
    F: FnOnce(Arc<Runtime>, Call) -> Fut,
    Fut: Future<Output = Value>,
{
    let rt = app.runtime()?;
    let call = Call::new(ctx);
    let value = rt.tokio.block_on(body(rt.clone(), call));
    Ok(crate::shape::fit(crate::shape::compact(value)))
}

/// `addr:4663` id Codex uses for tokens on Robinhood Chain.
pub(crate) fn codex_id(token: &str) -> String {
    format!("{token}:{}", crate::providers::NETWORK)
}

macro_rules! arg {
    ($value:expr) => {
        match $value {
            Ok(value) => value,
            Err(message) => return crate::shape::error("INVALID_ARGUMENT", &message, None),
        }
    };
}
pub(crate) use arg;
