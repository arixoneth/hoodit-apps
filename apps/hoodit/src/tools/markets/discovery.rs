use super::{
    normalization::{invalid_argument, response_rows, validate_page},
    security::normalize_security,
};
use crate::{
    app::{HooditApp, ReadContext},
    model,
    providers::{Gecko, GoPlus, included_map, pool},
    tools::provider_error,
};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bigdecimal::BigDecimal;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::str::FromStr;

#[derive(Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DecimalRange {
    /// Inclusive lower bound as a plain decimal string.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub min: Option<String>,
    /// Inclusive upper bound as a plain decimal string.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub max: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedDecimalRange {
    /// Inclusive lower bound as a signed plain decimal string.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^-?(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub min: Option<String>,
    /// Inclusive upper bound as a signed plain decimal string.
    #[serde(default)]
    #[schemars(with = "Option<String>", pattern(r"^-?(0|[1-9][0-9]*)(\.[0-9]+)?$"))]
    pub max: Option<String>,
}

#[derive(Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CountRange {
    /// Inclusive lower bound.
    #[serde(default)]
    #[schemars(with = "Option<u64>")]
    pub min: Option<u64>,
    /// Inclusive upper bound.
    #[serde(default)]
    #[schemars(with = "Option<u64>")]
    pub max: Option<u64>,
}

#[derive(Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverFilters {
    /// Canonical DEX IDs from hoodit_get_market_options.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub dex_ids: Option<Vec<String>>,
    /// Require either side of the pool to match one of these exact contracts.
    #[serde(default)]
    #[schemars(with = "Option<Vec<String>>")]
    pub paired_token_addresses: Option<Vec<String>>,
    /// Pool liquidity in USD.
    #[serde(default)]
    #[schemars(with = "Option<DecimalRange>")]
    pub liquidity_usd: Option<DecimalRange>,
    /// Pool volume in USD for volume_window.
    #[serde(default)]
    #[schemars(with = "Option<DecimalRange>")]
    pub volume_usd: Option<DecimalRange>,
    /// Volume window. Omit for h24.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub volume_window: Option<String>,
    /// Base token fully diluted valuation in USD. Requires enrichment.
    #[serde(default)]
    #[schemars(with = "Option<DecimalRange>")]
    pub fdv_usd: Option<DecimalRange>,
    /// Pool age in hours at evaluation time.
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub pool_age_hours: Option<CountRange>,
    /// Base-token price change in percentage points for price_change_window.
    #[serde(default)]
    #[schemars(with = "Option<SignedDecimalRange>")]
    pub price_change_pct: Option<SignedDecimalRange>,
    /// Price-change window. Omit for h1.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub price_change_window: Option<String>,
    /// Total buys plus sells for activity_window.
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub transactions: Option<CountRange>,
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub buys: Option<CountRange>,
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub sells: Option<CountRange>,
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub buyers: Option<CountRange>,
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub sellers: Option<CountRange>,
    /// Activity-count window. Omit for h24.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub activity_window: Option<String>,
    /// Minimum GeckoTerminal score. Unknown scores fail this filter.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub min_gt_score: Option<String>,
    /// Honeypot behavior. exclude_flagged permits unknown; require_clear
    /// requires at least one explicit clear observation and no conflict.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub honeypot: Option<String>,
    /// Maximum GoPlus tax in percentage points; 5 means 5%, not 0.05.
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub max_buy_tax_pct: Option<String>,
    #[serde(default)]
    #[schemars(with = "Option<String>")]
    pub max_sell_tax_pct: Option<String>,
    /// Require GeckoTerminal metadata verification. This is not source-code verification.
    #[serde(default)]
    #[schemars(with = "Option<bool>")]
    pub require_gt_verified: Option<bool>,
    /// Require GoPlus to explicitly report open source.
    #[serde(default)]
    #[schemars(with = "Option<bool>")]
    pub require_open_source: Option<bool>,
    #[serde(default)]
    #[schemars(with = "Option<CountRange>")]
    pub holder_count: Option<CountRange>,
    /// Top-ten holder concentration in percentage points.
    #[serde(default)]
    #[schemars(with = "Option<DecimalRange>")]
    pub top10_concentration_pct: Option<DecimalRange>,
    /// Require at least one project website or social link in indexed metadata.
    #[serde(default)]
    #[schemars(with = "Option<bool>")]
    pub require_social_presence: Option<bool>,
    /// Require an explicit CoinGecko coin identifier in indexed metadata.
    #[serde(default)]
    #[schemars(with = "Option<bool>")]
    pub require_coingecko_listed: Option<bool>,
}

