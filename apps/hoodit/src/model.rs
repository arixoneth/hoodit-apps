use serde_json::{Value, json};

pub const CHAIN_ID: u64 = 4663;
pub const NETWORK: &str = "robinhood";
pub const USDG: &str = "0x5fc5360d0400a0fd4f2af552add042d716f1d168";
pub const WETH: &str = "0x0bd7d308f8e1639fab988df18a8011f41eacad73";
pub const NATIVE: &str = "0x0000000000000000000000000000000000000000";

pub fn address(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() != 42 || !value.starts_with("0x") || !is_hex(&value[2..]) {
        return Err("expected a 20-byte 0x contract address".into());
    }
    Ok(value.to_ascii_lowercase())
}

/// A pool is a 20-byte pair/pool/curve contract or a 32-byte Uniswap v4 pool id.
pub fn pool_id(value: &str) -> Result<String, String> {
    let value = value.trim();
    if !matches!(value.len(), 42 | 66) || !value.starts_with("0x") || !is_hex(&value[2..]) {
        return Err("expected a pool_id returned by a Hoodit tool".into());
    }
    Ok(value.to_ascii_lowercase())
}

fn is_hex(value: &str) -> bool {
    value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn decimal(value: &str) -> Result<String, String> {
    let v = value.trim();
    let mut parts = v.split('.');
    let whole = parts.next().unwrap_or_default();
    let fractional = parts.next();
    if v.is_empty()
        || v.len() > 512
        || parts.next().is_some()
        || whole.is_empty()
        || (whole.len() > 1 && whole.starts_with('0'))
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fractional.is_some_and(|f| f.is_empty() || !f.bytes().all(|b| b.is_ascii_digit()))
    {
        Err("expected a non-negative plain decimal string".into())
    } else {
        Ok(v.into())
    }
}

/// Slippage for quotes when no pool snapshot is available: the curve and
/// thin-pool floor, so an exit check never looks tighter than a fill can be.
pub const FALLBACK_SLIPPAGE_BPS: u32 = 300;

/// Tool success. Notes disclose gaps in coverage and mark the result partial.
pub fn ok(mut data: Value, notes: Vec<String>) -> Value {
    if let Some(object) = data.as_object_mut() {
        object.insert(
            "status".into(),
            json!(if notes.is_empty() { "ok" } else { "partial" }),
        );
        if !notes.is_empty() {
            object.insert("notes".into(), json!(notes));
        }
    }
    data
}

pub fn error(code: &str, message: &str, retryable: bool) -> Value {
    json!({"status":"error","error":{"code":code,"message":message,"retryable":retryable}})
}

/// Token names and symbols are attacker-controlled text; spam tokens carry
/// names thousands of characters long. Keep them short.
pub fn label(text: String, max: usize) -> String {
    let text = text.trim();
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

pub fn get<'a>(v: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(v, |v, k| v.get(*k))
}
pub fn string(v: &Value, path: &[&str]) -> Option<String> {
    get(v, path).and_then(|v| match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}
pub fn number(v: &Value, path: &[&str]) -> Option<f64> {
    get(v, path).and_then(|v| match v {
        Value::Number(n) => n.to_string().parse().ok(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

/// Rounds to four significant figures: enough for prices of any magnitude.
pub fn sig(x: f64) -> Value {
    if !x.is_finite() {
        return Value::Null;
    }
    if x == 0.0 {
        return json!(0);
    }
    format!("{x:.3e}")
        .parse::<f64>()
        .map(|v| json!(v))
        .unwrap_or(Value::Null)
}
/// Dollar figures: whole dollars from $100, cents below.
pub fn usd(x: f64) -> Value {
    if !x.is_finite() {
        Value::Null
    } else if x.abs() >= 100.0 {
        json!(x.round() as i64)
    } else {
        json!((x * 100.0).round() / 100.0)
    }
}
/// Percentages and ratios to one decimal.
pub fn one(x: f64) -> Value {
    if x.is_finite() {
        json!((x * 10.0).round() / 10.0)
    } else {
        Value::Null
    }
}
pub fn opt(value: Option<f64>, f: fn(f64) -> Value) -> Value {
    value.map(f).unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_identifiers() {
        assert_eq!(
            address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(address("0x1").is_err());
        assert!(pool_id(&format!("0x{}", "a".repeat(64))).is_ok());
        assert!(pool_id("robinhood_0x1").is_err());
    }

    #[test]
    fn caps_spam_labels() {
        assert_eq!(label("PEPE".into(), 24), "PEPE");
        assert_eq!(label("币安人生币安人生".into(), 4), "币安人生…");
        assert_eq!(label("x".repeat(5000), 48).chars().count(), 49);
    }

    #[test]
    fn compacts_numbers() {
        assert_eq!(sig(0.000429612), json!(0.0004296));
        assert_eq!(sig(1234567.0), json!(1235000.0));
        assert_eq!(usd(52057.12), json!(52057));
        assert_eq!(usd(42.456), json!(42.46));
        assert_eq!(one(2166.812), json!(2166.8));
        assert_eq!(sig(f64::NAN), Value::Null);
    }

    #[test]
    fn notes_mark_results_partial() {
        assert_eq!(ok(json!({"a":1}), vec![])["status"], "ok");
        let partial = ok(json!({"a":1}), vec!["gap".into()]);
        assert_eq!(partial["status"], "partial");
        assert_eq!(partial["notes"][0], "gap");
    }
}
