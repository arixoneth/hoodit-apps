//! Shared market argument validation and GeckoTerminal response normalization.

use crate::{
    app::ReadContext,
    model,
    providers::{Gecko, ProviderError, included_map, pool},
};
use serde_json::Value;

macro_rules! invalid_argument {
    ($value:expr) => {
        match $value {
            Ok(value) => value,
            Err(message) => {
                return Ok($crate::model::error("INVALID_ARGUMENT", &message, false));
            }
        }
    };
}

pub(super) use invalid_argument;

pub(super) fn validate_page(page: Option<u8>) -> Result<u8, String> {
    let page = page.unwrap_or(1);
    if !(1..=10).contains(&page) {
        Err("page must be between 1 and 10".into())
    } else {
        Ok(page)
    }
}

pub(super) fn normalize_pool_id(value: &str) -> Result<String, String> {
    let pool_id = value.trim();
    if pool_id.is_empty()
        || pool_id.len() > 200
        || !pool_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'_' | b'-'))
    {
        Err("invalid opaque pool identifier".into())
    } else {
        Ok(
            if pool_id.starts_with("0x")
                && pool_id[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                pool_id.to_ascii_lowercase()
            } else {
                pool_id.to_string()
            },
        )
    }
}

pub(super) fn response_rows(response: &Value) -> Vec<Value> {
    response
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            response
                .get("data")
                .filter(|data| !data.is_null())
                .map(|data| vec![data.clone()])
        })
        .unwrap_or_default()
}

pub(super) fn resolve_pool(
    gecko: &Gecko,
    token: &str,
    explicit_pool_id: Option<&str>,
    read: &mut ReadContext,
) -> Result<(Value, String), ProviderError> {
    let response = if let Some(pool_id) = explicit_pool_id {
        gecko.pool(pool_id, read)?
    } else {
        let token_response = gecko.token(token, read)?;
        let top_pool_id = model::get(
            &token_response,
            &["data", "relationships", "top_pools", "data"],
        )
        .and_then(Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|pool| model::string(pool, &["id"]))
        .map(|pool_id| {
            pool_id
                .strip_prefix("robinhood_")
                .unwrap_or(&pool_id)
                .to_string()
        });
        if let Some(pool_id) = top_pool_id {
            gecko.pool(&pool_id, read)?
        } else {
            gecko.token_pools(token, read)?
        }
    };
    let included = included_map(&response);
    let row = response_rows(&response)
        .into_iter()
        .next()
        .ok_or(ProviderError {
            code: "NO_INDEXED_POOL",
            message: "no indexed pool was found".into(),
            retryable: false,
        })?;
    let pool = pool(&row, &included);
    let pool_id = model::string(&pool, &["pool_id"]).ok_or(ProviderError {
        code: "UPSTREAM_SCHEMA_CHANGED",
        message: "pool identifier is missing".into(),
        retryable: false,
    })?;
    let contains_token = ["base_token", "quote_token"]
        .iter()
        .filter_map(|side| model::string(&pool, &[side, "id"]))
        .any(|pool_token| pool_token.eq_ignore_ascii_case(token));
    if !contains_token {
        return Err(ProviderError {
            code: "TOKEN_NOT_IN_POOL",
            message: "the token is not a component of the selected pool".into(),
            retryable: false,
        });
    }
    Ok((pool, pool_id))
}

pub(super) fn decimal_at_least(actual: Option<&str>, minimum: &str) -> bool {
    use std::str::FromStr;

    let minimum = bigdecimal::BigDecimal::from_str(minimum).ok();
    let actual = actual.and_then(|value| bigdecimal::BigDecimal::from_str(value).ok());
    matches!((actual, minimum), (Some(actual), Some(minimum)) if actual >= minimum)
}
