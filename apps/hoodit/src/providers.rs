use crate::app::{ReadContext, Runtime};
use crate::model::{self, NETWORK};
use aomi_sdk::DynToolCallCtx;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::{StatusCode, header::RETRY_AFTER};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

#[allow(clippy::too_many_arguments)]
fn get(
    runtime: &Runtime,
    provider: &'static str,
    base: &str,
    path: &str,
    query: &[(String, String)],
    headers: &[(&str, &str)],
    ttl: Duration,
    credential: Option<&str>,
    read: &mut ReadContext,
) -> Result<Value, ProviderError> {
    let cache_key = cache_key(provider, path, query, credential);
    if let Some((value, fetched_at)) = runtime.cached(&cache_key, read.refresh) {
        read.source(provider, path, fetched_at, true);
        return Ok(value);
    }
    for attempt in 0..2 {
        let Some(remaining) = read.remaining() else {
            return Err(ProviderError {
                code: "DEADLINE_EXCEEDED",
                message: "provider request deadline exceeded".into(),
                retryable: true,
            });
        };
        if !read.spend(provider) {
            return Err(ProviderError {
                code: if provider == "lifi" {
                    "QUOTE_BUDGET_EXHAUSTED"
                } else {
                    "PROVIDER_BUDGET_EXHAUSTED"
                },
                message: "provider request budget exhausted".into(),
                retryable: false,
            });
        }
        if runtime.spend_rate(provider, credential).is_some() {
            return Err(ProviderError {
                code: "RATE_LIMITED",
                message: "provider rate limit reached".into(),
                retryable: true,
            });
        }
        let mut request = runtime.http.get(format!("{base}{path}")).query(query);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = match request
            .timeout(remaining.min(Duration::from_secs(10)))
            .send()
        {
            Ok(response) => response,
            Err(_) if attempt == 0 && read.remaining().is_some() => continue,
            Err(_) => return Err(ProviderError::unavailable("provider request failed")),
        };
        let status = response.status();
        if (status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS) && attempt == 0 {
            let delay = retry_delay(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|h| h.to_str().ok()),
            );
            if read.remaining().is_some_and(|remaining| delay < remaining) {
                std::thread::sleep(delay);
                continue;
            }
        }
        if !status.is_success() {
            let body = response.json::<Value>().ok();
            return Err(ProviderError::status(status, provider, body.as_ref()));
        }
        let value = response
            .json::<Value>()
            .map_err(|_| ProviderError::schema("provider returned invalid JSON"))?;
        let fetched_at = runtime.cache(cache_key.clone(), value.clone(), ttl);
        read.source(provider, path, fetched_at, false);
        return Ok(value);
    }
    Err(ProviderError::unavailable("provider request failed"))
}

fn cache_key(
    provider: &str,
    path: &str,
    query: &[(String, String)],
    credential: Option<&str>,
) -> String {
    let mut hasher = DefaultHasher::new();
    credential.unwrap_or("").hash(&mut hasher);
    let scope = hasher.finish();
    let public_query: Vec<_> = query.iter().filter(|(key, _)| key != "apikey").collect();
    format!("{provider}:{scope:016x}:{path}:{public_query:?}")
}

fn retry_delay(value: Option<&str>) -> Duration {
    if let Some(seconds) = value.and_then(|v| v.parse::<u64>().ok()) {
        return Duration::from_secs(seconds.min(3600));
    }
    if let Some(when) = value.and_then(|value| chrono::DateTime::parse_from_rfc2822(value).ok()) {
        let seconds = (when.with_timezone(&chrono::Utc) - chrono::Utc::now())
            .num_seconds()
            .max(0) as u64;
        return Duration::from_secs(seconds.min(3600));
    }
    Duration::from_millis(200)
}

