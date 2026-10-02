//! Swaps decoded straight from Robinhood Chain logs.
//!
//! Supported layouts: Uniswap v2-style pairs, Uniswap/PancakeSwap v3 pools,
//! the Uniswap v4 PoolManager (which also hosts Pons graduated, Bankr and
//! other hook pools), and Pons bonding curves.

use super::Snapshot;
use crate::app::{Call, Runtime};
use crate::model;
use crate::providers::{ProviderError, rpc::Rpc};
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap};

pub const POOL_MANAGER: &str = "0x8366a39cc670b4001a1121b8f6a443a643e40951";
pub const ENTRY_POINT: &str = "0x0000000071727de22e5e9d8baf0edac6f37da032";
const V2_SWAP: &str = "0xd78ad95fa46c994b6551d0da85fc275fe613ce37657fb8d5e3d130840159d822";
const V3_SWAP: &str = "0xc42079f94a6350d7e6235f29174924f928cc2ac818eb64fed8004e115fbcca67";
const PANCAKE_V3_SWAP: &str = "0x19b47279256b2a23a1665c810c8d55a1758940ee09377d4f8d26497a3577dc83";
const V4_SWAP: &str = "0x40e9cecb9f5f1f1c5b9c97dec2917b7ee92e57ba5563708daca94dd84ad7112f";
const CURVE_BUY: &str = "0xec36bf571f136799e8dc0b0b8bea4b04d8bd3d43de838aab0d5fc21d4cbfc455";
const CURVE_SELL: &str = "0x8113d738abdcb6b38357e9d53a54a7157861a09031b453651f0fe7fe151f59df";
const USER_OPERATION: &str = "0x49628fd1471006c1482da88028e9ce4dbb080b815c9b0344d39e5a8e6ec1419f";

/// Robinhood Chain produces about ten blocks a second.
const BLOCKS_PER_SEC: f64 = 10.0;
/// The public endpoint rejects address-filtered ranges over ten million blocks.
const MAX_RANGE: u64 = 9_000_000;
const MIN_RANGE: u64 = 3_000;
const MAX_LOG_CALLS: usize = 3;

/// A pool resolved for log decoding, oriented to the requested token.
#[derive(Clone, Debug)]
pub struct Market {
    pub kind: &'static str,
    /// The pool's swap event. One value per topic position matters: the
    /// official RPC allows 10M-block ranges for exact topics but only 100k
    /// for OR-lists.
    pub topic: Option<&'static str>,
    pub pool_id: String,
    pub token: String,
    pub quote: String,
    pub token_is_0: bool,
    pub token_decimals: u8,
    pub quote_decimals: u8,
    pub quote_usd: f64,
    /// Chain head (block, timestamp) when the market was resolved.
    pub head: (u64, i64),
}

impl Market {
    pub fn resolve(
        rt: &Runtime,
        call: &Call,
        pool: &Snapshot,
        token: &str,
        eth_usd: Option<f64>,
    ) -> Result<Self, ProviderError> {
        let kind = pool.kind.ok_or_else(|| {
            ProviderError::new(
                "UNSUPPORTED",
                format!("charts are not decoded for {} pools yet", pool.venue),
            )
        })?;
        let (quote, quote_usd) = if pool.token == token {
            (pool.quote.clone(), pool.quote_usd)
        } else if pool.quote == token {
            (pool.token.clone(), pool.price_usd)
        } else {
            return Err(ProviderError::new(
                "BAD_REQUEST",
                "the pool does not contain this token",
            ));
        };
        let quote_usd = quote_usd
            .or((quote == model::USDG).then_some(1.0))
            .or(eth_usd.filter(|_| quote == model::WETH || quote == model::NATIVE))
            .filter(|usd| *usd > 0.0)
            .ok_or_else(|| {
                ProviderError::new("BAD_RESPONSE", "no USD price for the pool's quote token")
            })?;
        let (block, ts, decimals) = Rpc::new(rt).head_and_decimals(call, &[token, &quote])?;
        let topic = match kind {
            "v2" => Some(V2_SWAP),
            "v3" if pool.venue.contains("pancake") => Some(PANCAKE_V3_SWAP),
            "v3" => Some(V3_SWAP),
            "v4" => Some(V4_SWAP),
            // A curve contract emits little besides its trades; read them all.
            _ => None,
        };
        Ok(Self {
            head: (block, ts),
            kind,
            topic,
            pool_id: pool.pool_id.clone(),
            token: token.to_string(),
            token_is_0: token < quote.as_str(),
            token_decimals: decimals[0],
            quote_decimals: decimals[1],
            quote,
            quote_usd,
        })
    }

