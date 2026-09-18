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
                    "mode":"free_bounded_scan",
                    "feeds":["trending","new","top_volume","top_activity","screened"],
                    "windows":["m5","m15","m30","h1","h6","h24"],
                    "sorts":["feed","liquidity","volume_24h","created_at","price_change"],
                    "security_filters":["fdv_usd","gt_score","honeypot","buy_tax","sell_tax","gt_verified","open_source","holder_count","top10_concentration","coingecko_listed","social_presence"],
                    "paid_megafilter":{"configured":false,"validated":false,"note":"CoinGecko Pro Megafilter is not enabled in this build; free screening preserves the requested scope within disclosed scan bounds."}
                }
            }),
            read.sources,
            read.warnings,
        ))
    }
}
