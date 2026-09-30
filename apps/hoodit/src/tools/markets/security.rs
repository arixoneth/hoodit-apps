//! Provider-specific token-security normalization.
//!
//! Every field remains nullable and source-scoped. Unknown, absent, and empty
//! provider values never become a reassuring `false` or zero.

use crate::model;
use bigdecimal::BigDecimal;
use serde_json::{Map, Value, json};
use std::str::FromStr;

fn provider_bool(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(value) if value == "1" => Some(true),
        Value::String(value) if value == "0" => Some(false),
        Value::Number(value) if value.as_u64() == Some(1) => Some(true),
        Value::Number(value) if value.as_u64() == Some(0) => Some(false),
        _ => None,
    }
}

fn decimal(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(value) if !value.trim().is_empty() => BigDecimal::from_str(value)
            .ok()
            .map(|value| value.normalized().to_plain_string()),
        Value::Number(value) => BigDecimal::from_str(&value.to_string())
            .ok()
            .map(|value| value.normalized().to_plain_string()),
        _ => None,
    })
}

fn fraction_pct(value: Option<&Value>) -> Option<String> {
    decimal(value).and_then(|value| {
        BigDecimal::from_str(&value).ok().map(|value| {
            (value * BigDecimal::from(100))
                .normalized()
                .to_plain_string()
        })
    })
}

fn gecko_attributes(value: Option<&Value>) -> Option<&Value> {
    let value = value?;
    model::get(value, &["data", "attributes"])
        .or_else(|| value.get("attributes"))
        .or(Some(value))
}

fn goplus_token<'a>(value: Option<&'a Value>, token: &str) -> Option<&'a Value> {
    let result = value?.get("result")?.as_object()?;
    result
        .iter()
        .find(|(address, _)| address.eq_ignore_ascii_case(token))
        .map(|(_, value)| value)
}

fn gecko_honeypot(value: Option<&Value>) -> Option<bool> {
    match value? {
        Value::Bool(value) => Some(*value),
        Value::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
        _ => None,
    }
}

fn assessment(left: Option<bool>, right: Option<bool>) -> &'static str {
    match (left, right) {
        (Some(true), Some(false)) | (Some(false), Some(true)) => "conflicting",
        (Some(true), _) | (_, Some(true)) => "flagged",
        (Some(false), Some(false)) => "clear",
        (Some(false), None) | (None, Some(false)) => "clear_single_source",
        (None, None) => "unknown",
    }
}

fn gt_components(attributes: Option<&Value>) -> Value {
    let details = attributes
        .and_then(|attributes| attributes.get("gt_score_details"))
        .and_then(Value::as_object);
    let keys = ["pool", "transaction", "creation", "info", "holders"];
    Value::Object(
        keys.into_iter()
            .map(|key| {
                let value = details.and_then(|details| details.get(key));
                (key.to_string(), json!(decimal(value)))
            })
            .collect::<Map<_, _>>(),
    )
}

fn control(goplus: Option<&Value>, name: &str) -> Value {
    json!({"value":provider_bool(goplus.and_then(|value|value.get(name))),"source":"goplus"})
}

