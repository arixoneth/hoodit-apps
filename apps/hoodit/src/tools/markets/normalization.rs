//! Shared market argument validation and GeckoTerminal response normalization.

use crate::{
    app::ReadContext,
    model,
    providers::{Gecko, ProviderError, included_map, pool},
};
use serde_json::{Value, json};

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

pub(crate) fn normalize_pool_id(value: &str) -> Result<String, String> {
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

pub(crate) fn response_rows(response: &Value) -> Vec<Value> {
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
        let resource = token_response.get("data").cloned().unwrap_or(Value::Null);
        let mut preferred = preferred_pool_ids(&resource).into_iter();
        match preferred.next() {
            Some(first) => match gecko.pool(&first, read) {
                Ok(response) => response,
                Err(error) => match preferred.next() {
                    Some(fallback) if error.code == "POOL_NOT_FOUND" => {
                        gecko.pool(&fallback, read)?
                    }
                    _ => return Err(error),
                },
            },
            None => gecko.token_pools(token, read)?,
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

/// Launchpad lifecycle as GeckoTerminal reports it on a token resource.
/// `not_reported` means no launchpad record, which is not "0% bonded": the
/// token may have launched straight into a pool or may not be indexed.
pub(crate) fn lifecycle(token_resource: &Value) -> Value {
    let details = model::get(token_resource, &["attributes", "launchpad_details"])
        .filter(|details| details.is_object());
    let Some(details) = details else {
        return json!({"state":"not_reported","graduation_pct":null,"graduated_at":null,"destination_pool_id":null,"source":"geckoterminal"});
    };
    let completed = details.get("completed").and_then(Value::as_bool);
    let destination = model::string(details, &["migrated_destination_pool_address"])
        .and_then(|pool| normalize_pool_id(&pool).ok());
    let state = match completed {
        Some(true) => "graduated",
        Some(false) => "bonding_curve",
        None => "unknown",
    };
    let graduation_pct =
        model::string(details, &["graduation_percentage"]).and_then(|pct| plain_decimal(&pct));
    json!({
        "state":state,
        "graduation_pct":graduation_pct,
        "graduated_at":model::string(details,&["completed_at"]).filter(|_| completed == Some(true)),
        "destination_pool_id":destination.filter(|_| completed == Some(true)),
        "source":"geckoterminal"
    })
}

/// A graduated token's destination pool first, then the indexed top pools.
pub(crate) fn preferred_pool_ids(token_resource: &Value) -> Vec<String> {
    let lifecycle = lifecycle(token_resource);
    let mut ids = model::string(&lifecycle, &["destination_pool_id"])
        .into_iter()
        .collect::<Vec<_>>();
    ids.extend(
        model::get(token_resource, &["relationships", "top_pools", "data"])
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|pool| model::string(pool, &["id"]))
            .filter_map(|id| normalize_pool_id(id.strip_prefix("robinhood_").unwrap_or(&id)).ok()),
    );
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(id.clone()));
    ids
}

fn plain_decimal(value: &str) -> Option<String> {
    use std::str::FromStr;
    bigdecimal::BigDecimal::from_str(value)
        .ok()
        .filter(|value| value >= &bigdecimal::BigDecimal::from(0))
        .map(|value| value.normalized().to_plain_string())
}

pub(super) fn decimal_at_least(actual: Option<&str>, minimum: &str) -> bool {
    use std::str::FromStr;

    let minimum = bigdecimal::BigDecimal::from_str(minimum).ok();
    let actual = actual.and_then(|value| bigdecimal::BigDecimal::from_str(value).ok());
    matches!((actual, minimum), (Some(actual), Some(minimum)) if actual >= minimum)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_distinguishes_curve_graduation_and_no_record() {
        let curve = json!({"attributes":{"launchpad_details":{"graduation_percentage":50.26,"completed":false,"completed_at":null,"migrated_destination_pool_address":null}}});
        let curve = lifecycle(&curve);
        assert_eq!(curve["state"], "bonding_curve");
        assert_eq!(curve["graduation_pct"], "50.26");
        assert!(curve["destination_pool_id"].is_null());

        let destination = "0xB749D03A2000E8189FDC9412507AC60F83CCA2AAC351157CC6A139CFD7055705";
        let graduated = json!({
            "attributes":{"launchpad_details":{"graduation_percentage":100.0,"completed":true,"completed_at":"2026-09-07T16:17:38.000Z","migrated_destination_pool_address":destination}},
            "relationships":{"top_pools":{"data":[{"id":"robinhood_0x1111111111111111111111111111111111111111"},{"id":format!("robinhood_{}", destination.to_ascii_lowercase())}]}}
        });
        let state = lifecycle(&graduated);
        assert_eq!(state["state"], "graduated");
        assert_eq!(state["graduation_pct"], "100");
        assert_eq!(state["graduated_at"], "2026-09-07T16:17:38.000Z");
        assert_eq!(
            state["destination_pool_id"],
            destination.to_ascii_lowercase()
        );
        assert_eq!(
            preferred_pool_ids(&graduated),
            vec![
                destination.to_ascii_lowercase(),
                "0x1111111111111111111111111111111111111111".to_string()
            ]
        );

        let plain = lifecycle(&json!({"attributes":{"launchpad_details":null}}));
        assert_eq!(plain["state"], "not_reported");
        assert!(plain["graduation_pct"].is_null());
    }
}
