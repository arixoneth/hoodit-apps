//! The one place model-facing JSON is shaped. Units live in field names:
//! `_usd` dollars, `_pct` percent 0–100, `_x` ratios, `_min`/`_h` ages.
//! `null` means unknown, never zero.
use serde_json::{Map, Value, json};

/// Longest reply a tool may return, in chars as the model sees it: the host
/// pretty-prints tool JSON. Guests share a 64k-byte model input with the
/// prompt, skills and whole history.
pub const MAX_REPLY: usize = 3000;

/// Reads a provider number that may arrive as a JSON number or a string.
pub fn num(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
    .filter(|v| v.is_finite())
}

pub fn field(value: &Value, path: &[&str]) -> Option<f64> {
    let mut cur = value;
    for key in path {
        cur = cur.get(key)?;
    }
    num(cur)
}

/// Rounds to `sig` significant figures so tiny prices survive and big ones
/// stay short.
pub fn sig(value: f64, sig: i32) -> f64 {
    if value == 0.0 || !value.is_finite() {
        return value;
    }
    let mag = value.abs().log10().floor() as i32;
    let scale = 10f64.powi(sig - 1 - mag);
    (value * scale).round() / scale
}

/// A count: always an integer.
pub fn int(value: Option<f64>) -> Value {
    value
        .map(|v| json!(v.round() as i64))
        .unwrap_or(Value::Null)
}

/// Dollars: whole dollars from $100, cents from $1, else 3 significant figures.
pub fn usd(value: Option<f64>) -> Value {
    match value {
        Some(v) if v.abs() >= 100.0 => json!(v.round() as i64),
        Some(v) if v.abs() >= 1.0 => json!((v * 100.0).round() / 100.0),
        Some(v) => json!(sig(v, 3)),
        None => Value::Null,
    }
}

/// Percent on a 0–100 scale. Small values keep two significant figures so
/// 0.04% never renders as 0.
pub fn pct(value: Option<f64>) -> Value {
    match value {
        Some(v) if v.abs() >= 1.0 => json!((v * 10.0).round() / 10.0),
        Some(v) => json!(sig(v, 2)),
        None => Value::Null,
    }
}

pub fn price(value: Option<f64>) -> Value {
    value.map(|v| json!(sig(v, 4))).unwrap_or(Value::Null)
}

/// Codex `change*` fields are fractions (0.25 = +25%).
pub fn change_pct(value: Option<f64>) -> Value {
    pct(value.map(|v| v * 100.0))
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Whole minutes since a unix timestamp.
pub fn min_ago(ts: Option<f64>) -> Value {
    ts.map(|t| json!(((now() as f64 - t) / 60.0).max(0.0).round() as i64))
        .unwrap_or(Value::Null)
}

/// Sets `<prefix>_min` (whole minutes) for spans up to 3 h, else
/// `<prefix>_h` with 2 significant figures.
pub fn put_span(out: &mut Value, prefix: &str, since: Option<f64>) {
    let Some(t) = since else { return };
    let minutes = ((now() as f64 - t) / 60.0).max(0.0);
    if minutes <= 180.0 {
        out[format!("{prefix}_min")] = json!(minutes.round() as i64);
    } else {
        let hours = sig(minutes / 60.0, 2);
        out[format!("{prefix}_h")] = if hours >= 10.0 {
            json!(hours as i64)
        } else {
            json!(hours)
        };
    }
}

/// Successful result: `status` is `partial` when any section was missing.
pub fn ok(data: Value, gaps: &[String]) -> Value {
    let mut out = Map::new();
    out.insert(
        "status".into(),
        json!(if gaps.is_empty() { "ok" } else { "partial" }),
    );
    out.insert("as_of".into(), json!(now()));
    if let Value::Object(map) = data {
        out.extend(map);
    }
    if !gaps.is_empty() {
        out.insert("gaps".into(), json!(gaps));
    }
    Value::Object(out)
}

/// Failure the model can act on: what failed, whether retrying in this
/// answer can help, and when.
pub fn error(code: &str, message: &str, retry_after_s: Option<u64>) -> Value {
    json!({
        "status": "error",
        "as_of": now(),
        "error": {
            "code": code,
            "message": message,
            "retry_in_this_answer": retry_after_s.is_some_and(|s| s <= 5),
            "retry_after_s": retry_after_s,
        }
    })
}

/// Drops nulls and empty containers so replies stay small.
pub fn compact(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, compact(v)))
                .filter(|(_, v)| match v {
                    Value::Null => false,
                    Value::Object(m) => !m.is_empty(),
                    Value::Array(a) => !a.is_empty(),
                    _ => true,
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(compact).collect()),
        other => other,
    }
}

/// Chars the model sees for this value (pretty-printed, like the host does).
pub fn size(value: &Value) -> usize {
    serde_json::to_string_pretty(value)
        .map(|s| s.len())
        .unwrap_or(0)
}

