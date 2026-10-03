//! Transport and parsing only: Codex (paid per request over MPP), LI.FI
//! quotes, and Robinhood Chain reads for the Pons launch record.
use crate::app::{Call, Runtime, Ttl};
use mpp::client::Fetch;
use serde_json::{Value, json};
use std::time::Duration;

pub const NETWORK: u64 = 4663;
pub const NATIVE: &str = "0x0000000000000000000000000000000000000000";
pub const PONS_FACTORY: &str = "0x7ed598bcef8bd9edd8c97a195c6d13f40801ec7e";
const CODEX_URL: &str = "https://graph.codex.io/graphql";
const LIFI_URL: &str = "https://li.quest/v1/quote";
const RPCS: [&str; 2] = [
    "https://rpc.ordofi.network",
    "https://rpc.mainnet.chain.robinhood.com",
];

#[derive(Debug)]
pub struct Fail {
    pub code: &'static str,
    pub message: String,
    pub retry_after_s: Option<u64>,
}
impl Fail {
    fn new(code: &'static str, message: impl Into<String>, retry_after_s: Option<u64>) -> Self {
        Self {
            code,
            message: message.into(),
            retry_after_s,
        }
    }
    pub fn to_value(&self) -> Value {
        crate::shape::error(self.code, &self.message, self.retry_after_s)
    }
}

/// One paid Codex request. Several queries can be aliased into `query`;
/// they are billed once. Returns `data` plus a note if some fields errored.
pub async fn codex(
    rt: &Runtime,
    call: &Call,
    query: &str,
    variables: Value,
    ttl: Ttl,
) -> Result<(Value, Option<String>), Fail> {
    let key = format!("codex:{query}:{variables}");
    if let Some(hit) = rt.cached(&key) {
        return Ok((hit, None));
    }
    let Some(secret) = call.codex_key.as_deref() else {
        return Err(Fail::new(
            "UNCONFIGURED",
            "market data is not configured for this app",
            None,
        ));
    };
    if let Err(why) = rt.spend(&call.turn) {
        return match rt.stale(&key) {
            Some(old) => Ok((old, Some(format!("{why}; showing the last cached read")))),
            None => Err(Fail::new("BUDGET", why, None)),
        };
    }
    let provider = rt
        .tempo(secret)
        .map_err(|m| Fail::new("UNCONFIGURED", m, None))?;
    let body = json!({ "query": query, "variables": variables });
    let sent = rt
        .http
        .post(CODEX_URL)
        .header("X-Codex-Payment", "mpp")
        .json(&body)
        .send_with_payment(provider.as_ref())
        .await;
    let response = match sent {
        Ok(r) => r,
        Err(_) => {
            return stale_or(
                rt,
                &key,
                Fail::new("UNAVAILABLE", "market data request failed", Some(10)),
            );
        }
    };
    let status = response.status();
    if !status.is_success() {
        let retry = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .or(Some(30));
        let code = if status.as_u16() == 429 {
            "RATE_LIMITED"
        } else {
            "UNAVAILABLE"
        };
        return stale_or(
            rt,
            &key,
            Fail::new(code, format!("market data returned HTTP {status}"), retry),
        );
    }
    let parsed: Value = response
        .json()
        .await
        .map_err(|_| Fail::new("BAD_RESPONSE", "market data response was not JSON", None))?;
    let errors = parsed
        .get("errors")
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty())
        .map(|e| {
            e.iter()
                .filter_map(|x| x.get("message").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("; ")
        });
    let data = parsed.get("data").cloned().unwrap_or(Value::Null);
    if data.is_null()
        || data
            .as_object()
            .is_some_and(|m| m.values().all(Value::is_null))
    {
        let message = errors.unwrap_or_else(|| "no data returned".into());
        if message.to_ascii_lowercase().contains("not found") {
            return Err(Fail::new(
                "NOT_FOUND",
                "no Robinhood Chain market indexed for this exact contract; check the address and chain",
                None,
            ));
        }
        return Err(Fail::new("BAD_QUERY", message, None));
    }
    if errors.is_none() {
        rt.store(key, data.clone(), ttl);
    }
    Ok((data, errors.map(|e| format!("partial market data: {e}"))))
}

fn stale_or(rt: &Runtime, key: &str, fail: Fail) -> Result<(Value, Option<String>), Fail> {
    match rt.stale(key) {
        Some(old) => Ok((
            old,
            Some(format!("{}; showing the last cached read", fail.message)),
        )),
        None => Err(fail),
    }
}

