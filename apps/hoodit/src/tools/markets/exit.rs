use super::normalization::{invalid_argument, response_rows};
use crate::{
    amount,
    app::{HooditApp, ReadContext},
    model,
    providers::{Gecko, Lifi, token_from_resource},
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use bigdecimal::BigDecimal;
use num_bigint::BigUint;
use num_traits::Zero;
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

/// LI.FI quotes need a sender; this neutral address is used when the caller
/// supplies none. Approvals and balances are therefore not evaluated.
const PLACEHOLDER_SENDER: &str = "0x000000000000000000000000000000000000dead";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitArgs {
    /// Exact Robinhood Chain ERC-20 0x contract address. Do not pass a symbol
    /// or name; resolve those with hoodit_search_tokens first.
    pub token: String,
    /// sell quotes selling `amount` of the token. round_trip quotes buying
    /// the token with `eth_amount` ETH, then selling the expected tokens
    /// straight back. Omit for sell.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["sell", "round_trip"], "default" = "sell"))]
    pub mode: Option<String>,
    /// Token amount to sell in whole token units as a decimal string, for
    /// example "2000000" or "0.5". Required for sell; ignored for round_trip.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub amount: Option<String>,
    /// Share of `amount` to sell in basis points: 5000 = 50%, 10000 = 100%.
    /// Sizing floors in atomic units. Omit for 10000.
    #[serde(default)]
    #[schemars(with = "u16", range(min = 1, max = 10000), extend("default" = 10000))]
    pub fraction_bps: Option<u16>,
    /// ETH to spend for round_trip as a decimal string, for example "0.05".
    /// Required for round_trip; ignored for sell.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub eth_amount: Option<String>,
    /// Asset received when selling: eth or usdg. Omit for eth.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["eth", "usdg"], "default" = "eth"))]
    pub receive: Option<String>,
    /// Optional 0x wallet the trade would come from. Omit to quote for a
    /// neutral sender; balances and approvals are then not evaluated.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub wallet_address: Option<String>,
}

pub struct CheckExit;

impl DynAomiTool for CheckExit {
    type App = HooditApp;
    type Args = ExitArgs;
    const NAME: &'static str = "hoodit_check_exit";
    const DESCRIPTION: &'static str = "Check whether a position can be exited at a specific size: a read-only LI.FI sell quote for an exact token amount (optionally a basis-point fraction of it), or a buy-then-sell round trip for an ETH amount. Returns route, expected and minimum output, USD values, loss, gas, and fees. Never prepares, signs, or broadcasts; the host's swap flow owns execution.";

    fn run(app: &HooditApp, args: ExitArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        let token = invalid_argument!(model::address(&args.token));
        let mode = args.mode.as_deref().unwrap_or("sell");
        if !["sell", "round_trip"].contains(&mode) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "mode must be sell or round_trip",
                false,
            ));
        }
        let receive = args.receive.as_deref().unwrap_or("eth");
        let (receive_id, receive_decimals) = match receive {
            "eth" => (model::NATIVE_SENTINEL, 18),
            "usdg" => (model::USDG, 6),
            _ => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "receive must be eth or usdg",
                    false,
                ));
            }
        };
        let fraction_bps = args.fraction_bps.unwrap_or(10_000);
        if !(1..=10_000).contains(&fraction_bps) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "fraction_bps must be 1 to 10000",
                false,
            ));
        }
        let wallet = invalid_argument!(
            args.wallet_address
                .as_deref()
                .map(model::address)
                .transpose()
        );
        let sender = wallet.clone().unwrap_or_else(|| PLACEHOLDER_SENDER.into());
        if mode == "sell" && args.amount.is_none() {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "sell requires amount in whole token units",
                false,
            ));
        }
        let eth_in = match (mode, args.eth_amount.as_deref()) {
            ("round_trip", Some(value)) => Some(invalid_argument!(amount::from_decimal(value, 18))),
            ("round_trip", None) => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "round_trip requires eth_amount",
                    false,
                ));
            }
            _ => None,
        };

        let runtime = app.runtime()?;
        let mut read = ReadContext::exit(false);
        let token_response = match Gecko::new(&runtime, &ctx).token(&token, &mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let resource = response_rows(&token_response)
            .into_iter()
            .next()
            .unwrap_or(Value::Null);
        let token_details = token_from_resource(&resource);
        let Some(decimals) = model::get(&token_details, &["decimals"])
            .and_then(Value::as_u64)
            .and_then(|decimals| u8::try_from(decimals).ok())
        else {
            return Ok(model::error(
                "TOKEN_NOT_INDEXED",
                "token decimals are unknown, so an exact amount cannot be sized",
                false,
            ));
        };
        let price_usd = model::string(&resource, &["attributes", "price_usd"]);
        let lifi = Lifi::new(&runtime);
        let mut warnings = vec![];

        let (buy, sell_size) = match eth_in {
            Some(eth_in) if eth_in.is_zero() => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "eth_amount must be greater than zero",
                    false,
                ));
            }
            Some(eth_in) => {
                let quote = lifi.quote(
                    &sender,
                    model::NATIVE_SENTINEL,
                    &token,
                    &eth_in.to_string(),
                    &mut read,
                );
                let quote = match quote {
                    Ok(quote) => quote,
                    Err(error) if error.code == "NOT_INDEXED" => {
                        warnings.push(model::warning(
                            "NO_ROUTE",
                            "LI.FI found no route to buy this token at this size",
                        ));
                        return Ok(model::ok(
                            data(
                                &token_details,
                                mode,
                                receive,
                                &sender,
                                wallet.is_some(),
                                no_route(&eth_in, 18),
                                Value::Null,
                                None,
                                None,
                            ),
                            read.sources,
                            warnings,
                        ));
                    }
                    Err(error) => return Ok(provider_error(error)),
                };
                let received = model::string(&quote, &["estimate", "toAmount"])
                    .and_then(|value| amount::atomic(&value).ok());
                let summary = summarize(&quote, &eth_in, 18, decimals);
                match received {
                    Some(received) => (summary, received),
                    None => {
                        return Ok(model::error(
                            "UPSTREAM_SCHEMA_CHANGED",
                            "LI.FI buy quote has no output amount",
                            false,
                        ));
                    }
                }
            }
            None => {
                let requested = invalid_argument!(amount::from_decimal(
                    args.amount.as_deref().unwrap_or("0"),
                    decimals
                ));
                (Value::Null, amount::fraction(&requested, fraction_bps))
            }
        };
        if sell_size.is_zero() {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "the sized sell amount floors to zero atomic units",
                false,
            ));
        }
        let mark_value_usd = price_usd
            .as_deref()
            .and_then(|price| BigDecimal::from_str(price).ok())
            .and_then(|price| {
                BigDecimal::from_str(&amount::format(&sell_size, decimals))
                    .ok()
                    .map(|size| (price * size).round(6).normalized().to_plain_string())
            });
        let sell = match lifi.quote(
            &sender,
            &token,
            receive_id,
            &sell_size.to_string(),
            &mut read,
        ) {
            Ok(quote) => summarize(&quote, &sell_size, decimals, receive_decimals),
            Err(error) if error.code == "NOT_INDEXED" => {
                warnings.push(model::warning(
                    "NO_ROUTE",
                    "LI.FI found no route to sell this token at this size; the exit is unproven",
                ));
                no_route(&sell_size, decimals)
            }
            Err(_) if !buy.is_null() => {
                warnings.push(model::warning(
                    "QUOTE_UNAVAILABLE",
                    "The buy leg was quoted but the sell leg could not be; the exit is unproven",
                ));
                no_route(&sell_size, decimals)
            }
            Err(error) => return Ok(provider_error(error)),
        };
        let round_trip_loss_pct = match (
            &buy,
            eth_in_of(&buy),
            model::string(&sell, &["expected_out", "atomic"]),
        ) {
            (Value::Object(_), Some(spent), Some(back)) if receive == "eth" => {
                loss_pct(&spent, &back)
            }
            _ => None,
        };
        Ok(model::ok(
            data(
                &token_details,
                mode,
                receive,
                &sender,
                wallet.is_some(),
                sell,
                buy,
                round_trip_loss_pct,
                mark_value_usd,
            ),
            read.sources,
            warnings,
        ))
    }
}