    fn address(&self) -> &str {
        if self.kind == "v4" {
            POOL_MANAGER
        } else {
            &self.pool_id
        }
    }

    fn topics(&self) -> Value {
        match (self.kind, self.topic) {
            ("v4", Some(topic)) => json!([topic, self.pool_id]),
            (_, Some(topic)) => json!([topic]),
            _ => json!([]),
        }
    }

    pub fn decode(&self, log: &Value) -> Option<Swap> {
        let topic = log.get("topics")?.get(0)?.as_str()?.to_ascii_lowercase();
        let data = log.get("data")?.as_str()?.strip_prefix("0x")?;
        let word = |i: usize| data.get(i * 64..(i + 1) * 64);
        let (t0, q0) = if self.token_is_0 { (0, 1) } else { (1, 0) };
        let (buy, token, quote) = match topic.as_str() {
            V2_SWAP => {
                let amount = |i: usize| word(i).map(uint);
                let (t_in, q_in, t_out, q_out) =
                    (amount(t0)?, amount(q0)?, amount(2 + t0)?, amount(2 + q0)?);
                (t_out > t_in, (t_out - t_in).abs(), (q_in - q_out).abs())
            }
            // Pool-side deltas: the token leaving the pool is a buy.
            V3_SWAP | PANCAKE_V3_SWAP => {
                let (t, q) = (int(word(t0)?), int(word(q0)?));
                (t < 0.0, t.abs(), q.abs())
            }
            // Swapper-side deltas: the swapper receiving the token is a buy.
            V4_SWAP => {
                let (t, q) = (int(word(t0)?), int(word(q0)?));
                (t > 0.0, t.abs(), q.abs())
            }
            // Curve events report gross quote and a quote-denominated fee.
            CURVE_BUY => {
                let (gross, out, fee) = (uint(word(0)?), uint(word(1)?), uint(word(2)?));
                (true, out, (gross - fee).max(0.0))
            }
            CURVE_SELL => {
                let (sold, out, fee) = (uint(word(0)?), uint(word(1)?), uint(word(2)?));
                (false, sold, out + fee)
            }
            _ => return None,
        };
        if token <= 0.0 || quote <= 0.0 {
            return None;
        }
        Some(Swap {
            block: crate::providers::rpc::hex_u64(log.get("blockNumber")?)?,
            index: crate::providers::rpc::hex_u64(log.get("logIndex")?)?,
            tx: log.get("transactionHash")?.as_str()?.to_ascii_lowercase(),
            buy,
            token: token / 10f64.powi(self.token_decimals as i32),
            quote: quote / 10f64.powi(self.quote_decimals as i32),
            ts: 0,
            wallet: None,
        })
    }
}

fn uint(word: &str) -> f64 {
    BigInt::parse_bytes(word.as_bytes(), 16)
        .and_then(|v| v.to_f64())
        .unwrap_or(0.0)
}

fn int(word: &str) -> f64 {
    let Some(value) = BigInt::parse_bytes(word.as_bytes(), 16) else {
        return 0.0;
    };
    let value = if word.as_bytes().first().is_some_and(|b| *b >= b'8') {
        value - (BigInt::from(1) << 256)
    } else {
        value
    };
    let magnitude = value.abs().to_f64().unwrap_or(0.0);
    if value.is_negative() {
        -magnitude
    } else {
        magnitude
    }
}

#[derive(Clone, Debug)]
pub struct Swap {
    pub block: u64,
    pub index: u64,
    pub tx: String,
    pub buy: bool,
    /// Token amount in whole units.
    pub token: f64,
    /// Quote amount in whole units.
    pub quote: f64,
    pub ts: i64,
    pub wallet: Option<String>,
}
impl Swap {
    pub fn usd(&self, market: &Market) -> f64 {
        self.quote * market.quote_usd
    }
    pub fn price_usd(&self, market: &Market) -> f64 {
        self.quote / self.token * market.quote_usd
    }
}

pub struct Swaps {
    /// Oldest first.
    pub swaps: Vec<Swap>,
    /// Sampled (time, USD price) points before the detailed window, when the
    /// swap budget ran out before the lookback did.
    pub context: Vec<(i64, f64)>,
    /// Start of the detailed window.
    pub from_ts: i64,
    /// Start of the requested lookback.
    pub lookback_ts: i64,
    pub to_ts: i64,
    /// True when the detailed window is shorter than the lookback.
    pub truncated: bool,
    head: u64,
    reached: u64,
    target: u64,
}