const RPC_TIMEOUT: Duration = Duration::from_secs(8);

/// One JSON-RPC call on Robinhood Chain, fast endpoint first.
async fn rpc(rt: &Runtime, method: &str, params: Value) -> Option<Value> {
    let body = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
    for url in RPCS {
        let sent = rt.http.post(url).timeout(RPC_TIMEOUT).json(&body).send();
        let Ok(response) = sent.await else { continue };
        let Ok(value) = response.json::<Value>().await else {
            continue;
        };
        if let Some(result) = value.get("result") {
            return Some(result.clone());
        }
    }
    None
}

async fn eth_call(rt: &Runtime, to: &str, data: &str) -> Option<String> {
    let result = rpc(
        rt,
        "eth_call",
        json!([{ "to": to, "data": data }, "latest"]),
    )
    .await?;
    result.as_str().map(str::to_string)
}

/// ERC-20 decimals, cached forever.
pub async fn decimals(rt: &Runtime, token: &str) -> Option<u8> {
    let key = format!("decimals:{token}");
    if let Some(hit) = rt.cached(&key) {
        return hit.as_u64().map(|d| d as u8);
    }
    let raw = eth_call(rt, token, "0x313ce567").await?;
    let value = u8::try_from(word_u128(words(&raw).first()?)?).ok()?;
    rt.store(key, json!(value), Ttl::Forever);
    Some(value)
}

/// ERC-20 balance in atomic units (uncached: balances move).
pub async fn balance_of(rt: &Runtime, token: &str, owner: &str) -> Option<u128> {
    let data = format!("0x70a08231{:0>64}", owner.trim_start_matches("0x"));
    let raw = eth_call(rt, token, &data).await?;
    word_u128(words(&raw).first()?)
}

/// ERC-20 symbol, cached forever. Handles string and bytes32 encodings.
pub async fn symbol(rt: &Runtime, token: &str) -> Option<String> {
    let key = format!("symbol:{token}");
    if let Some(hit) = rt.cached(&key) {
        return hit.as_str().map(str::to_string);
    }
    let raw = eth_call(rt, token, "0x95d89b41").await?;
    let value = decode_string(&raw)?;
    rt.store(key, json!(value), Ttl::Forever);
    Some(value)
}

fn decode_string(raw: &str) -> Option<String> {
    let hex = raw.trim_start_matches("0x");
    let bytes: Vec<u8> = (0..hex.len() / 2)
        .filter_map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect();
    let text = if bytes.len() >= 96 {
        let len = usize::try_from(word_u128(words(raw).get(1)?)?).ok()?;
        bytes.get(64..64 + len)?.to_vec()
    } else {
        bytes.into_iter().take_while(|b| *b != 0).collect()
    };
    let text = String::from_utf8(text).ok()?;
    let clean: String = text.chars().filter(|c| !c.is_control()).take(16).collect();
    (!clean.is_empty()).then_some(clean)
}

/// True when the address has no code (a plain wallet, not a contract).
pub async fn is_wallet(rt: &Runtime, address: &str) -> bool {
    has_code(rt, &[address.to_string()]).await.first() == Some(&Some(false))
}

/// Whether each address is a contract, in one batched read on the fast
/// endpoint. Cached forever; `None` when unreadable.
pub async fn has_code(rt: &Runtime, addresses: &[String]) -> Vec<Option<bool>> {
    let mut out: Vec<Option<bool>> = addresses
        .iter()
        .map(|a| rt.cached(&format!("code:{a}")).and_then(|v| v.as_bool()))
        .collect();
    let missing: Vec<usize> = (0..addresses.len()).filter(|&i| out[i].is_none()).collect();
    if missing.is_empty() {
        return out;
    }
    let batch: Vec<Value> = missing
        .iter()
        .map(|&i| json!({"jsonrpc":"2.0","id":i,"method":"eth_getCode","params":[addresses[i],"latest"]}))
        .collect();
    let sent = rt
        .http
        .post(RPCS[0])
        .timeout(RPC_TIMEOUT)
        .json(&batch)
        .send();
    let replies: Vec<Value> = match sent.await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(_) => vec![],
    };
    for reply in replies {
        let (Some(i), Some(code)) = (
            reply.get("id").and_then(Value::as_u64).map(|i| i as usize),
            reply.get("result").and_then(Value::as_str),
        ) else {
            continue;
        };
        if let Some(addr) = addresses.get(i) {
            let contract = code != "0x";
            rt.store(format!("code:{addr}"), json!(contract), Ttl::Forever);
            out[i] = Some(contract);
        }
    }
    out
}

