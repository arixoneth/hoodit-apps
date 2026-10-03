use super::{codex_id, exec};
use crate::app::{HooditApp, Ttl};
use crate::providers::codex;
use crate::shape::{self, num, ok, pct, price, usd};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema, Clone, Copy)]
pub enum Range {
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
    #[serde(rename = "30d")]
    D30,
    /// The whole life since launch, curve included.
    #[serde(rename = "life")]
    Life,
}

#[derive(Deserialize, JsonSchema, Clone, Copy)]
pub enum Interval {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "5m")]
    M5,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChartArgs {
    /// Exact 0x token contract.
    pub token: String,
    /// Main range. The candle size is picked to give 24–60 bars. null = 24h.
    pub range: Option<Range>,
    /// Optional second, wider zoom returned as closes only, e.g. life next to 6h. null = none.
    pub context_range: Option<Range>,
    /// Override the candle size for the main range (zooming in). null = automatic.
    pub interval: Option<Interval>,
    /// Bars for the main range when interval is set, 10–60. null = 48.
    pub bars: Option<u16>,
    /// Unix seconds the main range ends at, to page back in time. null = now.
    pub to: Option<i64>,
}

fn resolution(i: Interval) -> (&'static str, i64) {
    match i {
        Interval::M1 => ("1", 60),
        Interval::M5 => ("5", 300),
        Interval::M15 => ("15", 900),
        Interval::H1 => ("60", 3600),
        Interval::H4 => ("240", 14_400),
        Interval::D1 => ("1D", 86_400),
    }
}

/// (resolution, seconds per bar, bars) for a range; `life` uses the age.
fn plan(range: Range, age_s: i64) -> (&'static str, i64, i64) {
    match range {
        Range::H1 => ("1", 60, 60),
        Range::H6 => ("15", 900, 24),
        Range::H24 => ("30", 1800, 48),
        Range::D7 => ("240", 14_400, 42),
        Range::D30 => ("720", 43_200, 60),
        Range::Life => {
            let (res, step) = match age_s {
                s if s <= 6 * 3600 => ("5", 300),
                s if s <= 2 * 86_400 => ("60", 3600),
                s if s <= 10 * 86_400 => ("240", 14_400),
                _ => ("1D", 86_400),
            };
            (res, step, (age_s / step + 1).clamp(2, 60))
        }
    }
}

fn label(range: Range) -> &'static str {
    match range {
        Range::H1 => "1h",
        Range::H6 => "6h",
        Range::H24 => "24h",
        Range::D7 => "7d",
        Range::D30 => "30d",
        Range::Life => "life",
    }
}

fn floats(v: &Value) -> Vec<Option<f64>> {
    v.as_array()
        .map(|a| a.iter().map(num).collect())
        .unwrap_or_default()
}

/// Facts the model can quote, computed from the same bars it receives.
fn facts(
    o: &[Option<f64>],
    h: &[Option<f64>],
    l: &[Option<f64>],
    c: &[Option<f64>],
    vol: &[Option<f64>],
) -> Value {
    let first = o.iter().flatten().next().copied();
    let last = c.iter().rev().flatten().next().copied();
    let high = h.iter().flatten().copied().fold(f64::NAN, f64::max);
    let low = l.iter().flatten().copied().fold(f64::NAN, f64::min);
    let third = (vol.len() / 3).max(1);
    let sum = |s: &[Option<f64>]| s.iter().flatten().sum::<f64>();
    let early = sum(&vol[..third.min(vol.len())]);
    let late = sum(&vol[vol.len().saturating_sub(third)..]);
    let lows = |s: &[Option<f64>]| s.iter().flatten().copied().fold(f64::NAN, f64::min);
    let n = l.len();
    json!({
        "open_usd": price(first),
        "last_usd": price(last),
        "change_pct": match (first, last) { (Some(a), Some(b)) if a > 0.0 => pct(Some((b / a - 1.0) * 100.0)), _ => Value::Null },
        "high_usd": price(high.is_finite().then_some(high)),
        "low_usd": price(low.is_finite().then_some(low)),
        "from_high_pct": match last { Some(b) if high > 0.0 => pct(Some((b / high - 1.0) * 100.0)), _ => Value::Null },
        "from_low_pct": match last { Some(b) if low > 0.0 => pct(Some((b / low - 1.0) * 100.0)), _ => Value::Null },
        "volume_usd": usd(Some(sum(vol))),
        "volume_last_vs_first_third_x": if early > 0.0 { json!(shape::sig(late / early, 2)) } else { Value::Null },
        "low_last_third_vs_first_third_x": if n >= 6 {
            let (a, b) = (lows(&l[..n / 3]), lows(&l[n - n / 3..]));
            if a > 0.0 && b.is_finite() { json!(shape::sig(b / a, 3)) } else { Value::Null }
        } else { Value::Null },
    })
}

pub struct Chart;

impl DynAomiTool for Chart {
    type App = HooditApp;
    type Args = ChartArgs;
    const NAME: &'static str = "hoodit_chart";
    const DESCRIPTION: &'static str = "USD price candles for a Robinhood Chain token, curve phase included, plus facts computed from the same bars (change, high/low, distance from high and low, volume trend as a ratio _x, whether recent lows sit above early lows). Pick a range (1h, 6h, 24h, 7d, 30d, life) and get 24–60 bars; add context_range for a second, wider zoom as closes. Series (t_min, h, l, c, v_usd) are space-separated, one value per bar, `-` = no data. t0 is unix seconds, t_min minutes after t0. Read before any claim about chart structure.";