/// Price samples across the part of the lookback the budget didn't cover.
/// Small enough for one batch on either RPC endpoint.
const CONTEXT_SAMPLES: u64 = 12;
const SAMPLE_BLOCKS: u64 = 300;

/// Reads swaps backwards from the chain head until the lookback is covered or
/// `budget` swaps are found. `per_day` (the pool's 24h trade count, when
/// known) sizes the first block range so busy pools take one request. Call
/// `annotate` afterwards for timestamps and wallets.
pub fn fetch(
    rt: &Runtime,
    call: &mut Call,
    market: &Market,
    lookback_secs: i64,
    per_day: Option<u64>,
    budget: usize,
) -> Result<Swaps, ProviderError> {
    let rpc = Rpc::new(rt);
    let (head, head_ts) = market.head;
    let want = (lookback_secs.max(60) as f64 * BLOCKS_PER_SEC) as u64;
    let target = head.saturating_sub(want);
    let mut span = match per_day.filter(|n| *n > 0) {
        Some(n) => (budget as f64 / (n as f64 / (86_400.0 * BLOCKS_PER_SEC))) as u64,
        None => want,
    }
    .clamp(MIN_RANGE, MAX_RANGE);
    let (mut logs, mut to, mut reached, mut calls) = (Vec::new(), head, head, 0);
    let mut truncated = false;
    while to > target && calls < MAX_LOG_CALLS {
        calls += 1;
        let from = to.saturating_sub(span - 1).max(target);
        match rpc.logs(call, market.address(), market.topics(), from, to) {
            Ok(batch) => {
                let found = batch.len();
                logs.extend(batch);
                reached = from;
                // Close enough: the sampled context covers the rest more cheaply.
                if logs.len() * 10 >= budget * 6 {
                    break;
                }
                let density = found as f64 / (to - from + 1) as f64;
                let left = (budget - logs.len()) as f64;
                span = if density > 0.0 {
                    (left * 0.8 / density) as u64
                } else {
                    span.saturating_mul(4)
                }
                .clamp(MIN_RANGE, MAX_RANGE);
                if from == 0 {
                    break;
                }
                to = from - 1;
            }
            Err(error) if error.code == "RANGE_TOO_LARGE" && span > MIN_RANGE * 2 => span /= 4,
            Err(error) if logs.is_empty() => return Err(error),
            Err(error) => {
                call.note(format!("older history stopped early: {}", error.message));
                break;
            }
        }
    }
    let mut swaps: Vec<Swap> = logs.iter().filter_map(|log| market.decode(log)).collect();
    swaps.sort_by_key(|s| (s.block, s.index));
    if swaps.len() > budget {
        swaps.drain(..swaps.len() - budget);
    }
    if reached > target || swaps.len() >= budget {
        truncated = true;
        if swaps.len() >= budget
            && let Some(first) = swaps.first()
        {
            reached = reached.max(first.block);
        }
    }
    Ok(Swaps {
        swaps,
        context: vec![],
        from_ts: 0,
        lookback_ts: 0,
        to_ts: head_ts,
        truncated,
        head,
        reached,
        target: target.min(reached),
    })
}

/// The last priced swap in each of a few short windows spread over
/// [from, to): a cheap price line where reading every swap is too heavy.
fn sample(rpc: &Rpc, call: &mut Call, market: &Market, from: u64, to: u64) -> Vec<(u64, f64)> {
    let windows: Vec<(u64, u64)> = (0..CONTEXT_SAMPLES)
        .map(|i| from + (to - from) * i / CONTEXT_SAMPLES)
        .map(|start| (start, (start + SAMPLE_BLOCKS).min(to - 1)))
        .collect();
    match rpc.logs_many(call, market.address(), &market.topics(), &windows) {
        Ok(found) => found
            .iter()
            .filter_map(|logs| {
                logs.iter()
                    .filter_map(|log| market.decode(log))
                    .filter(|s| s.usd(market) >= 1.0)
                    .max_by_key(|s| (s.block, s.index))
                    .map(|s| (s.block, s.price_usd(market)))
            })
            .collect(),
        Err(error) => {
            call.note(format!(
                "earlier price context unavailable: {}",
                error.message
            ));
            vec![]
        }
    }
}

