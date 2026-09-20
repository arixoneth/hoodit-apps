use super::valuation::{
    HoldingInput, add_allocations, apply_market_valuations, make_holding,
    normalize_inventory_holding, quote_token, sort_holdings_by, sum_holding_values,
    sum_market_values, verify_usdg_token,
};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::{Blockscout, CoinGecko, Gecko, GoPlus, Lifi, ProviderError, included_map, pool},
    tools::markets::security::normalize_security,
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::str::FromStr;

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PortfolioArgs {
    /// Exact public 0x wallet address on Robinhood Chain. For "my wallet",
    /// resolve the funded executor with get_account_info on chain 4663 first.
    pub wallet_address: String,
    /// Opaque continuation returned by the previous portfolio response. Use
    /// JSON null for the first page (or omit the field when the client permits
    /// omission). For later pages, reuse only the exact next_cursor returned
    /// for this wallet; never invent a cursor or placeholder.
    #[serde(
        default,
        deserialize_with = "first_page_cursor",
        skip_serializing_if = "Option::is_none"
    )]
    // The host makes every property required for strict function calling, so
    // this field must retain Option's null type to represent the first page.
    #[schemars(
        with = "Option<String>",
        length(min = 1, max = 4096),
        pattern(r"^[A-Za-z0-9_-]+$")
    )]
    pub cursor: Option<String>,
    /// Estimate up to a bounded sample of holdings in USDG using LI.FI read
    /// quotes. Omit for false for a faster balance-only inventory read.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub include_quotes: Option<bool>,
    /// Valuation mode. none is balances only, market uses observed USD marks,
    /// and quote_sample uses bounded LI.FI samples into USDG. When omitted,
    /// legacy include_quotes=true selects quote_sample.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["none", "market", "quote_sample"], "default" = "none"))]
    pub valuation: Option<String>,
    /// Attach compact source-labelled security observations to a bounded set
    /// of holdings. Omit for none.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["none", "summary"], "default" = "none"))]
    pub security: Option<String>,
    /// Display ordering. Omit for value descending.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["value_desc", "symbol"], "default" = "value_desc"))]
    pub sort: Option<String>,
    /// Minimum observed USD value to display. Unpriced holdings remain visible
    /// unless include_unpriced=false. Omit for no minimum.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub min_value_usd: Option<String>,
    /// Keep unpriced holdings visible when a value filter is used. Omit for true.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = true))]
    pub include_unpriced: Option<bool>,
    /// Bypass Hoodit's short-lived read cache. Omit for false; use true only
    /// when the user explicitly asks for a fresh provider read.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub refresh: Option<bool>,
}

pub struct GetPortfolio;

impl DynAomiTool for GetPortfolio {
    type App = HooditApp;
    type Args = PortfolioArgs;
    const NAME: &'static str = "hoodit_get_portfolio";
    const DESCRIPTION: &'static str = "Read one public Robinhood Chain wallet inventory page with exact balances, optional USD market marks or bounded USDG quote samples, allocation denominators, compact security context, and explicit page-versus-wallet coverage. Use only the exact opaque continuation returned for this wallet; no result contains cost basis, P&L, or transaction history.";

