//! Provider-neutral pool snapshots, launchpad lifecycle, and setup flags.

use crate::model::{self, one, opt, sig, usd};
use serde_json::{Value, json};

pub mod chart;
pub mod swaps;

#[derive(Clone, Copy, Debug, Default)]
pub struct Win<T> {
    pub m5: Option<T>,
    pub h1: Option<T>,
    pub h6: Option<T>,
    pub h24: Option<T>,
}
impl<T: Copy> Win<T> {
    pub fn from(read: impl Fn(&str) -> Option<T>) -> Self {
        Self {
            m5: read("m5"),
            h1: read("h1"),
            h6: read("h6"),
            h24: read("h24"),
        }
    }
}

/// One pool as a market snapshot. `token` is the pool's base side.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub pool_id: String,
    pub venue: String,
    /// Swap-log layout: v2, v3, v4 or curve.
    pub kind: Option<&'static str>,
    pub token: String,
    pub symbol: String,
    pub name: String,
    pub quote: String,
    pub quote_symbol: String,
    pub price_usd: Option<f64>,
    pub quote_usd: Option<f64>,
    pub liquidity_usd: Option<f64>,
    pub fdv_usd: Option<f64>,
    pub mcap_usd: Option<f64>,
    pub created: Option<i64>,
    pub change: Win<f64>,
    pub volume: Win<f64>,
    pub buys: Win<u64>,
    pub sells: Win<u64>,
}

impl Snapshot {
    pub fn age_hours(&self, now: i64) -> Option<f64> {
        self.created
            .map(|created| (now - created).max(0) as f64 / 3600.0)
    }
    pub fn txns_24h(&self) -> Option<u64> {
        Some(self.buys.h24? + self.sells.h24?)
    }
    pub fn pair(&self) -> String {
        format!("{}/{}", self.symbol, self.quote_symbol)
    }
    pub fn view(&self, now: i64) -> Value {
        let mcap = self.mcap_usd.filter(|mcap| {
            self.fdv_usd
                .is_none_or(|fdv| (fdv - mcap).abs() > fdv * 0.01)
        });
        let pair = |w: &Win<u64>, s: &Win<u64>| json!({"h1":[w.h1,s.h1],"h6":[w.h6,s.h6],"h24":[w.h24,s.h24]});
        json!({
            "pool_id": self.pool_id,
            "venue": self.venue,
            "pair": self.pair(),
            "price_usd": opt(self.price_usd, sig),
            "liquidity_usd": opt(self.liquidity_usd, usd),
            "fdv_usd": opt(self.fdv_usd, usd),
            "mcap_usd": opt(mcap, usd),
            "age_h": opt(self.age_hours(now), one),
            "change_pct": {"m5":opt(self.change.m5,one),"h1":opt(self.change.h1,one),"h6":opt(self.change.h6,one),"h24":opt(self.change.h24,one)},
            "volume_usd": {"h1":opt(self.volume.h1,usd),"h6":opt(self.volume.h6,usd),"h24":opt(self.volume.h24,usd)},
            "buys_sells": pair(&self.buys, &self.sells),
        })
    }
}

/// Factual setup flags with the numbers behind them. They describe risk and
/// momentum; the skill explains how to weigh them.
pub fn flags(s: &Snapshot, now: i64) -> Vec<String> {
    let mut flags = vec![];
    let ratio = |a: Option<f64>, b: Option<f64>| match (a, b) {
        (Some(a), Some(b)) if b > 0.0 => Some(a / b),
        _ => None,
    };
    if let Some(age) = s.age_hours(now).filter(|age| *age < 6.0) {
        flags.push(format!("fresh: pool is {age:.1}h old"));
    }
    if let Some(liquidity) = s.liquidity_usd.filter(|l| *l < 10_000.0) {
        flags.push(format!("micro_liquidity: ${liquidity:.0}"));
    }
    let (h1, h6, h24) = (s.change.h1, s.change.h6, s.change.h24);
    if h24.is_some_and(|c| c >= 300.0) || h6.is_some_and(|c| c >= 150.0) {
        flags.push(format!(
            "extended: {:+.0}% 6h, {:+.0}% 24h",
            h6.unwrap_or(0.0),
            h24.unwrap_or(0.0)
        ));
    }
    if (h24.is_some_and(|c| c >= 100.0) || h6.is_some_and(|c| c >= 50.0))
        && h1.is_some_and(|c| c <= -10.0)
    {
        flags.push(format!(
            "fading: {:+.0}% last hour after the run",
            h1.unwrap_or(0.0)
        ));
    }
    if h24.is_some_and(|c| c <= -50.0) {
        flags.push(format!("dumping: {:+.0}% 24h", h24.unwrap_or(0.0)));
    }
    if let Some(turnover) = ratio(s.volume.h24, s.liquidity_usd).filter(|t| *t >= 20.0) {
        flags.push(format!("churn: 24h volume is {turnover:.0}x liquidity"));
    }
    if let Some(depth) = ratio(s.fdv_usd, s.liquidity_usd).filter(|d| *d >= 50.0) {
        flags.push(format!("thin_exit: fdv is {depth:.0}x liquidity"));
    }
    if let (Some(buys), Some(sells)) = (s.buys.h1, s.sells.h1)
        && buys + sells >= 30
    {
        let share = buys as f64 / (buys + sells) as f64;
        if share <= 0.4 {
            flags.push(format!(
                "sellers_in_control: {buys} buys vs {sells} sells last hour"
            ));
        } else if share >= 0.65 {
            flags.push(format!(
                "buyers_in_control: {buys} buys vs {sells} sells last hour"
            ));
        }
    }
    if s.txns_24h().is_some_and(|n| n < 20) {
        flags.push(format!(
            "quiet: {} trades in 24h",
            s.txns_24h().unwrap_or(0)
        ));
    }
    flags
}