#[derive(Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiscoverArgs {
    /// Ranking to browse, or screened for a bounded strict local scan.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["trending", "new", "top_volume", "top_activity", "screened"], "default" = "trending"))]
    pub feed: Option<String>,
    /// Ranking window for trending and screened feeds. Omit for 24h.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["5m", "1h", "6h", "24h"], "default" = "24h"))]
    pub duration: Option<String>,
    /// Backward-compatible one-based start page. Omit for page 1.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 10), extend("default" = 1))]
    pub page: Option<u8>,
    /// Backward-compatible local minimum liquidity filter.
    #[serde(default)]
    #[schemars(with = "String", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"), extend("default" = "0"))]
    pub min_liquidity_usd: Option<String>,
    /// Backward-compatible local minimum h24 volume filter.
    #[serde(default)]
    #[schemars(with = "String", pattern(r"^(0|[1-9][0-9]*)(\.[0-9]+)?$"), extend("default" = "0"))]
    pub min_volume_24h_usd: Option<String>,
    /// Structured strict filters. Unknown values fail explicitly required checks.
    #[serde(default)]
    #[schemars(with = "Option<DiscoverFilters>")]
    pub filters: Option<DiscoverFilters>,
    /// Result ordering within the scanned candidates.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["feed", "liquidity", "volume_24h", "created_at", "price_change"], "default" = "feed"))]
    pub sort: Option<String>,
    /// Sort direction within the scanned candidate set. Omit for descending.
    #[serde(default)]
    #[schemars(with = "String", extend("enum" = ["asc", "desc"], "default" = "desc"))]
    pub direction: Option<String>,
    /// Maximum returned pools, from 1 through 20. Omit for 10 in screened mode.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 20), extend("default" = 10))]
    pub limit: Option<u8>,
    /// Maximum raw provider pages to scan, from 1 through 3. Omit for 1.
    #[serde(default)]
    #[schemars(with = "u8", range(min = 1, max = 3), extend("default" = 1))]
    pub max_pages: Option<u8>,
    /// Keep at most one pool per base token across continuation pages.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub deduplicate_tokens: Option<bool>,
    /// Opaque screened continuation. It is bound to the normalized query.
    #[serde(default)]
    #[schemars(
        with = "Option<String>",
        length(min = 1, max = 8192),
        pattern(r"^[A-Za-z0-9_-]+$")
    )]
    pub cursor: Option<String>,
    /// Bypass short-lived caches. Omit for false.
    #[serde(default)]
    #[schemars(with = "bool", extend("default" = false))]
    pub refresh: Option<bool>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScreenCursor {
    v: u8,
    query: String,
    next_page: u8,
    seen_tokens: Vec<String>,
}

pub struct DiscoverPools;

impl DynAomiTool for DiscoverPools {
    type App = HooditApp;
    type Args = DiscoverArgs;
    const NAME: &'static str = "hoodit_discover_pools";
    const DESCRIPTION: &'static str = "Browse or strictly screen GeckoTerminal-indexed Robinhood Chain pools with bounded cross-page scanning, optional token security and ownership conditions, explicit unknown handling, and coverage accounting. Rankings are best among scanned candidates, not market-wide or executable routes.";

