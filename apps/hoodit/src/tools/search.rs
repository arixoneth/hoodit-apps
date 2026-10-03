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
    const DESCRIPTION: &'static str = "Find Robinhood Chain tokens by ticker, name, or contract. Returns each matching token's contract and its most active pool. Same-ticker copycats are common: when one contract is clearly the real one the result says `resolved` and lists the copycats apart; otherwise it says `ambiguous`.";

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

/// A busier contract using the same ticker, for a token that lacks the real
/// one's marks (a DexScreener profile, actual trading).
pub(crate) fn busier_namesake(dex: &Dex, call: &Call, token: &Snapshot) -> Option<String> {
    let own: f64 = token.volume.h24.unwrap_or(0.0);
    let mut volume: BTreeMap<String, f64> = BTreeMap::new();
    for pool in dex.search(call, &token.symbol).ok()? {
        if pool.token != token.token && pool.symbol.eq_ignore_ascii_case(&token.symbol) {
            *volume.entry(pool.token).or_default() += pool.volume.h24.unwrap_or(0.0);
        }
    }
    let (other, theirs) = volume.into_iter().max_by(|a, b| a.1.total_cmp(&b.1))?;
    (theirs >= 10_000.0 && theirs >= own * 10.0).then(|| {
        format!(
            "copycat_risk: {other} also uses {} and trades ${theirs:.0} in 24h vs ${own:.0} here",
            token.symbol
        )
    })
}

/// One contract's search row with what decides whether it is the real one.
struct Row {
    volume: f64,
    liquidity: f64,
    exact: bool,
    profile: bool,
    parked: Option<String>,
    view: Value,
}

impl Row {
    fn token(&self) -> &str {
        self.view["token"].as_str().unwrap_or_default()
    }
    /// Why a same-ticker contract looks like a copycat of the real one.
    fn copycat_reasons(&self, share_pct: f64) -> Vec<String> {
        let mut why = vec![format!("{share_pct:.0}% of the ticker's 24h volume")];
        if !self.profile {
            why.push("no DexScreener profile".into());
        }
        why.extend(self.parked.clone());
        why
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
    let mut rows: Vec<Row> = by_token
        .iter()
        .filter_map(|(token, pools)| {
            let best = main_pool(pools, token).or_else(|| pools.first())?;
            let profile = pools.iter().any(|p| p.profile);
            let parked = best.parked_liquidity().then(|| {
                format!(
                    "parked liquidity: ${:.0} deep but ${:.0} traded in 24h",
                    best.liquidity_usd.unwrap_or(0.0),
                    best.volume.h24.unwrap_or(0.0)
                )
            });
            let mut view = json!({
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
                "dexscreener_profile": profile,
            });
            if let Some(parked) = &parked {
                view["warning"] = json!(parked);
            }
            Some(Row {
                volume: pools.iter().filter_map(|p| p.volume.h24).sum(),
                liquidity: best.liquidity_usd.unwrap_or(0.0),
                exact: best.symbol.eq_ignore_ascii_case(query)
                    || best.token.eq_ignore_ascii_case(query),
                profile,
                parked,
                view,
            })
        })
        .collect();
    // Copycats often show big but idle liquidity; trading volume says which
    // contract the market actually means.
    rows.sort_by(|a, b| {
        b.exact
            .cmp(&a.exact)
            .then(b.volume.total_cmp(&a.volume))
            .then(b.liquidity.total_cmp(&a.liquidity))
    });
    let exact = rows.iter().filter(|r| r.exact).count();
    let total: f64 = rows.iter().filter(|r| r.exact).map(|r| r.volume).sum();
    let share = |r: &Row| {
        if total > 0.0 {
            r.volume / total * 100.0
        } else {
            0.0
        }
    };
    let mut out = json!({"query": query});
    // The real contract carries the trading, and usually the only profile.
    let real = (exact >= 2).then(|| &rows[0]).filter(|top| {
        let sole_profile = top.profile && !rows[1..exact].iter().any(|r| r.profile);
        top.volume >= 10_000.0 && (share(top) >= 90.0 || (sole_profile && share(top) >= 50.0))
    });
    if let Some(top) = real {
        let symbol = top.view["symbol"].as_str().unwrap_or_default().to_string();
        let mut basis = format!("{:.0}% of the ticker's 24h volume", share(top));
        if top.profile && !rows[1..exact].iter().any(|r| r.profile) {
            basis.push_str(" and the only DexScreener profile");
        }
        out["resolved"] = json!(format!(
            "{} is the real {symbol}: {basis}. The {} other contracts using the ticker are copycats; never offer them as options or cite their numbers, and name them only as a warning",
            top.token(),
            exact - 1
        ));
        out["copycats"] = json!(
            rows[1..exact]
                .iter()
                .take(5)
                .map(|r| json!({"token": r.token(), "why": r.copycat_reasons(share(r))}))
                .collect::<Vec<_>>()
        );
        let others: Vec<Value> = std::iter::once(&rows[0])
            .chain(&rows[exact..])
            .take(8)
            .map(|r| r.view.clone())
            .collect();
        out["tokens"] = json!(others);
        return out;
    }
    if exact >= 2 {
        out["ambiguous"] = json!(format!(
            "{exact} different contracts use this ticker and none clearly dominates; show the candidates with their volume and profile and ask which one, never leading with parked liquidity"
        ));
    }
    out["tokens"] = json!(rows.into_iter().take(8).map(|r| r.view).collect::<Vec<_>>());
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

    #[test]
    fn resolves_the_traded_contract_and_sets_copycats_apart() {
        let pool = |token: &str, liquidity: f64, volume: f64, profile: bool| Snapshot {
            pool_id: format!("0xpool{token}"),
            token: token.into(),
            symbol: "ROBINPEPE".into(),
            kind: Some("v4"),
            liquidity_usd: Some(liquidity),
            volume: crate::market::Win {
                h24: Some(volume),
                ..Default::default()
            },
            buys: crate::market::Win {
                h24: Some(50),
                ..Default::default()
            },
            profile,
            ..Default::default()
        };
        let pools = vec![
            pool("0xreal", 43_000.0, 1_060_000.0, true),
            pool("0xfake", 602_000.0, 3.0, false),
        ];
        let out = results(&pools, "ROBINPEPE", 0);
        assert_eq!(out["tokens"].as_array().unwrap().len(), 1);
        assert_eq!(out["tokens"][0]["token"], "0xreal");
        assert!(out.get("ambiguous").is_none());
        assert!(
            out["resolved"]
                .as_str()
                .unwrap()
                .starts_with("0xreal is the real ROBINPEPE: 100% of the ticker's 24h volume and the only DexScreener profile")
        );
        assert_eq!(out["copycats"][0]["token"], "0xfake");
        let why = out["copycats"][0]["why"].to_string();
        assert!(why.contains("no DexScreener profile"), "{why}");
        assert!(
            why.contains("parked liquidity: $602000 deep but $3"),
            "{why}"
        );
    }
}
