use super::{arg, failure};
use crate::amount;
use crate::app::{Call, HooditApp};
use crate::market::{MAX_SLIPPAGE_BPS, Slippage, main_pool};
use crate::model::{self, one};
use crate::providers::{dex::Dex, lifi::Lifi, rpc::Rpc};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use num_bigint::BigUint;
use num_traits::Zero;
use serde::Deserialize;
use serde_json::{Value, json};

/// LI.FI needs a sender; balances and approvals are not evaluated for it.
const PLACEHOLDER_SENDER: &str = "0x000000000000000000000000000000000000dead";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitArgs {
    /// Exact 0x token contract. Resolve tickers with hoodit_search first.
    pub token: String,
    /// sell: quote selling `amount` tokens. round_trip: quote buying with
    /// `eth_amount` ETH and selling the tokens straight back. Omit for sell.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["sell", "round_trip"], "default" = "sell"))]
    pub mode: Option<String>,
    /// Tokens to sell in whole units, e.g. "2000000" or "0.5". Required for sell.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub amount: Option<String>,
    /// Share of `amount` to sell in basis points (5000 = half). Omit for all.
    #[serde(default)]
    #[schemars(with = "u16", range(min = 1, max = 10000), extend("default" = 10000))]
    pub fraction_bps: Option<u16>,
    /// ETH to spend for round_trip, e.g. "0.05". Required for round_trip.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub eth_amount: Option<String>,
    /// Asset to receive when selling: eth or usdg. Omit for eth.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["eth", "usdg"], "default" = "eth"))]
    pub receive: Option<String>,
    /// Slippage tolerance in basis points (300 = 3%), 1 to 1000. Omit to use
    /// the tolerance Hoodit suggests for this coin.
    #[serde(default)]
    #[schemars(with = "Option<u32>", range(min = 1, max = 1000))]
    pub slippage_bps: Option<u32>,
    /// Optional 0x wallet the trade would come from; omit for a neutral sender.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub wallet_address: Option<String>,
}

pub struct CheckExit;

impl DynAomiTool for CheckExit {
    type App = HooditApp;
    type Args = ExitArgs;
    const NAME: &'static str = "hoodit_check_exit";
    const DESCRIPTION: &'static str = "Check whether a position can actually be exited at a given size with live LI.FI quotes: sell an exact token amount, or buy with an ETH amount and sell straight back (round_trip) to measure the total cost of getting in and out. Returns route, expected and minimum output at the slippage tolerance, loss, gas, and the tolerance a chat trade of this coin needs. Read-only: never prepares or signs a trade.";