    fn run(app: &HooditApp, args: DiscoverArgs, _: DynToolCallCtx) -> Result<Value, String> {
        let feed = args.feed.as_deref().unwrap_or("trending");
        if !["trending", "new", "top_volume", "top_activity", "screened"].contains(&feed) {
            return Ok(model::error("INVALID_ARGUMENT", "unsupported feed", false));
        }
        let duration = args.duration.as_deref().unwrap_or("24h");
        if !["5m", "1h", "6h", "24h"].contains(&duration) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "unsupported duration",
                false,
            ));
        }
        if args.cursor.is_some() && feed != "screened" {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "cursor is supported only by the screened feed",
                false,
            ));
        }
        let start_page = invalid_argument!(validate_page(args.page));
        let legacy_liquidity = invalid_argument!(model::decimal(
            args.min_liquidity_usd.as_deref().unwrap_or("0")
        ));
        let legacy_volume = invalid_argument!(model::decimal(
            args.min_volume_24h_usd.as_deref().unwrap_or("0")
        ));
        let filters = args.filters.clone().unwrap_or_default();
        if let Err(message) = validate_filters(&filters) {
            return Ok(model::error("INVALID_ARGUMENT", &message, false));
        }
        let sort = args.sort.as_deref().unwrap_or("feed");
        if ![
            "feed",
            "liquidity",
            "volume_24h",
            "created_at",
            "price_change",
        ]
        .contains(&sort)
        {
            return Ok(model::error("INVALID_ARGUMENT", "unsupported sort", false));
        }
        let direction = args.direction.as_deref().unwrap_or("desc");
        if !["asc", "desc"].contains(&direction) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "unsupported sort direction",
                false,
            ));
        }
        let limit = args
            .limit
            .unwrap_or(if feed == "screened" { 10 } else { 20 });
        if !(1..=20).contains(&limit) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "limit must be 1 to 20",
                false,
            ));
        }
        let max_pages = args.max_pages.unwrap_or(1);
        if !(1..=3).contains(&max_pages) {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "max_pages must be 1 to 3",
                false,
            ));
        }
        if feed != "screened" && max_pages != 1 {
            return Ok(model::error(
                "INVALID_ARGUMENT",
                "max_pages greater than one requires feed=screened",
                false,
            ));
        }

        let query = query_fingerprint(
            duration,
            &legacy_liquidity,
            &legacy_volume,
            &filters,
            sort,
            direction,
            limit,
            max_pages,
            args.deduplicate_tokens.unwrap_or(false),
        );
        let (mut page, mut seen_tokens) = match args.cursor.as_deref() {
            Some(cursor) => match decode_cursor(cursor, &query) {
                Ok(cursor) => (cursor.next_page, cursor.seen_tokens.into_iter().collect()),
                Err(message) => return Ok(model::error("INVALID_ARGUMENT", &message, false)),
            },
            None => (start_page, HashSet::new()),
        };

        let runtime = app.runtime()?;
        let gecko = Gecko::new(&runtime);
        let goplus = GoPlus::new(&runtime);
        let mut read = ReadContext::markets(args.refresh.unwrap_or(false));
        let mut candidates = vec![];
        let mut scanned = 0usize;
        let mut cheap_matches = 0usize;
        let mut enriched = 0usize;
        let mut unknown_exclusions = 0usize;
        let mut duplicate_exclusions = 0usize;
        let mut pages_scanned = 0u8;
        let mut last_page_full = false;
        let requires_enrichment = needs_enrichment(&filters);
        let enrichment_cap = if requires_enrichment { 4 } else { 0 };

        while page <= 10 && pages_scanned < max_pages && candidates.len() < limit as usize {
            let upstream_feed = if feed == "screened" { "trending" } else { feed };
            let response = match gecko.discover(upstream_feed, duration, page, &mut read) {
                Ok(response) => response,
                Err(error) if pages_scanned == 0 => return Ok(provider_error(error)),
                Err(_) => break,
            };
            pages_scanned += 1;
            let included = included_map(&response);
            let rows = response_rows(&response);
            last_page_full = rows.len() >= 20;
            scanned += rows.len();
            for row in &rows {
                let mut candidate = pool(row, &included);
                if !pool_matches(&candidate, &filters, &legacy_liquidity, &legacy_volume) {
                    continue;
                }
                cheap_matches += 1;
                let token = model::string(&candidate, &["base_token", "id"]);
                if args.deduplicate_tokens.unwrap_or(false)
                    && token
                        .as_ref()
                        .is_some_and(|token| !seen_tokens.insert(token.clone()))
                {
                    duplicate_exclusions += 1;
                    continue;
                }
                if requires_enrichment {
                    if enriched >= enrichment_cap {
                        unknown_exclusions += 1;
                        continue;
                    }
                    let Some(token) = token else {
                        unknown_exclusions += 1;
                        continue;
                    };
                    enriched += 1;
                    let token_response = gecko.token(&token, &mut read).ok();
                    let gecko_info = gecko.metadata(&token, &mut read).ok();
                    let goplus_info = goplus.token_security(&token, &mut read).ok();
                    let (security, ownership, coverage) = normalize_security(
                        &token,
                        gecko_info.as_ref(),
                        goplus_info.as_ref(),
                        "summary",
                        false,
                    );
                    let enrichment = json!({
                        "market":token_response.as_ref().and_then(|value|response_rows(value).into_iter().next()).and_then(|value|value.get("attributes").cloned()),
                        "metadata":gecko_info.as_ref().and_then(|value|response_rows(value).into_iter().next()).and_then(|value|value.get("attributes").cloned()),
                        "security":security,
                        "ownership":ownership,
                        "coverage":coverage
                    });
                    if !enrichment_matches(&enrichment, &filters) {
                        unknown_exclusions += 1;
                        continue;
                    }
                    candidate["enrichment"] = enrichment;
                }
                candidates.push(candidate);
                if candidates.len() >= limit as usize {
                    break;
                }
            }
            page += 1;
            if !last_page_full {
                break;
            }
        }

        if sort != "feed" {
            candidates.sort_by(|left, right| compare(left, right, sort, &filters));
            if direction == "desc" {
                candidates.reverse();
            }
        } else if direction == "asc" {
            candidates.reverse();
        }
        candidates.truncate(limit as usize);
        let returned = candidates.len();
        let has_more = last_page_full && page <= 10;
        let next_cursor = if feed == "screened" && has_more {
            Some(encode_cursor(ScreenCursor {
                v: 1,
                query: query.clone(),
                next_page: page,
                seen_tokens: seen_tokens.into_iter().take(200).collect(),
            }))
        } else {
            None
        };
        let next_page = if feed != "screened" && has_more {
            Some(page)
        } else {
            None
        };
        let mut warnings = read.warnings;
        if feed == "screened" && has_more {
            warnings.push(model::warning(
                "SCAN_BOUND_REACHED",
                "The strict screen stopped at its disclosed page or result bound; continue with next_cursor",
            ));
        }
        if requires_enrichment && cheap_matches > enriched {
            warnings.push(model::warning(
                "ENRICHMENT_BOUND_REACHED",
                "Security-enriched screening stopped at its disclosed candidate bound; no filter was relaxed",
            ));
        }
        Ok(model::ok(
            json!({
                "feed":feed,
                "duration":if feed=="new"||feed=="top_volume"||feed=="top_activity"{None}else{Some(duration)},
                "min_liquidity_usd":legacy_liquidity,
                "min_volume_24h_usd":legacy_volume,
                "filters":filters,
                "sort":sort,
                "direction":direction,
                "pools":candidates,
                "pagination":{"start_page":start_page,"pages_scanned":pages_scanned,"returned":returned,"next_page":next_page,"next_cursor":next_cursor},
                "coverage":{
                    "mode":if feed=="screened"{"free_bounded_scan"}else{"single_provider_page"},
                    "scanned":scanned,
                    "cheap_filter_matches":cheap_matches,
                    "enriched":enriched,
                    "enrichment_cap":enrichment_cap,
                    "unknown_or_failed_required_exclusions":unknown_exclusions,
                    "duplicate_exclusions":duplicate_exclusions,
                    "ranked_within_scanned_candidates":true,
                    "filters_relaxed":false,
                    "stop_reason":if returned>=limit as usize{"result_limit"}else if has_more{"scan_bound"}else{"provider_exhausted"}
                }
            }),
            read.sources,
            warnings,
        ))
    }
}

