//! GeckoTerminal: discovery feeds and launchpad stage only. Its shared public
//! allowance is about ten requests a minute, so every read is cached.

use super::{Body, ProviderError, fetch};
use crate::app::{Call, Runtime};
use crate::market::{Lifecycle, Snapshot, Win};
use crate::model::{self, NETWORK};
use serde_json::{Map, Value};
use std::time::Duration;

const NATIVE_ALIAS: &str = "0xeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

pub struct Gecko<'a> {
    rt: &'a Runtime,
}
impl<'a> Gecko<'a> {
    pub fn new(rt: &'a Runtime) -> Self {
        Self { rt }
    }
    fn get(
        &self,
        call: &Call,
        path: &str,
        query: &[(&str, String)],
        ttl: u64,
    ) -> Result<Value, ProviderError> {
        let url = format!("{}{path}", self.rt.origins.gecko);
        fetch(
            self.rt,
            call,
            "geckoterminal",
            &url,
            Body::Get(query),
            1,
            Some(Duration::from_secs(ttl)),
        )
    }
    /// One page (20 pools) of a ranked feed.
    pub fn feed(
        &self,
        call: &Call,
        feed: &str,
        window: &str,
    ) -> Result<Vec<Snapshot>, ProviderError> {
        let include = ("include", "base_token,quote_token,dex".to_string());
        let (path, query) = match feed {
            "new" => (format!("/networks/{NETWORK}/new_pools"), vec![include]),
            "launchpad" => (
                format!("/networks/{NETWORK}/dexes/pons-v2/pools"),
                vec![include, ("sort", "h24_tx_count_desc".into())],
            ),
            "volume" => (
                format!("/networks/{NETWORK}/pools"),
                vec![include, ("sort", "h24_volume_usd_desc".into())],
            ),
            _ => (
                format!("/networks/{NETWORK}/trending_pools"),
                vec![include, ("duration", window.to_string())],
            ),
        };
        let value = self.get(call, &path, &query, 60)?;
        let included = included(&value);
        Ok(value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|pool| snapshot(pool, &included))
            .collect())
    }
    /// Launchpad stage plus the token's indexed top pools.
    pub fn token(
        &self,
        call: &Call,
        token: &str,
    ) -> Result<(Lifecycle, Vec<Snapshot>), ProviderError> {
        let value = self.get(
            call,
            &format!("/networks/{NETWORK}/tokens/{token}"),
            &[("include", "top_pools".into())],
            120,
        )?;
        let data = value.get("data").unwrap_or(&Value::Null);
        let included = included(&value);
        let pools = value
            .get("included")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|row| row.get("type").and_then(Value::as_str) == Some("pool"))
            .filter_map(|pool| snapshot(pool, &included))
            .collect::<Vec<_>>();
        let symbol = model::label(
            model::string(data, &["attributes", "symbol"]).unwrap_or_default(),
            24,
        );
        let name = model::label(
            model::string(data, &["attributes", "name"]).unwrap_or_default(),
            48,
        );
        let pools: Vec<Snapshot> = pools
            .into_iter()
            .map(|mut pool| {
                if pool.token == token && pool.symbol.is_empty() {
                    pool.symbol = symbol.clone();
                    pool.name = name.clone();
                }
                pool
            })
            .collect();
        let mut lifecycle = lifecycle(data);
        lifecycle.curve_pool = pools
            .iter()
            .find(|pool| pool.kind == Some("curve"))
            .map(|pool| pool.pool_id.clone());
        // Some graduated tokens carry no launchpad record, but their pool sits
        // on Pons' graduated DEX; trust the pool.
        if lifecycle.stage == "none"
            && let Some(pool) = pools.iter().find(|p| p.venue.starts_with("pons graduated"))
        {
            lifecycle.stage = "graduated";
            lifecycle.destination_pool = Some(pool.pool_id.clone());
        }
        Ok((lifecycle, pools))
    }
    /// Launchpad stage for up to 30 tokens in one request.
    pub fn lifecycles(
        &self,
        call: &Call,
        tokens: &[String],
    ) -> Result<Vec<(String, Lifecycle)>, ProviderError> {
        let joined = tokens
            .iter()
            .take(30)
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        let value = self.get(
            call,
            &format!("/networks/{NETWORK}/tokens/multi/{joined}"),
            &[],
            120,
        )?;
        Ok(value
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|row| {
                Some((
                    model::address(&model::string(row, &["attributes", "address"])?).ok()?,
                    lifecycle(row),
                ))
            })
            .collect())
    }
}

