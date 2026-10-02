//! GoPlus token security: honeypot simulation, taxes, contract controls, and
//! top holders. Missing or empty values stay unknown, never a reassuring zero.

use super::{Body, ProviderError, fetch};
use crate::app::{Call, Runtime};
use crate::model::{self, CHAIN_ID, one};
use serde_json::{Map, Value, json};
use std::time::Duration;

/// Contracts that hold tokens on behalf of pools rather than as a trader.
const POOL_MANAGER: &str = "0x8366a39cc670b4001a1121b8f6a443a643e40951";
const BURN: [&str; 2] = [
    "0x000000000000000000000000000000000000dead",
    "0x0000000000000000000000000000000000000000",
];

pub struct GoPlus<'a> {
    rt: &'a Runtime,
}
impl<'a> GoPlus<'a> {
    pub fn new(rt: &'a Runtime) -> Self {
        Self { rt }
    }
    pub fn security(&self, call: &Call, token: &str) -> Result<Value, ProviderError> {
        let url = format!("{}/token_security/{CHAIN_ID}", self.rt.origins.goplus);
        let value = fetch(
            self.rt,
            call,
            "goplus",
            &url,
            Body::Get(&[("contract_addresses", token.to_string())]),
            1,
            Some(Duration::from_secs(300)),
        )?;
        value
            .get("result")
            .and_then(Value::as_object)
            .and_then(|result| {
                result
                    .iter()
                    .find(|(address, _)| address.eq_ignore_ascii_case(token))
            })
            .map(|(_, record)| summarize(record, token))
            .ok_or_else(|| {
                ProviderError::new(
                    "NOT_FOUND",
                    "GoPlus has no security record for this token yet",
                )
            })
    }
}

fn flag(record: &Value, key: &str) -> Value {
    match record.get(key) {
        Some(Value::String(v)) if v == "1" => json!(true),
        Some(Value::String(v)) if v == "0" => json!(false),
        Some(Value::Number(v)) => json!(v.as_u64() == Some(1)),
        _ => Value::Null,
    }
}

fn pct(record: &Value, key: &str) -> Value {
    model::number(record, &[key])
        .map(|v| one(v * 100.0))
        .unwrap_or(Value::Null)
}

pub fn summarize(record: &Value, token: &str) -> Value {
    let owner = model::string(record, &["owner_address"]).map(|o| o.to_ascii_lowercase());
    let renounced = owner.as_deref().map(|o| o.is_empty() || BURN.contains(&o));
    let lp_holders: Vec<String> = record
        .get("lp_holders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|h| model::string(h, &["address"]))
        .map(|a| a.to_ascii_lowercase())
        .collect();
    let mut top = vec![];
    let mut wallet_share = 0.0;
    for holder in record
        .get("holders")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(address) = model::string(holder, &["address"]).map(|a| a.to_ascii_lowercase())
        else {
            continue;
        };
        let share = model::number(holder, &["percent"]).unwrap_or(0.0) * 100.0;
        let kind = if address == POOL_MANAGER || lp_holders.contains(&address) {
            "pool"
        } else if BURN.contains(&address.as_str()) {
            "burn"
        } else if address == token {
            "token_contract"
        } else if holder.get("is_locked").and_then(Value::as_u64) == Some(1) {
            "locked"
        } else if holder.get("is_contract").and_then(Value::as_u64) == Some(1) {
            "contract"
        } else {
            "wallet"
        };
        if kind == "wallet" {
            wallet_share += share;
        }
        if top.len() < 5 {
            let mut row = Map::new();
            row.insert("address".into(), json!(address));
            row.insert("pct".into(), one(share));
            row.insert("kind".into(), json!(kind));
            if let Some(tag) = model::string(holder, &["tag"]).filter(|t| !t.is_empty()) {
                row.insert("tag".into(), json!(model::label(tag, 32)));
            }
            top.push(Value::Object(row));
        }
    }
    let taxes = |key: &str| match model::string(record, &[key]).filter(|t| !t.trim().is_empty()) {
        Some(_) => pct(record, key),
        None => Value::Null,
    };
    json!({
        "honeypot": flag(record, "is_honeypot"),
        "cannot_sell_all": flag(record, "cannot_sell_all"),
        "buy_tax_pct": taxes("buy_tax"),
        "sell_tax_pct": taxes("sell_tax"),
        "tax_changeable": flag(record, "slippage_modifiable"),
        "open_source": flag(record, "is_open_source"),
        "proxy": flag(record, "is_proxy"),
        "mintable": flag(record, "is_mintable"),
        "blacklist": flag(record, "is_blacklisted"),
        "pausable": flag(record, "transfer_pausable"),
        "hidden_owner": flag(record, "hidden_owner"),
        "owner_can_change_balance": flag(record, "owner_change_balance"),
        "owner_renounced": renounced,
        "creator_pct": pct(record, "creator_percent"),
        "holders": model::number(record, &["holder_count"]).map(|n| n as u64),
        "top10_wallets_pct": one(wallet_share),
        "top_holders": top,
        "source": "goplus"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_pool_holders_and_keeps_unknown_taxes_unknown() {
        let token = "0xa241395adcdf456f6dc04d1bc02b18e6f4c4052c";
        let record = json!({
            "is_honeypot":"0","buy_tax":"","sell_tax":"0.05","is_open_source":"1",
            "owner_address":"","creator_percent":"0.010000","holder_count":"11889",
            "holders":[
                {"address":"0x8366a39CC670B4001A1121B8F6A443A643e40951","is_contract":1,"percent":"0.075245"},
                {"address":"0x6868dDB8114Eb330e0c6D7Cac8D185a6b5c4433E","is_contract":0,"percent":"0.024555"},
                {"address":"0x000000000000000000000000000000000000dEaD","is_contract":0,"percent":"0.2"}
            ]
        });
        let s = summarize(&record, token);
        assert_eq!(s["honeypot"], false);
        assert_eq!(s["buy_tax_pct"], Value::Null);
        assert_eq!(s["sell_tax_pct"], json!(5.0));
        assert_eq!(s["owner_renounced"], true);
        assert_eq!(s["mintable"], Value::Null);
        assert_eq!(s["top_holders"][0]["kind"], "pool");
        assert_eq!(s["top_holders"][2]["kind"], "burn");
        assert_eq!(s["top10_wallets_pct"], json!(2.5));
        assert_eq!(s["holders"], 11889);
    }
}
