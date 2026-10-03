//! Shared market logic: Codex field sets, pair-token labels, the trade
//! support table, Pons curve progress, holder labels and the token rows and
//! card every tool reuses.
use crate::app::{Call, Runtime};
use crate::providers::{self, NATIVE, PonsLaunch};
use crate::shape::{change_pct, field, int, label, min_ago, num, pct, price, put_span, sig, usd};
use serde_json::{Value, json};

/// Fields read from `filterTokens` results for rows and cards.
pub const RESULT_FIELDS: &str = "createdAt priceUSD marketCap circulatingMarketCap liquidity volume24 \
change1 change4 change24 buyVolume1 sellVolume1 uniqueBuys1 uniqueSells1 buyVolume24 sellVolume24 \
uniqueBuys24 uniqueSells24 holders top10HoldersPercent devHeldPercentage sniperHeldPercentage \
bundlerHeldPercentage insiderHeldPercentage athPrice lastTransaction \
pair { address token0 token1 token0Data { symbol decimals } token1Data { symbol decimals } } \
token { address symbol name createdAt creatorAddress info { totalSupply } \
launchpad { launchpadName graduationPercent completed migrated } }";

pub const WETH: &str = "0x0bd7d308f8e1639fab988df18a8011f41eacad73";

/// Pair tokens Pons and other launchpads use on Robinhood Chain.
const PAIRS: &[(&str, &str, &str, u8)] = &[
    (NATIVE, "ETH", "eth", 18),
    (WETH, "WETH", "eth", 18),
    (
        "0x5fc5360d0400a0fd4f2af552add042d716f1d168",
        "USDG",
        "usd",
        6,
    ),
    (
        "0xd0601ce157db5bdc3162bbac2a2c8af5320d9eec",
        "NVDA",
        "stock",
        18,
    ),
    (
        "0x4a0e65a3eccec6dbe60ae065f2e7bb85fae35eea",
        "SPCX",
        "stock",
        18,
    ),
    (
        "0xc9a981fee1f9dec688bb123ccdecc63d0debfc4e",
        "GLD",
        "stock",
        18,
    ),
    (
        "0x1d11f0496982706c5e14a514d4e79f2e6bde4516",
        "DJT",
        "stock",
        18,
    ),
    (
        "0x117cc2133c37b721f49de2a7a74833232b3b4c0c",
        "SPY",
        "stock",
        18,
    ),
    (
        "0x322f0929c4625ed5bad873c95208d54e1c003b2d",
        "TSLA",
        "stock",
        18,
    ),
    (
        "0x1b0e319c6a659f002271b69db8a7df2f911c153e",
        "GME",
        "stock",
        18,
    ),
    (
        "0x2e0847e8910a9732eb3fb1bb4b70a580adad4fe3",
        "GOOGL",
        "stock",
        18,
    ),
    (
        "0xd5f3879160bc7c32ebb4dc785f8a4f505888de68",
        "QQQ",
        "stock",
        18,
    ),
];

/// Contracts that hold supply without being a trader.
const KNOWN_HOLDERS: &[(&str, &str)] = &[
    (
        "0x8366a39cc670b4001a1121b8f6a443a643e40951",
        "uniswap v4 pool",
    ),
    ("0x267444d099b10fb5ed7c3cc7b7c767adca574952", "pons locker"),
    ("0xe5e702641ea86f4ae6cc3cdaed2b886f976be044", "pons hook"),
    (
        "0xd3afeb2a57f70ef218aa82451c51b2fb0416ac9e",
        "pons fee escrow",
    ),
    ("0x000000000000000000000000000000000000dead", "burn"),
    (NATIVE, "burn"),
];

/// A pair token: symbol, kind (eth, usd, stock, other) and decimals.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair {
    pub symbol: String,
    pub kind: &'static str,
    pub decimals: u8,
}

