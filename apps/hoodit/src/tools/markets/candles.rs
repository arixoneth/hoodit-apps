use super::normalization::{invalid_argument, normalize_pool_id, resolve_pool};
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
pub struct CandlesArgs {
    /// Exact Robinhood Chain ERC-20 0x contract address. Do not pass a symbol
    /// or name; resolve those with hoodit_search_tokens first.
    pub token: String,
    /// Optional opaque pool_id returned by a Hoodit result. Omit to use the
    /// token's indexed top pool. Candles cover only the selected pool.
    #[serde(default)]
    #[schemars(
        with = "Option<String>",
        length(min = 1, max = 200),
        pattern(r"^[A-Za-z0-9:_-]+$")
    )]
    pub pool_id: Option<String>,
    /// Candle width. Omit for 1h.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["1m", "5m", "15m", "1h", "4h", "12h", "1d"], "default" = "1h"))]
    pub interval: Option<String>,
    /// Exclusive historical cutoff as a Unix timestamp in whole UTC seconds.
    /// Omit to use the current time. Never pass milliseconds or a future time.
    #[serde(default)]
    #[schemars(with = "Option<i64>")]
    pub before: Option<i64>,
    /// Maximum provider candles before open-candle filtering. Omit for 100.
    #[serde(default)]
    #[schemars(with = "u16", range(min = 1, max = 1000), extend("default" = 100))]
    pub limit: Option<u16>,
    /// Include the current incomplete candle. Omit for false when only closed
    /// candles should be compared.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub include_open: Option<bool>,
}

pub struct GetCandles;

impl DynAomiTool for GetCandles {
    type App = HooditApp;
    type Args = CandlesArgs;
    const NAME: &'static str = "hoodit_get_candles";
    const DESCRIPTION: &'static str = "Read USD OHLCV history for one exact token contract in one selected pool. Requires a 0x contract address; timestamps are Unix seconds, and results are single-pool market history rather than wallet performance or an executable quote.";

    fn run(app: &HooditApp, args: CandlesArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let token = invalid_argument!(model::address(&args.token));
        let interval = args.interval.as_deref().unwrap_or("1h");
        let (timeframe, aggregate, width_seconds) = match interval {
            "1m" => ("minute", 1, 60),
            "5m" => ("minute", 5, 300),
            "15m" => ("minute", 15, 900),
            "1h" => ("hour", 1, 3600),
            "4h" => ("hour", 4, 14_400),
            "12h" => ("hour", 12, 43_200),
            "1d" => ("day", 1, 86_400),
            _ => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "unsupported candle interval",
                    false,
                ));
            }
        };
        let before = args.before.unwrap_or_else(|| Utc::now().timestamp());
        if before > Utc::now().timestamp() + 5 {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "before cannot be in the future",
                false,
            ));
        }
        let limit = args.limit.unwrap_or(100);
        if !(1..=1000).contains(&limit) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "limit must be 1 to 1000",
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
        let response = match gecko.candles(
            &selected_pool_id,
            &token,
            timeframe,
            aggregate,
            before,
            limit,
            &mut read,
        ) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let rows = model::get(&response, &["data", "attributes", "ohlcv_list"])
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let response_page_full = rows.len() == limit as usize;
        let earliest_timestamp = rows
            .iter()
            .filter_map(Value::as_array)
            .filter_map(|values| values.first())
            .filter_map(Value::as_i64)
            .min();
        let now = Utc::now().timestamp();
        let include_open = args.include_open.unwrap_or(false);
        let mut candles = rows
            .into_iter()
            .filter_map(|row| normalize_candle(row, before, now, width_seconds, include_open))
            .collect::<Vec<_>>();
        candles.sort_by_key(|candle| candle["timestamp"].as_i64());
        candles.dedup_by_key(|candle| candle["timestamp"].as_i64());

        let returned = candles.len();
        let has_gaps = candles.windows(2).any(|window| {
            window[1]["timestamp"]
                .as_i64()
                .zip(window[0]["timestamp"].as_i64())
                .is_some_and(|(next, previous)| next - previous > width_seconds)
        });
        let summary = candle_summary(&candles);
        let next_before = earliest_timestamp.filter(|_| response_page_full);
        let includes_open_candle = candles.iter().any(|candle| candle["closed"] == false);
        let mut warnings = read.warnings;
        if has_gaps {
            warnings.push(model::warning(
                "GAPS_IN_CANDLES",
                "The returned single-pool candle series contains gaps",
            ));
        }
        if next_before.is_some() {
            warnings.push(model::warning(
                "HISTORY_LIMITED",
                "Additional older single-pool history may be available",
            ));
        }
        Ok(model::ok(
            json!({"token":token,"pool":{"pool_id":selected_pool["pool_id"],"dex_id":selected_pool["dex_id"],"dex_name":selected_pool["dex_name"]},"interval":interval,"before":before,"currency":"USD","candles":candles,"summary":summary,"coverage":{"scope":"single_pool","requested_limit":limit,"returned":returned,"has_gaps":has_gaps,"includes_open_candle":includes_open_candle,"next_before":next_before}}),
            read.sources,
            warnings,
        ))
    }
}

fn normalize_candle(
    row: Value,
    before: i64,
    now: i64,
    width_seconds: i64,
    include_open: bool,
) -> Option<Value> {
    let values = row.as_array()?;
    let timestamp = values.first()?.as_i64()?;
    if timestamp >= before || (!include_open && timestamp + width_seconds > now) {
        return None;
    }
    Some(
        json!({"timestamp":timestamp,"open":json_lexeme(values.get(1)),"high":json_lexeme(values.get(2)),"low":json_lexeme(values.get(3)),"close":json_lexeme(values.get(4)),"volume_usd":json_lexeme(values.get(5)),"closed":timestamp+width_seconds<=now}),
    )
}

fn json_lexeme(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(string) => Some(string.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

fn candle_summary(candles: &[Value]) -> Option<Value> {
    use std::str::FromStr;

    let first = candles.first()?;
    let last = candles.last()?;
    let decimals = |key: &str| {
        candles
            .iter()
            .filter_map(|candle| model::string(candle, &[key]))
            .filter_map(|value| bigdecimal::BigDecimal::from_str(&value).ok())
            .collect::<Vec<_>>()
    };
    let highs = decimals("high");
    let lows = decimals("low");
    let volumes = decimals("volume_usd");
    let open = bigdecimal::BigDecimal::from_str(&model::string(first, &["open"])?).ok()?;
    let close = bigdecimal::BigDecimal::from_str(&model::string(last, &["close"])?).ok()?;
    let change_pct = if num_traits::Zero::is_zero(&open) {
        None
    } else {
        Some(
            (((close.clone() / open.clone()) - bigdecimal::BigDecimal::from(1))
                * bigdecimal::BigDecimal::from(100))
            .with_scale_round(36, bigdecimal::RoundingMode::HalfEven)
            .normalized()
            .to_plain_string(),
        )
    };
    Some(
        json!({"first_timestamp":first["timestamp"],"last_timestamp":last["timestamp"],"open":open.normalized().to_plain_string(),"high":highs.into_iter().max().map(|value|value.normalized().to_plain_string()),"low":lows.into_iter().min().map(|value|value.normalized().to_plain_string()),"close":close.normalized().to_plain_string(),"volume_usd":volumes.into_iter().sum::<bigdecimal::BigDecimal>().normalized().to_plain_string(),"change_pct":change_pct}),
    )
}