    fn run(app: &HooditApp, args: ChartArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let id = codex_id(&token);
            let now = shape::now();
            let to = args.to.unwrap_or(now).min(now);
            // Token age is needed for `life`; ask for it in the same request.
            let range = args.range.unwrap_or(Range::H24);
            let needs_age =
                matches!(range, Range::Life) || matches!(args.context_range, Some(Range::Life));
            let age_s = if needs_age {
                let q = format!(
                    "{{ s: filterTokens(tokens: [\"{id}\"], limit: 1) {{ results {{ token {{ createdAt }} }} }} }}"
                );
                match codex(&rt, &call, &q, json!({}), Ttl::Slow).await {
                    Ok((d, _)) => d
                        .pointer("/s/results/0/token/createdAt")
                        .and_then(num)
                        .map(|c| now - c as i64)
                        .unwrap_or(30 * 86_400),
                    Err(fail) => return fail.to_value(),
                }
            } else {
                0
            };
            let (res, step, bars) = match args.interval {
                Some(i) => {
                    let (r, s) = resolution(i);
                    (r, s, args.bars.unwrap_or(48).clamp(10, 60) as i64)
                }
                None => plan(range, age_s),
            };
            let from = to - step * bars;
            let mut query = format!(
                "{{ m: getTokenBars(symbol: \"{id}\", from: {from}, to: {to}, resolution: \"{res}\", removeEmptyBars: true, countback: {bars}) {{ t o h l c volume buyVolume sellVolume }}"
            );
            let context = args.context_range.inspect(|&cr| {
                let (r2, s2, b2) = plan(cr, age_s);
                let b2 = b2.min(40);
                query.push_str(&format!(
                    " x: getTokenBars(symbol: \"{id}\", from: {}, to: {now}, resolution: \"{r2}\", removeEmptyBars: true, countback: {b2}) {{ t c }}",
                    now - s2 * b2
                ));
            });
            query.push_str(" }");
            let (data, note) = match codex(&rt, &call, &query, json!({}), Ttl::Live).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let m = &data["m"];
            let t: Vec<i64> = m["t"]
                .as_array()
                .map(|a| a.iter().filter_map(Value::as_i64).collect())
                .unwrap_or_default();
            if t.is_empty() {
                return shape::error(
                    "NO_DATA",
                    "no trades in this range; the token may be inactive or not indexed",
                    None,
                );
            }
            let (o, h, l, c) = (
                floats(&m["o"]),
                floats(&m["h"]),
                floats(&m["l"]),
                floats(&m["c"]),
            );
            let vol = floats(&m["volume"]);
            let (buy, sell) = (floats(&m["buyVolume"]), floats(&m["sellVolume"]));
            let sum = |s: &[Option<f64>]| s.iter().flatten().sum::<f64>();
            let sig3 = |s: &[Option<f64>]| shape::series(s.iter().copied(), 3);
            let covered_h =
                (t.last().unwrap_or(&0) - t.first().unwrap_or(&0) + step) as f64 / 3600.0;
            let mut out = json!({
                "token": token,
                "range": label(range),
                "candle": res,
                "bars": t.len(),
                "covered_h": shape::sig(covered_h, 3),
                "facts": facts(&o, &h, &l, &c, &vol),
                "flow_usd": { "buy": usd(Some(sum(&buy))), "sell": usd(Some(sum(&sell))) },
                "t0": t[0],
                "t_min": shape::series(t.iter().map(|x| Some(((x - t[0]) / 60) as f64)), 15),
                "h": sig3(&h), "l": sig3(&l), "c": sig3(&c),
                "v_usd": shape::series(vol.iter().copied(), 3),
            });
            if let Some(cr) = context {
                let x = &data["x"];
                out["context"] = json!({
                    "range": label(cr),
                    "t0": x["t"].get(0),
                    "step_min": x["t"].as_array().filter(|a| a.len() > 1).and_then(|a| Some((a[1].as_i64()? - a[0].as_i64()?) / 60)),
                    "c": sig3(&floats(&x["c"])),
                });
            }
            // Highs and lows are the first thing to go: facts keep the extremes.
            if shape::size(&out) > shape::MAX_REPLY - 300 {
                if let Some(m) = out.as_object_mut() {
                    m.remove("h");
                    m.remove("l");
                }
                call.gap("per-bar highs and lows left out to keep the reply short");
            }
            ok(out, &call.gaps)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_give_24_to_60_bars() {
        for r in [Range::H1, Range::H6, Range::H24, Range::D7, Range::D30] {
            let (_, _, bars) = plan(r, 0);
            assert!((24..=60).contains(&bars));
        }
        assert_eq!(plan(Range::Life, 3 * 3600).0, "5");
        assert_eq!(plan(Range::Life, 40 * 86_400).0, "1D");
    }

    #[test]
    fn facts_from_bars() {
        let s = |v: &[f64]| v.iter().map(|x| Some(*x)).collect::<Vec<_>>();
        let f = facts(
            &s(&[1.0, 2.0, 3.0]),
            &s(&[2.0, 4.0, 3.5]),
            &s(&[0.9, 1.8, 2.5]),
            &s(&[2.0, 3.0, 3.0]),
            &s(&[10.0, 10.0, 30.0]),
        );
        assert_eq!(f["change_pct"], json!(200.0));
        assert_eq!(f["from_high_pct"], json!(-25.0));
        assert_eq!(f["volume_last_vs_first_third_x"], json!(3.0));
    }
}
