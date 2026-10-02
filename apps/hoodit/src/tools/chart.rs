use super::{arg, eth_usd, failure, now};
use crate::app::{Call, HooditApp, Runtime};
use crate::market::chart::{
    self, auto_interval, candles, earlier, flow, rows, structure, wallet_picks,
};
use crate::market::swaps::{self, Market};
use crate::market::{Snapshot, deepest_pool, main_pool};
use crate::model::{self, one, sig, usd};
use crate::providers::{ProviderError, dex::Dex, gecko::Gecko};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

/// Most swaps read per call; very busy pools show their latest few hours.
const SWAP_BUDGET: usize = 2500;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartArgs {
    /// Exact 0x token contract. Resolve tickers with hoodit_search first.
    pub token: String,
    /// Optional pool_id from a Hoodit result. Omit to use the most active pool
    /// (the Pons curve while a token is still bonding).
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub pool_id: Option<String>,
    /// Hours of history to read, 1 to 168. Omit for 24 (72 for quiet pools).
    #[serde(default)]
    #[schemars(with = "u16", range(min = 1, max = 168))]
    pub hours: Option<u16>,
    /// Candle width. Omit to fit the window in about 48 candles.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["1m", "5m", "15m", "1h", "4h", "1d"]))]
    pub interval: Option<String>,
    /// Latest individual trades to list, 0 to 10. Omit for 0; flow already
    /// summarizes them.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 0, max = 10), extend("default" = 0))]
    pub recent_trades: Option<u8>,
}

pub struct GetChart;

impl DynAomiTool for GetChart {
    type App = HooditApp;
    type Args = ChartArgs;
    const NAME: &'static str = "hoodit_get_chart";
    const DESCRIPTION: &'static str = "Read a token's real chart and order flow from on-chain swaps in one pool: USD candles, structure facts (range, distance from high and low, rising lows, volume trend, VWAP), buy and sell flow for the last hour and the window, the largest trades, and the wallets doing the most buying and selling. Use it before any claim about chart structure, momentum, or who is selling.";

    fn run(app: &HooditApp, args: ChartArgs, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let token = arg!(model::address(&args.token));
        let pool_id = arg!(args.pool_id.as_deref().map(model::pool_id).transpose());
        let fixed = match args.interval.as_deref() {
            Some(label) => Some((
                label,
                arg!(chart::interval(label).ok_or_else(|| "unsupported interval".to_string())),
            )),
            None => None,
        };
        let rt = app.runtime()?;
        let mut call = Call::new(30);
        let pool = match select_pool(&rt, &mut call, &token, pool_id.as_deref()) {
            Ok(pool) => pool,
            Err(error) => return Ok(failure(error)),
        };
        let eth = pool
            .quote_usd
            .is_none()
            .then(|| eth_usd(&rt, &call))
            .flatten();
        let market = match Market::resolve(&rt, &call, &pool, &token, eth) {
            Ok(market) => market,
            Err(error) => return Ok(failure(error)),
        };
        let per_day = pool.txns_24h();
        let hours = args.hours.map(|h| h.clamp(1, 168) as i64).unwrap_or(
            if per_day.is_some_and(|n| n < 200) {
                72
            } else {
                24
            },
        );
        let mut found =
            match swaps::fetch(&rt, &mut call, &market, hours * 3600, per_day, SWAP_BUDGET) {
                Ok(found) => found,
                Err(error) => return Ok(failure(error)),
            };
        let now = now();
        let pool_view = json!({"pool_id": pool.pool_id, "venue": pool.venue, "pair": pool.pair()});
        if found.swaps.is_empty() {
            call.note(format!("no swaps in the last {hours}h in this pool"));
            return Ok(model::ok(
                json!({"token": token, "pool": pool_view, "hours_requested": hours}),
                call.notes,
            ));
        }
        let picks = wallet_picks(&found.swaps, &market);
        swaps::annotate(&rt, &mut call, &market, &mut found, &picks);
        let trades = &found.swaps;
        // Size candles to the data, not the window: a coin launched three hours
        // ago gets 5m candles even when 24h were requested.
        let first = trades
            .first()
            .map_or(found.from_ts, |s| s.ts.max(found.from_ts));
        let span = (found.to_ts - first).max(60);
        // A requested width too fine for the window widens to fit it, so the
        // structure always describes the whole window rather than its tail.
        let fitted = auto_interval(span);
        let (label, secs) = match fixed {
            Some((label, secs)) if secs >= fitted.1 => (label, secs),
            _ => fitted,
        };
        let mut series = candles(trades, &market, secs);
        if series.len() > 33 {
            series.drain(..series.len() - 33);
        }
        let covered = one((found.to_ts - found.from_ts) as f64 / 3600.0);
        let mut out = json!({
            "token": token,
            "pool": pool_view,
            "window": {
                "hours": covered,
                "hours_requested": hours,
                "swaps": trades.len(),
                "complete": !found.truncated,
                "coverage": if found.truncated {
                    format!("busy pool: candles and structure cover only the latest {covered}h; `earlier` samples the full {hours}h and has the change, high and low over it")
                } else {
                    format!("every swap of the last {covered}h")
                },
            },
            "interval": label,
            "candles": rows(&series, now),
            "structure": structure(&series, trades, &market, now),
            "flow": flow(trades, &market, now),
            "pricing": pricing(&rt, &mut call, &market, &pool),
        });
        if found.truncated && !found.context.is_empty() {
            out["earlier"] = earlier(&found.context, &series, found.lookback_ts, now);
        }
        let recent = args.recent_trades.unwrap_or(0).min(10) as usize;
        if recent > 0 {
            out["recent_trades"] = trades
                .iter()
                .rev()
                .take(recent)
                .map(|s| json!({"side": if s.buy {"buy"} else {"sell"}, "usd": usd(s.usd(&market)), "price_usd": sig(s.price_usd(&market)), "minutes_ago": (now - s.ts) / 60, "wallet": s.wallet}))
                .collect();
        }
        Ok(model::ok(out, call.notes))
    }
}