/// A number series as one space-separated string, so pretty-printing
/// doesn't spend a line per number. Unknown values are `-`.
pub fn series(values: impl IntoIterator<Item = Option<f64>>, digits: i32) -> Value {
    let text: Vec<String> = values
        .into_iter()
        .map(|v| match v {
            Some(x) => {
                let x = sig(x, digits);
                if x.fract() == 0.0 && x.abs() < 1e15 {
                    format!("{}", x as i64)
                } else {
                    format!("{x}")
                }
            }
            None => "-".into(),
        })
        .collect();
    json!(text.join(" "))
}

/// Keeps a reply under [`MAX_REPLY`] by dropping the last items of its
/// largest list of objects (rows, holders, trades) and saying so.
pub fn fit(mut value: Value) -> Value {
    let mut trimmed: Option<(String, usize)> = None;
    // Leave room for the gap note added below.
    while size(&value) > MAX_REPLY - 100 {
        let Some(map) = value.as_object_mut() else {
            break;
        };
        let largest = map
            .iter()
            .filter(|(_, v)| {
                v.as_array()
                    .is_some_and(|a| a.len() > 1 && a.iter().all(Value::is_object))
            })
            .max_by_key(|(_, v)| size(v))
            .map(|(k, _)| k.clone());
        let Some(key) = largest else { break };
        let list = map[&key].as_array_mut().expect("filtered to arrays");
        list.pop();
        trimmed = Some((key, list.len()));
    }
    if let Some((key, kept)) = trimmed {
        let note = json!(format!("{key} cut to {kept} to keep the reply short"));
        match value.get_mut("gaps").and_then(Value::as_array_mut) {
            Some(gaps) => gaps.push(note),
            None => value["gaps"] = json!([note]),
        }
        if value["status"] == "ok" {
            value["status"] = json!("partial");
        }
    }
    value
}

/// Checks and lowercases a 0x address.
pub fn address(value: &str) -> Result<String, String> {
    let v = value.trim();
    if v.len() == 42 && v.starts_with("0x") && v[2..].bytes().all(|b| b.is_ascii_hexdigit()) {
        Ok(v.to_ascii_lowercase())
    } else {
        Err(format!(
            "`{v}` is not a 0x address; resolve tickers with hoodit_find"
        ))
    }
}

/// Untrusted token metadata, cut short and stripped of control characters.
pub fn label(value: Option<&Value>, max: usize) -> Value {
    match value.and_then(Value::as_str) {
        Some(s) => json!(
            s.chars()
                .filter(|c| !c.is_control())
                .take(max)
                .collect::<String>()
                .trim()
        ),
        None => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_percentages_survive() {
        assert_eq!(pct(Some(0.04)), json!(0.04));
        assert_eq!(pct(Some(0.2)), json!(0.2));
        assert_eq!(pct(Some(49.83)), json!(49.8));
        assert_eq!(change_pct(Some(0.25)), json!(25.0));
        assert_eq!(pct(None), Value::Null);
    }

    #[test]
    fn numbers_parse_from_strings() {
        assert_eq!(num(&json!("18.77")), Some(18.77));
        assert_eq!(num(&json!(3)), Some(3.0));
        assert_eq!(num(&json!("x")), None);
        assert_eq!(usd(Some(26736.61)), json!(26737));
        assert_eq!(price(Some(0.000339119645)), json!(0.0003391));
    }

    #[test]
    fn counts_and_ages_are_integers() {
        assert_eq!(int(Some(14021.0)), json!(14021));
        assert_eq!(serde_json::to_string(&int(Some(14021.0))).unwrap(), "14021");
        let mut out = json!({});
        put_span(&mut out, "age", Some(now() as f64 - 11743.0 * 60.0));
        assert_eq!(serde_json::to_string(&out).unwrap(), r#"{"age_h":200}"#);
        let mut out = json!({});
        put_span(&mut out, "age", Some(now() as f64 - 51.0 * 60.0));
        assert_eq!(out, json!({ "age_min": 51 }));
    }

    #[test]
    fn compact_drops_unknowns() {
        let v = compact(json!({"a": null, "b": {"c": null}, "d": [], "e": 1}));
        assert_eq!(v, json!({"e": 1}));
    }

    #[test]
    fn fit_trims_the_largest_row_list() {
        let rows: Vec<Value> = (0..40)
            .map(|i| json!({ "i": i, "pad": "x".repeat(80) }))
            .collect();
        let out = fit(ok(
            json!({ "rows": rows, "small": [{ "a": 1 }, { "a": 2 }] }),
            &[],
        ));
        assert!(size(&out) <= MAX_REPLY);
        assert_eq!(out["status"], "partial");
        assert_eq!(out["small"].as_array().unwrap().len(), 2);
        assert!(out["gaps"][0].as_str().unwrap().starts_with("rows cut to"));
    }

    #[test]
    fn series_are_one_line() {
        assert_eq!(
            series([Some(0.00033312), None, Some(1606.4)], 3),
            json!("0.000333 - 1610")
        );
    }

    #[test]
    fn metadata_is_cut_and_cleaned() {
        assert_eq!(label(Some(&json!("ab\ncd")), 3), json!("abc"));
        assert_eq!(label(Some(&json!("CAT ")), 20), json!("CAT"));
    }
}
