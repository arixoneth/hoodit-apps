//! Shared portfolio balance normalization and optional USDG valuation.

use crate::{
    amount,
    app::ReadContext,
    model,
    providers::{Blockscout, CoinGecko, Gecko, Lifi, ProviderError},
};
use bigdecimal::BigDecimal;
use chrono::Utc;
use num_traits::Zero;
use serde_json::{Value, json};
use std::str::FromStr;

pub(super) struct HoldingInput<'a> {
    pub(super) token_id: &'a str,
    pub(super) symbol: Option<&'a str>,
    pub(super) name: Option<&'a str>,
    pub(super) decimals: Option<u8>,
    pub(super) raw_balance: &'a str,
    pub(super) wallet: &'a str,
}

pub(super) fn normalize_inventory_holding(
    item: &Value,
    wallet: &str,
    include_quote: bool,
    quote_attempts: &mut usize,
    lifi: &Lifi,
    read: &mut ReadContext,
    warnings: &mut Vec<Value>,
) -> Result<Option<Value>, ProviderError> {
    let invalid = || ProviderError {
        code: "UPSTREAM_SCHEMA_CHANGED",
        message: "Blockscout returned an invalid inventory holding".into(),
        retryable: false,
    };
    let raw_balance = model::string(item, &["value"]).ok_or_else(invalid)?;
    let balance = amount::atomic(&raw_balance).map_err(|_| invalid())?;
    if balance.is_zero() {
        return Ok(None);
    }
    let token = item.get("token").ok_or_else(invalid)?;
    let token_id = model::string(token, &["address_hash"])
        .ok_or_else(invalid)
        .and_then(|token_id| model::address(&token_id).map_err(|_| invalid()))?;
    make_holding(
        HoldingInput {
            token_id: &token_id,
            symbol: model::string(token, &["symbol"]).as_deref(),
            name: model::string(token, &["name"]).as_deref(),
            decimals: model::string(token, &["decimals"]).and_then(|value| value.parse().ok()),
            raw_balance: &raw_balance,
            wallet,
        },
        include_quote,
        100,
        quote_attempts,
        lifi,
        read,
        warnings,
    )
    .map(Some)
}

pub(super) fn verify_usdg_token(
    blockscout: &Blockscout,
    read: &mut ReadContext,
) -> Result<(), ProviderError> {
    let info = blockscout.token_info(model::USDG, read)?;
    let address =
        model::string(&info, &["address_hash"]).and_then(|value| model::address(&value).ok());
    let decimals = model::string(&info, &["decimals"]).and_then(|value| value.parse::<u8>().ok());
    if address
        .as_deref()
        .is_some_and(|value| value.eq_ignore_ascii_case(model::USDG))
        && decimals == Some(6)
    {
        Ok(())
    } else {
        Err(ProviderError {
            code: "UPSTREAM_SCHEMA_CHANGED",
            message: "USDG identity or decimals could not be verified".into(),
            retryable: false,
        })
    }
}

pub(super) fn quote_token(verified: bool) -> Value {
    model::token(
        model::USDG,
        verified.then_some("USDG"),
        verified.then_some("Global Dollar"),
        verified.then_some(6),
        None,
    )
}

#[cfg(test)]
pub(super) fn sort_holdings(holdings: &mut [Value]) {
    use std::cmp::Ordering;
    holdings.sort_by(|left, right| {
        let value = |holding: &Value| {
            model::string(holding, &["valuation", "value_usdg"])
                .and_then(|value| bigdecimal::BigDecimal::from_str(&value).ok())
        };
        match (value(left), value(right)) {
            (Some(left), Some(right)) => right.partial_cmp(&left).unwrap_or(Ordering::Equal),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| {
            model::string(left, &["token", "id"]).cmp(&model::string(right, &["token", "id"]))
        })
    });
}