/// How candle prices relate to USD. Trades are converted at the quote's
/// current price, which is exact for USDG, close for ETH, and misleading for
/// a volatile quote token, so that case carries the quote's own moves.
fn pricing(rt: &Runtime, call: &mut Call, market: &Market, pool: &Snapshot) -> Value {
    let quote_symbol = if pool.token == market.token {
        &pool.quote_symbol
    } else {
        &pool.symbol
    };
    match market.quote.as_str() {
        model::USDG => json!("USD (USDG pool)"),
        model::WETH | model::NATIVE => json!(format!(
            "USD at the current ETH price of ${}",
            usd(market.quote_usd)
        )),
        quote => {
            call.note(format!(
                "this pool trades against {quote_symbol}, not ETH or USDG: candles are {quote_symbol} prices converted at today's rate, so their moves include {quote_symbol}'s own moves"
            ));
            let moves = Dex::new(rt)
                .token_pools(call, quote)
                .ok()
                .and_then(|pools| main_pool(&pools, quote).map(|p| p.change))
                .map(|c| json!({"h1": c.h1.map(one), "h6": c.h6.map(one), "h24": c.h24.map(one)}));
            json!({
                "quote": quote_symbol,
                "converted_at_usd": sig(market.quote_usd),
                "quote_change_pct": moves,
                "read": "USD change of the token ≈ candle change combined with the quote's change; prefer the snapshot's change_pct for USD moves"
            })
        }
    }
}

/// The explicit pool, else the token's most active pool, else its Pons curve
/// while it is still bonding, else its deepest (possibly dead) pool.
fn select_pool(
    rt: &Runtime,
    call: &mut Call,
    token: &str,
    pool_id: Option<&str>,
) -> Result<Snapshot, ProviderError> {
    let pools = Dex::new(rt)
        .token_pools(call, token)
        .unwrap_or_else(|error| {
            call.note(format!("DexScreener unavailable: {}", error.message));
            vec![]
        });
    let explicit =
        |pools: &[Snapshot]| pool_id.and_then(|id| pools.iter().find(|p| p.pool_id == id).cloned());
    if let Some(pool) = explicit(&pools).or_else(|| {
        pool_id
            .is_none()
            .then(|| main_pool(&pools, token).cloned())
            .flatten()
    }) {
        return Ok(pool);
    }
    let launch = Gecko::new(rt).token(call, token);
    let found = match &launch {
        Ok((_, launch_pools)) if pool_id.is_some() => explicit(launch_pools),
        Ok((lifecycle, launch_pools)) if lifecycle.stage == "curve" => launch_pools
            .iter()
            .find(|p| p.kind == Some("curve"))
            .cloned(),
        Ok((_, launch_pools)) => main_pool(launch_pools, token).cloned(),
        Err(_) => None,
    };
    if let Some(pool) = found.or_else(|| {
        pool_id
            .is_none()
            .then(|| deepest_pool(&pools, token).cloned())
            .flatten()
    }) {
        return Ok(pool);
    }
    Err(match launch {
        Err(error) if pools.is_empty() => error,
        _ if pool_id.is_some() => ProviderError::new(
            "POOL_NOT_FOUND",
            "that pool_id is not a known pool of this token",
        ),
        _ => ProviderError::new("NOT_FOUND", "no chartable pool trades this token"),
    })
}
