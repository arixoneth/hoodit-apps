use super::{failure, now};
use crate::app::{Call, HooditApp};
use crate::market::{Snapshot, is_base_asset, main_pool};
use crate::model::{self, one, opt, sig, usd};
use crate::providers::dex::Dex;
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    /// Ticker, token name, or exact 0x contract address.
    pub query: String,
}

pub struct Search;

impl DynAomiTool for Search {
    type App = HooditApp;
    type Args = SearchArgs;
    const NAME: &'static str = "hoodit_search";
    const DESCRIPTION: &'static str = "Find Robinhood Chain tokens by ticker, name, or contract. Returns each matching token's contract and its most active pool. Same-ticker clones are common: never assume which one the user means.";

    fn run(app: &HooditApp, args: SearchArgs, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let query = args.query.trim();
        if query.is_empty() || query.len() > 100 {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "query must be 1 to 100 characters",
                false,
            ));
        }
        let rt = app.runtime()?;
        let call = Call::new(15);
        let dex = Dex::new(&rt);
        let pools = match model::address(query) {
            Ok(token) => dex.token_pools(&call, &token),
            Err(_) => dex.search(&call, query),
        };
        match pools {
            Ok(pools) => Ok(model::ok(results(&pools, query, now()), call.notes)),
            Err(error) => Ok(failure(error)),
        }
    }
}

fn results(pools: &[Snapshot], query: &str, now: i64) -> Value {
    // The chain name added to the query also matches "Robinhood ..." tokens;
    // keep only tokens that actually match, unless nothing does.
    let needle = query.to_ascii_lowercase();
    let matches = |p: &&Snapshot| {
        p.token == needle
            || p.symbol.to_ascii_lowercase().contains(&needle)
            || p.name.to_ascii_lowercase().contains(&needle)
    };
    let pools: Vec<&Snapshot> = match pools.iter().filter(matches).count() {
        0 => pools.iter().collect(),
        _ => pools.iter().filter(matches).collect(),
    };
    let mut by_token: BTreeMap<&str, Vec<Snapshot>> = BTreeMap::new();
    for pool in pools.into_iter().filter(|p| !is_base_asset(&p.token)) {
        by_token.entry(&pool.token).or_default().push(pool.clone());
    }
    let mut rows: Vec<(f64, Value, bool)> = by_token
        .iter()
        .filter_map(|(token, pools)| {
            let best = main_pool(pools, token).or_else(|| pools.first())?;
            let exact =
                best.symbol.eq_ignore_ascii_case(query) || best.token.eq_ignore_ascii_case(query);
            let row = json!({
                "token": best.token,
                "symbol": best.symbol,
                "name": best.name,
                "pool_id": best.pool_id,
                "venue": best.venue,
                "price_usd": opt(best.price_usd, sig),
                "liquidity_usd": opt(best.liquidity_usd, usd),
                "volume_24h_usd": opt(best.volume.h24, usd),
                "fdv_usd": opt(best.fdv_usd, usd),
                "change_24h_pct": opt(best.change.h24, one),
                "age_h": opt(best.age_hours(now), one),
            });
            Some((best.liquidity_usd.unwrap_or(0.0), row, exact))
        })
        .collect();
    rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.0.total_cmp(&a.0)));
    let same_ticker = rows.iter().filter(|r| r.2).count();
    let tokens: Vec<Value> = rows.into_iter().take(8).map(|r| r.1).collect();
    let mut out = json!({"query": query, "tokens": tokens});
    if same_ticker > 1 {
        out["ambiguous"] = json!(format!(
            "{same_ticker} different contracts use this ticker; confirm which one before judging it"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_pools_by_token_and_flags_ticker_clones() {
        let pool = |token: &str, liquidity: f64| Snapshot {
            pool_id: format!("0xpool{token}"),
            token: token.into(),
            symbol: "PEPE".into(),
            kind: Some("v4"),
            liquidity_usd: Some(liquidity),
            buys: crate::market::Win {
                h24: Some(5),
                ..Default::default()
            },
            sells: crate::market::Win {
                h24: Some(5),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut unrelated = pool("0xcc", 1e7);
        unrelated.symbol = "WALLET".into();
        unrelated.name = "Robinhood Wallet".into();
        let pools = vec![
            pool("0xaa", 100.0),
            pool("0xbb", 5000.0),
            pool("0xbb", 10.0),
            pool(model::WETH, 1e9),
            unrelated,
        ];
        let out = results(&pools, "pepe", 0);
        assert_eq!(out["tokens"].as_array().unwrap().len(), 2);
        assert_eq!(out["tokens"][0]["token"], "0xbb");
        assert!(
            out["ambiguous"]
                .as_str()
                .unwrap()
                .starts_with("2 different")
        );
    }
}