pub(super) fn sum_holding_values(holdings: &[Value]) -> Option<String> {
    let values = holdings
        .iter()
        .filter_map(|holding| model::string(holding, &["valuation", "value_usdg"]))
        .filter_map(|value| bigdecimal::BigDecimal::from_str(&value).ok())
        .collect::<Vec<_>>();
    if values.is_empty() {
        None
    } else {
        Some(
            values
                .into_iter()
                .sum::<bigdecimal::BigDecimal>()
                .normalized()
                .to_plain_string(),
        )
    }
}

pub(super) fn sum_market_values(holdings: &[Value]) -> Option<String> {
    sum_values(holdings, "value_usd")
}

fn sum_values(holdings: &[Value], field: &str) -> Option<String> {
    use std::str::FromStr;
    let values = holdings
        .iter()
        .filter_map(|holding| model::string(holding, &["valuation", field]))
        .filter_map(|value| bigdecimal::BigDecimal::from_str(&value).ok())
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| {
        values
            .into_iter()
            .sum::<bigdecimal::BigDecimal>()
            .normalized()
            .to_plain_string()
    })
}

pub(super) fn apply_market_valuations(
    holdings: &mut [Value],
    gecko: &Gecko,
    coingecko: &CoinGecko,
    read: &mut ReadContext,
    warnings: &mut Vec<Value>,
) {
    use std::collections::HashMap;
    use std::str::FromStr;

    let tokens = holdings
        .iter()
        .filter_map(|holding| model::string(holding, &["token", "id"]))
        .filter(|token| token != "native")
        .collect::<Vec<_>>();
    let mut prices = HashMap::<String, String>::new();
    for batch in tokens.chunks(30) {
        match gecko.token_prices(batch, read) {
            Ok(response) => {
                if let Some(values) = model::get(&response, &["data", "attributes", "token_prices"])
                    .and_then(Value::as_object)
                {
                    for (token, price) in values {
                        if let Some(price) = price
                            .as_str()
                            .map(str::to_string)
                            .or_else(|| price.as_number().map(ToString::to_string))
                        {
                            prices.insert(token.to_ascii_lowercase(), price);
                        }
                    }
                }
            }
            Err(_) => warnings.push(model::warning(
                "MARKET_PRICE_UNAVAILABLE",
                "A batch of token market prices could not be read",
            )),
        }
    }
    let native_price = if holdings
        .iter()
        .any(|holding| holding["token"]["id"] == "native")
    {
        coingecko
            .eth_price(read)
            .ok()
            .and_then(|response| model::string(&response, &["ethereum", "usd"]))
    } else {
        None
    };

    for holding in holdings {
        let token = model::string(holding, &["token", "id"]).unwrap_or_default();
        let price = if token == "native" {
            native_price.clone()
        } else {
            prices.get(&token.to_ascii_lowercase()).cloned()
        };
        let formatted = model::string(holding, &["balance", "formatted"]);
        let value = price
            .as_deref()
            .and_then(|price| BigDecimal::from_str(price).ok())
            .zip(
                formatted
                    .as_deref()
                    .and_then(|balance| BigDecimal::from_str(balance).ok()),
            )
            .map(|(price, balance)| (price * balance).normalized().to_plain_string());
        holding["valuation"] = match (price, value) {
            (Some(price), Some(value)) => json!({
                "status":"market",
                "currency":"USD",
                "unit_price_usd":price,
                "value_usd":value,
                "unit_price_usdg":null,
                "value_usdg":null,
                "reason":null,
                "quote":null
            }),
            _ if holding["token"]["decimals"].is_null() => json!({
                "status":"unpriced","currency":"USD","unit_price_usd":null,"value_usd":null,
                "unit_price_usdg":null,"value_usdg":null,"reason":"unknown_decimals","quote":null
            }),
            _ => json!({
                "status":"unpriced","currency":"USD","unit_price_usd":null,"value_usd":null,
                "unit_price_usdg":null,"value_usdg":null,"reason":"not_indexed_or_unavailable","quote":null
            }),
        };
    }
}