fn validate_filters(filters: &DiscoverFilters) -> Result<(), String> {
    let windows = ["m5", "m15", "m30", "h1", "h6", "h24"];
    for window in [
        filters.volume_window.as_deref(),
        filters.price_change_window.as_deref(),
        filters.activity_window.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !windows.contains(&window) {
            return Err(format!("unsupported filter window: {window}"));
        }
    }
    if let Some(policy) = filters.honeypot.as_deref()
        && !["any", "exclude_flagged", "require_clear"].contains(&policy)
    {
        return Err("honeypot must be any, exclude_flagged, or require_clear".into());
    }
    for range in [
        filters.liquidity_usd.as_ref(),
        filters.volume_usd.as_ref(),
        filters.fdv_usd.as_ref(),
        filters.top10_concentration_pct.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_decimal_range(range)?;
    }
    if let Some(range) = filters.price_change_pct.as_ref() {
        validate_signed_decimal_range(range)?;
    }
    for value in [
        filters.min_gt_score.as_deref(),
        filters.max_buy_tax_pct.as_deref(),
        filters.max_sell_tax_pct.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        model::decimal(value)?;
    }
    for range in [
        filters.pool_age_hours.as_ref(),
        filters.transactions.as_ref(),
        filters.buys.as_ref(),
        filters.sells.as_ref(),
        filters.buyers.as_ref(),
        filters.sellers.as_ref(),
        filters.holder_count.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if range.min.zip(range.max).is_some_and(|(min, max)| min > max) {
            return Err("range minimum cannot exceed maximum".into());
        }
    }
    for address in filters
        .paired_token_addresses
        .as_deref()
        .unwrap_or_default()
    {
        model::address(address)?;
    }
    if filters.dex_ids.as_deref().unwrap_or_default().len() > 20
        || filters
            .dex_ids
            .as_deref()
            .unwrap_or_default()
            .iter()
            .any(|id| id.is_empty() || id.len() > 100)
    {
        return Err("invalid dex_ids".into());
    }
    Ok(())
}