/// Launchpad stage from GeckoTerminal's launchpad record.
#[derive(Clone, Debug, Default)]
pub struct Lifecycle {
    pub stage: &'static str,
    pub progress_pct: Option<f64>,
    pub graduated_at: Option<String>,
    pub destination_pool: Option<String>,
    pub curve_pool: Option<String>,
}
impl Lifecycle {
    pub fn unknown() -> Self {
        Self {
            stage: "unknown",
            ..Default::default()
        }
    }
    pub fn view(&self) -> Value {
        let mut view = json!({"stage": self.stage});
        let object = view.as_object_mut().expect("object");
        if let Some(pct) = self.progress_pct {
            object.insert("curve_progress_pct".into(), one(pct));
        }
        for (key, value) in [
            ("graduated_at", &self.graduated_at),
            ("destination_pool_id", &self.destination_pool),
            ("curve_pool_id", &self.curve_pool),
        ] {
            if let Some(value) = value {
                object.insert(key.into(), json!(value));
            }
        }
        view
    }
}

/// Picks the pool that best represents a token: most 24h volume among pools
/// where it is the base token, then liquidity. Pools without a trade in 24h
/// and near-empty pools are skipped, so a dormant pre-created pool (Pons makes
/// one per curve) never hides a live curve.
pub fn main_pool<'a>(pools: &'a [Snapshot], token: &str) -> Option<&'a Snapshot> {
    let score = |s: &Snapshot| s.volume.h24.unwrap_or(0.0) + 0.1 * s.liquidity_usd.unwrap_or(0.0);
    pools
        .iter()
        .filter(|s| s.token == token && s.kind.is_some())
        .filter(|s| {
            s.liquidity_usd.unwrap_or(0.0) >= 500.0 || s.volume.h24.unwrap_or(0.0) >= 5000.0
        })
        .filter(|s| {
            s.txns_24h()
                .map_or(s.liquidity_usd.unwrap_or(0.0) >= 1000.0, |n| n > 0)
        })
        .max_by(|a, b| score(a).total_cmp(&score(b)))
}

/// The deepest pool of the token, active or not: what a dead coin is judged on.
pub fn deepest_pool<'a>(pools: &'a [Snapshot], token: &str) -> Option<&'a Snapshot> {
    pools
        .iter()
        .filter(|s| s.token == token && s.kind.is_some())
        .max_by(|a, b| {
            a.liquidity_usd
                .unwrap_or(0.0)
                .total_cmp(&b.liquidity_usd.unwrap_or(0.0))
        })
}

pub fn is_base_asset(token: &str) -> bool {
    [model::WETH, model::USDG, model::NATIVE].contains(&token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            pool_id: "0xpool".into(),
            token: "0xtoken".into(),
            kind: Some("v3"),
            symbol: "SC".into(),
            quote_symbol: "WETH".into(),
            liquidity_usd: Some(275_000.0),
            fdv_usd: Some(21_000_000.0),
            created: Some(0),
            change: Win {
                h1: Some(-11.0),
                h6: Some(40.0),
                h24: Some(2166.0),
                ..Default::default()
            },
            volume: Win {
                h24: Some(18_900_000.0),
                ..Default::default()
            },
            buys: Win {
                h1: Some(400),
                h24: Some(17_000),
                ..Default::default()
            },
            sells: Win {
                h1: Some(700),
                h24: Some(8_500),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn blow_off_top_is_flagged_with_numbers() {
        let flags = flags(&snapshot(), 86_400);
        let joined = flags.join(" | ");
        assert!(joined.contains("extended: +40% 6h, +2166% 24h"), "{joined}");
        assert!(joined.contains("fading: -11%"), "{joined}");
        assert!(
            joined.contains("churn: 24h volume is 69x liquidity"),
            "{joined}"
        );
        assert!(
            joined.contains("thin_exit: fdv is 76x liquidity"),
            "{joined}"
        );
        assert!(
            joined.contains("sellers_in_control: 400 buys vs 700 sells"),
            "{joined}"
        );
        assert!(!joined.contains("fresh"));
    }

    #[test]
    fn steady_pool_has_no_flags() {
        let steady = Snapshot {
            liquidity_usd: Some(500_000.0),
            fdv_usd: Some(5_000_000.0),
            created: Some(0),
            change: Win {
                h1: Some(1.0),
                h6: Some(5.0),
                h24: Some(12.0),
                ..Default::default()
            },
            volume: Win {
                h24: Some(400_000.0),
                ..Default::default()
            },
            buys: Win {
                h1: Some(20),
                h24: Some(900),
                ..Default::default()
            },
            sells: Win {
                h1: Some(18),
                h24: Some(800),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(flags(&steady, 30 * 86_400).is_empty());
    }

    #[test]
    fn main_pool_prefers_active_base_pools() {
        let mut quiet = snapshot();
        quiet.pool_id = "quiet".into();
        quiet.volume.h24 = Some(10.0);
        quiet.liquidity_usd = Some(10.0);
        let mut other = snapshot();
        other.pool_id = "quote-side".into();
        other.token = "0xother".into();
        let pools = vec![quiet, snapshot(), other];
        assert_eq!(main_pool(&pools, "0xtoken").unwrap().pool_id, "0xpool");
    }
}