#[allow(clippy::too_many_arguments)]
fn data(
    token: &Value,
    mode: &str,
    receive: &str,
    sender: &str,
    wallet_supplied: bool,
    sell: Value,
    buy: Value,
    round_trip_loss_pct: Option<String>,
    mark_value_usd: Option<String>,
) -> Value {
    json!({
        "token":token,
        "mode":mode,
        "receive":receive,
        "sender":sender,
        "sender_kind":if wallet_supplied {"wallet"} else {"placeholder"},
        "buy":buy,
        "sell":sell,
        "round_trip_loss_pct":round_trip_loss_pct,
        "sell_mark_value_usd":mark_value_usd,
        "coverage":{"source":"lifi","slippage":"0.005","executable":false,"balances_checked":false,"approvals_checked":wallet_supplied}
    })
}

fn no_route(amount_in: &BigUint, decimals: u8) -> Value {
    json!({"route_found":false,"amount_in":{"atomic":amount_in.to_string(),"formatted":amount::format(amount_in, decimals)},"route":null,"expected_out":null,"min_out":null,"value_in_usd":null,"value_out_usd":null,"loss_pct":null,"gas_usd":null,"fees_usd":null,"approval_required":null})
}

fn summarize(quote: &Value, amount_in: &BigUint, in_decimals: u8, out_decimals: u8) -> Value {
    let estimate = quote.get("estimate").cloned().unwrap_or(json!({}));
    let out = |key: &str| {
        model::string(&estimate, &[key])
            .and_then(|value| amount::atomic(&value).ok())
            .map(|value| json!({"atomic":value.to_string(),"formatted":amount::format(&value, out_decimals)}))
    };
    let usd = |key: &str| {
        model::string(&estimate, &[key]).and_then(|value| BigDecimal::from_str(&value).ok())
    };
    let total = |key: &str| {
        let values = estimate
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|cost| model::string(cost, &["amountUSD"]))
            .filter_map(|value| BigDecimal::from_str(&value).ok())
            .collect::<Vec<_>>();
        (!values.is_empty()).then(|| {
            values
                .into_iter()
                .sum::<BigDecimal>()
                .normalized()
                .to_plain_string()
        })
    };
    let (value_in, value_out) = (usd("fromAmountUSD"), usd("toAmountUSD"));
    let loss = match (&value_in, &value_out) {
        (Some(value_in), Some(value_out)) if value_in > &BigDecimal::from(0) => Some(
            ((value_in - value_out) * BigDecimal::from(100) / value_in)
                .round(2)
                .normalized()
                .to_plain_string(),
        ),
        _ => None,
    };
    let from_native = model::string(quote, &["action", "fromToken", "address"])
        .is_some_and(|address| address.eq_ignore_ascii_case(model::NATIVE_SENTINEL));
    json!({
        "route_found":true,
        "amount_in":{"atomic":amount_in.to_string(),"formatted":amount::format(amount_in, in_decimals)},
        "route":model::string(quote,&["tool"]),
        "expected_out":out("toAmount"),
        "min_out":out("toAmountMin"),
        "value_in_usd":value_in.map(|value| value.normalized().to_plain_string()),
        "value_out_usd":value_out.map(|value| value.normalized().to_plain_string()),
        "loss_pct":loss,
        "gas_usd":total("gasCosts"),
        "fees_usd":total("feeCosts"),
        "approval_required":!from_native && model::string(&estimate,&["approvalAddress"]).is_some()
    })
}