fn validate_bounds(min: Option<String>, max: Option<String>) -> Result<(), String> {
    if let (Some(min), Some(max)) = (min, max)
        && BigDecimal::from_str(&min).ok() > BigDecimal::from_str(&max).ok()
    {
        return Err("range minimum cannot exceed maximum".into());
    }
    Ok(())
}

fn validate_decimal_range(range: &DecimalRange) -> Result<(), String> {
    validate_bounds(
        range.min.as_deref().map(model::decimal).transpose()?,
        range.max.as_deref().map(model::decimal).transpose()?,
    )
}

fn validate_signed_decimal_range(range: &SignedDecimalRange) -> Result<(), String> {
    validate_bounds(
        range
            .min
            .as_deref()
            .map(model::signed_decimal)
            .transpose()?,
        range
            .max
            .as_deref()
            .map(model::signed_decimal)
            .transpose()?,
    )
}

fn decimal_in_bounds(value: Option<String>, min: Option<&str>, max: Option<&str>) -> bool {
    let Some(value) = value.and_then(|value| BigDecimal::from_str(&value).ok()) else {
        return false;
    };
    min.and_then(|value| BigDecimal::from_str(value).ok())
        .is_none_or(|minimum| value >= minimum)
        && max
            .and_then(|value| BigDecimal::from_str(value).ok())
            .is_none_or(|maximum| value <= maximum)
}

fn decimal_in(value: Option<String>, range: Option<&DecimalRange>) -> bool {
    let Some(range) = range else { return true };
    decimal_in_bounds(value, range.min.as_deref(), range.max.as_deref())
}

fn signed_decimal_in(value: Option<String>, range: Option<&SignedDecimalRange>) -> bool {
    let Some(range) = range else { return true };
    decimal_in_bounds(value, range.min.as_deref(), range.max.as_deref())
}

fn count_in(value: Option<u64>, range: Option<&CountRange>) -> bool {
    let Some(range) = range else { return true };
    let Some(value) = value else { return false };
    range.min.is_none_or(|minimum| value >= minimum)
        && range.max.is_none_or(|maximum| value <= maximum)
}

fn pool_matches(
    pool: &Value,
    filters: &DiscoverFilters,
    legacy_liquidity: &str,
    legacy_volume: &str,
) -> bool {
    let dex_matches = filters.dex_ids.as_deref().is_none_or(|ids| {
        model::string(pool, &["dex_id"]).is_some_and(|id| ids.iter().any(|allowed| allowed == &id))
    });
    let pair_matches = filters
        .paired_token_addresses
        .as_deref()
        .is_none_or(|addresses| {
            ["base_token", "quote_token"].into_iter().any(|side| {
                model::string(pool, &[side, "id"]).is_some_and(|id| {
                    addresses
                        .iter()
                        .any(|address| address.eq_ignore_ascii_case(&id))
                })
            })
        });
    let liquidity = model::string(pool, &["liquidity_usd"]);
    let volume_window = filters.volume_window.as_deref().unwrap_or("h24");
    let volume = model::string(pool, &["windows", volume_window, "volume_usd"]);
    let change_window = filters.price_change_window.as_deref().unwrap_or("h1");
    let change = model::string(pool, &["windows", change_window, "base_price_change_pct"]);
    let activity_window = filters.activity_window.as_deref().unwrap_or("h24");
    let count =
        |name: &str| model::get(pool, &["windows", activity_window, name]).and_then(Value::as_u64);
    let age = model::string(pool, &["created_at"])
        .and_then(|created| chrono::DateTime::parse_from_rfc3339(&created).ok())
        .map(|created| (Utc::now().timestamp() - created.timestamp()).max(0) as u64 / 3600);
    dex_matches
        && pair_matches
        && decimal_in(liquidity.clone(), filters.liquidity_usd.as_ref())
        && decimal_in(volume.clone(), filters.volume_usd.as_ref())
        && signed_decimal_in(change, filters.price_change_pct.as_ref())
        && decimal_in(
            liquidity,
            Some(&DecimalRange {
                min: Some(legacy_liquidity.into()),
                max: None,
            }),
        )
        && decimal_in(
            volume,
            Some(&DecimalRange {
                min: Some(legacy_volume.into()),
                max: None,
            }),
        )
        && count_in(age, filters.pool_age_hours.as_ref())
        && count_in(
            count("buys")
                .zip(count("sells"))
                .map(|(buys, sells)| buys + sells),
            filters.transactions.as_ref(),
        )
        && count_in(count("buys"), filters.buys.as_ref())
        && count_in(count("sells"), filters.sells.as_ref())
        && count_in(count("buyers"), filters.buyers.as_ref())
        && count_in(count("sellers"), filters.sellers.as_ref())
}

