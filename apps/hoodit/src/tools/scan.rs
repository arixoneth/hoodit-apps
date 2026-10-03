use super::exec;
use crate::app::{HooditApp, Ttl};
use crate::market::{Basics, RESULT_FIELDS, apply_pons, known_pair, row};
use crate::providers::{self, codex};
use crate::shape::{self, ok};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Board {
    /// Moving right now (1h trending score).
    Trending,
    /// Launched recently (see max_age_h), ranked by 1h volume.
    New,
    /// Still on a launchpad bonding curve, closest to graduating first.
    Bonding,
    /// Graduated from a curve recently, ranked by 24h volume.
    Graduated,
}

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PairKind {
    Eth,
    Usd,
    Stock,
    Other,
}

impl PairKind {
    fn name(self) -> &'static str {
        match self {
            PairKind::Eth => "eth",
            PairKind::Usd => "usd",
            PairKind::Stock => "stock",
            PairKind::Other => "other",
        }
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScanArgs {
    /// Which board. null = trending.
    pub board: Option<Board>,
    /// Only tokens from this launchpad, e.g. "pons", "Virtuals", "bow.fun". null = all.
    pub launchpad: Option<String>,
    /// Only tokens whose main pair is ETH, USD (USDG), a stock token, or anything else. null = any.
    pub pair: Option<PairKind>,
    /// Max token age in hours. null = 6 for `new`, 24 for `graduated`, no limit otherwise.
    pub max_age_h: Option<f64>,
    /// Minimum 24h volume in USD. null = 500.
    pub min_volume_24h_usd: Option<f64>,
    /// Max share of supply held by the top 10 wallets, percent 0–100. null = no limit.
    pub max_top10_pct: Option<f64>,
    /// Max share held by the dev, percent 0–100. null = no limit.
    pub max_dev_pct: Option<f64>,
    /// Max share held by snipers, percent 0–100. null = no limit.
    pub max_snipers_pct: Option<f64>,
    /// Rows to return, 1–6. null = 5.
    pub limit: Option<u8>,
}

/// Launchpad names exactly as Codex spells them, matched case-insensitively.
const LAUNCHPADS: &[&str] = &[
    "Virtuals",
    "Clanker V4",
    "BAGS",
    "bow.fun",
    "Sushi Launch",
    "Bankr",
    "Trench",
    "Flap",
    "NOXA Fun",
    "hood.fun",
    "pons",
    "LONG",
    "Feel.cash",
    "o1.exchange",
    "UniswapCCA",
    "Launchfair",
];

pub struct Scan;

impl DynAomiTool for Scan {
    type App = HooditApp;
    type Args = ScanArgs;
    const NAME: &'static str = "hoodit_scan";
    const DESCRIPTION: &'static str = "List Robinhood Chain memecoins from one board (trending, new, bonding curve, just graduated) with filters. Each row: launchpad and stage, pair token, trade_support (can LI.FI trade it now), age, price, FDV, liquidity, 24h volume, % change 1h/24h, last-hour buy and sell dollars, holders, and the share held by top 10 / dev / snipers (percent 0–100). Pons curve rows carry curve_pct = funds raised ÷ graduation target, as Pons shows it. A ranked starting list, not picks and not every coin on the chain.";