pub fn known_pair(address: &str) -> Option<Pair> {
    let address = address.to_ascii_lowercase();
    PAIRS
        .iter()
        .find(|(a, ..)| *a == address)
        .map(|(_, symbol, kind, decimals)| Pair {
            symbol: symbol.to_string(),
            kind,
            decimals: *decimals,
        })
}

/// Any pair token; unknown ones are read from chain and count as `other`.
pub async fn pair_of(rt: &Runtime, address: &str) -> Option<Pair> {
    if let Some(pair) = known_pair(address) {
        return Some(pair);
    }
    let (decimals, symbol) = tokio::join!(
        providers::decimals(rt, address),
        providers::symbol(rt, address)
    );
    Some(Pair {
        symbol: symbol.unwrap_or_else(|| "?".into()),
        kind: "other",
        decimals: decimals?,
    })
}

/// The non-target side of the token's main Codex pair.
fn pair_from_result(result: &Value, token: &str) -> Option<Pair> {
    let pair = result.get("pair")?;
    let t0 = pair.get("token0")?.as_str()?.to_ascii_lowercase();
    let (address, data) = if t0 == token {
        (
            pair.get("token1")?.as_str()?.to_ascii_lowercase(),
            pair.get("token1Data"),
        )
    } else {
        (t0, pair.get("token0Data"))
    };
    known_pair(&address).or_else(|| {
        let data = data?;
        Some(Pair {
            symbol: label(data.get("symbol"), 12).as_str()?.to_string(),
            kind: "other",
            decimals: data.get("decimals").and_then(num).unwrap_or(18.0) as u8,
        })
    })
}

/// Can LI.FI (the host's swap router) trade this token now? From live
/// quotes on 2026-10-03; re-checked by the eval harness.
pub fn trade_support(launchpad: Option<&str>, on_curve: bool, pair_kind: &str) -> &'static str {
    let pad = launchpad.unwrap_or("").to_ascii_lowercase();
    if !on_curve {
        return match pad.as_str() {
            "launchfair" | "noxa fun" => "research_only",
            _ => "full",
        };
    }
    match pad.as_str() {
        "pons" if matches!(pair_kind, "eth" | "usd") => "full",
        "pons" => "research_only",
        "virtuals" | "bow.fun" | "bankr" | "long" | "uniswapcca" | "sushi launch" | "feel.cash"
        | "clanker v4" => "full",
        "flap" | "hood.fun" | "bags" | "trench" => "after_graduation",
        _ => "research_only",
    }
}

/// Pons curve progress the way Pons shows it: real pair-token reserve over
/// the launch's graduation target.
pub async fn pons_curve(rt: &Runtime, launch: &PonsLaunch) -> Value {
    let (pair, reserve) = tokio::join!(
        pair_of(rt, &launch.pair_token),
        providers::pons_reserve(rt, &launch.curve)
    );
    let Some(pair) = pair else {
        return json!({ "pair": "unknown" });
    };
    let scale = 10f64.powi(pair.decimals as i32);
    let raised = reserve.map(|r| r as f64 / scale);
    let target = launch.target as f64 / scale;
    json!({
        "pct": pct(raised.filter(|_| target > 0.0).map(|r| r / target * 100.0)),
        "raised": raised.map(|r| sig(r, 4)),
        "target": sig(target, 4),
        "pair": pair.symbol,
        "raised_usd": if pair.kind == "usd" { usd(raised) } else { Value::Null },
    })
}

/// What a `filterTokens` result says about the launch, before any chain read.
pub struct Basics {
    pub token: String,
    pub launchpad: Option<String>,
    pub on_curve: bool,
    pub graduated: bool,
    pub pair: Option<Pair>,
}