fn needs_enrichment(filters: &DiscoverFilters) -> bool {
    filters.min_gt_score.is_some()
        || filters
            .honeypot
            .as_deref()
            .is_some_and(|value| value != "any")
        || filters.max_buy_tax_pct.is_some()
        || filters.max_sell_tax_pct.is_some()
        || filters.require_gt_verified.unwrap_or(false)
        || filters.require_open_source.unwrap_or(false)
        || filters.holder_count.is_some()
        || filters.top10_concentration_pct.is_some()
        || filters.require_social_presence.unwrap_or(false)
        || filters.require_coingecko_listed.unwrap_or(false)
        || filters.fdv_usd.is_some()
}

fn enrichment_matches(enrichment: &Value, filters: &DiscoverFilters) -> bool {
    let minimum_score = filters.min_gt_score.as_deref().map(|value| DecimalRange {
        min: Some(value.into()),
        max: None,
    });
    let gt_score = model::string(enrichment, &["security", "gt", "score"]);
    let taxes = |side: &str| model::string(enrichment, &["security", "taxes", side]);
    let maximum = |value: Option<&String>| {
        value.map(|value| DecimalRange {
            min: None,
            max: Some(value.clone()),
        })
    };
    let holder_count = model::get(enrichment, &["ownership", "holder_count", "geckoterminal"])
        .and_then(Value::as_u64)
        .or_else(|| {
            model::get(enrichment, &["ownership", "holder_count", "goplus"]).and_then(Value::as_u64)
        });
    let top10 = model::string(
        enrichment,
        &["ownership", "top10_concentration_pct", "geckoterminal"],
    )
    .or_else(|| {
        model::string(
            enrichment,
            &["ownership", "top10_concentration_pct", "goplus"],
        )
    });
    let honeypot = model::string(enrichment, &["security", "honeypot", "assessment"])
        .unwrap_or_else(|| "unknown".into());
    let honeypot_matches = match filters.honeypot.as_deref().unwrap_or("any") {
        "exclude_flagged" => !matches!(honeypot.as_str(), "flagged" | "conflicting"),
        "require_clear" => matches!(honeypot.as_str(), "clear" | "clear_single_source"),
        _ => true,
    };
    let social = model::get(enrichment, &["metadata"]).is_some_and(|metadata| {
        [
            "websites",
            "twitter_handle",
            "twitter_url",
            "telegram_handle",
            "telegram_url",
            "discord_url",
        ]
        .into_iter()
        .any(|field| match metadata.get(field) {
            Some(Value::String(value)) => !value.trim().is_empty(),
            Some(Value::Array(values)) => !values.is_empty(),
            _ => false,
        })
    });
    let listed = model::string(enrichment, &["metadata", "coingecko_coin_id"])
        .is_some_and(|value| !value.is_empty());
    let fdv = model::string(enrichment, &["market", "fdv_usd"]);
    decimal_in(gt_score, minimum_score.as_ref())
        && honeypot_matches
        && decimal_in(
            taxes("buy_pct"),
            maximum(filters.max_buy_tax_pct.as_ref()).as_ref(),
        )
        && decimal_in(
            taxes("sell_pct"),
            maximum(filters.max_sell_tax_pct.as_ref()).as_ref(),
        )
        && (!filters.require_gt_verified.unwrap_or(false)
            || model::get(enrichment, &["security", "gt", "verified"]).and_then(Value::as_bool)
                == Some(true))
        && (!filters.require_open_source.unwrap_or(false)
            || model::get(enrichment, &["security", "contract", "open_source"])
                .and_then(Value::as_bool)
                == Some(true))
        && count_in(holder_count, filters.holder_count.as_ref())
        && decimal_in(top10, filters.top10_concentration_pct.as_ref())
        && (!filters.require_social_presence.unwrap_or(false) || social)
        && (!filters.require_coingecko_listed.unwrap_or(false) || listed)
        && decimal_in(fdv, filters.fdv_usd.as_ref())
}