    fn run(app: &HooditApp, args: PortfolioArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        let mut read = ReadContext::portfolio(args.refresh.unwrap_or(false));
        let wallet = match model::address(&args.wallet_address) {
            Ok(wallet) => wallet,
            Err(message) => return Ok(model::error("INVALID_ARGUMENT", &message, false)),
        };
        let valuation = match (args.valuation.as_deref(), args.include_quotes) {
            (Some(value), Some(legacy)) if (value == "quote_sample") != legacy => {
                return Ok(model::error(
                    "INVALID_ARGUMENT",
                    "valuation conflicts with legacy include_quotes",
                    false,
                ));
            }
            (Some(value), _) => value,
            (None, Some(true)) => "quote_sample",
            _ => "none",
        };
        if !["none", "market", "quote_sample"].contains(&valuation) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "valuation must be none, market, or quote_sample",
                false,
            ));
        }
        let security = args.security.as_deref().unwrap_or("none");
        if !["none", "summary"].contains(&security) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "security must be none or summary",
                false,
            ));
        }
        let sort = args.sort.as_deref().unwrap_or("value_desc");
        if !["value_desc", "symbol"].contains(&sort) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "sort must be value_desc or symbol",
                false,
            ));
        }
        let min_value_usd = match args.min_value_usd.as_deref() {
            Some(value) => match model::decimal(value) {
                Ok(value) => Some(value),
                Err(message) => return Ok(model::error("INVALID_ARGUMENT", &message, false)),
            },
            None => None,
        };
        let runtime = app.runtime()?;
        let blockscout = match Blockscout::from_ctx(&runtime, &ctx) {
            Ok(blockscout) => blockscout,
            Err(error) => return Ok(provider_error(error)),
        };
        let inventory = match blockscout.inventory(&wallet, args.cursor.as_deref(), &mut read) {
            Ok(inventory) => inventory,
            Err(error) => return Ok(provider_error(error)),
        };
        let include_quotes = valuation == "quote_sample";
        if include_quotes && let Err(error) = verify_usdg_token(&blockscout, &mut read) {
            return Ok(provider_error(error));
        }
        let lifi = Lifi::from_ctx(&runtime, &ctx);
        let mut holdings = vec![];
        let mut warnings = vec![];
        let mut quote_attempts = 0;
        let mut seen_token_ids = HashSet::new();
        let native_balance = if args.cursor.is_none() {
            match blockscout.native_balance(&wallet, &mut read) {
                Ok(balance) => Some(balance),
                Err(_) => {
                    warnings.push(model::warning(
                        "NATIVE_BALANCE_UNAVAILABLE",
                        "Native balance could not be established",
                    ));
                    None
                }
            }
        } else {
            None
        };
        let native_included = native_balance.is_some();
        let mut returned = 0;
        for item in &inventory.items {
            match normalize_inventory_holding(
                item,
                &wallet,
                include_quotes,
                &mut quote_attempts,
                &lifi,
                &mut read,
                &mut warnings,
            ) {
                Ok(Some(holding)) => {
                    let token_id = model::string(&holding, &["token", "id"]).unwrap_or_default();
                    if !seen_token_ids.insert(token_id) {
                        return Ok(provider_error(ProviderError {
                            code: "UPSTREAM_SCHEMA_CHANGED",
                            message: "Blockscout returned a duplicate token holding".into(),
                            retryable: false,
                        }));
                    }
                    returned += 1;
                    holdings.push(holding);
                }
                Ok(None) => {}
                Err(error) => return Ok(provider_error(error)),
            }
        }
        if let Some(balance) = native_balance
            && balance != "0"
        {
            match make_holding(
                HoldingInput {
                    token_id: "native",
                    symbol: Some("ETH"),
                    name: Some("Ether"),
                    decimals: Some(18),
                    raw_balance: &balance,
                    wallet: &wallet,
                },
                include_quotes,
                100,
                &mut quote_attempts,
                &lifi,
                &mut read,
                &mut warnings,
            ) {
                Ok(holding) => holdings.push(holding),
                Err(error) => return Ok(provider_error(error)),
            }
        }
        let gecko = Gecko::new(&runtime);
        if valuation == "market" {
            apply_market_valuations(
                &mut holdings,
                &gecko,
                &CoinGecko::new(&runtime),
                &mut read,
                &mut warnings,
            );
        }
        let security_enriched = if security == "summary" {
            attach_security_context(&mut holdings, &gecko, &GoPlus::new(&runtime), &mut read)
        } else {
            0
        };
        sort_holdings_by(&mut holdings, sort);
        let scope = if inventory.next_cursor.is_none() && native_included {
            "wallet"
        } else {
            warnings.push(model::warning(
                "PARTIAL_PAGE",
                "This response does not cover a complete wallet traversal",
            ));
            "page"
        };
        let original_holdings_count = holdings.len();
        let minimum = min_value_usd
            .as_deref()
            .and_then(|value| bigdecimal::BigDecimal::from_str(value).ok());
        let include_unpriced = args.include_unpriced.unwrap_or(true);
        holdings.retain(|holding| {
            let value = model::string(holding, &["valuation", "value_usd"])
                .and_then(|value| bigdecimal::BigDecimal::from_str(&value).ok());
            match (value, minimum.as_ref()) {
                (Some(value), Some(minimum)) => value >= *minimum,
                (Some(_), None) => true,
                (None, _) => include_unpriced,
            }
        });
        let filtered_out_count = original_holdings_count - holdings.len();
        let priced_count = holdings
            .iter()
            .filter(|holding| {
                holding["valuation"]["value_usd"].is_string()
                    || holding["valuation"]["value_usdg"].is_string()
            })
            .count();
        let unpriced_count = holdings.len() - priced_count;
        let priced_value_usdg = sum_holding_values(&holdings);
        let priced_value_usd = sum_market_values(&holdings);
        add_allocations(&mut holdings, priced_value_usd.as_deref());
        let total_value_usdg = if scope == "wallet" && include_quotes && unpriced_count == 0 {
            priced_value_usdg.clone().or_else(|| Some("0".into()))
        } else {
            None
        };
        let total_value_usd = if scope == "wallet" && valuation == "market" && unpriced_count == 0 {
            priced_value_usd.clone().or_else(|| Some("0".into()))
        } else {
            None
        };
        let mut exposure_holdings = holdings.clone();
        sort_holdings_by(&mut exposure_holdings, "value_desc");
        let top_exposures = exposure_holdings
            .iter()
            .filter(|holding| holding["allocation"]["percentage"].is_string())
            .take(5)
            .map(|holding| {
                json!({
                    "token":holding["token"],
                    "value_usd":holding["valuation"]["value_usd"],
                    "allocation_pct":holding["allocation"]["percentage"]
                })
            })
            .collect::<Vec<_>>();
        Ok(model::ok(
            json!({"wallet_address":wallet,"valuation_mode":valuation,"security_mode":security,"quote_token":quote_token(include_quotes),"native_included":native_included,"holdings":holdings,"pagination":{"provider_rows_returned":returned,"displayed":priced_count+unpriced_count,"next_cursor":inventory.next_cursor},"summary":{"scope":scope,"display_scope":"filtered_page","holdings_count":priced_count+unpriced_count,"priced_count":priced_count,"unpriced_count":unpriced_count,"filtered_out_count":filtered_out_count,"priced_value_usd":if valuation=="market"{priced_value_usd}else{None},"total_value_usd":total_value_usd,"priced_value_usdg":if include_quotes {priced_value_usdg}else{None},"total_value_usdg":total_value_usdg,"allocation_denominator":"displayed_priced_holdings","top_exposures":top_exposures},"coverage":{"security_enriched":security_enriched,"security_enrichment_cap":if security=="summary"{4}else{0},"min_value_usd":min_value_usd,"include_unpriced":include_unpriced,"usd_and_usdg_not_conflated":true}}),
            read.sources,
            {
                warnings.extend(read.warnings);
                warnings
            },
        ))
    }
}

