use super::normalization::{
    decimal_at_least, invalid_argument, normalize_pool_id, resolve_pool, response_rows,
};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::Gecko,
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TradesArgs {
    /// Exact Robinhood Chain ERC-20 0x contract address. Do not pass a symbol
    /// or name; resolve those with hoodit_search_tokens first.
    pub token: String,
    /// Optional opaque pool_id returned by a Hoodit result. Omit to use the
    /// token's indexed top pool. Trades cover only the selected pool.
    #[serde(default)]
    #[schemars(
        with = "Option<String>",
        length(min = 1, max = 200),
        pattern(r"^[A-Za-z0-9:_-]+$")
    )]
    pub pool_id: Option<String>,
    /// Maximum trades to return after filtering, from 1 through 100. Omit for
    /// 20.
    #[serde(default)]
    #[schemars(with = "u16", range(min = 1, max = 100), extend("default" = 20))]
    pub limit: Option<u16>,
    /// Minimum per-trade USD volume as a non-negative decimal string, for
    /// example "1000". Omit for "0".
    #[serde(default)]
    #[schemars(with = "String", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"), extend("default" = "0"))]
    pub min_volume_usd: Option<String>,
    /// Side oriented to the requested token. Omit or use both for both sides.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["buy", "sell", "both"], "default" = "both"))]
    pub side: Option<String>,
}

pub struct GetTrades;

impl DynAomiTool for GetTrades {
    type App = HooditApp;
    type Args = TradesArgs;
    const NAME: &'static str = "hoodit_get_trades";
    const DESCRIPTION: &'static str = "Read recent public trades for one exact token contract in one selected pool, with buy/sell side normalized to that token. Requires a 0x contract address; this is public pool activity, not the user's personal history.";

    fn run(app: &HooditApp, args: TradesArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let token = invalid_argument!(model::address(&args.token));
        let limit = args.limit.unwrap_or(20);
        if !(1..=100).contains(&limit) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "limit must be 1 to 100",
                false,
            ));
        }
        let min_volume_usd = invalid_argument!(model::decimal(
            args.min_volume_usd.as_deref().unwrap_or("0")
        ));
        if let Some(side) = args.side.as_deref()
            && !["buy", "sell", "both"].contains(&side)
        {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "side must be buy, sell, or both",
                false,
            ));
        }
        let explicit_pool_id =
            invalid_argument!(args.pool_id.as_deref().map(normalize_pool_id).transpose());
        let runtime = app.runtime()?;
        let gecko = Gecko::new(&runtime);
        let mut read = ReadContext::markets(false);
        let (selected_pool, selected_pool_id) =
            match resolve_pool(&gecko, &token, explicit_pool_id.as_deref(), &mut read) {
                Ok(selection) => selection,
                Err(error) => return Ok(provider_error(error)),
            };
        let response = match gecko.trades(&selected_pool_id, &token, &min_volume_usd, &mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let cutoff = Utc::now().timestamp() - 86_400;
        let observed_provider_rows = response_rows(&response);
        let mut trades = observed_provider_rows
            .into_iter()
            .map(|resource| normalize_trade(resource, &token))
            .filter(|trade| {
                trade["timestamp"]
                    .as_i64()
                    .is_some_and(|timestamp| timestamp >= cutoff)
                    && decimal_at_least(
                        model::string(trade, &["volume_usd"]).as_deref(),
                        &min_volume_usd,
                    )
                    && args
                        .side
                        .as_deref()
                        .is_none_or(|side| side == "both" || trade["side"] == side)
            })
            .collect::<Vec<_>>();
        trades.sort_by_key(|trade| {
            std::cmp::Reverse(trade["timestamp"].as_i64().unwrap_or_default())
        });
        let matched_before_limit = trades.len();
        trades.truncate(limit as usize);
        let returned = trades.len();
        let buys = trades.iter().filter(|trade| trade["side"] == "buy").count();
        let sells = trades
            .iter()
            .filter(|trade| trade["side"] == "sell")
            .count();
        let observed_volume_usd = {
            use std::str::FromStr;
            let values = trades
                .iter()
                .filter_map(|trade| model::string(trade, &["volume_usd"]))
                .filter_map(|value| bigdecimal::BigDecimal::from_str(&value).ok())
                .collect::<Vec<_>>();
            (!values.is_empty()).then(|| {
                values
                    .into_iter()
                    .sum::<bigdecimal::BigDecimal>()
                    .normalized()
                    .to_plain_string()
            })
        };
        Ok(model::ok(
            json!({"token":token,"pool":{"pool_id":selected_pool["pool_id"],"dex_id":selected_pool["dex_id"],"dex_name":selected_pool["dex_name"]},"side":args.side.as_deref().unwrap_or("both"),"trades":trades,"summary":{"scope":"returned_sample","buys":buys,"sells":sells,"volume_usd":observed_volume_usd,"unique_traders":null},"coverage":{"scope":"single_pool","lookback_seconds":86400,"provider_trade_cap":300,"matched_before_limit":matched_before_limit,"returned":returned,"complete_history":false}}),
            read.sources,
            read.warnings,
        ))
    }
}

fn normalize_trade(resource: Value, requested_token: &str) -> Value {
    let attributes = resource.get("attributes").cloned().unwrap_or(json!({}));
    let from_token = model::string(&attributes, &["from_token_address"]).unwrap_or_default();
    let to_token = model::string(&attributes, &["to_token_address"]).unwrap_or_default();
    let (side, token_amount, counter_token, counter_amount, price_usd) =
        if to_token.eq_ignore_ascii_case(requested_token) {
            (
                "buy",
                model::string(&attributes, &["to_token_amount"]),
                from_token,
                model::string(&attributes, &["from_token_amount"]),
                model::string(&attributes, &["price_to_in_usd"]),
            )
        } else if from_token.eq_ignore_ascii_case(requested_token) {
            (
                "sell",
                model::string(&attributes, &["from_token_amount"]),
                to_token,
                model::string(&attributes, &["to_token_amount"]),
                model::string(&attributes, &["price_from_in_usd"]),
            )
        } else {
            ("unknown", None, String::new(), None, None)
        };
    let timestamp = model::string(&attributes, &["block_timestamp"])
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|timestamp| timestamp.timestamp());
    json!({"id":model::string(&resource,&["id"]),"tx_hash":model::string(&attributes,&["tx_hash"]),"timestamp":timestamp,"side":side,"token_amount":token_amount,"counter_token":if counter_token.is_empty(){None}else{Some(counter_token)},"counter_amount":counter_amount,"price_usd":price_usd,"volume_usd":model::string(&attributes,&["volume_in_usd"])})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn side_and_amount_are_oriented_to_the_requested_token() {
        let token = "0x1111111111111111111111111111111111111111";
        let counter = "0x2222222222222222222222222222222222222222";
        let buy = normalize_trade(
            json!({"attributes":{"to_token_address":token,"from_token_address":counter,"to_token_amount":"2","from_token_amount":"1","price_to_in_usd":"0.5"}}),
            token,
        );
        assert_eq!(buy["side"], "buy");
        assert_eq!(buy["token_amount"], "2");
        let sell = normalize_trade(
            json!({"attributes":{"from_token_address":token,"to_token_address":counter,"from_token_amount":"3","to_token_amount":"1","price_from_in_usd":"0.4"}}),
            token,
        );
        assert_eq!(sell["side"], "sell");
        assert_eq!(sell["token_amount"], "3");
    }
}