pub(super) fn add_allocations(holdings: &mut [Value], denominator_usd: Option<&str>) {
    use std::str::FromStr;
    let denominator = denominator_usd.and_then(|value| BigDecimal::from_str(value).ok());
    for holding in holdings {
        holding["allocation"] = match (
            model::string(holding, &["valuation", "value_usd"])
                .and_then(|value| BigDecimal::from_str(&value).ok()),
            denominator.as_ref(),
        ) {
            (Some(value), Some(denominator)) if !denominator.is_zero() => json!({
                "percentage":((value / denominator) * BigDecimal::from(100)).with_scale_round(8,bigdecimal::RoundingMode::HalfEven).normalized().to_plain_string(),
                "denominator_currency":"USD",
                "denominator_scope":"displayed_priced_holdings"
            }),
            _ => {
                json!({"percentage":null,"denominator_currency":"USD","denominator_scope":"displayed_priced_holdings"})
            }
        };
    }
}

pub(super) fn sort_holdings_by(holdings: &mut [Value], mode: &str) {
    if mode == "symbol" {
        holdings.sort_by(|left, right| {
            model::string(left, &["token", "symbol"])
                .unwrap_or_default()
                .to_ascii_lowercase()
                .cmp(
                    &model::string(right, &["token", "symbol"])
                        .unwrap_or_default()
                        .to_ascii_lowercase(),
                )
                .then_with(|| {
                    model::string(left, &["token", "id"])
                        .cmp(&model::string(right, &["token", "id"]))
                })
        });
    } else {
        use std::cmp::Ordering;
        use std::str::FromStr;
        holdings.sort_by(|left, right| {
            let value = |holding: &Value| {
                model::string(holding, &["valuation", "value_usd"])
                    .or_else(|| model::string(holding, &["valuation", "value_usdg"]))
                    .and_then(|value| bigdecimal::BigDecimal::from_str(&value).ok())
            };
            match (value(left), value(right)) {
                (Some(left), Some(right)) => right.partial_cmp(&left).unwrap_or(Ordering::Equal),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            }
            .then_with(|| {
                model::string(left, &["token", "id"]).cmp(&model::string(right, &["token", "id"]))
            })
        });
    }
}