#[derive(Debug)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}
impl ProviderError {
    fn unavailable(message: &str) -> Self {
        Self {
            code: "UPSTREAM_UNAVAILABLE",
            message: message.into(),
            retryable: true,
        }
    }
    fn schema(message: &str) -> Self {
        Self {
            code: "UPSTREAM_SCHEMA_CHANGED",
            message: message.into(),
            retryable: false,
        }
    }
    fn status(status: StatusCode, provider: &str, body: Option<&Value>) -> Self {
        let no_route = provider == "lifi"
            && body
                .and_then(|value| value.get("code"))
                .is_some_and(|code| {
                    code.as_u64() == Some(1002)
                        || code.as_str().is_some_and(|code| {
                            matches!(code, "NO_QUOTE" | "NO_ROUTE" | "NoQuoteError")
                        })
                });
        if no_route {
            return Self {
                code: "NOT_INDEXED",
                message: "LI.FI found no route".into(),
                retryable: false,
            };
        }
        let (code, message, retryable) = if status == StatusCode::TOO_MANY_REQUESTS {
            ("RATE_LIMITED", "provider rate limit reached", true)
        } else if status == StatusCode::NOT_FOUND {
            (
                "UPSTREAM_NOT_FOUND",
                "provider resource was not found",
                false,
            )
        } else if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            (
                "UPSTREAM_UNAVAILABLE",
                "provider authentication failed",
                false,
            )
        } else {
            (
                "UPSTREAM_UNAVAILABLE",
                "provider request was unsuccessful",
                status.is_server_error(),
            )
        };
        Self {
            code,
            message: message.into(),
            retryable,
        }
    }
}

fn address_segment(value: &str) -> Result<String, ProviderError> {
    model::address(value)
        .map_err(|_| ProviderError::schema("invalid address passed to provider adapter"))
}

fn pool_segment(value: &str) -> Result<String, ProviderError> {
    let value = value.trim();
    if (1..=200).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
    {
        Ok(value.to_string())
    } else {
        Err(ProviderError::schema(
            "invalid pool identifier passed to provider adapter",
        ))
    }
}

fn not_found(
    result: Result<Value, ProviderError>,
    code: &'static str,
    message: &'static str,
) -> Result<Value, ProviderError> {
    result.map_err(|error| {
        if error.code == "UPSTREAM_NOT_FOUND" {
            ProviderError {
                code,
                message: message.into(),
                retryable: false,
            }
        } else {
            error
        }
    })
}