fn eth_in_of(buy: &Value) -> Option<String> {
    model::string(buy, &["amount_in", "atomic"])
}

/// Round-trip loss in percent of the ETH spent, from exact atomic amounts.
fn loss_pct(spent: &str, back: &str) -> Option<String> {
    let spent = BigDecimal::from_str(spent).ok()?;
    let back = BigDecimal::from_str(back).ok()?;
    (spent > 0).then(|| {
        ((&spent - back) * BigDecimal::from(100) / spent)
            .round(2)
            .normalized()
            .to_plain_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_summary_reports_loss_costs_and_exact_amounts() {
        let quote = json!({
            "tool":"kyberswap",
            "action":{"fromToken":{"address":"0x1111111111111111111111111111111111111111"}},
            "estimate":{"toAmount":"359197303636875","toAmountMin":"357401317118691","fromAmountUSD":"1.0133","toAmountUSD":"0.9644","approvalAddress":"0xB477751B76CF82d00a686A1232f5fCD772414Af3","gasCosts":[{"amountUSD":"0.0189"}],"feeCosts":[{"amountUSD":"0.0025"}]}
        });
        let summary = summarize(
            &quote,
            &amount::atomic("1000000000000000000000").unwrap(),
            18,
            18,
        );
        assert_eq!(summary["route"], "kyberswap");
        assert_eq!(summary["amount_in"]["formatted"], "1000");
        assert_eq!(summary["expected_out"]["formatted"], "0.000359197303636875");
        assert_eq!(summary["loss_pct"], "4.83");
        assert_eq!(summary["gas_usd"], "0.0189");
        assert_eq!(summary["approval_required"], true);
    }

    #[test]
    fn round_trip_loss_uses_exact_eth_amounts() {
        assert_eq!(
            loss_pct("50000000000000000", "45000000000000000").as_deref(),
            Some("10")
        );
        assert_eq!(loss_pct("0", "1"), None);
    }
}