fn words(hex: &str) -> Vec<&str> {
    let hex = hex.trim_start_matches("0x");
    (0..hex.len() / 64)
        .map(|i| &hex[i * 64..i * 64 + 64])
        .collect()
}
fn word_addr(word: &str) -> String {
    format!("0x{}", &word[24..])
}
fn word_u128(word: &str) -> Option<u128> {
    let (high, low) = word.split_at(word.len().saturating_sub(32));
    high.bytes()
        .all(|b| b == b'0')
        .then(|| u128::from_str_radix(low, 16).ok())?
}

/// What the Pons V2 factory recorded at launch. Everything but `phase`
/// is fixed at launch.
#[derive(Clone, Debug, PartialEq)]
pub struct PonsLaunch {
    pub curve: String,
    pub deployer: String,
    pub pair_token: String,
    pub target: u128,
    pub creator_tax_bps: u128,
    /// 0 curve, 1 swept (graduating), 2 trading on Uniswap v4, 3 rescued.
    pub phase: u128,
}

impl PonsLaunch {
    pub fn on_curve(&self) -> bool {
        self.phase == 0
    }
    pub fn stage(&self) -> &'static str {
        match self.phase {
            0 => "curve",
            1 => "graduating",
            3 => "rescued",
            _ => "graduated",
        }
    }
}

/// Pons launch record via `getLaunchedToken(address)`; `None` when the
/// token wasn't launched on Pons V2. Cached forever once graduated.
pub async fn pons_launch(rt: &Runtime, token: &str) -> Option<PonsLaunch> {
    let key = format!("pons:{token}");
    if let Some(hit) = rt.cached(&key) {
        return parse_launch(hit.as_str()?);
    }
    let data = format!("0x3cf28b5a{:0>64}", token.trim_start_matches("0x"));
    let raw = eth_call(rt, PONS_FACTORY, &data).await?;
    let launch = parse_launch(&raw);
    let ttl = match &launch {
        Some(l) if l.on_curve() || l.phase == 1 => Ttl::Minute,
        _ => Ttl::Forever,
    };
    rt.store(key, json!(raw), ttl);
    launch
}

fn parse_launch(raw: &str) -> Option<PonsLaunch> {
    let w = words(raw);
    if w.len() < 15 || word_u128(w[14])? != 1 {
        return None;
    }
    Some(PonsLaunch {
        curve: word_addr(w[1]),
        deployer: word_addr(w[2]),
        pair_token: word_addr(w[4]),
        target: word_u128(w[5])?,
        creator_tax_bps: word_u128(w[8])?,
        phase: word_u128(w[10])?,
    })
}

/// Real (non-virtual) pair-token reserve on a Pons curve, in atomic units.
pub async fn pons_reserve(rt: &Runtime, curve: &str) -> Option<u128> {
    let key = format!("reserve:{curve}");
    if let Some(hit) = rt.cached(&key) {
        return hit.as_str().and_then(|s| s.parse().ok());
    }
    let raw = eth_call(rt, curve, "0x4f1f58fd").await?;
    let value = word_u128(words(&raw).first()?)?;
    rt.store(key, json!(value.to_string()), Ttl::Live);
    Some(value)
}

#[derive(Debug, Clone)]
pub struct Quote {
    pub to_amount: String,
    pub to_amount_min: String,
    pub route: Vec<String>,
    pub fee_usd: Option<f64>,
    pub gas_usd: Option<f64>,
}

/// LI.FI same-chain quote. `from_address` matters: curve sells only quote
/// from a wallet that holds the token.
pub async fn lifi_quote(
    rt: &Runtime,
    call: &Call,
    from_token: &str,
    to_token: &str,
    amount: &str,
    from_address: &str,
    slippage: f64,
) -> Result<Quote, Fail> {
    if !rt.lifi_slot(call.lifi_key.is_some()) {
        return Err(Fail::new(
            "RATE_LIMITED",
            "quote allowance used up for this hour",
            Some(600),
        ));
    }
    let keyed = call.lifi_key.as_deref();
    match quote_once(
        rt,
        keyed,
        from_token,
        to_token,
        amount,
        from_address,
        slippage,
    )
    .await
    {
        // A rejected key falls back to the keyless allowance.
        Err(fail) if fail.code == "UNCONFIGURED" && keyed.is_some() && rt.lifi_slot(false) => {
            quote_once(
                rt,
                None,
                from_token,
                to_token,
                amount,
                from_address,
                slippage,
            )
            .await
        }
        other => other,
    }
}

