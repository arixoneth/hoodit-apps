use super::{failure, now};
use crate::app::{Call, HooditApp};
use crate::market::{Lifecycle, Snapshot, flags, is_base_asset};
use crate::model::{self, one, opt, sig, usd};
use crate::providers::gecko::Gecko;
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverArgs {
    /// trending: momentum ranking. new: newest pools. launchpad: live Pons
    /// bonding curves by activity, with curve progress. volume: biggest 24h
    /// volume. Omit for trending.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["trending", "new", "launchpad", "volume"], "default" = "trending"))]
    pub feed: Option<String>,
    /// Trending window. Omit for 6h.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["5m", "1h", "6h", "24h"], "default" = "6h"))]
    pub window: Option<String>,
    /// Coins to return, 1 to 15. Omit for 10.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 15), extend("default" = 10))]
    pub limit: Option<u8>,
}

pub struct Discover;

impl DynAomiTool for Discover {
    type App = HooditApp;
    type Args = DiscoverArgs;
    const NAME: &'static str = "hoodit_discover";
    const DESCRIPTION: &'static str = "Scan a Robinhood Chain market feed for candidate coins: one page of trending, new, Pons launchpad, or top-volume pools, each with its numbers, launchpad stage, and setup flags (extended, fading, churn, thin exit, who is in control). A starting list to investigate, not a set of picks.";

    fn run(app: &HooditApp, args: DiscoverArgs, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let feed = args.feed.as_deref().unwrap_or("trending");
        let window = args.window.as_deref().unwrap_or("6h");
        if !["trending", "new", "launchpad", "volume"].contains(&feed)
            || !["5m", "1h", "6h", "24h"].contains(&window)
        {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "unsupported feed or window",
                false,
            ));
        }
        let limit = args.limit.unwrap_or(10).clamp(1, 15) as usize;
        let rt = app.runtime()?;
        let mut call = Call::new(20);
        let gecko = Gecko::new(&rt);
        let pools = match gecko.feed(&call, feed, window) {
            Ok(pools) => pools,
            Err(error) => return Ok(failure(error)),
        };
        let mut seen = HashSet::new();
        let pools: Vec<Snapshot> = pools
            .into_iter()
            .filter(|p| !is_base_asset(&p.token) && seen.insert(p.token.clone()))
            .take(limit)
            .collect();
        let curve_tokens: Vec<String> = pools
            .iter()
            .filter(|p| p.kind == Some("curve"))
            .map(|p| p.token.clone())
            .collect();
        // Curve progress costs a second GeckoTerminal call; only the launchpad
        // feed is about it.
        let progress: HashMap<String, Lifecycle> = if curve_tokens.is_empty() || feed != "launchpad"
        {
            HashMap::new()
        } else {
            match gecko.lifecycles(&call, &curve_tokens) {
                Ok(found) => found.into_iter().collect(),
                Err(error) => {
                    call.note(format!("curve progress unavailable: {}", error.message));
                    HashMap::new()
                }
            }
        };
        let now = now();
        let coins: Vec<Value> = pools
            .iter()
            .map(|p| row(p, progress.get(&p.token), now))
            .collect();
        let scope = match feed {
            "trending" => format!("GeckoTerminal trending pools, {window} window"),
            "new" => "GeckoTerminal newest pools".into(),
            "launchpad" => "live Pons bonding curves by 24h trades".into(),
            _ => "pools by 24h volume".into(),
        };
        Ok(model::ok(
            json!({"scope": scope, "coins": coins}),
            call.notes,
        ))
    }
}

fn row(pool: &Snapshot, lifecycle: Option<&Lifecycle>, now: i64) -> Value {
    let stage = match (pool.kind, pool.venue.as_str()) {
        (Some("curve"), _) => {
            let mut stage = json!({"stage": "curve"});
            if let Some(pct) = lifecycle.and_then(|l| l.progress_pct) {
                stage["curve_progress_pct"] = one(pct);
            }
            stage
        }
        (_, venue) if venue.starts_with("pons graduated") => json!({"stage": "graduated"}),
        _ => Value::Null,
    };
    let pair = |b: Option<u64>, s: Option<u64>| json!([b, s]);
    let mut row = json!({
        "token": pool.token,
        "symbol": pool.symbol,
        "pool_id": pool.pool_id,
        "venue": pool.venue,
        "price_usd": opt(pool.price_usd, sig),
        "liquidity_usd": opt(pool.liquidity_usd, usd),
        "fdv_usd": opt(pool.fdv_usd, usd),
        "age_h": opt(pool.age_hours(now), one),
        "change_pct": {"h1": opt(pool.change.h1, one), "h6": opt(pool.change.h6, one), "h24": opt(pool.change.h24, one)},
        "volume_24h_usd": opt(pool.volume.h24, usd),
        "buys_sells": {"h1": pair(pool.buys.h1, pool.sells.h1), "h24": pair(pool.buys.h24, pool.sells.h24)},
        "flags": flags(pool, now),
    });
    if !stage.is_null() {
        row["launchpad"] = stage;
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve_rows_carry_progress() {
        let pool = Snapshot {
            token: "0xaa".into(),
            kind: Some("curve"),
            venue: "pons curve".into(),
            ..Default::default()
        };
        let lifecycle = Lifecycle {
            stage: "curve",
            progress_pct: Some(81.26),
            ..Default::default()
        };
        let r = row(&pool, Some(&lifecycle), 0);
        assert_eq!(r["launchpad"]["curve_progress_pct"], json!(81.3));
        let graduated = Snapshot {
            venue: "pons graduated (uniswap v4)".into(),
            kind: Some("v4"),
            ..pool
        };
        assert_eq!(row(&graduated, None, 0)["launchpad"]["stage"], "graduated");
    }
}
