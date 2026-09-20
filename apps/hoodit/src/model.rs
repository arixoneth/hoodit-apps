use chrono::Utc;
use serde_json::{Value, json};

pub const CHAIN_ID: u64 = 4663;
pub const NETWORK: &str = "robinhood";
pub const USDG: &str = "0x5fc5360d0400a0fd4f2af552add042d716f1d168";
pub const NATIVE_SENTINEL: &str = "0x0000000000000000000000000000000000000000";

pub fn address(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.len() != 42
        || !value.starts_with("0x")
        || !value[2..].bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("expected a 20-byte 0x-prefixed address".into());
    }
    Ok(value.to_ascii_lowercase())
}
pub fn token_id(value: &str) -> Result<String, String> {
    if value.eq_ignore_ascii_case("native") {
        Ok("native".into())
    } else {
        address(value)
    }
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
pub fn signed_decimal(value: &str) -> Result<String, String> {
    let value = value.trim();
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    if unsigned.is_empty() {
        return Err("expected a plain signed decimal string".into());
    }
    decimal(unsigned)
        .map(|_| value.to_string())
        .map_err(|_| "expected a plain signed decimal string".into())
}
pub fn meta(sources: Vec<Value>, warnings: Vec<Value>) -> Value {
    json!({"chain_id":CHAIN_ID,"generated_at":Utc::now().to_rfc3339(),"sources":sources,"warnings":warnings})
}
pub fn warning(code: &str, message: &str) -> Value {
    json!({"code":code,"message":message,"subject":null})
}
pub fn ok(data: Value, sources: Vec<Value>, warnings: Vec<Value>) -> Value {
    let partial = warnings
        .iter()
        .any(|warning| string(warning, &["code"]).as_deref() != Some("NOT_INDEXED"));
    envelope(
        if partial { "partial" } else { "ok" },
        Some(data),
        sources,
        warnings,
        None,
    )
}
pub fn error(code: &str, message: &str, retryable: bool) -> Value {
    envelope(
        "error",
        None,
        vec![],
        vec![],
        Some(
            json!({"code":code,"message":message,"retryable":retryable,"retry_after_seconds":null}),
        ),
    )
}
fn envelope(
    status: &str,
    data: Option<Value>,
    sources: Vec<Value>,
    warnings: Vec<Value>,
    error: Option<Value>,
) -> Value {
    json!({"schema_version":"1.3.0","status":status,"data":data,"meta":meta(sources,warnings),"error":error})
}
pub fn token(
    id: &str,
    symbol: Option<&str>,
    name: Option<&str>,
    decimals: Option<u8>,
    image_url: Option<&str>,
) -> Value {
    let symbol = symbol.map(|v| v.chars().take(64).collect::<String>());
    let name = name.map(|v| v.chars().take(200).collect::<String>());
    let image_url = image_url.map(|v| v.chars().take(2048).collect::<String>());
    json!({"id":id,"kind":if id=="native" {"native"} else {"erc20"},"symbol":symbol,"name":name,"decimals":decimals,"image_url":image_url})
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_addresses() {
        assert_eq!(
            address("0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA").unwrap(),
            "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        assert!(address("0x1").is_err());
    }
}