pub(super) fn make_holding(
    input: HoldingInput<'_>,
    include_quote: bool,
    sample_bps: u16,
    quote_attempts: &mut usize,
    lifi: &Lifi,
    read: &mut ReadContext,
    warnings: &mut Vec<Value>,
) -> Result<Value, ProviderError> {
    let HoldingInput {
        token_id,
        symbol,
        name,
        decimals,
        raw_balance,
        wallet,
    } = input;
    let balance = amount::atomic(raw_balance).map_err(|_| ProviderError {
        code: "UPSTREAM_SCHEMA_CHANGED",
        message: "provider returned an invalid balance".into(),
        retryable: false,
    })?;
    let formatted = decimals.map(|d| amount::format(&balance, d));
    let valuation = if balance.is_zero() {
        json!({"status":"zero_balance","currency":null,"unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":"0","reason":null,"quote":null})
    } else if decimals.is_none() {
        warnings.push(model::warning(
            "UNKNOWN_DECIMALS",
            "Token decimals are unavailable; the raw balance is preserved",
        ));
        json!({"status":"unpriced","currency":null,"unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":"unknown_decimals","quote":null})
    } else if !include_quote {
        json!({"status":"not_requested","currency":null,"unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":null,"quote":null})
    } else if token_id.eq_ignore_ascii_case(model::USDG) {
        json!({"status":"quote_currency","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":"1","value_usdg":formatted,"reason":null,"quote":null})
    } else {
        let sample_amount = amount::fraction(&balance, sample_bps);
        if sample_amount.is_zero() {
            warnings.push(model::warning(
                "QUOTE_SIZE_ROUNDS_TO_ZERO",
                "Requested fraction rounds down to zero atomic units",
            ));
            json!({"status":"unpriced","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":"rounds_to_zero","quote":null})
        } else if *quote_attempts >= 20 {
            warnings.push(model::warning(
                "QUOTE_BUDGET_EXHAUSTED",
                "At most twenty non-USDG quotes are attempted per response",
            ));
            json!({"status":"unpriced","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":"budget_exhausted","quote":null})
        } else {
            *quote_attempts += 1;
            let from_token = if token_id == "native" {
                model::NATIVE_SENTINEL
            } else {
                token_id
            };
            match lifi.quote(wallet, from_token, &sample_amount.to_string(), read) {
                Ok(quote) => match validated_quote_amounts(&quote, decimals.unwrap()) {
                    Some((output_amount, minimum_output_amount)) => {
                        let (unit_price_usdg, value_usdg) = amount::extrapolate(
                            &balance,
                            &sample_amount,
                            &output_amount,
                            decimals.unwrap(),
                            6,
                        )
                        .unwrap();
                        json!({"status":"quoted","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":unit_price_usdg,"value_usdg":value_usdg,"reason":null,"quote":{"input_amount":{"atomic":sample_amount.to_string(),"formatted":decimals.map(|decimals|amount::format(&sample_amount,decimals))},"expected_output":{"atomic":output_amount.to_string(),"formatted":amount::format(&output_amount,6)},"minimum_output":{"atomic":minimum_output_amount.to_string(),"formatted":amount::format(&minimum_output_amount,6)},"route_name":model::string(&quote,&["tool"]),"gas_cost_usd":sum_gas_cost_usd(&quote),"quoted_at":Utc::now().to_rfc3339(),"slippage_bps":50,"preflighted":false}})
                    }
                    _ => {
                        warnings.push(model::warning(
                            "QUOTE_UNAVAILABLE",
                            "LI.FI returned an inconsistent quote",
                        ));
                        json!({"status":"unpriced","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":"quote_unavailable","quote":null})
                    }
                },
                Err(error) => {
                    let reason = if error.code == "RATE_LIMITED" {
                        "rate_limited"
                    } else if error.code == "NOT_INDEXED" {
                        "no_route"
                    } else if error.code == "QUOTE_BUDGET_EXHAUSTED" {
                        "budget_exhausted"
                    } else if error.code == "DEADLINE_EXCEEDED" {
                        "deadline_exceeded"
                    } else {
                        "quote_unavailable"
                    };
                    warnings.push(model::warning(
                        match reason {
                            "rate_limited" => "RATE_LIMITED",
                            "budget_exhausted" => "QUOTE_BUDGET_EXHAUSTED",
                            "deadline_exceeded" => "DEADLINE_EXCEEDED",
                            _ => "QUOTE_UNAVAILABLE",
                        },
                        "A requested holding valuation was unavailable",
                    ));
                    json!({"status":"unpriced","currency":"USDG","unit_price_usd":null,"value_usd":null,"unit_price_usdg":null,"value_usdg":null,"reason":reason,"quote":null})
                }
            }
        }
    };
    Ok(
        json!({"token":model::token(token_id,symbol,name,decimals,None),"balance":{"atomic":raw_balance,"formatted":formatted},"valuation":valuation}),
    )
}

fn validated_quote_amounts(
    quote: &Value,
    input_decimals: u8,
) -> Option<(num_bigint::BigUint, num_bigint::BigUint)> {
    let decimal = |path: &[&str]| {
        model::get(quote, path)
            .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
    };
    if decimal(&["action", "fromToken", "decimals"]) != Some(input_decimals.into())
        || decimal(&["action", "toToken", "decimals"]) != Some(6)
    {
        return None;
    }
    let output_amount = model::string(quote, &["estimate", "toAmount"])
        .and_then(|value| amount::atomic(&value).ok())?;
    let minimum_output_amount = model::string(quote, &["estimate", "toAmountMin"])
        .and_then(|value| amount::atomic(&value).ok())?;
    (minimum_output_amount <= output_amount).then_some((output_amount, minimum_output_amount))
}

