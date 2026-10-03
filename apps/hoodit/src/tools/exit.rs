use super::{codex_id, exec};
use crate::amount;
use crate::app::{Call, HooditApp, Runtime, Ttl};
use crate::providers::{self, Fail, NATIVE, Quote, codex, lifi_quote};
use crate::shape::{self, num, ok, pct, usd};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use num_bigint::BigUint;
use serde::Deserialize;
use serde_json::{Value, json};

/// Quotes come from this address when the user has no wallet connected.
/// LI.FI prices a route without moving funds; it only needs an address.
const NEUTRAL: &str = "0x0bf3e6f5ef3d32dccce208a1dd8fa9251893dd91";
const THIN_EXIT_PCT: f64 = 10.0;

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Buy with eth_amount (or usd_amount) and sell straight back: the cost of a round trip.
    RoundTrip,
    /// Sell token_amount (or usd_amount worth) for ETH.
    Sell,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExitArgs {
    /// Exact 0x token contract.
    pub token: String,
    /// round_trip or sell. null = round_trip when eth_amount or usd_amount is set, else sell.
    pub mode: Option<Mode>,
    /// ETH to buy with for a round trip, e.g. "0.1". null = use usd_amount.
    pub eth_amount: Option<String>,
    /// Tokens to sell, in whole units, e.g. "250000". null = use usd_amount.
    pub token_amount: Option<String>,
    /// Size in USD instead; converted at the current price. null = not used.
    pub usd_amount: Option<f64>,
    /// Slippage tolerance in percent for the quotes. null = 10.
    pub slippage_pct: Option<f64>,
}

pub struct Exit;

fn leg(result: &Result<Quote, Fail>) -> Value {
    match result {
        Ok(q) => {
            json!({ "status": "quoted", "route": q.route, "gas_usd": usd(q.gas_usd), "fees_usd": usd(q.fee_usd) })
        }
        Err(f) if f.code == "NO_ROUTE" => json!({ "status": "no_route", "why": f.message }),
        Err(f) => {
            json!({ "status": "unavailable", "why": f.message, "retry_after_s": f.retry_after_s })
        }
    }
}

fn units(raw: &str, decimals: u8) -> Option<f64> {
    raw.parse::<f64>()
        .ok()
        .map(|v| v / 10f64.powi(decimals as i32))
}

/// Sells from the user's wallet if it holds enough, else the neutral
/// address, else (curve tokens only quote from a holder) a real holder.
async fn sell_quote(
    rt: &Runtime,
    call: &Call,
    token: &str,
    amount: &BigUint,
    slippage: f64,
) -> (Result<Quote, Fail>, &'static str) {
    let raw = amount.to_string();
    let wanted = raw.parse::<u128>().unwrap_or(u128::MAX);
    if let Some(w) = &call.wallet
        && providers::balance_of(rt, token, w)
            .await
            .is_some_and(|b| b >= wanted)
    {
        let quote = lifi_quote(rt, call, token, NATIVE, &raw, w, slippage).await;
        return (quote, "your wallet");
    }
    let first = lifi_quote(rt, call, token, NATIVE, &raw, NEUTRAL, slippage).await;
    if !matches!(&first, Err(f) if f.code == "NO_ROUTE") {
        return (first, "a neutral address");
    }
    let q = format!(
        "{{ h: holders(input: {{tokenId: \"{}\", limit: 8}}) {{ items {{ address }} }} }}",
        codex_id(token)
    );
    let Ok((data, _)) = codex(rt, call, &q, json!({}), Ttl::Minute).await else {
        return (first, "a neutral address");
    };
    for h in data
        .pointer("/h/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(addr) = h.get("address").and_then(Value::as_str) else {
            continue;
        };
        if providers::is_wallet(rt, addr).await
            && providers::balance_of(rt, token, addr)
                .await
                .is_some_and(|b| b >= wanted)
        {
            let quote = lifi_quote(rt, call, token, NATIVE, &raw, addr, slippage).await;
            return (
                quote,
                "a wallet holding the token (curve sells only quote from holders)",
            );
        }
    }
    (first, "a neutral address")
}

impl DynAomiTool for Exit {
    type App = HooditApp;
    type Args = ExitArgs;
    const NAME: &'static str = "hoodit_exit";
    const DESCRIPTION: &'static str = "Can a size get out? Live LI.FI quotes (the router the host trades through) on Robinhood Chain. round_trip: buy with eth_amount and sell the tokens straight back; loss_pct is ETH lost, fees and price impact included, gas reported separately. sell: ETH received for token_amount. Each leg reports quoted, no_route or unavailable and its route; thin_exit is true above 10% loss. Quotes, not fills: independent legs at current prices.";