fn included(value: &Value) -> Map<String, Value> {
    value
        .get("included")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| Some((model::string(row, &["id"])?, row.clone())))
        .collect()
}

fn strip(id: &str) -> &str {
    id.strip_prefix("robinhood_").unwrap_or(id)
}

pub fn lifecycle(token: &Value) -> Lifecycle {
    let Some(details) =
        model::get(token, &["attributes", "launchpad_details"]).filter(|d| d.is_object())
    else {
        return Lifecycle {
            stage: "none",
            ..Default::default()
        };
    };
    let completed = details.get("completed").and_then(Value::as_bool);
    Lifecycle {
        stage: match completed {
            Some(true) => "graduated",
            Some(false) => "curve",
            None => "unknown",
        },
        progress_pct: model::number(details, &["graduation_percentage"])
            .filter(|_| completed == Some(false)),
        graduated_at: model::string(details, &["completed_at"]).filter(|_| completed == Some(true)),
        destination_pool: model::string(details, &["migrated_destination_pool_address"])
            .and_then(|pool| model::pool_id(&pool).ok())
            .filter(|_| completed == Some(true)),
        curve_pool: None,
    }
}

/// Venue label and log layout for a GeckoTerminal DEX id.
fn venue(dex: &str) -> (String, Option<&'static str>) {
    match dex {
        "pons-v2" => ("pons curve".into(), Some("curve")),
        "pons-v2-dex" => ("pons graduated (uniswap v4)".into(), Some("v4")),
        "pons-dot-family" => ("pons v1".into(), None),
        other => {
            let kind = ["v4", "v3", "v2"].into_iter().find(|v| other.contains(v));
            (other.trim_end_matches("-robinhood").replace('-', " "), kind)
        }
    }
}