fn sum_gas_cost_usd(quote: &Value) -> Option<String> {
    use std::str::FromStr;
    let values = model::get(quote, &["estimate", "gasCosts"])
        .and_then(Value::as_array)?
        .iter()
        .map(|cost| {
            model::string(cost, &["amountUSD"])
                .and_then(|value| bigdecimal::BigDecimal::from_str(&value).ok())
        })
        .collect::<Option<Vec<_>>>()?;
    (!values.is_empty()).then(|| {
        values
            .into_iter()
            .sum::<bigdecimal::BigDecimal>()
            .normalized()
            .to_plain_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{ProviderOrigins, Runtime};
    use aomi_sdk::DynToolCallCtx;
    use reqwest::blocking::Client;
    use std::collections::HashMap;

    fn fixture() -> (Runtime, DynToolCallCtx) {
        (
            Runtime::fixture(Client::new(), ProviderOrigins::default()),
            DynToolCallCtx {
                session_id: "test".into(),
                tool_name: "portfolio".into(),
                call_id: "1".into(),
                state_attributes: Default::default(),
                secrets: HashMap::new(),
            },
        )
    }

    #[test]
    fn malformed_inventory_balance_is_an_upstream_error() {
        let (runtime, ctx) = fixture();
        let lifi = Lifi::from_ctx(&runtime, &ctx);
        let mut read = ReadContext::portfolio(false);
        let mut warnings = vec![];
        let error = normalize_inventory_holding(&json!({"value":"not-an-integer","token":{"address_hash":"0x1111111111111111111111111111111111111111"}}), "0x3333333333333333333333333333333333333333", false, &mut 0, &lifi, &mut read, &mut warnings).unwrap_err();
        assert_eq!(error.code, "UPSTREAM_SCHEMA_CHANGED");
    }

    #[test]
    fn unknown_decimals_warn_even_without_quotes() {
        let (runtime, ctx) = fixture();
        let lifi = Lifi::from_ctx(&runtime, &ctx);
        let mut read = ReadContext::portfolio(false);
        let mut warnings = vec![];
        let holding = make_holding(
            HoldingInput {
                token_id: "0x1111111111111111111111111111111111111111",
                symbol: None,
                name: None,
                decimals: None,
                raw_balance: "1",
                wallet: "0x3333333333333333333333333333333333333333",
            },
            false,
            100,
            &mut 0,
            &lifi,
            &mut read,
            &mut warnings,
        )
        .unwrap();
        assert_eq!(holding["valuation"]["reason"], "unknown_decimals");
        assert_eq!(warnings[0]["code"], "UNKNOWN_DECIMALS");
    }

    #[test]
    fn quote_validation_checks_decimals_minimum_and_sums_gas() {
        let mut quote = json!({"action":{"fromToken":{"decimals":18},"toToken":{"decimals":6}},"estimate":{"toAmount":"100","toAmountMin":"90","gasCosts":[{"amountUSD":"0.1"},{"amountUSD":"0.2"}]}});
        assert!(validated_quote_amounts(&quote, 18).is_some());
        assert_eq!(sum_gas_cost_usd(&quote).as_deref(), Some("0.3"));
        quote["estimate"]["toAmountMin"] = json!("101");
        assert!(validated_quote_amounts(&quote, 18).is_none());
    }

    #[test]
    fn holdings_sort_priced_first_then_by_token() {
        let mut holdings = vec![
            json!({"token":{"id":"b"},"valuation":{"value_usdg":null}}),
            json!({"token":{"id":"c"},"valuation":{"value_usdg":"2"}}),
            json!({"token":{"id":"a"},"valuation":{"value_usdg":"2"}}),
            json!({"token":{"id":"d"},"valuation":{"value_usdg":"3"}}),
        ];
        sort_holdings(&mut holdings);
        assert_eq!(
            holdings
                .iter()
                .map(|value| value["token"]["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["d", "a", "c", "b"]
        );
    }

    #[test]
    fn allocations_use_only_the_disclosed_priced_denominator() {
        let mut holdings = vec![
            json!({"valuation":{"value_usd":"75"}}),
            json!({"valuation":{"value_usd":"25"}}),
            json!({"valuation":{"value_usd":null}}),
        ];
        add_allocations(&mut holdings, Some("100"));
        assert_eq!(holdings[0]["allocation"]["percentage"], "75");
        assert_eq!(holdings[1]["allocation"]["percentage"], "25");
        assert!(holdings[2]["allocation"]["percentage"].is_null());
    }
}