pub struct Gecko<'a> {
    runtime: &'a Runtime,
}
impl<'a> Gecko<'a> {
    pub fn new(runtime: &'a Runtime) -> Self {
        Self { runtime }
    }
    pub fn search(
        &self,
        query: &str,
        page: u8,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            "/search/pools",
            &[
                ("network".into(), NETWORK.into()),
                ("query".into(), query.into()),
                ("page".into(), page.to_string()),
                ("include".into(), "base_token,quote_token,dex".into()),
            ],
            &[],
            Duration::from_secs(20),
            None,
            read,
        )
    }
    pub fn discover(
        &self,
        feed: &str,
        duration: &str,
        page: u8,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let (path, mut query) = match feed {
            "new" => (format!("/networks/{NETWORK}/new_pools"), vec![]),
            "top_volume" => (
                format!("/networks/{NETWORK}/pools"),
                vec![("sort".into(), "h24_volume_usd_desc".into())],
            ),
            "top_activity" => (
                format!("/networks/{NETWORK}/pools"),
                vec![("sort".into(), "h24_tx_count_desc".into())],
            ),
            _ => (
                format!("/networks/{NETWORK}/trending_pools"),
                vec![("duration".into(), duration.into())],
            ),
        };
        query.extend([
            ("page".into(), page.to_string()),
            ("include".into(), "base_token,quote_token,dex".into()),
        ]);
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &path,
            &query,
            &[],
            Duration::from_secs(20),
            None,
            read,
        )
    }
    pub fn token(&self, token: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        let token = address_segment(token)?;
        not_found(
            get(
                self.runtime,
                "geckoterminal",
                &self.runtime.origins.gecko,
                &format!("/networks/{NETWORK}/tokens/{token}"),
                &[("include".into(), "top_pools".into())],
                &[],
                Duration::from_secs(20),
                None,
                read,
            ),
            "TOKEN_NOT_INDEXED",
            "token is not indexed",
        )
    }
    pub fn token_pools(&self, token: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        self.token_pools_page(token, 1, read)
    }
    pub fn token_pools_page(
        &self,
        token: &str,
        page: u8,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let token = address_segment(token)?;
        not_found(
            get(
                self.runtime,
                "geckoterminal",
                &self.runtime.origins.gecko,
                &format!("/networks/{NETWORK}/tokens/{token}/pools"),
                &[
                    ("include".into(), "base_token,quote_token,dex".into()),
                    ("page".into(), page.to_string()),
                ],
                &[],
                Duration::from_secs(20),
                None,
                read,
            ),
            "NO_INDEXED_POOL",
            "no indexed pool was found",
        )
    }
    pub fn pool(&self, pool: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        let pool = pool_segment(pool)?;
        not_found(
            get(
                self.runtime,
                "geckoterminal",
                &self.runtime.origins.gecko,
                &format!("/networks/{NETWORK}/pools/{pool}"),
                &[("include".into(), "base_token,quote_token,dex".into())],
                &[],
                Duration::from_secs(20),
                None,
                read,
            ),
            "POOL_NOT_FOUND",
            "pool was not found",
        )
    }
    pub fn metadata(&self, token: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        let token = address_segment(token)?;
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/networks/{NETWORK}/tokens/{token}/info"),
            &[],
            &[],
            Duration::from_secs(60),
            None,
            read,
        )
    }
    pub fn pool_info(&self, pool: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        let pool = pool_segment(pool)?;
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/networks/{NETWORK}/pools/{pool}/info"),
            &[("include".into(), "pool".into())],
            &[],
            Duration::from_secs(60),
            None,
            read,
        )
    }
    pub fn dexes(&self, read: &mut ReadContext) -> Result<Value, ProviderError> {
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/networks/{NETWORK}/dexes"),
            &[],
            &[],
            Duration::from_secs(300),
            None,
            read,
        )
    }
    pub fn token_prices(
        &self,
        tokens: &[String],
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        if tokens.is_empty() || tokens.len() > 30 {
            return Err(ProviderError::schema(
                "token price batch must contain 1 to 30 addresses",
            ));
        }
        let addresses = tokens
            .iter()
            .map(|token| address_segment(token))
            .collect::<Result<Vec<_>, _>>()?
            .join(",");
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/simple/networks/{NETWORK}/token_price/{addresses}"),
            &[],
            &[],
            Duration::from_secs(20),
            None,
            read,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn candles(
        &self,
        pool: &str,
        token: &str,
        timeframe: &str,
        aggregate: u8,
        before: i64,
        limit: u16,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let pool = pool_segment(pool)?;
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/networks/{NETWORK}/pools/{pool}/ohlcv/{timeframe}"),
            &[
                ("aggregate".into(), aggregate.to_string()),
                ("before_timestamp".into(), before.to_string()),
                ("limit".into(), limit.to_string()),
                ("currency".into(), "usd".into()),
                ("token".into(), token.into()),
                ("include_empty_intervals".into(), "false".into()),
            ],
            &[],
            Duration::from_secs(15),
            None,
            read,
        )
    }
    pub fn trades(
        &self,
        pool: &str,
        token: &str,
        min: &str,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let pool = pool_segment(pool)?;
        get(
            self.runtime,
            "geckoterminal",
            &self.runtime.origins.gecko,
            &format!("/networks/{NETWORK}/pools/{pool}/trades"),
            &[
                ("token".into(), token.into()),
                ("trade_volume_in_usd_greater_than".into(), min.into()),
            ],
            &[],
            Duration::from_secs(10),
            None,
            read,
        )
    }
}

pub struct GoPlus<'a> {
    runtime: &'a Runtime,
}

pub struct CoinGecko<'a> {
    runtime: &'a Runtime,
}
impl<'a> CoinGecko<'a> {
    pub fn new(runtime: &'a Runtime) -> Self {
        Self { runtime }
    }

    pub fn eth_price(&self, read: &mut ReadContext) -> Result<Value, ProviderError> {
        get(
            self.runtime,
            "coingecko",
            &self.runtime.origins.coingecko,
            "/simple/price",
            &[
                ("ids".into(), "ethereum".into()),
                ("vs_currencies".into(), "usd".into()),
            ],
            &[],
            Duration::from_secs(20),
            None,
            read,
        )
    }
}
impl<'a> GoPlus<'a> {
    pub fn new(runtime: &'a Runtime) -> Self {
        Self { runtime }
    }