/// Timestamps every swap and resolves the chosen swaps' wallets (the two
/// lookups run in parallel), then samples earlier prices for a truncated
/// window. Sampling goes last: its log batch is what the RPC throttles first.
pub fn annotate(
    rt: &Runtime,
    call: &mut Call,
    market: &Market,
    found: &mut Swaps,
    picks: &[usize],
) {
    let rpc = Rpc::new(rt);
    let anchors: Vec<u64> = (0..6)
        .map(|i| found.target + (found.head - found.target) * i / 6)
        .chain([found.reached])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut side = Call {
        deadline: call.deadline,
        notes: vec![],
    };
    let times = std::thread::scope(|scope| {
        let times = scope.spawn(|| rpc.block_times(&side, &anchors));
        resolve_wallets(rt, call, &mut found.swaps, picks);
        times.join().ok()
    });
    let mut line: Vec<(u64, i64)> = match times {
        Some(Ok(times)) => times.into_iter().collect(),
        _ => {
            side.note("trade times are estimated from block numbers");
            vec![]
        }
    };
    line.push((found.head, found.to_ts));
    line.sort();
    line.dedup_by_key(|(block, _)| *block);
    let line = Timeline(line);
    for swap in &mut found.swaps {
        swap.ts = line.at(swap.block);
    }
    if found.truncated && found.reached > found.target + SAMPLE_BLOCKS {
        found.context = sample(&rpc, call, market, found.target, found.reached)
            .into_iter()
            .map(|(block, price)| (line.at(block), price))
            .collect();
    }
    found.from_ts = line.at(found.reached);
    found.lookback_ts = line.at(found.target);
    for note in side.notes {
        call.note(note);
    }
}

/// Block-to-time interpolation between a handful of fetched anchors.
struct Timeline(Vec<(u64, i64)>);
impl Timeline {
    fn at(&self, block: u64) -> i64 {
        let anchors = &self.0;
        let i = anchors.partition_point(|(b, _)| *b < block);
        let (lo, hi) = match (
            i.checked_sub(1).and_then(|j| anchors.get(j)),
            anchors.get(i),
        ) {
            (Some(lo), Some(hi)) => (*lo, *hi),
            (None, Some(hi)) => (*hi, *hi),
            (Some(lo), None) => (*lo, *lo),
            (None, None) => return 0,
        };
        if hi.0 == lo.0 {
            return lo.1 + ((block as f64 - lo.0 as f64) / BLOCKS_PER_SEC) as i64;
        }
        lo.1 + ((block - lo.0) as f64 / (hi.0 - lo.0) as f64 * (hi.1 - lo.1) as f64) as i64
    }
}