    fn run(app: &HooditApp, args: ExitArgs, _ctx: DynToolCallCtx) -> Result<Value, String> {
        let token = arg!(model::address(&args.token));
        let mode = args.mode.as_deref().unwrap_or("sell");
        let receive = args.receive.as_deref().unwrap_or("eth");
        let (receive_id, receive_decimals) = match receive {
            "eth" => (model::NATIVE, 18),
            "usdg" => (model::USDG, 6),
            _ => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "receive must be eth or usdg",
                    false,
                ));
            }
        };
        let fraction = args.fraction_bps.unwrap_or(10_000);
        if !(1..=10_000).contains(&fraction) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "fraction_bps must be 1 to 10000",
                false,
            ));
        }
        let wallet = arg!(
            args.wallet_address
                .as_deref()
                .map(model::address)
                .transpose()
        );
        let sender = wallet.clone().unwrap_or_else(|| PLACEHOLDER_SENDER.into());
        let eth_in = match (mode, args.eth_amount.as_deref(), args.amount.as_deref()) {
            ("round_trip", Some(eth), _) => Some(arg!(amount::from_decimal(eth, 18))),
            ("round_trip", None, _) => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "round_trip requires eth_amount",
                    false,
                ));
            }
            ("sell", _, Some(_)) => None,
            ("sell", _, None) => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "sell requires amount in whole token units",
                    false,
                ));
            }
            _ => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "mode must be sell or round_trip",
                    false,
                ));
            }
        };
        if eth_in.as_ref().is_some_and(Zero::is_zero) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "eth_amount must be greater than zero",
                false,
            ));
        }
        if args
            .slippage_bps
            .is_some_and(|bps| !(1..=MAX_SLIPPAGE_BPS).contains(&bps))
        {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "slippage_bps must be 1 to 1000 (0.01% to 10%)",
                false,
            ));
        }
        let rt = app.runtime()?;
        let mut call = Call::new(25);
        let suggested = match Dex::new(&rt).token_pools(&call, &token) {
            Ok(pools) => {
                main_pool(&pools, &token).map(|p| Slippage::of(p, p.kind == Some("curve")))
            }
            Err(error) => {
                call.note(format!(
                    "slippage suggestion unavailable: {}",
                    error.message
                ));
                None
            }
        };
        let slippage_bps = args
            .slippage_bps
            .or_else(|| suggested.as_ref().map(Slippage::suggested_bps))
            .unwrap_or(model::FALLBACK_SLIPPAGE_BPS);
        let decimals = match Rpc::new(&rt).head_and_decimals(&call, &[&token]) {
            Ok((_, _, decimals)) => decimals[0],
            Err(error) => return Ok(failure(error)),
        };
        let lifi = Lifi::new(&rt);
        let (buy, size) = match &eth_in {
            Some(eth_in) => {
                match lifi.quote(
                    &call,
                    &sender,
                    model::NATIVE,
                    &token,
                    &eth_in.to_string(),
                    slippage_bps,
                ) {
                    Ok(quote) => {
                        let Some(received) = model::string(&quote, &["estimate", "toAmount"])
                            .and_then(|v| amount::atomic(&v).ok())
                        else {
                            return Ok(model::error(
                                "BAD_RESPONSE",
                                "LI.FI buy quote has no output amount",
                                false,
                            ));
                        };
                        (summarize(&quote, eth_in, 18, decimals), received)
                    }
                    Err(error) if error.code == "NO_ROUTE" => {
                        call.note("no route to buy at this size");
                        return Ok(model::ok(
                            json!({"token":token,"mode":mode,"buy":no_route(eth_in, 18),"sell":Value::Null}),
                            call.notes,
                        ));
                    }
                    Err(error) => return Ok(failure(error)),
                }
            }
            None => {
                let requested = arg!(amount::from_decimal(
                    args.amount.as_deref().unwrap_or("0"),
                    decimals
                ));
                (Value::Null, amount::fraction(&requested, fraction))
            }
        };
        if size.is_zero() {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "the sell size rounds to zero",
                false,
            ));
        }
        let sell = match lifi.quote(
            &call,
            &sender,
            &token,
            receive_id,
            &size.to_string(),
            slippage_bps,
        ) {
            Ok(quote) => summarize(&quote, &size, decimals, receive_decimals),
            Err(error) if error.code == "NO_ROUTE" || !buy.is_null() => {
                call.note(format!(
                    "the sell could not be quoted ({}): the exit is unproven",
                    error.message
                ));
                no_route(&size, decimals)
            }
            Err(error) => return Ok(failure(error)),
        };
        let round_trip_loss_pct = match (&eth_in, model::string(&sell, &["expected_out_atomic"])) {
            (Some(spent), Some(back)) if receive == "eth" => loss(spent, &back),
            _ => None,
        };
        let mut sell = sell;
        if let Some(object) = sell.as_object_mut() {
            object.remove("expected_out_atomic");
        }
        let mut buy = buy;
        if let Some(object) = buy.as_object_mut() {
            object.remove("expected_out_atomic");
        }
        Ok(model::ok(
            json!({
                "token": token,
                "mode": mode,
                "receive": receive,
                "sender": if wallet.is_some() { "wallet" } else { "neutral placeholder (balances and approvals not checked)" },
                "buy": buy,
                "sell": sell,
                "round_trip_loss_pct": round_trip_loss_pct,
                "slippage_bps": slippage_bps,
                "slippage": suggested.as_ref().map(Slippage::view),
                "executable": false,
            }),
            call.notes,
        ))
    }
}

