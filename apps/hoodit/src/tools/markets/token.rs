use super::{
    normalization::{invalid_argument, normalize_pool_id, resolve_pool, response_rows},
    security::{normalize_liquidity_security, normalize_security},
};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::{Gecko, GoPlus, included_map, pool, token_from_resource},
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenArgs {
    /// Exact Robinhood Chain ERC-20 0x contract address. Do not pass a symbol
    /// or name; resolve those with hoodit_search_tokens first.
    pub token: String,
    /// Optional opaque pool_id returned by a Hoodit search, discovery, token,
    /// candle, or trade result. Omit to use the token's indexed top pool.
    #[serde(default)]
    #[schemars(
        with = "Option<String>",
        length(min = 1, max = 200),
        pattern(r"^[A-Za-z0-9:_-]+$")
    )]
    pub pool_id: Option<String>,
    /// Include bounded public project metadata such as description and links.
    /// Omit for false when only market statistics are needed.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub include_metadata: Option<bool>,
    /// Security detail to request. Summary returns compact score, honeypot,
    /// tax, source-verification, proxy, and ownership facts. Full also returns
    /// available contract-control flags. Omit for summary.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["none", "summary", "full"], "default" = "summary"))]
    pub security: Option<String>,
    /// Include up to ten source-labelled top-holder rows when available.
    /// Omit for false; aggregate holder facts may still be returned.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub include_holders: Option<bool>,
    /// Bypass short-lived provider caches. Omit for false and use true only
    /// when a fresh observation materially matters.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub refresh: Option<bool>,
}

pub struct GetToken;

impl DynAomiTool for GetToken {
    type App = HooditApp;
    type Args = TokenArgs;
    const NAME: &'static str = "hoodit_get_token";
    const DESCRIPTION: &'static str = "Read market, selected-pool, token-security, and ownership observations for one exact Robinhood Chain ERC-20 contract. Requires a 0x contract address, not a symbol; use hoodit_search_tokens first when identity is ambiguous. Provider facts remain source-labelled and are not an executable quote or a Hoodit safety score.";