fn compare(
    left: &Value,
    right: &Value,
    sort: &str,
    filters: &DiscoverFilters,
) -> std::cmp::Ordering {
    if sort == "created_at" {
        return model::string(left, &["created_at"]).cmp(&model::string(right, &["created_at"]));
    }
    let path: Vec<&str> = match sort {
        "liquidity" => vec!["liquidity_usd"],
        "volume_24h" => vec!["windows", "h24", "volume_usd"],
        "price_change" => vec![
            "windows",
            filters.price_change_window.as_deref().unwrap_or("h1"),
            "base_price_change_pct",
        ],
        _ => vec!["liquidity_usd"],
    };
    let decimal = |value: &Value| {
        model::string(value, &path).and_then(|value| BigDecimal::from_str(&value).ok())
    };
    decimal(left)
        .partial_cmp(&decimal(right))
        .unwrap_or(std::cmp::Ordering::Equal)
}

#[allow(clippy::too_many_arguments)]
fn query_fingerprint(
    duration: &str,
    legacy_liquidity: &str,
    legacy_volume: &str,
    filters: &DiscoverFilters,
    sort: &str,
    direction: &str,
    limit: u8,
    max_pages: u8,
    deduplicate_tokens: bool,
) -> String {
    let normalized = serde_json::to_string(&json!({
        "duration":duration,"legacy_liquidity":legacy_liquidity,"legacy_volume":legacy_volume,
        "filters":filters,"sort":sort,"direction":direction,"limit":limit,"max_pages":max_pages,
        "deduplicate_tokens":deduplicate_tokens
    }))
    .unwrap_or_default();
    let mut hasher = DefaultHasher::new();
    normalized.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn encode_cursor(cursor: ScreenCursor) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(&cursor).unwrap_or_default())
}

fn decode_cursor(raw: &str, query: &str) -> Result<ScreenCursor, String> {
    if raw.len() > 8192 {
        return Err("discovery cursor is too large".into());
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| "invalid discovery cursor")?;
    let cursor: ScreenCursor =
        serde_json::from_slice(&bytes).map_err(|_| "invalid discovery cursor")?;
    if cursor.v != 1
        || cursor.query != query
        || !(1..=10).contains(&cursor.next_page)
        || cursor.seen_tokens.len() > 200
        || cursor
            .seen_tokens
            .iter()
            .any(|token| model::address(token).is_err())
    {
        return Err("cursor does not belong to this discovery query".into());
    }
    Ok(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_unknown_security_fails_required_filters() {
        let filters = DiscoverFilters {
            min_gt_score: Some("75".into()),
            require_open_source: Some(true),
            ..Default::default()
        };
        assert!(!enrichment_matches(&json!({}), &filters));
    }

    #[test]
    fn cursor_is_bound_to_query_and_dedup_state() {
        let cursor = encode_cursor(ScreenCursor {
            v: 1,
            query: "abc".into(),
            next_page: 2,
            seen_tokens: vec!["0x1111111111111111111111111111111111111111".into()],
        });
        assert_eq!(decode_cursor(&cursor, "abc").unwrap().next_page, 2);
        assert!(decode_cursor(&cursor, "different").is_err());
    }

    #[test]
    fn empty_social_metadata_and_missing_fdv_fail_strict_filters() {
        let filters = DiscoverFilters {
            require_social_presence: Some(true),
            fdv_usd: Some(DecimalRange {
                min: Some("1".into()),
                max: None,
            }),
            ..Default::default()
        };
        let enrichment = json!({"metadata":{"websites":[],"twitter_handle":""},"market":{}});
        assert!(!enrichment_matches(&enrichment, &filters));
    }
}
