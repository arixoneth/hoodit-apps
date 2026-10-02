//! Candles, chart structure, and order flow computed from decoded swaps.

use super::swaps::{Market, Swap};
use crate::model::{one, sig, usd};
use serde_json::{Value, json};
use std::collections::HashMap;

pub const INTERVALS: [(&str, i64); 6] = [
    ("1m", 60),
    ("5m", 300),
    ("15m", 900),
    ("1h", 3600),
    ("4h", 14_400),
    ("1d", 86_400),
];
const MAX_CANDLES: i64 = 48;
/// Dust trades round badly and print fake wicks; they still count as volume.
const MIN_PRICED_USD: f64 = 1.0;

pub fn interval(label: &str) -> Option<i64> {
    INTERVALS.iter().find(|(l, _)| *l == label).map(|(_, s)| *s)
}

/// The finest interval that shows the span in at most 48 candles.
pub fn auto_interval(span: i64) -> (&'static str, i64) {
    INTERVALS
        .into_iter()
        .find(|(_, secs)| span / secs <= MAX_CANDLES)
        .unwrap_or(INTERVALS[5])
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candle {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub volume: f64,
    pub buys: u32,
    pub sells: u32,
}

/// USD candles, oldest first. Intervals without trades are omitted.
pub fn candles(swaps: &[Swap], market: &Market, secs: i64) -> Vec<Candle> {
    let mut out: Vec<Candle> = vec![];
    for swap in swaps {
        let t = swap.ts - swap.ts.rem_euclid(secs);
        let value = swap.usd(market);
        let price = (value >= MIN_PRICED_USD).then(|| swap.price_usd(market));
        if out.last().is_none_or(|c| c.t != t) {
            let open = price.or(out.last().map(|c| c.c));
            let Some(open) = open else { continue };
            out.push(Candle {
                t,
                o: open,
                h: open,
                l: open,
                c: open,
                volume: 0.0,
                buys: 0,
                sells: 0,
            });
        }
        let candle = out.last_mut().expect("pushed");
        if let Some(price) = price {
            candle.h = candle.h.max(price);
            candle.l = candle.l.min(price);
            candle.c = price;
        }
        candle.volume += value;
        if swap.buy {
            candle.buys += 1
        } else {
            candle.sells += 1
        }
    }
    out
}

pub fn rows(candles: &[Candle]) -> Value {
    json!({
        "columns": ["t", "open", "high", "low", "close", "volume_usd", "buys", "sells"],
        "rows": candles.iter().map(|c| json!([c.t, sig(c.o), sig(c.h), sig(c.l), sig(c.c), usd(c.volume), c.buys, c.sells])).collect::<Vec<_>>()
    })
}

/// Facts about the shape of the chart, so the answer cites structure instead
/// of guessing it from percentage windows.
pub fn structure(candles: &[Candle], swaps: &[Swap], market: &Market, now: i64) -> Value {
    let (Some(first), Some(last)) = (candles.first(), candles.last()) else {
        return Value::Null;
    };
    let ago_h = |t: i64| one((now - t) as f64 / 3600.0);
    let high = candles
        .iter()
        .max_by(|a, b| a.h.total_cmp(&b.h))
        .expect("non-empty");
    let low = candles
        .iter()
        .min_by(|a, b| a.l.total_cmp(&b.l))
        .expect("non-empty");
    let pct = |a: f64, b: f64| one((a / b - 1.0) * 100.0);
    let (value, tokens) = swaps
        .iter()
        .fold((0.0, 0.0), |(v, t), s| (v + s.usd(market), t + s.token));
    let vwap = (tokens > 0.0).then(|| value / tokens);
    let biggest = candles
        .iter()
        .map(|c| (c.c / c.o - 1.0) * 100.0)
        .max_by(|a, b| a.abs().total_cmp(&b.abs()))
        .unwrap_or(0.0);
    let mut view = json!({
        "open": sig(first.o),
        "last": sig(last.c),
        "change_pct": pct(last.c, first.o),
        "high": sig(high.h),
        "high_hours_ago": ago_h(high.t),
        "from_high_pct": pct(last.c, high.h),
        "low": sig(low.l),
        "low_hours_ago": ago_h(low.t),
        "from_low_pct": pct(last.c, low.l),
        "vwap": vwap.map(sig),
        "last_vs_vwap_pct": vwap.map(|v| pct(last.c, v)),
        "biggest_candle_pct": one(biggest),
        "last_trade_minutes_ago": swaps.last().map(|s| (now - s.ts) / 60),
    });
    if candles.len() >= 6 {
        let third = candles.len() / 3;
        let parts = [
            &candles[..third],
            &candles[third..candles.len() - third],
            &candles[candles.len() - third..],
        ];
        let lows = parts.map(|p| p.iter().map(|c| c.l).fold(f64::INFINITY, f64::min));
        let highs = parts.map(|p| p.iter().map(|c| c.h).fold(0.0, f64::max));
        let volumes = parts.map(|p| p.iter().map(|c| c.volume).sum::<f64>());
        let object = view.as_object_mut().expect("object");
        object.insert(
            "lows_rising".into(),
            json!(lows[0] < lows[1] && lows[1] < lows[2]),
        );
        object.insert(
            "highs_falling".into(),
            json!(highs[0] > highs[1] && highs[1] > highs[2]),
        );
        if volumes[0] > 0.0 {
            object.insert(
                "volume_last_vs_first_third".into(),
                one(volumes[2] / volumes[0]),
            );
        }
    }
    view
}

/// The sampled price line before the detailed window, with the high and low
/// over the whole lookback (samples plus candles), so a busy pool's chart
/// still shows where today's top was.
pub fn earlier(points: &[(i64, f64)], candles: &[Candle], lookback_ts: i64, now: i64) -> Value {
    let Some(last) = candles.last().map(|c| c.c) else {
        return Value::Null;
    };
    let mut all: Vec<(i64, f64, f64)> = points.iter().map(|(t, p)| (*t, *p, *p)).collect();
    all.extend(candles.iter().map(|c| (c.t, c.h, c.l)));
    let high = all
        .iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("non-empty");
    let low = all
        .iter()
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .expect("non-empty");
    let start = points.first().map(|p| p.1).unwrap_or(candles[0].o);
    let ago = |t: i64| one((now - t) as f64 / 3600.0);
    json!({
        "from": lookback_ts,
        "points": points.iter().map(|(t, p)| json!([t, sig(*p)])).collect::<Vec<_>>(),
        "lookback_high": sig(high.1),
        "lookback_high_hours_ago": ago(high.0),
        "from_lookback_high_pct": one((last / high.1 - 1.0) * 100.0),
        "lookback_low": sig(low.2),
        "lookback_low_hours_ago": ago(low.0),
        "change_over_lookback_pct": one((last / start - 1.0) * 100.0),
        "note": "sampled points: real highs and lows between samples can be more extreme",
    })
}

#[derive(Default)]
struct Tally {
    buys: u32,
    sells: u32,
    buy_usd: f64,
    sell_usd: f64,
}
impl Tally {
    fn add(&mut self, swap: &Swap, market: &Market) {
        if swap.buy {
            self.buys += 1;
            self.buy_usd += swap.usd(market);
        } else {
            self.sells += 1;
            self.sell_usd += swap.usd(market);
        }
    }
    fn view(&self) -> Value {
        json!({"buys":self.buys,"sells":self.sells,"buy_usd":usd(self.buy_usd),"sell_usd":usd(self.sell_usd),"net_usd":usd(self.buy_usd - self.sell_usd)})
    }
}

/// Swaps whose wallets are worth resolving: the largest on each side and the
/// most recent, so "who's selling" and "who's buying now" are answerable.
pub fn wallet_picks(swaps: &[Swap], market: &Market) -> Vec<usize> {
    let mut by_size: Vec<usize> = (0..swaps.len()).collect();
    by_size.sort_by(|&a, &b| swaps[b].usd(market).total_cmp(&swaps[a].usd(market)));
    let sells = by_size.iter().copied().filter(|&i| !swaps[i].buy).take(20);
    let buys = by_size.iter().copied().filter(|&i| swaps[i].buy).take(15);
    let recent = (0..swaps.len()).rev().take(15);
    let mut picks: Vec<usize> = sells.chain(buys).chain(recent).collect();
    picks.sort_unstable();
    picks.dedup();
    picks
}

pub fn flow(swaps: &[Swap], market: &Market, now: i64) -> Value {
    let mut window = Tally::default();
    let mut hour = Tally::default();
    let mut wallets: HashMap<&str, Tally> = HashMap::new();
    let (mut resolved, mut resolved_sell_usd) = (0, 0.0);
    for swap in swaps {
        window.add(swap, market);
        if now - swap.ts <= 3600 {
            hour.add(swap, market);
        }
        if let Some(wallet) = &swap.wallet {
            resolved += 1;
            if !swap.buy {
                resolved_sell_usd += swap.usd(market);
            }
            wallets.entry(wallet).or_default().add(swap, market);
        }
    }
    let largest = |buy: bool| {
        let mut side: Vec<&Swap> = swaps.iter().filter(|s| s.buy == buy).collect();
        side.sort_by(|a, b| b.usd(market).total_cmp(&a.usd(market)));
        side.iter()
            .take(3)
            .map(|s| json!({"usd":usd(s.usd(market)),"minutes_ago":(now - s.ts) / 60,"wallet":s.wallet,"tx":s.tx}))
            .collect::<Vec<_>>()
    };
    let top = |sell: bool| {
        let mut ranked: Vec<(&&str, &Tally)> = wallets
            .iter()
            .filter(|(_, t)| if sell { t.sells > 0 } else { t.buys > 0 })
            .collect();
        ranked.sort_by(|a, b| {
            let key = |t: &Tally| if sell { t.sell_usd } else { t.buy_usd };
            key(b.1).total_cmp(&key(a.1))
        });
        ranked
            .into_iter()
            .take(3)
            .map(|(wallet, t)| {
                let mut row = json!({"wallet":wallet,"buy_usd":usd(t.buy_usd),"sell_usd":usd(t.sell_usd),"trades":t.buys + t.sells});
                if sell && window.sell_usd > 0.0 {
                    row["share_of_all_sells_pct"] = one(t.sell_usd / window.sell_usd * 100.0);
                }
                row
            })
            .collect::<Vec<_>>()
    };
    json!({
        "last_hour": hour.view(),
        "whole_window": window.view(),
        "largest_buys": largest(true),
        "largest_sells": largest(false),
        "wallets": {
            "resolved_trades": resolved,
            "resolved_share_of_sell_usd_pct": (window.sell_usd > 0.0).then(|| one(resolved_sell_usd / window.sell_usd * 100.0)),
            "distinct_wallets": wallets.len(),
            "top_sellers": top(true),
            "top_buyers": top(false),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn market() -> Market {
        Market {
            kind: "v3",
            topic: None,
            pool_id: "0xpool".into(),
            token: "0xtoken".into(),
            quote: "0xquote".into(),
            token_is_0: true,
            token_decimals: 18,
            quote_decimals: 18,
            quote_usd: 1.0,
            head: (0, 0),
        }
    }
    fn swap(ts: i64, buy: bool, token: f64, quote: f64, wallet: &str) -> Swap {
        Swap {
            block: ts as u64,
            index: 0,
            tx: format!("0x{ts}"),
            buy,
            token,
            quote,
            ts,
            wallet: Some(wallet.into()),
        }
    }

    #[test]
    fn builds_candles_and_ignores_dust_wicks() {
        let m = market();
        let swaps = vec![
            swap(0, true, 10.0, 10.0, "a"),
            swap(30, true, 10.0, 20.0, "a"),
            swap(40, false, 1.0, 0.5, "b"), // dust at a silly price
            swap(70, false, 10.0, 15.0, "b"),
        ];
        let c = candles(&swaps, &m, 60);
        assert_eq!(c.len(), 2);
        assert_eq!((c[0].o, c[0].h, c[0].l, c[0].c), (1.0, 2.0, 1.0, 2.0));
        assert_eq!((c[0].buys, c[0].sells), (2, 1));
        assert_eq!(c[1].o, 1.5);
        assert!((c[0].volume - 30.5).abs() < 1e-9);
    }

    #[test]
    fn structure_reports_distance_from_high_and_rising_lows() {
        let m = market();
        let prices = [1.0, 1.2, 0.9, 1.1, 1.0, 1.3, 1.2, 1.5, 1.4];
        let swaps: Vec<Swap> = prices
            .iter()
            .enumerate()
            .map(|(i, p)| swap(i as i64 * 3600, true, 100.0, 100.0 * p, "a"))
            .collect();
        let c = candles(&swaps, &m, 3600);
        let s = structure(&c, &swaps, &m, 9 * 3600);
        assert_eq!(s["change_pct"], json!(40.0));
        assert_eq!(s["from_high_pct"], json!(-6.7));
        assert_eq!(s["lows_rising"], true);
        assert_eq!(s["highs_falling"], false);
        assert_eq!(s["high_hours_ago"], json!(2.0));
    }

    #[test]
    fn flow_ranks_wallets_and_reports_sell_share() {
        let m = market();
        let swaps = vec![
            swap(0, false, 10.0, 600.0, "whale"),
            swap(10, false, 10.0, 300.0, "whale"),
            swap(20, false, 10.0, 100.0, "small"),
            swap(3700, true, 10.0, 50.0, "buyer"),
        ];
        let f = flow(&swaps, &m, 3700);
        assert_eq!(f["whole_window"]["sell_usd"], json!(1000));
        assert_eq!(f["last_hour"]["buys"], 1);
        assert_eq!(f["last_hour"]["sells"], 0);
        assert_eq!(f["wallets"]["top_sellers"][0]["wallet"], "whale");
        assert_eq!(
            f["wallets"]["top_sellers"][0]["share_of_all_sells_pct"],
            json!(90.0)
        );
        assert_eq!(f["largest_sells"][0]["usd"], json!(600));
        let picks = wallet_picks(&swaps, &m);
        assert_eq!(picks, vec![0, 1, 2, 3]);
    }

    #[test]
    fn picks_an_interval_for_the_span() {
        assert_eq!(auto_interval(3600).0, "5m");
        assert_eq!(auto_interval(12 * 3600).0, "15m");
        assert_eq!(auto_interval(40 * 3600).0, "1h");
        assert_eq!(auto_interval(6 * 86_400).0, "4h");
    }
}
