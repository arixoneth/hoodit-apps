use super::normalization::{invalid_argument, response_rows, validate_page};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::{Gecko, included_map, pool, token_from_resource},
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashSet;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchArgs {
    /// Token name, ticker symbol, or exact 0x contract address to search for.
    /// A name or symbol can return multiple candidates and must not be treated
    /// as an exact token identity.
    pub query: String,
    /// One-based GeckoTerminal search-results page. Omit for page 1; use the
    /// returned next_page value for another page.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 10), extend("default" = 1))]
    pub page: Option<u8>,
}

pub struct SearchTokens;

impl DynAomiTool for SearchTokens {
    type App = HooditApp;
    type Args = SearchArgs;
    const NAME: &'static str = "hoodit_search_tokens";
    const DESCRIPTION: &'static str = "Search Robinhood Chain tokens by name, symbol, or exact 0x contract address. Use this before exact-token tools when the user supplied only a name or symbol; return candidates and never guess among ambiguous matches.";

    fn run(app: &HooditApp, args: SearchArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let query = args.query.trim();
        if query.is_empty() || query.len() > 100 {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "query must contain 1 to 100 characters",
                false,
            ));
        }
        let page = invalid_argument!(validate_page(args.page));
        let runtime = app.runtime()?;
        let mut read = ReadContext::markets(false);
        if let Ok(exact_address) = model::address(query) {
            let hit = match Gecko::new(&runtime).token(&exact_address, &mut read) {
                Ok(response) => response_rows(&response).into_iter().next().map(|row| {
                    json!({"token":token_from_resource(&row),"match":"address","reference_pool":null,"reference_price_usd":model::string(&row,&["attributes","price_usd"]),"reference_pool_liquidity_usd":null})
                }),
                Err(error) if matches!(error.code, "NOT_INDEXED" | "TOKEN_NOT_INDEXED") => None,
                Err(error) => return Ok(provider_error(error)),
            };
            let mut warnings = read.warnings;
            if hit.is_none() {
                warnings.push(model::warning(
                    "NOT_INDEXED",
                    "The exact contract is not indexed by GeckoTerminal",
                ));
            }
            let tokens = hit.into_iter().collect::<Vec<_>>();
            let returned = tokens.len();
            return Ok(model::ok(
                json!({"query":query,"tokens":tokens,"pagination":{"page":1,"page_size":20,"returned":returned,"next_page":null}}),
                read.sources,
                warnings,
            ));
        }

        let response = match Gecko::new(&runtime).search(query, page, &mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let included = included_map(&response);
        let rows = response_rows(&response);
        let response_page_full = rows.len() >= 20;
        let mut seen_token_ids = HashSet::new();
        let mut hits = vec![];
        for row in rows {
            let pool = pool(&row, &included);
            for side in ["base_token", "quote_token"] {
                if let Some(token) = pool.get(side).filter(|value| !value.is_null()) {
                    let token_id = model::string(token, &["id"]).unwrap_or_default();
                    if seen_token_ids.insert(token_id.clone()) {
                        let symbol = model::string(token, &["symbol"]).unwrap_or_default();
                        let name = model::string(token, &["name"]).unwrap_or_default();
                        let match_kind = if token_id.eq_ignore_ascii_case(query) {
                            "address"
                        } else if symbol.eq_ignore_ascii_case(query) {
                            "symbol"
                        } else if name.eq_ignore_ascii_case(query) {
                            "name"
                        } else {
                            "pair"
                        };
                        hits.push(json!({"token":token,"match":match_kind,"reference_pool":{"pool_id":pool["pool_id"],"dex_id":pool["dex_id"],"dex_name":pool["dex_name"]},"reference_price_usd":if side=="base_token"{pool["base_price_usd"].clone()}else{pool["quote_price_usd"].clone()},"reference_pool_liquidity_usd":pool["liquidity_usd"]}));
                    }
                }
            }
        }
        hits.sort_by_key(|hit| match model::string(hit, &["match"]).as_deref() {
            Some("address") => 0,
            Some("symbol") => 1,
            Some("name") => 2,
            _ => 3,
        });
        hits.truncate(40);

        let returned = hits.len();
        let next_page = if page < 10 && response_page_full {
            Some(page + 1)
        } else {
            None
        };
        let mut warnings = read.warnings;
        if page == 10 && response_page_full {
            warnings.push(model::warning(
                "PARTIAL_PAGE",
                "The search reached its page limit while more results may exist",
            ));
        }
        Ok(model::ok(
            json!({"query":query,"tokens":hits,"pagination":{"page":page,"page_size":20,"returned":returned,"next_page":next_page}}),
            read.sources,
            warnings,
        ))
    }
}