    pub fn token_security(
        &self,
        token: &str,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let token = address_segment(token)?;
        get(
            self.runtime,
            "goplus",
            &self.runtime.origins.goplus,
            &format!("/token_security/{}", model::CHAIN_ID),
            &[("contract_addresses".into(), token)],
            &[],
            Duration::from_secs(60),
            None,
            read,
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    v: u8,
    chain: u64,
    wallet: String,
    hop: u8,
    next: InventoryNext,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryNext {
    id: u64,
    value: String,
    fiat_value: Option<String>,
    items_count: u16,
}
pub struct Inventory {
    pub items: Vec<Value>,
    pub next_cursor: Option<String>,
}
pub struct Blockscout<'a> {
    runtime: &'a Runtime,
    key: String,
}
impl<'a> Blockscout<'a> {
    pub fn from_ctx(runtime: &'a Runtime, ctx: &DynToolCallCtx) -> Result<Self, ProviderError> {
        ctx.secrets.get("BLOCKSCOUT_API_KEY").map(|key|key.trim()).filter(|key|!key.is_empty()).map(|key|Self{runtime,key:key.to_string()}).ok_or_else(|| ProviderError {
            code: "PROVIDER_NOT_CONFIGURED",
            message: "Hoodit wallet reads are temporarily unavailable because the operator-managed provider configuration is missing.".into(),
            retryable: false,
        })
    }
    pub fn inventory(
        &self,
        wallet: &str,
        cursor: Option<&str>,
        read: &mut ReadContext,
    ) -> Result<Inventory, ProviderError> {
        let wallet = address_segment(wallet)?;
        let mut hop = 0;
        let mut query = vec![
            ("type".into(), "ERC-20".into()),
            ("apikey".into(), self.key.clone()),
        ];
        if let Some(raw) = cursor {
            if raw.len() > 4096 {
                return Err(ProviderError {
                    code: "INVALID_CURSOR",
                    message: "inventory cursor is too large".into(),
                    retryable: false,
                });
            }
            let decoded = URL_SAFE_NO_PAD.decode(raw).map_err(|_| ProviderError {
                code: "INVALID_CURSOR",
                message: "invalid inventory cursor".into(),
                retryable: false,
            })?;
            let c: Cursor = serde_json::from_slice(&decoded).map_err(|_| ProviderError {
                code: "INVALID_CURSOR",
                message: "invalid inventory cursor".into(),
                retryable: false,
            })?;
            if c.v != 1
                || c.chain != model::CHAIN_ID
                || c.wallet != wallet
                || c.hop >= 100
                || !valid_cursor_value(&c.next.value)
                || c.next.items_count != 50
            {
                return Err(ProviderError {
                    code: "INVALID_CURSOR",
                    message: "cursor does not belong to this wallet or traversal".into(),
                    retryable: false,
                });
            }
            hop = c.hop;
            query.extend(c.next.query());
        }
        let value = get(
            self.runtime,
            "blockscout",
            &self.runtime.origins.blockscout,
            &format!("/api/v2/addresses/{wallet}/tokens"),
            &query,
            &[],
            Duration::from_secs(10),
            Some(&self.key),
            read,
        )?;
        let items = value
            .get("items")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| ProviderError::schema("Blockscout inventory is missing items"))?;
        if items.len() > 50 {
            return Err(ProviderError::schema(
                "Blockscout inventory page exceeds the supported bound",
            ));
        }
        let next = parse_next(value.get("next_page_params"))?
            .map(|next| {
                serde_json::to_vec(&Cursor {
                    v: 1,
                    chain: model::CHAIN_ID,
                    wallet,
                    hop: hop + 1,
                    next,
                })
                .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
                .map_err(|_| ProviderError::schema("failed to encode inventory cursor"))
            })
            .transpose()?;
        Ok(Inventory {
            items,
            next_cursor: next,
        })
    }
    fn compat(
        &self,
        action: &str,
        wallet: &str,
        token: Option<&str>,
        read: &mut ReadContext,
    ) -> Result<String, ProviderError> {
        let wallet = address_segment(wallet)?;
        let mut q = vec![
            ("module".into(), "account".into()),
            ("action".into(), action.into()),
            ("address".into(), wallet),
            ("apikey".into(), self.key.clone()),
        ];
        if let Some(t) = token {
            q.push(("contractaddress".into(), address_segment(t)?))
        }
        let v = get(
            self.runtime,
            "blockscout",
            &self.runtime.origins.blockscout,
            "/api",
            &q,
            &[],
            Duration::from_secs(5),
            Some(&self.key),
            read,
        )?;
        if model::string(&v, &["status"]).as_deref() != Some("1") {
            return Err(ProviderError::unavailable(
                "Blockscout could not establish the balance",
            ));
        }
        model::string(&v, &["result"])
            .ok_or_else(|| ProviderError::schema("Blockscout balance is missing result"))
    }
    pub fn native_balance(
        &self,
        wallet: &str,
        read: &mut ReadContext,
    ) -> Result<String, ProviderError> {
        self.compat("balance", wallet, None, read)
    }
    pub fn token_balance(
        &self,
        wallet: &str,
        token: &str,
        read: &mut ReadContext,
    ) -> Result<String, ProviderError> {
        self.compat("tokenbalance", wallet, Some(token), read)
    }
    pub fn token_info(&self, token: &str, read: &mut ReadContext) -> Result<Value, ProviderError> {
        let token = address_segment(token)?;
        get(
            self.runtime,
            "blockscout",
            &self.runtime.origins.blockscout,
            &format!("/api/v2/tokens/{token}"),
            &[("apikey".into(), self.key.clone())],
            &[],
            Duration::from_secs(60),
            Some(&self.key),
            read,
        )
    }
}

impl InventoryNext {
    fn query(self) -> Vec<(String, String)> {
        let mut query = vec![
            ("id".into(), self.id.to_string()),
            ("value".into(), self.value),
            ("items_count".into(), self.items_count.to_string()),
        ];
        if let Some(value) = self.fiat_value {
            query.push(("fiat_value".into(), value));
        }
        query
    }
}

fn valid_cursor_value(value: &str) -> bool {
    !value.is_empty() && value.len() <= 78 && value.bytes().all(|b| b.is_ascii_digit())
}

fn parse_next(value: Option<&Value>) -> Result<Option<InventoryNext>, ProviderError> {
    let Some(value) = value else {
        return Err(ProviderError::schema(
            "Blockscout inventory is missing next_page_params",
        ));
    };
    if value.is_null() {
        return Ok(None);
    }
    let next: InventoryNext = serde_json::from_value(value.clone()).map_err(|_| {
        ProviderError::schema("Blockscout returned an invalid inventory continuation")
    })?;
    if !valid_cursor_value(&next.value) || next.items_count != 50 {
        return Err(ProviderError::schema(
            "Blockscout returned an unsupported inventory continuation",
        ));
    }
    Ok(Some(next))
}

pub struct Lifi<'a> {
    runtime: &'a Runtime,
}
impl<'a> Lifi<'a> {
    pub fn from_ctx(runtime: &'a Runtime, _ctx: &DynToolCallCtx) -> Self {
        Self { runtime }
    }
    pub fn quote(
        &self,
        wallet: &str,
        from: &str,
        amount: &str,
        read: &mut ReadContext,
    ) -> Result<Value, ProviderError> {
        let wallet = address_segment(wallet)?;
        let from = address_segment(from)?;
        if amount.is_empty() || amount.len() > 78 || !amount.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ProviderError::schema(
                "invalid quote amount passed to provider adapter",
            ));
        }
        let q = vec![
            ("fromChain".into(), model::CHAIN_ID.to_string()),
            ("toChain".into(), model::CHAIN_ID.to_string()),
            ("fromToken".into(), from.clone()),
            ("toToken".into(), model::USDG.into()),
            ("fromAmount".into(), amount.into()),
            ("fromAddress".into(), wallet.clone()),
            ("toAddress".into(), wallet.clone()),
            ("order".into(), "RECOMMENDED".into()),
            ("slippage".into(), "0.005".into()),
        ];
        let mut value = get(
            self.runtime,
            "lifi",
            &self.runtime.origins.lifi,
            "/quote",
            &q,
            &[],
            Duration::from_secs(5),
            None,
            read,
        )?;
        validate_quote(&value, &wallet, &from, amount)?;
        value
            .as_object_mut()
            .map(|object| object.remove("transactionRequest"));
        Ok(value)
    }
}

