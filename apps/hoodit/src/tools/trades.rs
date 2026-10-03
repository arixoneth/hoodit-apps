use super::{codex_id, exec};
use crate::app::{HooditApp, Ttl};
use crate::providers::{self, codex};
use crate::shape::{self, field, num, ok, pct, usd};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TradesArgs {
    /// Exact 0x token contract.
    pub token: String,
    /// Only buys or only sells. null = both.
    pub side: Option<Side>,
    /// Only trades of at least this many USD. null = all.
    pub min_usd: Option<f64>,
    /// Largest trades to list, 1–10. null = 6.
    pub limit: Option<u8>,
}

pub struct Trades;

impl DynAomiTool for Trades {
    type App = HooditApp;
    type Args = TradesArgs;
    const NAME: &'static str = "hoodit_trades";
    const DESCRIPTION: &'static str = "Who is buying and selling a Robinhood Chain token: buy vs sell dollars and unique buyers/sellers for 5m, 1h, 4h and 24h, and the largest trades among the last 100 plus recent trades of $1k or more with the real wallet (the smart account, not the bundler, for Fomo and other ERC-4337 trades), size in USD, minutes ago and wallet labels. Also the share of sell dollars from the single biggest seller in that sample, and how many minutes the last-100 sample covers. Judge demand in dollars, not trade counts.";

    fn run(app: &HooditApp, args: TradesArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let sides = match args.side {
                Some(Side::Buy) => "[Buy]",
                Some(Side::Sell) => "[Sell]",
                None => "[Buy, Sell]",
            };
            let windows = ["5m", "1", "4", "24"]
                .iter()
                .map(|w| format!("buyVolume{w} sellVolume{w} uniqueBuys{w} uniqueSells{w}"))
                .collect::<Vec<_>>()
                .join(" ");
            let min = args.min_usd.unwrap_or(0.0).max(0.0);
            let big = min.max(1000.0);
            let events = |alias: &str, limit: u32, floor: f64| {
                let floor = if floor > 0.0 {
                    format!(", priceUsdTotal: {{gte: {floor}}}")
                } else {
                    String::new()
                };
                format!(
                    "{alias}: getTokenEvents(limit: {limit}, query: {{address: \"{token}\", networkId: {}, eventDisplayType: {sides}{floor}}}) {{ items {{ maker timestamp eventDisplayType transactionHash walletLabels data {{ ... on SwapEventData {{ priceUsdTotal }} }} }} }}",
                    providers::NETWORK
                )
            };
            // The last 100 events cover minutes on a busy token; the second
            // alias reaches back for the big trades.
            let query = format!(
                "{{ {} {} s: filterTokens(tokens: [\"{}\"], limit: 1) {{ results {{ {windows} token {{ symbol }} }} }} }}",
                events("e", 100, min),
                events("b", 50, big),
                codex_id(&token)
            );
            let (data, note) = match codex(&rt, &call, &query, json!({}), Ttl::Live).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let s = data.pointer("/s/results/0").cloned().unwrap_or(Value::Null);
            let no_events = ["/e/items", "/b/items"].iter().all(|p| {
                data.pointer(p)
                    .and_then(Value::as_array)
                    .is_none_or(|a| a.is_empty())
            });
            if s.is_null() && no_events {
                return shape::error(
                    "NOT_FOUND",
                    "no Robinhood Chain market indexed for this exact contract; check the address and chain",
                    None,
                );
            }
            let window = |w: &str| {
                let (b, sl) = (
                    field(&s, &[&format!("buyVolume{w}")]),
                    field(&s, &[&format!("sellVolume{w}")]),
                );
                json!({
                    "buy_usd": usd(b), "sell_usd": usd(sl),
                    "net_usd": usd(b.zip(sl).map(|(b, s)| b - s)),
                    "buyers": shape::int(field(&s, &[&format!("uniqueBuys{w}")])),
                    "sellers": shape::int(field(&s, &[&format!("uniqueSells{w}")])),
                })
            };
            let now = shape::now() as f64;
            let mut trades: Vec<(f64, Value)> = vec![];
            let mut sold: HashMap<String, f64> = HashMap::new();
            let mut sold_total = 0.0;
            let mut seen = std::collections::HashSet::new();
            let mut oldest_recent: Option<f64> = None;
            let recent = data
                .pointer("/e/items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for e in &recent {
                if let Some(t) = e.get("timestamp").and_then(num) {
                    oldest_recent = Some(oldest_recent.map_or(t, |o: f64| o.min(t)));
                }
            }
            let large = data
                .pointer("/b/items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for e in recent.iter().chain(large.iter()) {
                let id = format!(
                    "{}:{}:{}",
                    e.get("transactionHash")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                    e.get("maker").and_then(Value::as_str).unwrap_or(""),
                    e.get("eventDisplayType")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                );
                if !seen.insert(id) {
                    continue;
                }
                let usd_size = e
                    .pointer("/data/priceUsdTotal")
                    .and_then(num)
                    .unwrap_or(0.0);
                let side = e
                    .get("eventDisplayType")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let maker = e
                    .get("maker")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if side == "Sell" {
                    *sold.entry(maker.clone()).or_default() += usd_size;
                    sold_total += usd_size;
                }
                trades.push((
                    usd_size,
                    json!({
                        "side": side.to_lowercase(),
                        "usd": usd(Some(usd_size)),
                        "wallet": maker,
                        "min_ago": shape::min_ago(e.get("timestamp").and_then(num)),
                        "labels": e.get("walletLabels"),
                    }),
                ));
            }
            let sample = trades.len();
            trades.sort_by(|a, b| b.0.total_cmp(&a.0));
            let top_seller = sold.values().copied().fold(0.0, f64::max);
            ok(
                json!({
                    "symbol": s.pointer("/token/symbol"),
                    "token": token,
                    "windows": { "5m": window("5m"), "1h": window("1"), "4h": window("4"), "24h": window("24") },
                    "sample_trades": sample,
                    "recent_sample_covers_min": oldest_recent.map(|o| ((now - o) / 60.0).round() as i64),
                    "largest": trades.into_iter().take(args.limit.unwrap_or(6).clamp(1, 10) as usize).map(|t| t.1).collect::<Vec<_>>(),
                    "top_seller_share_of_sample_sells_pct": if sold_total > 0.0 { pct(Some(top_seller / sold_total * 100.0)) } else { Value::Null },
                }),
                &call.gaps,
            )
        })
    }
}