fn first_page_cursor<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let cursor = Option::<String>::deserialize(deserializer)?;
    Ok(cursor.and_then(|value| {
        let value = value.trim();
        if value.is_empty() || value.eq_ignore_ascii_case("null") {
            None
        } else {
            Some(value.to_string())
        }
    }))
}

fn attach_security_context(
    holdings: &mut [Value],
    gecko: &Gecko,
    goplus: &GoPlus,
    read: &mut ReadContext,
) -> usize {
    let mut enriched = 0usize;
    let mut liquidity_enriched = 0usize;
    for holding in holdings {
        let Some(token) =
            model::string(holding, &["token", "id"]).filter(|token| token != "native")
        else {
            holding["security"] = json!({"status":"unsupported_for_native"});
            continue;
        };
        if enriched >= 4 {
            holding["security"] = json!({"status":"not_enriched","reason":"response_bound"});
            continue;
        }
        let gecko_info = gecko.metadata(&token, read).ok();
        let goplus_info = goplus.token_security(&token, read).ok();
        let (security, ownership, coverage) = normalize_security(
            &token,
            gecko_info.as_ref(),
            goplus_info.as_ref(),
            "summary",
            false,
        );
        holding["security"] = security;
        holding["ownership"] = ownership;
        holding["security_coverage"] = coverage;
        enriched += 1;

        if liquidity_enriched < 3
            && let Ok(response) = gecko.token_pools(&token, read)
        {
            let included = included_map(&response);
            if let Some(selected) = response
                .get("data")
                .and_then(Value::as_array)
                .and_then(|rows| rows.first())
                .map(|row| pool(row, &included))
            {
                let liquidity = model::string(&selected, &["liquidity_usd"]);
                let position = model::string(holding, &["valuation", "value_usd"]);
                let ratio_pct = position
                    .as_deref()
                    .and_then(|value| bigdecimal::BigDecimal::from_str(value).ok())
                    .zip(
                        liquidity
                            .as_deref()
                            .and_then(|value| bigdecimal::BigDecimal::from_str(value).ok()),
                    )
                    .and_then(|(position, liquidity)| {
                        (!num_traits::Zero::is_zero(&liquidity)).then(|| {
                            ((position / liquidity) * bigdecimal::BigDecimal::from(100))
                                .with_scale_round(8, bigdecimal::RoundingMode::HalfEven)
                                .normalized()
                                .to_plain_string()
                        })
                    });
                holding["liquidity_context"] = json!({
                    "selected_pool":{"pool_id":selected["pool_id"],"dex_id":selected["dex_id"],"liquidity_usd":liquidity},
                    "position_to_pool_liquidity_pct":ratio_pct,
                    "interpretation":"size_context_only_not_a_slippage_estimate_or_executable_route"
                });
                liquidity_enriched += 1;
            }
        }
    }
    enriched
}