/// Fills in the trading wallet for the chosen swaps: the transaction sender,
/// or for ERC-4337 bundles the smart account whose operation emitted the swap.
fn resolve_wallets(rt: &Runtime, call: &mut Call, swaps: &mut [Swap], picks: &[usize]) {
    let rpc = Rpc::new(rt);
    let hashes: Vec<String> = picks
        .iter()
        .map(|&i| swaps[i].tx.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if hashes.is_empty() {
        return;
    }
    let txs = match rpc.transactions(call, &hashes) {
        Ok(txs) => txs,
        Err(error) => {
            call.note(format!("wallets unresolved: {}", error.message));
            return;
        }
    };
    let bundled: Vec<String> = txs
        .iter()
        .filter(|(_, tx)| tx.to.as_deref() == Some(ENTRY_POINT))
        .map(|(hash, _)| hash.clone())
        .take(20)
        .collect();
    let receipts = if bundled.is_empty() {
        HashMap::new()
    } else {
        rpc.receipts(call, &bundled).unwrap_or_default()
    };
    for swap in swaps.iter_mut() {
        let Some(tx) = txs.get(&swap.tx) else {
            continue;
        };
        swap.wallet = if tx.to.as_deref() == Some(ENTRY_POINT) {
            receipts
                .get(&swap.tx)
                .and_then(|receipt| operation_sender(receipt, swap.index))
        } else {
            Some(tx.from.clone())
        };
    }
}

/// The smart account of the first user operation logged after the swap.
fn operation_sender(receipt: &Value, swap_index: u64) -> Option<String> {
    receipt
        .get("logs")?
        .as_array()?
        .iter()
        .filter(|log| {
            log.get("topics")
                .and_then(|t| t.get(0))
                .and_then(Value::as_str)
                == Some(USER_OPERATION)
        })
        .find(|log| {
            log.get("logIndex")
                .and_then(crate::providers::rpc::hex_u64)
                .is_some_and(|i| i > swap_index)
        })
        .and_then(|log| {
            log.get("topics")?
                .get(2)?
                .as_str()
                .map(|t| format!("0x{}", &t[t.len() - 40..]))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market(kind: &'static str, token_is_0: bool, quote_decimals: u8) -> Market {
        Market {
            kind,
            topic: None,
            pool_id: "0xpool".into(),
            token: "0xtoken".into(),
            quote: "0xquote".into(),
            token_is_0,
            token_decimals: 18,
            quote_decimals,
            quote_usd: 2700.0,
            head: (0, 0),
        }
    }
    fn log(topic: &str, data: &str) -> Value {
        json!({"topics":[topic],"data":data,"blockNumber":"0x10","logIndex":"0x2","transactionHash":"0xABC"})
    }

    // Real logs captured from Robinhood Chain on 2026-10-02.
    #[test]
    fn decodes_v4_swapper_deltas_as_a_buy() {
        let data = "0xffffffffffffffffffffffffffffffffffffffffffffffffffcab5c0f49e7bfe0000000000000000000000000000000000000000000013d7e4681b559a1d830900000000000000000000000000000000000009c17edda733e959a74733608f1300000000000000000000000000000000000000000000051301cbe47385cf090f00000000000000000000000000000000000000000000000000000000000263340000000000000000000000000000000000000000000000000000000000000001";
        let swap = market("v4", false, 18).decode(&log(V4_SWAP, data)).unwrap();
        assert!(swap.buy);
        assert!((swap.quote - 0.0149998084).abs() < 1e-9, "{}", swap.quote);
        assert!(
            (swap.token - 93707.4715852635).abs() < 1e-6,
            "{}",
            swap.token
        );
        assert_eq!(swap.tx, "0xabc");
    }

    #[test]
    fn decodes_v3_pool_deltas_as_a_buy() {
        let data = "0x00000000000000000000000000000000000000000000000006e7799d37c1c000fffffffffffffffffffffffffffffffffffffffffffff0fd379473a3db156c2d0000000000000000000000000000000000000178625aea910f912dbbf7bf55fb0000000000000000000000000000000000000000000006c397458edc30ea1322000000000000000000000000000000000000000000000000000000000001cf5a";
        let swap = market("v3", false, 18).decode(&log(V3_SWAP, data)).unwrap();
        assert!(swap.buy);
        assert!((swap.quote - 0.4975).abs() < 1e-9);
        assert!(
            (swap.token - 70886.83252214958).abs() < 1e-6,
            "{}",
            swap.token
        );
    }

    #[test]
    fn decodes_v2_and_curve_swaps() {
        // token0 sells 100 for 1 quote.
        let words = ["56bc75e2d63100000", "0", "0", "de0b6b3a7640000"].map(|w| format!("{w:0>64}"));
        let swap = market("v2", true, 18)
            .decode(&log(V2_SWAP, &format!("0x{}", words.concat())))
            .unwrap();
        assert!(!swap.buy);
        assert_eq!((swap.token, swap.quote), (100.0, 1.0));

        let sell = "0x000000000000000000000000000000000000000000081013557e2d32b7003f4d0000000000000000000000000000000000000000000000000057c232e44ac8950000000000000000000000000000000000000000000000000000e2ee69bd85e70000000000000000000000000000000000000000000000000000000000000000";
        let swap = market("curve", false, 18)
            .decode(&log(CURVE_SELL, sell))
            .unwrap();
        assert!(!swap.buy);
        assert!((swap.token - 9_747_321.069).abs() < 0.01, "{}", swap.token);
        assert!((swap.quote - 0.0249513).abs() < 1e-6, "{}", swap.quote);
    }

    #[test]
    fn interpolates_block_times_between_anchors() {
        let line = Timeline(vec![(100, 1_000), (200, 1_010)]);
        assert_eq!(line.at(150), 1_005);
        assert_eq!(line.at(100), 1_000);
        assert_eq!(line.at(220), 1_012);
        assert_eq!(line.at(90), 999);
    }

    #[test]
    fn finds_the_smart_account_behind_a_bundled_swap() {
        let account = "0x6987146d23004c54a6ef75357c5966bac2c45c12";
        let receipt = json!({"logs":[
            {"topics":[USER_OPERATION,"0x01","0x0000000000000000000000001111111111111111111111111111111111111111"],"logIndex":"0x1"},
            {"topics":[V4_SWAP],"logIndex":"0x3"},
            {"topics":[USER_OPERATION,"0x02",format!("0x000000000000000000000000{}", &account[2..])],"logIndex":"0x5"}
        ]});
        assert_eq!(operation_sender(&receipt, 3).as_deref(), Some(account));
    }
}