impl Basics {
    pub fn of(result: &Value) -> Self {
        let token = result
            .pointer("/token/address")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let launch = result.pointer("/token/launchpad");
        let flag = |k: &str| {
            launch
                .and_then(|l| l.get(k))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        let launchpad = launch
            .and_then(|l| l.get("launchpadName"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let graduated = flag("completed") || flag("migrated");
        let on_curve = launchpad.is_some()
            && !graduated
            && launch.is_some_and(|l| l.get("graduationPercent").is_some_and(|g| !g.is_null()));
        let pair = pair_from_result(result, &token);
        Self {
            token,
            launchpad,
            on_curve,
            graduated,
            pair,
        }
    }

    pub fn is_pons(&self) -> bool {
        self.launchpad.as_deref() == Some("pons")
    }

    pub fn stage(&self) -> &'static str {
        match (&self.launchpad, self.on_curve, self.graduated) {
            (Some(_), true, _) => "curve",
            (Some(_), _, true) => "graduated",
            _ => "pool",
        }
    }

    pub fn pair_kind(&self) -> &'static str {
        self.pair.as_ref().map(|p| p.kind).unwrap_or("other")
    }
}

/// Compact scan row, about 550 chars once pretty-printed. Price is on the
/// card; FDV carries the size here.
pub fn row(result: &Value) -> Value {
    let b = Basics::of(result);
    let f = |k: &str| field(result, &[k]);
    let mut out = json!({
        "symbol": label(result.pointer("/token/symbol"), 20),
        "token": b.token,
        "launchpad": b.launchpad,
        "stage": b.stage(),
        "pair": b.pair.as_ref().map(|p| p.symbol.clone()),
        "trade_support": trade_support(b.launchpad.as_deref(), b.on_curve, b.pair_kind()),
    });
    put_span(
        &mut out,
        "age",
        field(result, &["token", "createdAt"]).or(f("createdAt")),
    );
    let rest = json!({
        "fdv_usd": usd(f("marketCap")),
        "liquidity_usd": usd(f("liquidity")),
        "volume_24h_usd": usd(f("volume24")),
        "change_1h_pct": change_pct(f("change1")),
        "change_24h_pct": change_pct(f("change24")),
        "buy_1h_usd": usd(f("buyVolume1")),
        "sell_1h_usd": usd(f("sellVolume1")),
        "holders": int(f("holders")),
        "top10_pct": pct(f("top10HoldersPercent")),
        "dev_pct": pct(f("devHeldPercentage")),
        "snipers_pct": pct(f("sniperHeldPercentage")),
    });
    merge(&mut out, rest);
    if b.on_curve && !b.is_pons() {
        out["graduation_pct"] = pct(field(result, &["token", "launchpad", "graduationPercent"]));
    }
    out
}

/// The full card for one token, before chain enrichment.
pub fn card(result: &Value) -> Value {
    let b = Basics::of(result);
    let f = |k: &str| field(result, &[k]);
    let mut out = json!({
        "symbol": label(result.pointer("/token/symbol"), 20),
        "name": label(result.pointer("/token/name"), 40),
        "token": b.token,
        "launchpad": b.launchpad,
        "stage": b.stage(),
        "pair": b.pair.as_ref().map(|p| p.symbol.clone()),
        "pair_kind": b.pair_kind(),
        "trade_support": trade_support(b.launchpad.as_deref(), b.on_curve, b.pair_kind()),
    });
    put_span(
        &mut out,
        "age",
        field(result, &["token", "createdAt"]).or(f("createdAt")),
    );
    let flow = |w: &str| {
        json!({
            "buy_usd": usd(f(&format!("buyVolume{w}"))),
            "sell_usd": usd(f(&format!("sellVolume{w}"))),
            "buyers": int(f(&format!("uniqueBuys{w}"))),
            "sellers": int(f(&format!("uniqueSells{w}"))),
        })
    };
    let (ath, last) = (f("athPrice"), f("priceUSD"));
    let rest = json!({
        "price_usd": price(last),
        "fdv_usd": usd(f("marketCap")),
        "mcap_usd": usd(f("circulatingMarketCap")),
        "liquidity_usd": usd(f("liquidity")),
        "volume_24h_usd": usd(f("volume24")),
        "change_pct": {
            "h1": change_pct(f("change1")), "h4": change_pct(f("change4")), "h24": change_pct(f("change24")),
        },
        "flow_1h": flow("1"),
        "flow_24h": flow("24"),
        "holders": int(f("holders")),
        "held_pct": {
            "top10": pct(f("top10HoldersPercent")), "dev": pct(f("devHeldPercentage")),
            "snipers": pct(f("sniperHeldPercentage")), "bundlers": pct(f("bundlerHeldPercentage")),
            "insiders": pct(f("insiderHeldPercentage")),
        },
        "ath_usd": price(ath),
        "from_ath_pct": match (ath, last) {
            (Some(a), Some(l)) if a > 0.0 => pct(Some((l / a - 1.0) * 100.0)),
            _ => Value::Null,
        },
        "last_trade_min_ago": min_ago(f("lastTransaction")),
        "explorer_url": format!("https://robin.etherscan.io/token/{}", b.token),
    });
    merge(&mut out, rest);
    if b.on_curve && !b.is_pons() {
        out["graduation_pct"] = pct(field(result, &["token", "launchpad", "graduationPercent"]));
    }
    out
}

fn merge(out: &mut Value, rest: Value) {
    if let (Some(out), Value::Object(rest)) = (out.as_object_mut(), rest) {
        out.extend(rest);
    }
}

/// Applies the Pons launch record to a row or card: true stage, curve
/// progress, trade support and the dev wallet. `full` adds the card fields.
pub async fn apply_pons(rt: &Runtime, out: &mut Value, launch: &PonsLaunch, full: bool) {
    out["stage"] = json!(launch.stage());
    let pair = pair_of(rt, &launch.pair_token).await;
    let kind = pair.as_ref().map(|p| p.kind).unwrap_or("other");
    if let Some(pair) = &pair {
        out["pair"] = json!(pair.symbol);
        if full {
            out["pair_kind"] = json!(pair.kind);
        }
    }
    out["trade_support"] = json!(trade_support(Some("pons"), launch.on_curve(), kind));
    if launch.on_curve() {
        let curve = pons_curve(rt, launch).await;
        if full {
            out["curve"] = curve;
        } else {
            out["curve_pct"] = curve["pct"].clone();
        }
    }
    if full {
        out["creator_tax_pct"] = pct(Some(launch.creator_tax_bps as f64 / 100.0));
    }
}

/// Who launched a token. For Pons the launch record's deployer is the dev
/// unless it is a contract (a fee splitter); then the launch tx sender,
/// which Codex records as `creatorAddress`, is.
pub async fn dev_wallet(
    rt: &Runtime,
    launch: Option<&PonsLaunch>,
    codex_creator: Option<&str>,
) -> Option<(String, &'static str)> {
    let creator = codex_creator.map(str::to_ascii_lowercase);
    match launch {
        Some(l) => {
            let contract = providers::has_code(rt, std::slice::from_ref(&l.deployer)).await;
            if contract.first() == Some(&Some(true)) {
                creator.map(|c| (c, "launch tx sender (Pons deployer field is a contract)"))
            } else {
                Some((l.deployer.clone(), "Pons launch record"))
            }
        }
        None => creator.map(|c| (c, "launch tx sender")),
    }
}

/// Labels for holder addresses: known contracts, the token's own curve
/// and pool, and the dev.
pub struct HolderLabels {
    known: Vec<(String, &'static str)>,
}

impl HolderLabels {
    pub fn new(launch: Option<&PonsLaunch>, pool: Option<&str>, dev: Option<&str>) -> Self {
        let mut known: Vec<(String, &'static str)> = KNOWN_HOLDERS
            .iter()
            .map(|(a, l)| (a.to_string(), *l))
            .collect();
        if let Some(l) = launch {
            known.push((l.curve.clone(), "pons curve"));
        }
        if let Some(p) = pool.filter(|p| p.len() == 42) {
            known.push((p.to_ascii_lowercase(), "pool"));
        }
        if let Some(d) = dev {
            known.push((d.to_ascii_lowercase(), "dev"));
        }
        Self { known }
    }

    pub fn of(&self, address: &str) -> Option<&'static str> {
        let address = address.to_ascii_lowercase();
        self.known
            .iter()
            .find(|(a, _)| *a == address)
            .map(|(_, l)| *l)
    }

