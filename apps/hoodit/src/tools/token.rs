use super::{arg, failure, now, search::busier_namesake};
use crate::app::{Call, HooditApp};
use crate::market::{Lifecycle, Slippage, Snapshot, deepest_pool, flags, main_pool};
use crate::model::{self, opt, usd};
use crate::providers::{dex::Dex, gecko::Gecko, goplus::GoPlus};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenArgs {
    /// Exact 0x token contract. Resolve tickers with hoodit_search first.
    pub token: String,
    /// Optional pool_id from a Hoodit result. Omit to use the most active pool.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub pool_id: Option<String>,
}

pub struct GetToken;

impl DynAomiTool for GetToken {
    type App = HooditApp;
    type Args = TokenArgs;
    const NAME: &'static str = "hoodit_get_token";
    const DESCRIPTION: &'static str = "Snapshot one Robinhood Chain token: launchpad stage (Pons curve progress or graduation), its most active pool's price, liquidity, FDV, age, volume and buy/sell counts, other pools, setup flags, the slippage tolerance a chat trade needs, and GoPlus contract security (honeypot, taxes, owner powers, top holders). Prices are observations, not executable quotes.";

    fn run(app: &HooditApp, args: TokenArgs, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let token = arg!(model::address(&args.token));
        let pool_id = arg!(args.pool_id.as_deref().map(model::pool_id).transpose());
        let rt = app.runtime()?;
        let mut call = Call::new(20);
        let dex = Dex::new(&rt);
        let pools = dex.token_pools(&call, &token).unwrap_or_else(|error| {
            call.note(format!("pool snapshot unavailable: {}", error.message));
            vec![]
        });
        // Pons and other launchpads graduate into Uniswap v4 pools, so a token
        // whose most active pool is v2/v3 needs no launchpad lookup (and the
        // scarce GeckoTerminal allowance is saved).
        let active = main_pool(&pools, &token);
        let (lifecycle, launch_pools) = if active.is_some_and(|p| p.kind != Some("v4")) {
            (
                Lifecycle {
                    stage: "none",
                    ..Default::default()
                },
                vec![],
            )
        } else {
            match Gecko::new(&rt).token(&call, &token) {
                Ok(found) => found,
                Err(error) if error.code == "NOT_FOUND" => (
                    Lifecycle {
                        stage: "none",
                        ..Default::default()
                    },
                    vec![],
                ),
                Err(error) => {
                    call.note(format!("launchpad stage unknown: {}", error.message));
                    (Lifecycle::unknown(), vec![])
                }
            }
        };
        let main = match &pool_id {
            Some(id) => match pools.iter().chain(&launch_pools).find(|p| &p.pool_id == id) {
                Some(pool) => Some(pool.clone()),
                None => {
                    return Ok(model::error(
                        "POOL_NOT_FOUND",
                        "that pool_id is not a known pool of this token",
                        false,
                    ));
                }
            },
            None if lifecycle.stage == "curve" => launch_pools
                .iter()
                .find(|p| p.kind == Some("curve"))
                .cloned(),
            None => main_pool(&pools, &token)
                .or_else(|| main_pool(&launch_pools, &token))
                .or_else(|| deepest_pool(&pools, &token))
                .cloned(),
        };
        let Some(mut main) = main else {
            if pools.is_empty() && launch_pools.is_empty() && call.notes.is_empty() {
                return Ok(model::error(
                    "NOT_FOUND",
                    "no indexed pool trades this contract on Robinhood Chain",
                    false,
                ));
            }
            return Ok(failure_from_notes(call));
        };
        if main.symbol.is_empty()
            && let Some(named) = pools
                .iter()
                .find(|p| p.token == token && !p.symbol.is_empty())
        {
            main.symbol = named.symbol.clone();
            main.name = named.name.clone();
        }
        let security = GoPlus::new(&rt)
            .security(&call, &token)
            .unwrap_or_else(|error| {
                call.note(format!("contract security unknown: {}", error.message));
                Value::Null
            });
        let now = now();
        let mut others: Vec<&Snapshot> =
            pools.iter().filter(|p| p.pool_id != main.pool_id).collect();
        others.sort_by(|a, b| {
            b.liquidity_usd
                .unwrap_or(0.0)
                .total_cmp(&a.liquidity_usd.unwrap_or(0.0))
        });
        let other_pools: Vec<Value> = others
            .iter()
            .take(4)
            .map(|p| json!({"pool_id":p.pool_id,"venue":p.venue,"pair":p.pair(),"liquidity_usd":opt(p.liquidity_usd, usd),"volume_24h_usd":opt(p.volume.h24, usd)}))
            .collect();
        let profile = pools.iter().any(|p| p.token == token && p.profile);
        let mut flags = flags(&main, now);
        if (!profile || main.parked_liquidity()) && !main.symbol.is_empty() {
            flags.extend(busier_namesake(&dex, &call, &main));
        }
        let out = json!({
            "token": {"address": token, "symbol": main.symbol, "name": main.name, "dexscreener_profile": profile},
            "launchpad": lifecycle.view(),
            "main_pool": main.view(now),
            "other_pools": other_pools,
            "flags": flags,
            "slippage": Slippage::of(&main, main.kind == Some("curve")).view(),
            "security": security,
            "links": dex.token_links(&call, &token),
        });
        Ok(model::ok(out, call.notes))
    }
}

fn failure_from_notes(call: Call) -> Value {
    failure(crate::providers::ProviderError::new(
        "UNAVAILABLE",
        call.notes.join("; "),
    ))
}