fn validate_quote(
    value: &Value,
    wallet: &str,
    from: &str,
    amount: &str,
) -> Result<(), ProviderError> {
    let chain = |path: &[&str]| {
        model::get(value, path).and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
    };
    let matches = chain(&["action", "fromChainId"]) == Some(model::CHAIN_ID)
        && chain(&["action", "toChainId"]) == Some(model::CHAIN_ID)
        && chain(&["action", "fromToken", "chainId"]) == Some(model::CHAIN_ID)
        && chain(&["action", "toToken", "chainId"]) == Some(model::CHAIN_ID)
        && model::string(value, &["action", "fromAmount"]).as_deref() == Some(amount)
        && model::string(value, &["action", "fromToken", "address"])
            .is_some_and(|v| v.eq_ignore_ascii_case(from))
        && model::string(value, &["action", "toToken", "address"])
            .is_some_and(|v| v.eq_ignore_ascii_case(model::USDG))
        && model::string(value, &["action", "fromAddress"])
            .is_some_and(|v| v.eq_ignore_ascii_case(wallet))
        && model::string(value, &["action", "toAddress"])
            .is_some_and(|v| v.eq_ignore_ascii_case(wallet))
        && model::string(value, &["estimate", "fromAmount"]).as_deref() == Some(amount)
        && model::string(value, &["estimate", "toAmount"]).is_some_and(|v| valid_cursor_value(&v));
    if matches {
        Ok(())
    } else {
        Err(ProviderError::schema(
            "LI.FI quote does not match the request",
        ))
    }
}