    fn run(app: &HooditApp, args: TokenArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let token = invalid_argument!(model::address(&args.token));
        let explicit_pool_id =
            invalid_argument!(args.pool_id.as_deref().map(normalize_pool_id).transpose());
        let security_level = args.security.as_deref().unwrap_or("summary");
        if !["none", "summary", "full"].contains(&security_level) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "security must be none, summary, or full",
                false,
            ));
        }
        let runtime = app.runtime()?;
        let gecko = Gecko::new(&runtime);
        let mut read = ReadContext::markets(args.refresh.unwrap_or(false));
        let token_response = match gecko.token(&token, &mut read) {
            Ok(response) => response,
            Err(error) => return Ok(provider_error(error)),
        };
        let Some(token_resource) = response_rows(&token_response).into_iter().next() else {
            return Ok(model::error(
                "UPSTREAM_SCHEMA_CHANGED",
                "token response is empty",
                false,
            ));
        };
        let attributes = token_resource
            .get("attributes")
            .cloned()
            .unwrap_or(json!({}));
        let token_details = token_from_resource(&token_resource);
        let mut warnings = vec![];
        let top_pool_ids = model::get(&token_resource, &["relationships", "top_pools", "data"])
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|pool| model::string(pool, &["id"]))
            .map(|id| id.strip_prefix("robinhood_").unwrap_or(&id).to_string())
            .collect::<Vec<_>>();
        let automatic_pool_id = top_pool_ids.first().map(String::as_str);
        let selected_pool = match resolve_pool(
            &gecko,
            &token,
            explicit_pool_id.as_deref().or(automatic_pool_id),
            &mut read,
        ) {
            Ok((pool, _)) => Some(pool),
            Err(error) if explicit_pool_id.is_none() => {
                warnings.push(model::warning(
                    if error.code == "NO_INDEXED_POOL" {
                        "NOT_INDEXED"
                    } else {
                        "METADATA_UNAVAILABLE"
                    },
                    if error.code == "NO_INDEXED_POOL" {
                        "No indexed reference pool was found"
                    } else {
                        "Token detail is available, but selected-pool context could not be read"
                    },
                ));
                None
            }
            Err(error) => return Ok(provider_error(error)),
        };
        let wants_metadata = args.include_metadata.unwrap_or(false);
        let gecko_info = if wants_metadata || security_level != "none" {
            match gecko.metadata(&token, &mut read) {
                Ok(response) => Some(response),
                Err(_) => {
                    warnings.push(model::warning(
                        "METADATA_UNAVAILABLE",
                        "Requested token metadata or security context could not be read",
                    ));
                    None
                }
            }
        } else {
            None
        };
        let metadata = wants_metadata
            .then(|| {
                gecko_info
                    .as_ref()
                    .and_then(|response| response_rows(response).into_iter().next())
                    .map(|resource| normalize_metadata(&resource))
            })
            .flatten();
        let goplus = if security_level != "none" {
            match GoPlus::new(&runtime).token_security(&token, &mut read) {
                Ok(response) => Some(response),
                Err(_) => {
                    warnings.push(model::warning(
                        "SECURITY_DATA_UNAVAILABLE",
                        "One token-security source could not be read; unknown checks remain unknown",
                    ));
                    None
                }
            }
        } else {
            None
        };
        let (security, ownership, security_coverage) = normalize_security(
            &token,
            gecko_info.as_ref(),
            goplus.as_ref(),
            security_level,
            args.include_holders.unwrap_or(false),
        );
        let selected_token_price_usd = selected_pool.as_ref().and_then(|pool| {
            if model::string(pool, &["base_token", "id"]).as_deref() == Some(token.as_str()) {
                pool.get("base_price_usd").cloned()
            } else {
                pool.get("quote_price_usd").cloned()
            }
        });
        let selected_pool_id = selected_pool
            .as_ref()
            .and_then(|pool| model::string(pool, &["pool_id"]));
        let other_pools = match gecko.token_pools(&token, &mut read) {
            Ok(response) => {
                let included = included_map(&response);
                response_rows(&response)
                    .iter()
                    .map(|row| pool(row, &included))
                    .filter(|pool| {
                        model::string(pool, &["pool_id"]).as_deref() != selected_pool_id.as_deref()
                    })
                    .take(8)
                    .map(|pool| json!({"pool_id":pool["pool_id"],"dex_id":pool["dex_id"],"dex_name":pool["dex_name"],"liquidity_usd":pool["liquidity_usd"],"volume_24h_usd":pool["windows"]["h24"]["volume_usd"]}))
                    .collect::<Vec<_>>()
            }
            Err(_) => {
                warnings.push(model::warning(
                    "POOL_COVERAGE_PARTIAL",
                    "Additional indexed pools could not be listed",
                ));
                Vec::new()
            }
        };
        let community = if security_level == "full" {
            selected_pool_id
                .as_deref()
                .and_then(|pool_id| gecko.pool_info(pool_id, &mut read).ok())
                .and_then(|response| response_rows(&response).into_iter().next())
                .map(|resource| {
                    let attributes = resource.get("attributes").unwrap_or(&resource);
                    json!({
                        "sus_report":model::get(attributes,&["community_sus_report"]).cloned(),
                        "sentiment_positive_pct":model::string(attributes,&["sentiment_vote_positive_percentage"]),
                        "sentiment_negative_pct":model::string(attributes,&["sentiment_vote_negative_percentage"]),
                        "scope":"community_reports_not_verified_findings"
                    })
                })
        } else {
            None
        };
        let liquidity_security =
            normalize_liquidity_security(&token, selected_pool_id.as_deref(), goplus.as_ref());
        Ok(model::ok(
            json!({"token":token_details,"price_usd":model::string(&attributes,&["price_usd"]),"market_cap_usd":model::string(&attributes,&["market_cap_usd"]),"fdv_usd":model::string(&attributes,&["fdv_usd"]),"volume_24h_usd":model::string(&attributes,&["volume_usd","h24"]),"selected_pool":selected_pool,"selected_token_price_usd":selected_token_price_usd,"other_pools":other_pools,"metadata":metadata,"security":security,"ownership":ownership,"liquidity_security":liquidity_security,"community":community,"coverage":{"market":"token_and_selected_pool","other_pools_returned":other_pools.len(),"security":security_coverage,"prices_executable":false}}),
            read.sources,
            {
                warnings.extend(read.warnings);
                warnings
            },
        ))
    }
}

fn normalize_metadata(resource: &Value) -> Value {
    let attributes = resource.get("attributes").unwrap_or(resource);
    let description = model::string(attributes, &["description"])
        .map(|description| description.chars().take(1000).collect::<String>());
    let websites = attributes
        .get("websites")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .take(5)
        .collect::<Vec<_>>();
    json!({"description":description,"websites":websites,"twitter":model::string(attributes,&["twitter_handle"]).or_else(||model::string(attributes,&["twitter_url"])),"telegram":model::string(attributes,&["telegram_handle"]).or_else(||model::string(attributes,&["telegram_url"])),"discord":model::string(attributes,&["discord_url"])})
}