    /// Supply held by a contract, not a trader. The dev is a trader here.
    pub fn is_contract(&self, address: &str) -> bool {
        self.of(address).is_some_and(|l| l != "dev")
    }
}

/// Top-10 share of supply by wallets, excluding the pool, curve, locker
/// and burn contracts. `holders` are Codex `holders.items`.
pub fn top10_wallets_pct(holders: &[Value], labels: &HolderLabels, supply: Option<f64>) -> Value {
    let Some(supply) = supply.filter(|s| *s > 0.0) else {
        return Value::Null;
    };
    let held: f64 = holders
        .iter()
        .filter(|h| {
            h.get("address")
                .and_then(Value::as_str)
                .is_some_and(|a| !labels.is_contract(a))
        })
        .take(10)
        .filter_map(|h| h.get("shiftedBalance").and_then(num))
        .sum();
    pct(Some(held / supply * 100.0))
}

/// Fills a card's Pons fields, dev wallet and wallet-only top-10 share.
pub async fn enrich_card(
    rt: &Runtime,
    call: &mut Call,
    card: &mut Value,
    result: &Value,
    holders: &[Value],
) {
    let b = Basics::of(result);
    let launch = if b.is_pons() {
        let launch = providers::pons_launch(rt, &b.token).await;
        if launch.is_none() {
            call.gap("Pons launch record unreadable; curve % and dev unknown");
        }
        launch
    } else {
        None
    };
    if let Some(l) = &launch {
        apply_pons(rt, card, l, true).await;
    }
    let creator = result
        .pointer("/token/creatorAddress")
        .and_then(Value::as_str);
    let dev = dev_wallet(rt, launch.as_ref(), creator).await;
    if let Some((wallet, _)) = &dev {
        card["dev"] = json!(wallet);
    }
    let pool = result.pointer("/pair/address").and_then(Value::as_str);
    let labels = HolderLabels::new(launch.as_ref(), pool, None);
    let supply = field(result, &["token", "info", "totalSupply"]);
    if !holders.is_empty() {
        card["held_pct"]["top10"] = top10_wallets_pct(holders, &labels, supply);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn support_table_matches_routing_tests() {
        assert_eq!(trade_support(Some("pons"), true, "eth"), "full");
        assert_eq!(trade_support(Some("pons"), true, "usd"), "full");
        assert_eq!(trade_support(Some("pons"), true, "stock"), "research_only");
        assert_eq!(trade_support(Some("pons"), true, "other"), "research_only");
        assert_eq!(trade_support(Some("Flap"), true, "eth"), "after_graduation");
        assert_eq!(trade_support(Some("Flap"), false, "eth"), "full");
        assert_eq!(trade_support(None, false, "eth"), "full");
        assert_eq!(
            trade_support(Some("Launchfair"), true, "eth"),
            "research_only"
        );
    }

    #[test]
    fn pair_labels_known_quote_tokens() {
        let usdg = known_pair("0x5FC5360D0400A0FD4F2AF552ADD042D716F1D168").unwrap();
        assert_eq!((usdg.kind, usdg.decimals), ("usd", 6));
        assert_eq!(known_pair(NATIVE).unwrap().symbol, "ETH");
        assert!(known_pair("0x1111111111111111111111111111111111111111").is_none());
    }

    #[test]
    fn unknown_codex_pair_is_other() {
        let result = json!({
            "token": { "address": "0xaa" },
            "pair": { "token0": "0xAA", "token1": "0xbb", "token1Data": { "symbol": "IBIT", "decimals": 8 } }
        });
        let pair = Basics::of(&result).pair.unwrap();
        assert_eq!(
            (pair.symbol.as_str(), pair.kind, pair.decimals),
            ("IBIT", "other", 8)
        );
    }

    /// Live `filterTokens` results (trending and bonding boards, 2026-10-03).
    fn recorded() -> Vec<Value> {
        let data: Value =
            serde_json::from_str(include_str!("fixtures/filter-tokens.json")).unwrap();
        ["s", "b"]
            .iter()
            .flat_map(|k| data["data"][*k]["results"].as_array().unwrap().clone())
            .collect()
    }

    #[test]
    fn four_widest_scan_rows_fit_one_reply() {
        let mut rows: Vec<Value> = recorded()
            .iter()
            .map(|r| {
                let mut row = crate::shape::compact(row(r));
                row["curve_pct"] = json!(85.4);
                row["symbol"] = json!("X".repeat(12));
                row
            })
            .collect();
        rows.sort_by_key(|r| std::cmp::Reverse(crate::shape::size(r)));
        rows.truncate(4);
        let reply = crate::shape::ok(
            json!({ "board": "bonding curves, closest to graduating", "scanned": 10, "returned": 4, "rows": rows }),
            &["2 Pons launch records unreadable; their curve % is unknown".into()],
        );
        let reply = crate::shape::compact(reply);
        assert!(
            crate::shape::size(&reply) <= crate::shape::MAX_REPLY,
            "{}",
            crate::shape::size(&reply)
        );
    }

    #[test]
    fn widest_card_fits_one_reply() {
        for r in recorded() {
            let mut card = card(&r);
            card["name"] = json!("N".repeat(40));
            card["curve"] = json!({ "pct": 85.4, "raised": 3.587, "target": 4.2, "pair": "USDG", "raised_usd": 3587 });
            card["dev"] = json!("0x767de1a44a0adf710aac16450de9542bd8e75caa");
            card["creator_tax_pct"] = json!(2.0);
            card["security"] = json!({
                "honeypot": false, "cannot_sell_all": false, "buy_tax_pct": 0.0, "sell_tax_pct": 0.0,
                "tax_changeable": false, "mintable": false, "owner_can_change_balance": false,
                "blacklist": false, "pausable": false, "proxy": false, "hidden_owner": false, "source": "GoPlus",
            });
            let reply = crate::shape::compact(crate::shape::ok(
                card,
                &["Pons launch record unreadable; curve % and dev unknown".into()],
            ));
            assert!(
                crate::shape::size(&reply) <= crate::shape::MAX_REPLY,
                "{}",
                crate::shape::size(&reply)
            );
        }
    }

    #[test]
    fn rows_use_integer_counts() {
        for r in recorded() {
            let row = row(&r);
            assert!(
                row["holders"].is_null() || row["holders"].is_i64(),
                "{}",
                row["holders"]
            );
        }
    }

    #[test]
    fn wallet_top10_skips_pool_and_locker() {
        let holders = vec![
            json!({ "address": "0x8366a39cc670b4001a1121b8f6a443a643e40951", "shiftedBalance": 108.0 }),
            json!({ "address": "0x267444d099b10fb5ed7c3cc7b7c767adca574952", "shiftedBalance": 82.0 }),
            json!({ "address": "0x4cfd59ad1d7236af5c98248435b38e96554cd15b", "shiftedBalance": 27.0 }),
        ];
        let labels = HolderLabels::new(None, None, None);
        assert_eq!(
            top10_wallets_pct(&holders, &labels, Some(1000.0)),
            json!(2.7)
        );
    }
}