pub fn included_map(value: &Value) -> Map<String, Value> {
    value
        .get("included")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| model::string(v, &["id"]).map(|id| (id, v.clone())))
        .collect()
}
pub fn relation_id(value: &Value, name: &str) -> Option<String> {
    model::string(value, &["relationships", name, "data", "id"])
}
pub fn token_from_resource(value: &Value) -> Value {
    let Some(id) = resource_address(value) else {
        return Value::Null;
    };
    model::token(
        &id,
        model::string(value, &["attributes", "symbol"]).as_deref(),
        model::string(value, &["attributes", "name"]).as_deref(),
        model::string(value, &["attributes", "decimals"]).and_then(|s| s.parse().ok()),
        model::string(value, &["attributes", "image_url"]).as_deref(),
    )
}
fn resource_address(value: &Value) -> Option<String> {
    model::string(value, &["attributes", "address"])
        .or_else(|| {
            model::string(value, &["id"])
                .map(|id| id.strip_prefix("robinhood_").unwrap_or(&id).to_string())
        })
        .and_then(|id| model::address(&id).ok())
}
fn relation_token(value: &Value, name: &str, included: &Map<String, Value>) -> Value {
    let Some(id) = relation_id(value, name) else {
        return Value::Null;
    };
    included
        .get(&id)
        .map(token_from_resource)
        .filter(|v| !v.is_null())
        .unwrap_or_else(|| {
            let address = id.strip_prefix("robinhood_").unwrap_or(&id);
            model::address(address)
                .map(|address| model::token(&address, None, None, None, None))
                .unwrap_or(Value::Null)
        })
}
fn window(attrs: &Value, key: &str) -> Value {
    json!({
        "base_price_change_pct": model::string(attrs, &["price_change_percentage", key]),
        "volume_usd": model::string(attrs, &["volume_usd", key]),
        "buys": model::get(attrs, &["transactions", key, "buys"]).and_then(Value::as_u64),
        "sells": model::get(attrs, &["transactions", key, "sells"]).and_then(Value::as_u64),
        "buyers": model::get(attrs, &["transactions", key, "buyers"]).and_then(Value::as_u64),
        "sellers": model::get(attrs, &["transactions", key, "sellers"]).and_then(Value::as_u64)
    })
}
pub fn pool(value: &Value, included: &Map<String, Value>) -> Value {
    let attrs = value.get("attributes").cloned().unwrap_or(json!({}));
    let base = relation_token(value, "base_token", included);
    let quote = relation_token(value, "quote_token", included);
    let dex_id = relation_id(value, "dex");
    let dex_name = dex_id
        .as_ref()
        .and_then(|id| included.get(id))
        .and_then(|dex| model::string(dex, &["attributes", "name"]));
    let windows: Map<String, Value> = ["m5", "m15", "m30", "h1", "h6", "h24"]
        .into_iter()
        .map(|key| (key.into(), window(&attrs, key)))
        .collect();
    let pool_id = model::string(value, &["attributes", "address"]).or_else(|| {
        model::string(value, &["id"])
            .map(|id| id.strip_prefix("robinhood_").unwrap_or(&id).to_string())
    });
    json!({"pool_id":pool_id,"dex_id":dex_id,"dex_name":dex_name,"name":model::string(value,&["attributes","name"]),"base_token":base,"quote_token":quote,"base_price_usd":model::string(&attrs,&["base_token_price_usd"]),"quote_price_usd":model::string(&attrs,&["quote_token_price_usd"]),"liquidity_usd":model::string(&attrs,&["reserve_in_usd"]),"created_at":model::string(&attrs,&["pool_created_at"]),"windows":windows})
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::ProviderOrigins;
    use reqwest::blocking::Client;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::thread;

    fn server(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 2048];
                let _ = stream.read(&mut request).unwrap();
                seen.fetch_add(1, Ordering::SeqCst);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        (base, count)
    }

    fn capture_server(response_body: String) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 8192];
            let read = stream.read(&mut request).unwrap();
            sender
                .send(String::from_utf8_lossy(&request[..read]).into_owned())
                .unwrap();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        (base, receiver)
    }

    #[test]
    fn inventory_continuation_accepts_only_the_live_shape() {
        let live = json!({"id":2215310099_u64,"value":"100436000000000000000","fiat_value":null,"items_count":50});
        let next = parse_next(Some(&live)).unwrap().unwrap();
        assert_eq!(
            next.query(),
            vec![
                ("id".into(), "2215310099".into()),
                ("value".into(), "100436000000000000000".into()),
                ("items_count".into(), "50".into())
            ]
        );

        let override_attempt =
            json!({"id":1,"value":"1","fiat_value":null,"items_count":50,"type":"ERC-721"});
        assert!(parse_next(Some(&override_attempt)).is_err());
        let wrong_bound = json!({"id":1,"value":"1","fiat_value":null,"items_count":20});
        assert!(parse_next(Some(&wrong_bound)).is_err());
    }

    #[test]
    fn pool_normalization_keeps_v4_id_and_complete_windows() {
        let raw = json!({
            "id":"robinhood_0x4be9657ec9002e528f4f17a5c43edc525a07f888f7b180c2afbf75e096c4f38a",
            "attributes":{"name":"PONS / USDG","transactions":{"h24":{"buys":7,"sells":3}},"volume_usd":{"h24":"12.5"}},
            "relationships":{
                "base_token":{"data":{"id":"robinhood_0x39dbed3a2bd333467115de45665cc57f813c4571"}},
                "quote_token":{"data":{"id":"robinhood_0x5fc5360d0400a0fd4f2af552add042d716f1d168"}},
                "dex":{"data":{"id":"uniswap-v4-robinhood"}}
            }
        });
        let normalized = pool(&raw, &Map::new());
        assert_eq!(
            model::string(&normalized, &["pool_id"]).as_deref(),
            Some("0x4be9657ec9002e528f4f17a5c43edc525a07f888f7b180c2afbf75e096c4f38a")
        );
        assert_eq!(
            model::string(&normalized, &["base_token", "id"]).as_deref(),
            Some("0x39dbed3a2bd333467115de45665cc57f813c4571")
        );
        assert_eq!(
            model::get(&normalized, &["windows"])
                .and_then(Value::as_object)
                .unwrap()
                .len(),
            6
        );
        assert_eq!(
            model::get(&normalized, &["windows", "h24", "buys"]).and_then(Value::as_u64),
            Some(7)
        );
        assert!(normalized.get("volume_24h_usd").is_none());
    }

    #[test]
    fn lifi_quote_must_echo_request_invariants() {
        let wallet = "0xb202bb725c85b90bd847d350ebc7f16ff8408ed8";
        let from = "0x39dbed3a2bd333467115de45665cc57f813c4571";
        let quote = json!({"action":{"fromChainId":4663,"toChainId":4663,"fromAmount":"1000000000000000000","fromAddress":wallet,"toAddress":wallet,"fromToken":{"address":from,"chainId":4663},"toToken":{"address":model::USDG,"chainId":4663}},"estimate":{"fromAmount":"1000000000000000000","toAmount":"636098"}});
        assert!(validate_quote(&quote, wallet, from, "1000000000000000000").is_ok());
        let mut wrong = quote;
        wrong["action"]["toChainId"] = json!(1);
        assert!(validate_quote(&wrong, wallet, from, "1000000000000000000").is_err());
    }

    #[test]
    fn lifi_is_keyless_even_when_context_contains_a_legacy_key() {
        let wallet = "0xb202bb725c85b90bd847d350ebc7f16ff8408ed8";
        let from = "0x39dbed3a2bd333467115de45665cc57f813c4571";
        let amount = "1000000000000000000";
        let body = json!({
            "action": {
                "fromChainId": 4663, "toChainId": 4663, "fromAmount": amount,
                "fromAddress": wallet, "toAddress": wallet,
                "fromToken": {"address": from, "chainId": 4663},
                "toToken": {"address": model::USDG, "chainId": 4663}
            },
            "estimate": {"fromAmount": amount, "toAmount": "636098"}
        })
        .to_string();
        let (base, request) = capture_server(body);
        let runtime = Runtime::fixture(
            Client::new(),
            ProviderOrigins {
                gecko: base.clone(),
                goplus: base.clone(),
                coingecko: base.clone(),
                blockscout: base.clone(),
                lifi: base,
            },
        );
        let mut secrets = std::collections::HashMap::new();
        secrets.insert("LIFI_API_KEY".into(), "legacy-user-key".into());
        let ctx = DynToolCallCtx {
            session_id: "test".into(),
            tool_name: "holding".into(),
            call_id: "1".into(),
            state_attributes: Default::default(),
            secrets,
        };
        let lifi = Lifi::from_ctx(&runtime, &ctx);
        let mut read = ReadContext::portfolio(false);
        lifi.quote(wallet, from, amount, &mut read).unwrap();
        let request = request.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(!request.to_ascii_lowercase().contains("x-lifi-api-key"));
        assert!(!request.contains("legacy-user-key"));
    }

    #[test]
    fn retry_after_beyond_deadline_does_not_retry() {
        let response = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 60\r\nContent-Type: application/json\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"code\":1005}";
        let (base, count) = server(vec![response]);
        let runtime = Runtime::fixture(
            Client::new(),
            ProviderOrigins {
                gecko: base.clone(),
                goplus: base.clone(),
                coingecko: base.clone(),
                blockscout: base.clone(),
                lifi: base.clone(),
            },
        );
        let mut read = ReadContext::markets(false);
        let error = get(
            &runtime,
            "geckoterminal",
            &base,
            "/limited",
            &[],
            &[],
            Duration::from_secs(1),
            None,
            &mut read,
        )
        .unwrap_err();
        assert_eq!(error.code, "RATE_LIMITED");
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cache_refresh_and_provenance_follow_actual_requests() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}";
        let (base, count) = server(vec![response, response, response]);
        let runtime = Runtime::fixture(
            Client::new(),
            ProviderOrigins {
                gecko: base.clone(),
                goplus: base.clone(),
                coingecko: base.clone(),
                blockscout: base.clone(),
                lifi: base.clone(),
            },
        );
        let mut first = ReadContext::markets(false);
        get(
            &runtime,
            "geckoterminal",
            &base,
            "/cached",
            &[],
            &[],
            Duration::from_secs(30),
            Some("secret-value"),
            &mut first,
        )
        .unwrap();
        let mut cached = ReadContext::markets(false);
        get(
            &runtime,
            "geckoterminal",
            &base,
            "/cached",
            &[],
            &[],
            Duration::from_secs(30),
            Some("secret-value"),
            &mut cached,
        )
        .unwrap();
        let mut refresh = ReadContext::markets(true);
        get(
            &runtime,
            "geckoterminal",
            &base,
            "/cached",
            &[],
            &[],
            Duration::from_secs(30),
            Some("secret-value"),
            &mut refresh,
        )
        .unwrap();
        let mut other_credential = ReadContext::markets(false);
        get(
            &runtime,
            "geckoterminal",
            &base,
            "/cached",
            &[],
            &[],
            Duration::from_secs(30),
            Some("other-secret"),
            &mut other_credential,
        )
        .unwrap();
        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(first.sources[0]["cached"], false);
        assert_eq!(cached.sources[0]["cached"], true);
        assert_eq!(refresh.sources[0]["cached"], false);
        assert!(
            !cache_key("geckoterminal", "/cached", &[], Some("secret-value"))
                .contains("secret-value")
        );
    }

    #[test]
    fn expired_deadline_sends_no_request() {
        let runtime = Runtime::fixture(Client::new(), ProviderOrigins::default());
        let mut read = ReadContext::markets(false);
        read.deadline = std::time::Instant::now();
        let error = get(
            &runtime,
            "geckoterminal",
            "http://127.0.0.1:1",
            "/never",
            &[],
            &[],
            Duration::ZERO,
            None,
            &mut read,
        )
        .unwrap_err();
        assert_eq!(error.code, "DEADLINE_EXCEEDED");
        assert!(read.sources.is_empty());
    }
}