pub fn snapshot(pool: &Value, included: &Map<String, Value>) -> Option<Snapshot> {
    let attrs = pool.get("attributes")?;
    let pool_id = model::pool_id(
        &model::string(attrs, &["address"])
            .or_else(|| model::string(pool, &["id"]).map(|id| strip(&id).to_string()))?,
    )
    .ok()?;
    let related = |name: &str| model::string(pool, &["relationships", name, "data", "id"]);
    let token_of = |name: &str| -> Option<(String, String, String)> {
        let id = related(name)?;
        let address = model::address(strip(&id)).ok()?;
        // GeckoTerminal writes native ETH as 0xeee...e.
        let address = if address == NATIVE_ALIAS {
            model::NATIVE.to_string()
        } else {
            address
        };
        let row = included.get(&id);
        let text = |key: &str| {
            row.and_then(|r| model::string(r, &["attributes", key]))
                .unwrap_or_default()
        };
        let symbol = match text("symbol") {
            known if !known.is_empty() => known,
            _ if address == model::NATIVE => "ETH".into(),
            _ if address == model::WETH => "WETH".into(),
            _ if address == model::USDG => "USDG".into(),
            _ => String::new(),
        };
        Some((
            address,
            model::label(symbol, 24),
            model::label(text("name"), 48),
        ))
    };
    let (token, symbol, name) = token_of("base_token")?;
    let (quote, quote_symbol, _) = token_of("quote_token")?;
    let (venue, kind) = venue(&related("dex").unwrap_or_default());
    // Hook launchpads (Bankr, Clanker, ...) are Uniswap v4 pools with 32-byte ids.
    let kind = kind.or((pool_id.len() == 66).then_some("v4"));
    let count = |side: &'static str| {
        move |w: &str| model::get(attrs, &["transactions", w, side]).and_then(Value::as_u64)
    };
    Some(Snapshot {
        pool_id,
        venue,
        kind,
        token,
        symbol,
        name,
        quote,
        quote_symbol,
        price_usd: model::number(attrs, &["base_token_price_usd"]),
        quote_usd: model::number(attrs, &["quote_token_price_usd"]),
        liquidity_usd: model::number(attrs, &["reserve_in_usd"]),
        fdv_usd: model::number(attrs, &["fdv_usd"]),
        mcap_usd: model::number(attrs, &["market_cap_usd"]),
        created: model::string(attrs, &["pool_created_at"])
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
            .map(|t| t.timestamp()),
        change: Win::from(|w| model::number(attrs, &["price_change_percentage", w])),
        volume: Win::from(|w| model::number(attrs, &["volume_usd", w])),
        buys: Win::from(count("buys")),
        sells: Win::from(count("sells")),
        // DexScreener profiles are read from DexScreener pools.
        profile: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn lifecycle_distinguishes_curve_graduation_and_no_record() {
        let curve = lifecycle(
            &json!({"attributes":{"launchpad_details":{"graduation_percentage":50.26,"completed":false}}}),
        );
        assert_eq!(curve.stage, "curve");
        assert_eq!(curve.progress_pct, Some(50.26));
        let destination = "0xB749D03A2000E8189FDC9412507AC60F83CCA2AAC351157CC6A139CFD7055705";
        let graduated = lifecycle(
            &json!({"attributes":{"launchpad_details":{"graduation_percentage":100.0,"completed":true,"completed_at":"2026-09-07T16:17:38Z","migrated_destination_pool_address":destination}}}),
        );
        assert_eq!(graduated.stage, "graduated");
        assert_eq!(graduated.progress_pct, None);
        assert_eq!(
            graduated.destination_pool.as_deref(),
            Some(destination.to_ascii_lowercase().as_str())
        );
        assert_eq!(lifecycle(&json!({"attributes":{}})).stage, "none");
    }

    #[test]
    fn maps_a_pons_curve_row() {
        let token = "robinhood_0xab8e09eaa576a7db1dbe9cd1fb524298b82f49fe";
        let weth = format!("robinhood_{}", model::WETH);
        let pool = json!({
            "id":"robinhood_0xfac731f80f22152b0cb3e5ccd2a7c9d22d53afb7",
            "attributes":{"address":"0xfac731f80f22152b0cb3e5ccd2a7c9d22d53afb7","reserve_in_usd":"6729.88","pool_created_at":"2026-10-01T10:00:00Z",
                "price_change_percentage":{"h1":"-3.1","h24":"45"},"transactions":{"h1":{"buys":12,"sells":9}},"volume_usd":{"h24":"155562.4"}},
            "relationships":{"base_token":{"data":{"id":token}},"quote_token":{"data":{"id":weth}},"dex":{"data":{"id":"pons-v2"}}}
        });
        let included = included(
            &json!({"included":[{"id":token,"attributes":{"symbol":"SCRAMBLE","name":"Scramble"}}]}),
        );
        let s = snapshot(&pool, &included).unwrap();
        assert_eq!(s.kind, Some("curve"));
        assert_eq!(s.venue, "pons curve");
        assert_eq!(s.symbol, "SCRAMBLE");
        assert_eq!(s.quote, model::WETH);
        assert_eq!(s.change.h1, Some(-3.1));
        assert_eq!(s.buys.h1, Some(12));
        assert_eq!(
            venue("uniswap-v3-robinhood"),
            ("uniswap v3".into(), Some("v3"))
        );
    }
}