async fn quote_once(
    rt: &Runtime,
    key: Option<&str>,
    from_token: &str,
    to_token: &str,
    amount: &str,
    from_address: &str,
    slippage: f64,
) -> Result<Quote, Fail> {
    let mut request = rt
        .http
        .get(LIFI_URL)
        .timeout(Duration::from_secs(25))
        .query(&[
            ("fromChain", NETWORK.to_string()),
            ("toChain", NETWORK.to_string()),
            ("fromToken", from_token.to_string()),
            ("toToken", to_token.to_string()),
            ("fromAmount", amount.to_string()),
            ("fromAddress", from_address.to_string()),
            ("slippage", slippage.to_string()),
            ("maxPriceImpact", "0.3".to_string()),
            ("integrator", "hoodit".to_string()),
        ]);
    if let Some(key) = key {
        request = request.header("x-lifi-api-key", key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| Fail::new("UNAVAILABLE", "quote request failed", Some(10)))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 429 {
        return Err(Fail::new(
            "RATE_LIMITED",
            "quote service is rate limiting",
            Some(600),
        ));
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(Fail::new(
            "UNCONFIGURED",
            "quote service rejected the API key",
            None,
        ));
    }
    let Some(estimate) = body.get("estimate") else {
        let message = body
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("no route");
        let code = if status.is_client_error() {
            "NO_ROUTE"
        } else {
            "UNAVAILABLE"
        };
        return Err(Fail::new(code, message.to_string(), None));
    };
    let sum = |key: &str| {
        estimate.get(key).and_then(Value::as_array).map(|items| {
            items
                .iter()
                .filter_map(|c| crate::shape::num(c.get("amountUSD").unwrap_or(&Value::Null)))
                .sum::<f64>()
        })
    };
    let route = body
        .get("includedSteps")
        .and_then(Value::as_array)
        .map(|steps| {
            steps
                .iter()
                .filter_map(|s| {
                    s.pointer("/toolDetails/name")
                        .or_else(|| s.get("tool"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Quote {
        to_amount: estimate
            .get("toAmount")
            .and_then(Value::as_str)
            .unwrap_or("0")
            .to_string(),
        to_amount_min: estimate
            .get("toAmountMin")
            .and_then(Value::as_str)
            .unwrap_or("0")
            .to_string(),
        route,
        fee_usd: sum("feeCosts"),
        gas_usd: sum("gasCosts"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols_decode_both_encodings() {
        let string = "0x000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000045553444700000000000000000000000000000000000000000000000000000000";
        assert_eq!(decode_string(string).as_deref(), Some("USDG"));
        let bytes32 = "0x4d4b520000000000000000000000000000000000000000000000000000000000";
        assert_eq!(decode_string(bytes32).as_deref(), Some("MKR"));
    }

    #[test]
    fn launch_record_decodes() {
        // getLaunchedToken for a USDG-paired Pons V2 launch (live read, 2026-10-03).
        let raw = "0x000000000000000000000000c093f78bb9859817072cf7e1d2819511f2860a100000000000000000000000009c69b2a354f73b93b80966b707d797b322c5a4fd0000000000000000000000008d64b1cb8834a9aac51075cddd388f8a47b094d90000000000000000000000008d64b1cb8834a9aac51075cddd388f8a47b094d90000000000000000000000005fc5360d0400a0fd4f2af552add042d716f1d16800000000000000000000000000000000000000000000000000000001e2339a80000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000c80000000000000000000000000000000000000000000000000000000000000064000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001";
        let launch = parse_launch(raw).unwrap();
        assert_eq!(
            launch.pair_token,
            "0x5fc5360d0400a0fd4f2af552add042d716f1d168"
        );
        assert_eq!(launch.target, 8_090_000_000);
        assert_eq!(
            launch.deployer,
            "0x8d64b1cb8834a9aac51075cddd388f8a47b094d9"
        );
        assert_eq!(launch.creator_tax_bps, 100);
        assert_eq!(launch.stage(), "curve");
        assert!(parse_launch("0x").is_none());
    }
}
