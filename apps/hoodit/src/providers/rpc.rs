//! Robinhood Chain public JSON-RPC: swap logs, block times, and transaction
//! senders. Batches count against the allowance item by item.

use super::{Body, ProviderError, fetch};
use crate::app::{Call, Runtime};
use crate::model;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::time::Duration;

const DAY: Duration = Duration::from_secs(86_400);
/// Largest batch the public endpoint reliably accepts in one request.
const BATCH: usize = 50;

pub struct Rpc<'a> {
    rt: &'a Runtime,
}

pub struct Tx {
    pub from: String,
    pub to: Option<String>,
}

impl<'a> Rpc<'a> {
    pub fn new(rt: &'a Runtime) -> Self {
        Self { rt }
    }

    fn call(
        &self,
        call: &Call,
        method: &str,
        params: Value,
        ttl: Option<Duration>,
    ) -> Result<Value, ProviderError> {
        let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        let response = fetch(
            self.rt,
            call,
            "rpc",
            &self.rt.origins.rpc,
            Body::Post(&body),
            1,
            ttl,
        )?;
        if let Some(error) = response.get("error") {
            let message = model::string(error, &["message"]).unwrap_or_default();
            let lower = message.to_ascii_lowercase();
            let code = if [
                "range",
                "spans",
                "more than",
                "too many",
                "exceeds",
                "limit exceeded",
            ]
            .iter()
            .any(|p| lower.contains(p))
            {
                "RANGE_TOO_LARGE"
            } else {
                "UNAVAILABLE"
            };
            return Err(ProviderError::new(
                code,
                format!("Robinhood RPC: {message}"),
            ));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Batched calls, each cached under its key for `ttl` (None: no cache).
    /// Failed items come back as null.
    pub fn batch(
        &self,
        call: &Call,
        items: &[(&str, Value, Option<String>)],
        ttl: Duration,
    ) -> Result<Vec<Value>, ProviderError> {
        let mut results = vec![Value::Null; items.len()];
        let missing: Vec<usize> = (0..items.len())
            .filter(
                |&i| match items[i].2.as_deref().and_then(|key| self.rt.cached(key)) {
                    Some(hit) => {
                        results[i] = hit;
                        false
                    }
                    None => true,
                },
            )
            .collect();
        // The fast endpoint first; whatever it drops goes to the official one.
        let mut missing = missing;
        let mut last_error = None;
        for (provider, origin) in [
            ("rpc-fast", &self.rt.origins.rpc_fast),
            ("rpc", &self.rt.origins.rpc),
        ] {
            for chunk in missing.chunks(BATCH) {
                let body = Value::Array(
                    chunk
                        .iter()
                        .map(|&i| json!({"jsonrpc":"2.0","id":i,"method":items[i].0,"params":items[i].1}))
                        .collect(),
                );
                let response = match fetch(
                    self.rt,
                    call,
                    provider,
                    origin,
                    Body::Post(&body),
                    chunk.len() as u32,
                    None,
                ) {
                    Ok(response) => response,
                    Err(error) => {
                        last_error = Some(error);
                        continue;
                    }
                };
                for item in response.as_array().into_iter().flatten() {
                    let (Some(i), Some(result)) =
                        (item.get("id").and_then(Value::as_u64), item.get("result"))
                    else {
                        continue;
                    };
                    let i = i as usize;
                    if i < results.len() && !result.is_null() {
                        if let Some(key) = &items[i].2 {
                            self.rt.store(key.clone(), result.clone(), ttl);
                        }
                        results[i] = result.clone();
                    }
                }
            }
            missing.retain(|&i| results[i].is_null());
            if missing.is_empty() {
                return Ok(results);
            }
        }
        match last_error {
            Some(error) if missing.len() == items.len() => Err(error),
            _ => Ok(results),
        }
    }

    /// Several small log windows in one request; failed windows are empty.
    pub fn logs_many(
        &self,
        call: &Call,
        address: &str,
        topics: &Value,
        windows: &[(u64, u64)],
    ) -> Result<Vec<Vec<Value>>, ProviderError> {
        let items: Vec<(&str, Value, Option<String>)> = windows
            .iter()
            .map(|(from, to)| {
                let filter = json!([{"address":address,"topics":topics,"fromBlock":format!("0x{from:x}"),"toBlock":format!("0x{to:x}")}]);
                ("eth_getLogs", filter, None)
            })
            .collect();
        Ok(self
            .batch(call, &items, Duration::ZERO)?
            .into_iter()
            .map(|r| r.as_array().cloned().unwrap_or_default())
            .collect())
    }

    pub fn logs(
        &self,
        call: &Call,
        address: &str,
        topics: Value,
        from: u64,
        to: u64,
    ) -> Result<Vec<Value>, ProviderError> {
        let filter = json!([{"address":address,"topics":topics,"fromBlock":format!("0x{from:x}"),"toBlock":format!("0x{to:x}")}]);
        let value = self.call(call, "eth_getLogs", filter, Some(Duration::from_secs(15)))?;
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    /// Unix timestamps of the given blocks. Logs on this chain carry no usable
    /// timestamp, so callers interpolate between these anchors.
    pub fn block_times(
        &self,
        call: &Call,
        blocks: &[u64],
    ) -> Result<HashMap<u64, i64>, ProviderError> {
        let items: Vec<_> = blocks
            .iter()
            .map(|b| {
                (
                    "eth_getBlockByNumber",
                    json!([format!("0x{b:x}"), false]),
                    Some(format!("rpc:block:{b}")),
                )
            })
            .collect();
        let results = self.batch(call, &items, DAY)?;
        Ok(blocks
            .iter()
            .zip(results)
            .filter_map(|(b, r)| Some((*b, hex_u64(r.get("timestamp")?)? as i64)))
            .collect())
    }

    pub fn transactions(
        &self,
        call: &Call,
        hashes: &[String],
    ) -> Result<HashMap<String, Tx>, ProviderError> {
        let items: Vec<_> = hashes
            .iter()
            .map(|h| {
                (
                    "eth_getTransactionByHash",
                    json!([h]),
                    Some(format!("rpc:tx:{h}")),
                )
            })
            .collect();
        let results = self.batch(call, &items, DAY)?;
        Ok(hashes
            .iter()
            .zip(results)
            .filter_map(|(h, r)| {
                let from = model::address(&model::string(&r, &["from"])?).ok()?;
                let to = model::string(&r, &["to"]).and_then(|t| model::address(&t).ok());
                Some((h.clone(), Tx { from, to }))
            })
            .collect())
    }

    pub fn receipts(
        &self,
        call: &Call,
        hashes: &[String],
    ) -> Result<HashMap<String, Value>, ProviderError> {
        let items: Vec<_> = hashes
            .iter()
            .map(|h| {
                (
                    "eth_getTransactionReceipt",
                    json!([h]),
                    Some(format!("rpc:receipt:{h}")),
                )
            })
            .collect();
        let results = self.batch(call, &items, DAY)?;
        Ok(hashes
            .iter()
            .cloned()
            .zip(results)
            .filter(|(_, r)| !r.is_null())
            .collect())
    }

    /// The chain head (number, timestamp) and ERC-20 decimals for several
    /// tokens, in one round trip. Native ETH is 18.
    pub fn head_and_decimals(
        &self,
        call: &Call,
        tokens: &[&str],
    ) -> Result<(u64, i64, Vec<u8>), ProviderError> {
        let mut items = vec![("eth_getBlockByNumber", json!(["latest", false]), None)];
        items.extend(tokens.iter().filter(|t| **t != model::NATIVE).map(|t| {
            (
                "eth_call",
                json!([{"to":t,"data":"0x313ce567"},"latest"]),
                Some(format!("rpc:decimals:{t}")),
            )
        }));
        let results = self.batch(call, &items, DAY)?;
        let head = &results[0];
        let (Some(number), Some(ts)) = (
            head.get("number").and_then(hex_u64),
            head.get("timestamp").and_then(hex_u64),
        ) else {
            return Err(ProviderError::new(
                "UNAVAILABLE",
                "Robinhood RPC returned no latest block",
            ));
        };
        let mut found = results[1..].iter();
        let decimals = tokens
            .iter()
            .map(|token| {
                if *token == model::NATIVE {
                    return Ok(18);
                }
                found
                    .next()
                    .and_then(hex_u64)
                    .and_then(|d| u8::try_from(d).ok())
                    .filter(|d| *d <= 36)
                    .ok_or_else(|| {
                        ProviderError::new("BAD_RESPONSE", "token does not report its decimals")
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((number, ts as i64, decimals))
    }
}

pub fn hex_u64(value: &Value) -> Option<u64> {
    let text = value
        .as_str()?
        .strip_prefix("0x")
        .filter(|t| !t.is_empty())?;
    let text = text.trim_start_matches('0');
    if text.is_empty() {
        return Some(0);
    }
    if text.len() > 16 {
        return None;
    }
    u64::from_str_radix(text, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::super::testing::{reply, server};
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn batches_match_results_by_id_and_cache_them() {
        let body = r#"[{"jsonrpc":"2.0","id":1,"result":{"timestamp":"0x20"}},{"jsonrpc":"2.0","id":0,"result":{"timestamp":"0x10"}}]"#;
        let (rt, count) = server(vec![reply("200 OK", body)]);
        let rpc = Rpc::new(&rt);
        let call = Call::new(10);
        let times = rpc.block_times(&call, &[100, 200]).unwrap();
        assert_eq!(times[&100], 16);
        assert_eq!(times[&200], 32);
        assert_eq!(rpc.block_times(&call, &[200]).unwrap()[&200], 32);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn oversized_log_ranges_are_classified() {
        let body = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"query spans 78164480 blocks, but only 10000000 are allowed"}}"#;
        let (rt, _) = server(vec![reply("200 OK", body)]);
        let error = Rpc::new(&rt)
            .logs(&Call::new(10), model::WETH, json!([]), 0, 1)
            .unwrap_err();
        assert_eq!(error.code, "RANGE_TOO_LARGE");
    }

    #[test]
    fn parses_hex_quantities() {
        assert_eq!(hex_u64(&json!("0x4a8a730")), Some(78161712));
        assert_eq!(hex_u64(&json!("0x0")), Some(0));
        assert_eq!(hex_u64(&json!("12")), None);
        // Empty return data (a call to a non-contract) is not zero.
        assert_eq!(hex_u64(&json!("0x")), None);
    }
}