    fn run(app: &HooditApp, args: ScanArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let board = args.board.unwrap_or(Board::Trending);
            let limit = args.limit.unwrap_or(5).clamp(1, 6) as usize;
            let post_filters = args.pair.is_some() || args.max_top10_pct.is_some();
            let fetch = if post_filters {
                (limit * 4).min(40)
            } else {
                limit + 4
            };
            // Minute-rounded so the cache key stays stable within a minute.
            let now = (shape::now() / 60 * 60) as f64;
            let mut filters = json!({
                "network": [providers::NETWORK],
                "volume24": { "gte": args.min_volume_24h_usd.unwrap_or(500.0).max(0.0) },
            });
            let age_h = args.max_age_h.filter(|h| *h > 0.0).or(match board {
                Board::New => Some(6.0),
                Board::Graduated => Some(24.0),
                _ => None,
            });
            let (ranking, scope) = match board {
                Board::Trending => ("trendingScore1", "1h trending board"),
                Board::New => ("volume1", "newest launches by 1h volume"),
                Board::Bonding => ("graduationPercent", "bonding curves, closest to graduating"),
                Board::Graduated => ("volume24", "recent graduates by 24h volume"),
            };
            match board {
                Board::Bonding => {
                    filters["launchpadCompleted"] = json!(false);
                    filters["launchpadMigrated"] = json!(false);
                    filters["launchpadGraduationPercent"] = json!({ "gt": 0 });
                }
                Board::Graduated => {
                    filters["launchpadMigrated"] = json!(true);
                    if let Some(h) = age_h {
                        filters["launchpadMigratedAt"] =
                            json!({ "gte": (now - h * 3600.0) as i64 });
                    }
                }
                _ => {}
            }
            if let (Some(h), true) = (age_h, board != Board::Graduated) {
                filters["tokenCreatedAt"] = json!({ "gte": (now - h * 3600.0) as i64 });
            }
            if let Some(pad) = &args.launchpad {
                let Some(name) = LAUNCHPADS
                    .iter()
                    .find(|p| p.eq_ignore_ascii_case(pad.trim()))
                else {
                    return shape::error(
                        "INVALID_ARGUMENT",
                        &format!("unknown launchpad; known: {}", LAUNCHPADS.join(", ")),
                        None,
                    );
                };
                filters["launchpadName"] = json!([name]);
            }
            if let Some(v) = args.max_dev_pct {
                filters["devHeldPercentage"] = json!({ "lte": v });
            }
            if let Some(v) = args.max_snipers_pct {
                filters["sniperHeldPercentage"] = json!({ "lte": v });
            }
            let query = format!(
                "query($f: TokenFilters) {{ s: filterTokens(filters: $f, rankings: [{{attribute: {ranking}, direction: DESC}}], limit: {fetch}) {{ count results {{ {RESULT_FIELDS} }} }} }}"
            );
            let (data, note) =
                match codex(&rt, &call, &query, json!({ "f": filters }), Ttl::Live).await {
                    Ok(found) => found,
                    Err(fail) => return fail.to_value(),
                };
            if let Some(note) = note {
                call.gap(note);
            }
            let results = data
                .pointer("/s/results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            // Base assets (WETH, USDG, stock tokens) top every board; they aren't coins.
            let kept: Vec<&Value> = results
                .iter()
                .filter(|r| known_pair(&Basics::of(r).token).is_none())
                .filter(|r| match args.pair {
                    None => true,
                    Some(kind) => Basics::of(r).pair_kind() == kind.name(),
                })
                .filter(|r| match args.max_top10_pct {
                    Some(max) => {
                        shape::field(r, &["top10HoldersPercent"]).is_some_and(|v| v <= max)
                    }
                    None => true,
                })
                .take(limit)
                .collect();
            let mut rows: Vec<Value> = kept.iter().map(|r| row(r)).collect();
            // Pons rows: true stage and curve progress from the launch record.
            let mut tasks = tokio::task::JoinSet::new();
            for (i, r) in kept.iter().enumerate() {
                if Basics::of(r).is_pons() {
                    let rt = rt.clone();
                    let token = rows[i]["token"].as_str().unwrap_or("").to_string();
                    tasks.spawn(async move { (i, providers::pons_launch(&rt, &token).await) });
                }
            }
            let mut unread = 0;
            while let Some(done) = tasks.join_next().await {
                match done {
                    Ok((i, Some(launch))) => apply_pons(&rt, &mut rows[i], &launch, false).await,
                    _ => unread += 1,
                }
            }
            if unread > 0 {
                call.gap(format!(
                    "{unread} Pons launch records unreadable; their curve % is unknown"
                ));
            }
            ok(
                json!({
                    "board": scope,
                    "scanned": results.len(),
                    "returned": rows.len(),
                    "rows": rows,
                }),
                &call.gaps,
            )
        })
    }
}