    fn run(app: &HooditApp, args: ExitArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let slippage = (args.slippage_pct.unwrap_or(10.0).clamp(0.5, 30.0)) / 100.0;
            let mode = args.mode.unwrap_or(if args.token_amount.is_some() {
                Mode::Sell
            } else {
                Mode::RoundTrip
            });
            let Some(decimals) = providers::decimals(&rt, &token).await else {
                return shape::error(
                    "UNAVAILABLE",
                    "could not read the token's decimals",
                    Some(10),
                );
            };
            // USD sizing needs prices: one bundled request for ETH and the token.
            let (eth_usd, token_usd) = if args.usd_amount.is_some() {
                let q = format!(
                    "{{ s: filterTokens(tokens: [\"{}\", \"{}\"], limit: 2) {{ results {{ priceUSD token {{ address }} }} }} }}",
                    codex_id(&token),
                    codex_id("0x0bd7d308f8e1639fab988df18a8011f41eacad73")
                );
                match codex(&rt, &call, &q, json!({}), Ttl::Live).await {
                    Ok((d, _)) => {
                        let price_of = |a: &str| {
                            d.pointer("/s/results")
                                .and_then(Value::as_array)
                                .and_then(|rs| {
                                    rs.iter()
                                        .find(|r| {
                                            r.pointer("/token/address")
                                                .and_then(Value::as_str)
                                                .is_some_and(|x| x.eq_ignore_ascii_case(a))
                                        })
                                        .and_then(|r| r.get("priceUSD").and_then(num))
                                })
                        };
                        (
                            price_of("0x0bd7d308f8e1639fab988df18a8011f41eacad73"),
                            price_of(&token),
                        )
                    }
                    Err(fail) => return fail.to_value(),
                }
            } else {
                (None, None)
            };
            match mode {
                Mode::RoundTrip => {
                    let eth = match (&args.eth_amount, args.usd_amount, eth_usd) {
                        (Some(e), _, _) => e.clone(),
                        (None, Some(u), Some(p)) if p > 0.0 => format!("{:.6}", u / p),
                        _ => {
                            return shape::error(
                                "INVALID_ARGUMENT",
                                "round_trip needs eth_amount or usd_amount",
                                None,
                            );
                        }
                    };
                    let wei = super::arg!(amount::from_decimal(&eth, 18));
                    let from = call.wallet.clone().unwrap_or_else(|| NEUTRAL.to_string());
                    let buy = lifi_quote(
                        &rt,
                        &call,
                        NATIVE,
                        &token,
                        &wei.to_string(),
                        &from,
                        slippage,
                    )
                    .await;
                    let (sell, received, sold_from) = match &buy {
                        Ok(b) => {
                            let got = b.to_amount.parse::<BigUint>().unwrap_or_default();
                            let (s, from) = sell_quote(&rt, &call, &token, &got, slippage).await;
                            (Some(s), Some(got), Some(from))
                        }
                        Err(_) => (None, None, None),
                    };
                    let eth_in = units(&wei.to_string(), 18);
                    let eth_back = sell
                        .as_ref()
                        .and_then(|s| s.as_ref().ok())
                        .and_then(|q| units(&q.to_amount, 18));
                    let loss = match (eth_in, eth_back) {
                        (Some(i), Some(b)) if i > 0.0 => Some((1.0 - b / i) * 100.0),
                        _ => None,
                    };
                    if sell
                        .as_ref()
                        .is_some_and(|s| matches!(s, Err(f) if f.code == "NO_ROUTE"))
                    {
                        call.gap("no sell route at this size: buying would be a trap");
                    }
                    ok(
                        json!({
                            "token": token,
                            "mode": "round_trip",
                            "eth_in": eth,
                            "usd_in": usd(eth_usd.zip(eth_in).map(|(p, e)| p * e)),
                            "tokens_bought": received.map(|r| amount::format(&r, decimals)),
                            "eth_back": eth_back.map(|b| shape::sig(b, 4)),
                            "loss_pct": pct(loss),
                            "thin_exit": loss.map(|l| l > THIN_EXIT_PCT),
                            "buy": leg(&buy),
                            "sell": sell.as_ref().map(leg),
                            "buy_quoted_from": if call.wallet.is_some() { "your wallet" } else { "a neutral address" },
                            "sell_quoted_from": sold_from,
                            "note": "independent quotes at current prices; gas excluded from loss_pct",
                        }),
                        &call.gaps,
                    )
                }
                Mode::Sell => {
                    let size = match (&args.token_amount, args.usd_amount, token_usd) {
                        (Some(t), _, _) => super::arg!(amount::from_decimal(t, decimals)),
                        (None, Some(u), Some(p)) if p > 0.0 => {
                            super::arg!(amount::from_decimal(
                                &format!("{:.*}", decimals.min(6) as usize, u / p),
                                decimals
                            ))
                        }
                        _ => {
                            return shape::error(
                                "INVALID_ARGUMENT",
                                "sell needs token_amount or usd_amount",
                                None,
                            );
                        }
                    };
                    let (sell, sold_from) = sell_quote(&rt, &call, &token, &size, slippage).await;
                    let eth_out = sell.as_ref().ok().and_then(|q| units(&q.to_amount, 18));
                    let eth_min = sell.as_ref().ok().and_then(|q| units(&q.to_amount_min, 18));
                    let value_usd =
                        token_usd.and_then(|p| units(&size.to_string(), decimals).map(|t| t * p));
                    let impact = match (eth_out, eth_usd, value_usd) {
                        (Some(o), Some(p), Some(v)) if v > 0.0 => Some((1.0 - o * p / v) * 100.0),
                        _ => None,
                    };
                    ok(
                        json!({
                            "token": token,
                            "mode": "sell",
                            "tokens_in": amount::format(&size, decimals),
                            "eth_out": eth_out.map(|o| shape::sig(o, 4)),
                            "eth_min_out": eth_min.map(|o| shape::sig(o, 4)),
                            "loss_vs_price_pct": pct(impact),
                            "thin_exit": impact.map(|l| l > THIN_EXIT_PCT),
                            "sell": leg(&sell),
                            "quoted_from": sold_from,
                            "note": "a quote, not a fill",
                        }),
                        &call.gaps,
                    )
                }
            }
        })
    }
}