pub(crate) fn normalize_security(
    token: &str,
    gecko: Option<&Value>,
    goplus: Option<&Value>,
    level: &str,
    include_holders: bool,
) -> (Value, Value, Value) {
    if level == "none" {
        return (
            json!({"status":"not_requested"}),
            json!({"status":"not_requested"}),
            json!({"requested_level":"none","missing":[],"sources":[]}),
        );
    }

    let gt = gecko_attributes(gecko);
    let gp = goplus_token(goplus, token);
    let gt_honeypot = gecko_honeypot(gt.and_then(|value| value.get("is_honeypot")));
    let gp_honeypot = provider_bool(gp.and_then(|value| value.get("is_honeypot")));
    let gt_verified = provider_bool(gt.and_then(|value| value.get("gt_verified")));
    let open_source = provider_bool(gp.and_then(|value| value.get("is_open_source")));

    let controls = if level == "full" {
        json!({
            "proxy":control(gp,"is_proxy"),
            "mintable":control(gp,"is_mintable"),
            "pausable":control(gp,"transfer_pausable"),
            "blacklist":control(gp,"is_blacklisted"),
            "whitelist":control(gp,"is_whitelisted"),
            "modifiable_tax":control(gp,"slippage_modifiable"),
            "hidden_owner":control(gp,"hidden_owner"),
            "owner_change_balance":control(gp,"owner_change_balance"),
            "self_destruct":control(gp,"selfdestruct"),
            "external_call":control(gp,"external_call")
        })
    } else {
        Value::Null
    };

    let holders = gt.and_then(|value| value.get("holders"));
    let gt_holder_count = holders
        .and_then(|value| value.get("count"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    let gp_holder_count = gp
        .and_then(|value| value.get("holder_count"))
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
    let gt_top10 = holders
        .and_then(|value| value.get("distribution_percentage"))
        .and_then(|value| value.get("top_10"))
        .and_then(|value| decimal(Some(value)));
    let gp_top10 = gp
        .and_then(|value| value.get("holders"))
        .and_then(Value::as_array)
        .map(|holders| {
            holders
                .iter()
                .take(10)
                .filter_map(|holder| {
                    fraction_pct(holder.get("percent"))
                        .and_then(|value| BigDecimal::from_str(&value).ok())
                })
                .sum::<BigDecimal>()
                .normalized()
                .to_plain_string()
        })
        .filter(|value| value != "0");
    let top_holders = if include_holders {
        gp.and_then(|value| value.get("holders"))
            .and_then(Value::as_array)
            .map(|holders| {
                holders
                    .iter()
                    .take(10)
                    .map(|holder| {
                        json!({
                            "address":model::string(holder,&["address"]),
                            "balance":model::string(holder,&["balance"]),
                            "percentage":fraction_pct(holder.get("percent")),
                            "is_contract":provider_bool(holder.get("is_contract")),
                            "tag":model::string(holder,&["tag"])
                        })
                    })
                    .collect::<Vec<_>>()
            })
    } else {
        None
    };

    let security = json!({
        "status":if gt.is_some()||gp.is_some(){"available"}else{"unavailable"},
        "gt":{
            "score":decimal(gt.and_then(|value|value.get("gt_score"))),
            "components":gt_components(gt),
            "verified":gt_verified,
            "scope":"token_metadata"
        },
        "honeypot":{
            "assessment":assessment(gt_honeypot,gp_honeypot),
            "geckoterminal":gt_honeypot,
            "goplus":gp_honeypot
        },
        "taxes":{
            "buy_pct":fraction_pct(gp.and_then(|value|value.get("buy_tax"))),
            "sell_pct":fraction_pct(gp.and_then(|value|value.get("sell_tax"))),
            "source":"goplus"
        },
        "contract":{
            "open_source":open_source,
            "proxy":provider_bool(gp.and_then(|value|value.get("is_proxy"))),
            "controls":controls
        }
    });
    let ownership = json!({
        "status":if holders.is_some()||gp.is_some(){"available"}else{"unavailable"},
        "holder_count":{"geckoterminal":gt_holder_count,"goplus":gp_holder_count},
        "top10_concentration_pct":{"geckoterminal":gt_top10,"goplus":gp_top10},
        "creator":{"address":gp.and_then(|value|model::string(value,&["creator_address"])),"percentage":fraction_pct(gp.and_then(|value|value.get("creator_percent")))},
        "owner":{"address":gp.and_then(|value|model::string(value,&["owner_address"])),"percentage":fraction_pct(gp.and_then(|value|value.get("owner_percent")))},
        "top_holders":top_holders,
        "holder_data_updated_at":holders.and_then(|value|model::string(value,&["last_updated"]))
    });
    let mut missing = vec![];
    if gt.and_then(|value| value.get("gt_score")).is_none() {
        missing.push("gt_score");
    }
    if gt_honeypot.is_none() && gp_honeypot.is_none() {
        missing.push("honeypot");
    }
    if open_source.is_none() {
        missing.push("open_source");
    }
    if gt_holder_count.is_none() && gp_holder_count.is_none() {
        missing.push("holder_count");
    }
    let sources = [
        gt.is_some().then_some("geckoterminal"),
        gp.is_some().then_some("goplus"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    let coverage = json!({
        "requested_level":level,
        "holders_requested":include_holders,
        "sources":sources,
        "missing":missing
    });
    (security, ownership, coverage)
}

pub(crate) fn normalize_liquidity_security(
    token: &str,
    pool_id: Option<&str>,
    goplus: Option<&Value>,
) -> Value {
    let Some(pool_id) = pool_id else {
        return json!({"status":"unknown","scope":"no_selected_pool"});
    };
    let Some(pair) = goplus_token(goplus, token)
        .and_then(|token| token.get("dex"))
        .and_then(Value::as_array)
        .and_then(|pairs| {
            pairs.iter().find(|pair| {
                model::string(pair, &["pair"])
                    .or_else(|| model::string(pair, &["pair_address"]))
                    .is_some_and(|address| address.eq_ignore_ascii_case(pool_id))
            })
        })
    else {
        return json!({"status":"unknown","scope":"exact_selected_pool","pool_id":pool_id,"note":"No exact-pool lock evidence was returned; concentrated-liquidity positions may not use fungible LP tokens."});
    };
    let locked_holders = pair
        .get("lp_holders")
        .and_then(Value::as_array)
        .map(|holders| {
            holders
                .iter()
                .filter(|holder| provider_bool(holder.get("is_locked")) == Some(true))
                .map(|holder| {
                    json!({
                        "address":model::string(holder,&["address"]),
                        "percentage":fraction_pct(holder.get("percent")),
                        "unlock_time":model::string(holder,&["unlock_time"]),
                        "tag":model::string(holder,&["tag"])
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let locked_pct = locked_holders
        .iter()
        .filter_map(|holder| model::string(holder, &["percentage"]))
        .filter_map(|value| BigDecimal::from_str(&value).ok())
        .sum::<BigDecimal>()
        .normalized()
        .to_plain_string();
    json!({
        "status":if locked_holders.is_empty(){"unknown"}else{"available"},
        "scope":"exact_selected_pool",
        "pool_id":pool_id,
        "locked_percentage":if locked_holders.is_empty(){None}else{Some(locked_pct)},
        "locked_holders":locked_holders,
        "source":"goplus"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_string_booleans_remain_tri_state() {
        assert_eq!(provider_bool(Some(&json!("1"))), Some(true));
        assert_eq!(provider_bool(Some(&json!("0"))), Some(false));
        assert_eq!(provider_bool(Some(&json!(""))), None);
        assert_eq!(fraction_pct(Some(&json!("0.05"))).as_deref(), Some("5"));
        assert_eq!(fraction_pct(Some(&json!(""))), None);
    }

    #[test]
    fn provider_disagreement_is_preserved() {
        let gt =
            json!({"data":{"attributes":{"is_honeypot":"false","gt_score":80,"gt_verified":true}}});
        let gp = json!({"result":{"0x1111111111111111111111111111111111111111":{"is_honeypot":"1","is_open_source":"0"}}});
        let (security, _, _) = normalize_security(
            "0x1111111111111111111111111111111111111111",
            Some(&gt),
            Some(&gp),
            "summary",
            false,
        );
        assert_eq!(security["honeypot"]["assessment"], "conflicting");
        assert_eq!(security["contract"]["open_source"], false);
    }

    #[test]
    fn liquidity_lock_evidence_is_bound_to_the_exact_pool() {
        let gp = json!({"result":{"0x1111111111111111111111111111111111111111":{"dex":[
            {"pair":"0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","lp_holders":[{"address":"0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","is_locked":"1","percent":"0.25","unlock_time":"1800000000"}]},
            {"pair":"0xcccccccccccccccccccccccccccccccccccccccc","lp_holders":[{"is_locked":"1","percent":"0.9"}]}
        ]}}});
        let evidence = normalize_liquidity_security(
            "0x1111111111111111111111111111111111111111",
            Some("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            Some(&gp),
        );
        assert_eq!(evidence["locked_percentage"], "25");
        assert_eq!(evidence["locked_holders"].as_array().unwrap().len(), 1);
        let unknown = normalize_liquidity_security(
            "0x1111111111111111111111111111111111111111",
            Some("0xdddddddddddddddddddddddddddddddddddddddd"),
            Some(&gp),
        );
        assert_eq!(unknown["status"], "unknown");
    }
}
