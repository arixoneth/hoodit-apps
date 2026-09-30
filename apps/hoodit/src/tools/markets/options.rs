use super::normalization::response_rows;
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::Gecko,
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarketOptionsArgs {
    /// Bypass the cached DEX capability list. Omit for false.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub refresh: Option<bool>,
}

pub struct GetMarketOptions;

impl DynAomiTool for GetMarketOptions {
    type App = HooditApp;
    type Args = MarketOptionsArgs;
    const NAME: &'static str = "hoodit_get_market_options";
    const DESCRIPTION: &'static str = "List current canonical Robinhood Chain DEX IDs plus Hoodit's available discovery filters, sorts, windows, and provider capability mode. Use before constructing a strict screen; it never requests credentials or performs a trade.";

    fn run(app: &HooditApp, args: MarketOptionsArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let runtime = app.runtime()?;
        let mut read = ReadContext::markets(args.refresh.unwrap_or(false));
        let response = match Gecko::new(&runtime).dexes(&mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let dexes = response_rows(&response)
            .into_iter()
            .filter_map(|resource| {
                let id = model::string(&resource, &["id"])?;
                Some(json!({
                    "id":id,
                    "name":model::string(&resource,&["attributes","name"])
                }))
            })
            .collect::<Vec<_>>();
        Ok(model::ok(
            json!({
                "network":{"id":model::NETWORK,"chain_id":model::CHAIN_ID},
                "dexes":dexes,
                "discovery":{
                    "mode":"free_paginated_scan",
                    "feeds":["trending","new","top_volume","top_activity","screened"],
                    "screened_source_feeds":["trending","new","top_volume","top_activity"],
                    "windows":["m5","m15","m30","h1","h6","h24"],
                    "sorts":["feed","liquidity","volume","volume_24h","transactions","buys","sells","created_at","price_change","fdv","market_cap","price"],
                    "filters":{
                        "pool":["dex_ids","paired_token_addresses","liquidity_usd","volume_usd","fdv_usd","market_cap_usd","price_usd","pool_age_hours","price_change_pct"],
                        "activity":["transactions","buys","sells","buyers","sellers"],
                        "activity_windows":["activity_window","transactions_window","buys_window","sells_window"],
                        "security":["min_gt_score","honeypot","max_buy_tax_pct","max_sell_tax_pct","require_gt_verified","require_open_source"],
                        "ownership_and_metadata":["holder_count","top10_concentration_pct","require_coingecko_listed","require_social_presence"]
                    },
                    "pagination":{"provider_page_size":20,"provider_page_max":10,"max_pages_per_call":3,"cursor_all_feeds":true,"query_bound":true,"resumes_within_page":true},
                    "enrichment":{"conditional":true,"default_requested_limit":8,"max_requested_limit":8,"effective_gecko_metadata_limit":"requested limit capped at 10 minus max_pages to reserve the shared GeckoTerminal call budget","cursor_preserves_unprocessed_candidates":true},
                    "market_cap_semantics":"Only explicit provider market_cap_usd values satisfy the filter; unknown market cap is excluded and FDV is never substituted.",
                    "paid_megafilter":{"configured":false,"validated":false,"note":"CoinGecko Pro Megafilter is not enabled in this build; free screening preserves the requested scope within disclosed scan bounds."}
                }
            }),
            read.sources,
            read.warnings,
        ))
    }
}
