use super::normalization::{invalid_argument, response_rows, validate_page};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::{Gecko, included_map, pool},
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use bigdecimal::BigDecimal;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{cmp::Ordering, str::FromStr};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenPoolsArgs {
    /// Exact Robinhood Chain ERC-20 contract address.
    pub token: String,
    /// Optional canonical DEX IDs returned by hoodit_get_market_options.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub dex_ids: Option<Vec<String>>,
    /// Sort within the scanned provider page. Omit for liquidity.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["liquidity", "volume_24h", "created_at", "price"], "default" = "liquidity"))]
    pub sort: Option<String>,
    /// Sort direction. Omit for descending.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["asc", "desc"], "default" = "desc"))]
    pub direction: Option<String>,
    /// One-based provider page, from 1 through 10.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 10), extend("default" = 1))]
    pub page: Option<u8>,
    /// Bypass short-lived provider caches. Omit for false.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub refresh: Option<bool>,
}

pub struct GetTokenPools;

impl DynAomiTool for GetTokenPools {
    type App = HooditApp;
    type Args = TokenPoolsArgs;
    const NAME: &'static str = "hoodit_get_token_pools";
    const DESCRIPTION: &'static str = "Compare GeckoTerminal-indexed pools containing one exact Robinhood Chain token, optionally narrowed to canonical DEX IDs. Rankings are observational within the scanned page and do not select an executable swap route.";

    fn run(app: &HooditApp, args: TokenPoolsArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let token = invalid_argument!(model::address(&args.token));
        let page = invalid_argument!(validate_page(args.page));
        let sort = args.sort.as_deref().unwrap_or("liquidity");
        if !["liquidity", "volume_24h", "created_at", "price"].contains(&sort) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "unsupported pool sort",
                false,
            ));
        }
        let direction = args.direction.as_deref().unwrap_or("desc");
        if !["asc", "desc"].contains(&direction) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "unsupported sort direction",
                false,
            ));
        }
        let dex_ids = args.dex_ids.unwrap_or_default();
        if dex_ids.len() > 20
            || dex_ids.iter().any(|id| {
                id.is_empty()
                    || id.len() > 100
                    || !id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            })
        {
            return Ok(model::error("INVALID_ARGUMENT", "invalid dex_ids", false));
        }

        let runtime = app.runtime()?;
        let mut read = ReadContext::markets(args.refresh.unwrap_or(false));
        let response = match Gecko::new(&runtime).token_pools_page(&token, page, &mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let included = included_map(&response);
        let rows = response_rows(&response);
        let page_full = rows.len() >= 20;
        let mut pools = rows
            .iter()
            .map(|row| pool(row, &included))
            .filter(|pool| {
                dex_ids.is_empty()
                    || model::string(pool, &["dex_id"])
                        .is_some_and(|id| dex_ids.iter().any(|allowed| allowed == &id))
            })
            .collect::<Vec<_>>();
        pools.sort_by(|left, right| compare_pools(left, right, sort));
        if direction == "desc" {
            pools.reverse();
        }
        let returned = pools.len();
        let next_page = (page < 10 && page_full).then_some(page + 1);
        Ok(model::ok(
            json!({
                "token":token,
                "dex_ids":dex_ids,
                "sort":sort,
                "direction":direction,
                "pools":pools,
                "pagination":{"page":page,"page_size":20,"returned":returned,"next_page":next_page},
                "coverage":{"scope":"provider_page","ranked_within_scanned_candidates":true,"executable_route":false}
            }),
            read.sources,
            read.warnings,
        ))
    }
}

fn compare_pools(left: &Value, right: &Value, sort: &str) -> Ordering {
    if sort == "created_at" {
        return model::string(left, &["created_at"]).cmp(&model::string(right, &["created_at"]));
    }
    let path: &[&str] = match sort {
        "volume_24h" => &["windows", "h24", "volume_usd"],
        "price" => &["base_price_usd"],
        _ => &["liquidity_usd"],
    };
    let decimal = |value: &Value| {
        model::string(value, path).and_then(|value| BigDecimal::from_str(&value).ok())
    };
    decimal(left)
        .partial_cmp(&decimal(right))
        .unwrap_or(Ordering::Equal)
        .then_with(|| model::string(left, &["pool_id"]).cmp(&model::string(right, &["pool_id"])))
}
