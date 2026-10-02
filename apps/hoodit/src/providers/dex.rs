//! DexScreener: keyless pool snapshots (~300 requests/minute).

use super::{Body, ProviderError, fetch};
use crate::app::{Call, Runtime};
use crate::market::{Snapshot, Win};
use crate::model::{self, NETWORK};
use serde_json::Value;
use std::time::Duration;

const TTL: Duration = Duration::from_secs(20);

pub struct Dex<'a> {
    rt: &'a Runtime,
}
impl<'a> Dex<'a> {
    pub fn new(rt: &'a Runtime) -> Self {
        Self { rt }
    }
    fn get(
        &self,
        call: &Call,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.rt.origins.dexscreener);
        fetch(
            self.rt,
            call,
            "dexscreener",
            &url,
            Body::Get(query),
            1,
            Some(TTL),
        )
    }
    /// Every Robinhood pool containing the token, on either side.
    pub fn token_pools(&self, call: &Call, token: &str) -> Result<Vec<Snapshot>, ProviderError> {
        let value = self.get(call, &format!("/token-pairs/v1/{NETWORK}/{token}"), &[])?;
        Ok(snapshots(&value))
    }
    /// The project's registered website and socials (served from cache after
    /// `token_pools`).
    pub fn token_links(&self, call: &Call, token: &str) -> Vec<String> {
        self.get(call, &format!("/token-pairs/v1/{NETWORK}/{token}"), &[])
            .ok()
            .and_then(|value| {
                value
                    .as_array()
                    .and_then(|pairs| pairs.iter().map(links).find(|l| !l.is_empty()))
            })
            .unwrap_or_default()
    }
    /// Name or ticker search. The response is capped at 30 pairs across all
    /// chains, so the plain query and one scoped to the chain name are merged:
    /// each finds Robinhood pools the other misses.
    pub fn search(&self, call: &Call, query: &str) -> Result<Vec<Snapshot>, ProviderError> {
        let mut found: Vec<Snapshot> = vec![];
        let mut last_error = None;
        for q in [query.to_string(), format!("{query} {NETWORK}")] {
            match self.get(call, "/latest/dex/search", &[("q", q)]) {
                Ok(value) => {
                    for pool in snapshots(value.get("pairs").unwrap_or(&Value::Null)) {
                        if !found.iter().any(|f| f.pool_id == pool.pool_id) {
                            found.push(pool);
                        }
                    }
                }
                Err(error) => last_error = Some(error),
            }
        }
        match last_error {
            Some(error) if found.is_empty() => Err(error),
            _ => Ok(found),
        }
    }
}

fn snapshots(value: &Value) -> Vec<Snapshot> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pair| pair.get("chainId").and_then(Value::as_str) == Some(NETWORK))
        .filter_map(snapshot)
        .collect()
}

pub fn snapshot(pair: &Value) -> Option<Snapshot> {
    let pool_id = model::pool_id(&model::string(pair, &["pairAddress"])?).ok()?;
    let token = model::address(&model::string(pair, &["baseToken", "address"])?).ok()?;
    let quote = model::address(&model::string(pair, &["quoteToken", "address"])?).ok()?;
    let label = pair
        .get("labels")
        .and_then(Value::as_array)
        .and_then(|labels| {
            labels
                .iter()
                .filter_map(Value::as_str)
                .find(|l| l.starts_with('v'))
        })
        .map(str::to_string);
    let kind = match label.as_deref() {
        Some("v2") => Some("v2"),
        Some("v3") => Some("v3"),
        Some("v4") => Some("v4"),
        _ if pool_id.len() == 66 => Some("v4"),
        _ => None,
    };
    let dex = model::string(pair, &["dexId"]).unwrap_or_default();
    let price_usd = model::number(pair, &["priceUsd"]);
    let price_native = model::number(pair, &["priceNative"]);
    let quote_usd = match (price_usd, price_native) {
        (Some(usd), Some(native)) if native > 0.0 => Some(usd / native),
        _ if quote == model::USDG => Some(1.0),
        _ => None,
    };
    let count = |side: &'static str| {
        move |w: &str| model::get(pair, &["txns", w, side]).and_then(Value::as_u64)
    };
    Some(Snapshot {
        pool_id,
        venue: [dex, label.unwrap_or_default()]
            .join(" ")
            .trim()
            .to_string(),
        kind,
        token,
        symbol: model::label(
            model::string(pair, &["baseToken", "symbol"]).unwrap_or_default(),
            24,
        ),
        name: model::label(
            model::string(pair, &["baseToken", "name"]).unwrap_or_default(),
            48,
        ),
        quote,
        quote_symbol: model::label(
            model::string(pair, &["quoteToken", "symbol"]).unwrap_or_default(),
            24,
        ),
        price_usd,
        quote_usd,
        liquidity_usd: model::number(pair, &["liquidity", "usd"]),
        fdv_usd: model::number(pair, &["fdv"]),
        mcap_usd: model::number(pair, &["marketCap"]),
        created: model::number(pair, &["pairCreatedAt"]).map(|ms| (ms / 1000.0) as i64),
        change: Win::from(|w| model::number(pair, &["priceChange", w])),
        volume: Win::from(|w| model::number(pair, &["volume", w])),
        buys: Win::from(count("buys")),
        sells: Win::from(count("sells")),
    })
}

/// Website and social links, if the project registered any. Untrusted text.
pub fn links(pair: &Value) -> Vec<String> {
    let info = pair.get("info");
    let urls = |key: &str| {
        info.and_then(|i| i.get(key))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|entry| model::string(entry, &["url"]))
            .map(|url| model::label(url, 100))
            .collect::<Vec<_>>()
    };
    let mut links = urls("websites");
    links.extend(urls("socials"));
    links.truncate(4);
    links
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_a_v4_pair_and_derives_quote_price() {
        let pair = json!({
            "chainId":"robinhood","dexId":"uniswap","labels":["v4"],
            "pairAddress":"0xa32a077a3e9d1ce561ec2a4b136db701a3166195455da4d8d364d6189443afe1",
            "baseToken":{"address":"0xA241395aDCdF456F6Dc04d1Bc02B18e6f4c4052c","symbol":"ROBINPEPE","name":"Robin Pepe"},
            "quoteToken":{"address":"0x0000000000000000000000000000000000000000","symbol":"ETH"},
            "priceNative":"0.0000001565","priceUsd":"0.0004296",
            "txns":{"h1":{"buys":701,"sells":679},"h24":{"buys":21065,"sells":20354}},
            "volume":{"h24":1731759.21},"priceChange":{"h24":-12.23},
            "liquidity":{"usd":52057.12},"fdv":429611,"marketCap":429611,"pairCreatedAt":1790354039000u64
        });
        let s = snapshot(&pair).unwrap();
        assert_eq!(s.kind, Some("v4"));
        assert_eq!(s.token, "0xa241395adcdf456f6dc04d1bc02b18e6f4c4052c");
        assert_eq!(s.quote, model::NATIVE);
        assert_eq!(s.venue, "uniswap v4");
        assert!((s.quote_usd.unwrap() - 2745.0).abs() < 1.0);
        assert_eq!(s.buys.h1, Some(701));
        assert_eq!(s.txns_24h(), Some(41419));
        assert_eq!(s.created, Some(1790354039));
    }
}