fn no_route(amount_in: &BigUint, decimals: u8) -> Value {
    json!({"route_found": false, "amount_in": amount::format(amount_in, decimals)})
}

fn summarize(quote: &Value, amount_in: &BigUint, in_decimals: u8, out_decimals: u8) -> Value {
    let estimate = quote.get("estimate").cloned().unwrap_or(Value::Null);
    let out = |key: &str| model::string(&estimate, &[key]).and_then(|v| amount::atomic(&v).ok());
    let usd = |key: &str| model::number(&estimate, &[key]);
    let costs = |key: &str| {
        let values: Vec<f64> = estimate
            .get(key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|c| model::number(c, &["amountUSD"]))
            .collect();
        (!values.is_empty()).then(|| values.iter().sum::<f64>())
    };
    let (value_in, value_out) = (usd("fromAmountUSD"), usd("toAmountUSD"));
    let loss_pct = match (value_in, value_out) {
        (Some(i), Some(o)) if i > 0.0 => Some(one((i - o) / i * 100.0)),
        _ => None,
    };
    let from_native = model::string(quote, &["action", "fromToken", "address"])
        .is_some_and(|a| a == model::NATIVE);
    json!({
        "route_found": true,
        "route": model::string(quote, &["tool"]),
        "amount_in": amount::format(amount_in, in_decimals),
        "expected_out": out("toAmount").map(|v| amount::format(&v, out_decimals)),
        "expected_out_atomic": out("toAmount").map(|v| v.to_string()),
        "min_out": out("toAmountMin").map(|v| amount::format(&v, out_decimals)),
        "value_in_usd": value_in.map(model::usd),
        "value_out_usd": value_out.map(model::usd),
        "loss_pct": loss_pct,
        "gas_usd": costs("gasCosts").map(model::usd),
        "fees_usd": costs("feeCosts").map(model::usd),
        "approval_required": !from_native && model::string(&estimate, &["approvalAddress"]).is_some(),
    })
}

/// Round-trip loss as a percentage of the ETH spent, from exact amounts.
fn loss(spent: &BigUint, back: &str) -> Option<Value> {
    let back = amount::atomic(back).ok()?;
    let spent_f: f64 = spent.to_string().parse().ok()?;
    let back_f: f64 = back.to_string().parse().ok()?;
    (spent_f > 0.0).then(|| one((spent_f - back_f) / spent_f * 100.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_route_loss_and_costs() {
        let quote = json!({
            "tool":"kyberswap",
            "action":{"fromToken":{"address":"0x1111111111111111111111111111111111111111"}},
            "estimate":{"toAmount":"359197303636875","toAmountMin":"357401317118691","fromAmountUSD":"1.0133","toAmountUSD":"0.9644","approvalAddress":"0xB477751B76CF82d00a686A1232f5fCD772414Af3","gasCosts":[{"amountUSD":"0.0189"}],"feeCosts":[{"amountUSD":"0.0025"}]}
        });
        let s = summarize(
            &quote,
            &amount::atomic("1000000000000000000000").unwrap(),
            18,
            18,
        );
        assert_eq!(s["route"], "kyberswap");
        assert_eq!(s["amount_in"], "1000");
        assert_eq!(s["expected_out"], "0.000359197303636875");
        assert_eq!(s["loss_pct"], json!(4.8));
        assert_eq!(s["gas_usd"], json!(0.02));
        assert_eq!(s["approval_required"], true);
    }

    #[test]
    fn round_trip_loss_uses_exact_eth_amounts() {
        let spent = amount::atomic("50000000000000000").unwrap();
        assert_eq!(loss(&spent, "45000000000000000"), Some(json!(10.0)));
        assert_eq!(loss(&BigUint::zero(), "1"), None);
    }
}
