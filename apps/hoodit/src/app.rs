//! Process-wide runtime shared by every user and thread on a backend host:
//! one HTTP client, one async runtime, a TTL cache, per-turn request
//! budgets and a daily paid-request cap.
use aomi_sdk::DynToolCallCtx;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_CAPACITY: usize = 4096;
/// Paid Codex requests one turn may make before tools return partial data.
pub const TURN_REQUEST_BUDGET: u32 = 10;
/// Paid Codex requests per host per UTC day (~$0.001 each).
pub const DAILY_REQUEST_CAP: u32 = 5000;
/// Keyless LI.FI allows ~100 quotes per 2 h per IP; stay well under it.
const LIFI_KEYLESS_PER_HOUR: u32 = 40;

pub const CODEX_KEY: &str = "CODEX_MPP_KEY";
pub const LIFI_KEY: &str = "LIFI_API_KEY";

/// How long a class of data stays fresh.
#[derive(Clone, Copy)]
pub enum Ttl {
    Live,
    Minute,
    Search,
    Slow,
    Forever,
}
impl Ttl {
    fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Ttl::Live => 20,
            Ttl::Minute => 60,
            Ttl::Search => 180,
            Ttl::Slow => 1800,
            Ttl::Forever => 7 * 24 * 3600,
        })
    }
}

struct Day {
    day: i64,
    used: u32,
}

pub struct Runtime {
    pub http: reqwest::Client,
    pub tokio: tokio::runtime::Runtime,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
    turns: Mutex<HashMap<String, u32>>,
    spend: Mutex<Day>,
    lifi_window: Mutex<(Instant, u32)>,
    tempo: Mutex<Option<(String, Arc<mpp::client::TempoProvider>)>>,
}

impl Runtime {
    fn new() -> Result<Self, String> {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(45))
            .user_agent("hoodit/2.0")
            .build()
            .map_err(|_| "HTTP client initialization failed".to_string())?;
        let tokio = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|_| "async runtime initialization failed".to_string())?;
        Ok(Self {
            http,
            tokio,
            cache: Mutex::new(HashMap::new()),
            turns: Mutex::new(HashMap::new()),
            spend: Mutex::new(Day { day: 0, used: 0 }),
            lifi_window: Mutex::new((Instant::now(), 0)),
            tempo: Mutex::new(None),
        })
    }

    pub fn cached(&self, key: &str) -> Option<Value> {
        let cache = self.cache.lock().ok()?;
        cache
            .get(key)
            .filter(|(expires, _)| *expires > Instant::now())
            .map(|(_, v)| v.clone())
    }

    /// Last known value even if expired, for stale-on-error answers.
    pub fn stale(&self, key: &str) -> Option<Value> {
        self.cache.lock().ok()?.get(key).map(|(_, v)| v.clone())
    }

    pub fn store(&self, key: String, value: Value, ttl: Ttl) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        if cache.len() >= CACHE_CAPACITY {
            let now = Instant::now();
            cache.retain(|_, (expires, _)| *expires > now);
            if cache.len() >= CACHE_CAPACITY {
                let drop: Vec<String> = cache.keys().take(CACHE_CAPACITY / 8).cloned().collect();
                for key in drop {
                    cache.remove(&key);
                }
            }
        }
        cache.insert(key, (Instant::now() + ttl.duration(), value));
    }

    /// Reserves one paid request for this turn and today. `Err` explains
    /// which budget ran out.
    pub fn spend(&self, turn: &str) -> Result<(), &'static str> {
        let today = crate::shape::now() / 86_400;
        {
            let mut day = self.spend.lock().map_err(|_| "spend lock poisoned")?;
            if day.day != today {
                *day = Day {
                    day: today,
                    used: 0,
                };
            }
            if day.used >= DAILY_REQUEST_CAP {
                return Err("daily data budget used up");
            }
            day.used += 1;
        }
        let mut turns = self.turns.lock().map_err(|_| "turn lock poisoned")?;
        if turns.len() > 512 {
            turns.clear();
        }
        let used = turns.entry(turn.to_string()).or_insert(0);
        if *used >= TURN_REQUEST_BUDGET {
            return Err("this answer's data budget is used up");
        }
        *used += 1;
        Ok(())
    }

    /// Keyless LI.FI pacing; keyed calls are not limited here.
    pub fn lifi_slot(&self, keyed: bool) -> bool {
        if keyed {
            return true;
        }
        let Ok(mut window) = self.lifi_window.lock() else {
            return false;
        };
        if window.0.elapsed() >= Duration::from_secs(3600) {
            *window = (Instant::now(), 0);
        }
        if window.1 >= LIFI_KEYLESS_PER_HOUR {
            return false;
        }
        window.1 += 1;
        true
    }

    /// Tempo payment provider for the operator's MPP wallet, built once per key.
    pub fn tempo(&self, key: &str) -> Result<Arc<mpp::client::TempoProvider>, String> {
        let mut slot = self
            .tempo
            .lock()
            .map_err(|_| "payment lock poisoned".to_string())?;
        if let Some((cached_key, provider)) = slot.as_ref()
            && cached_key == key
        {
            return Ok(provider.clone());
        }
        let signer: mpp::PrivateKeySigner = key
            .trim()
            .parse()
            .map_err(|_| "data provider payment key is invalid".to_string())?;
        let provider = mpp::client::TempoProvider::new(signer, "https://rpc.tempo.xyz")
            .map_err(|_| "data provider payment setup failed".to_string())?
            .with_client_id("hoodit");
        let provider = Arc::new(provider);
        *slot = Some((key.to_string(), provider.clone()));
        Ok(provider)
    }
}

/// Per-tool-call context: credentials, the turn it belongs to, the user's
/// connected wallet, and the gaps the answer must disclose.
pub struct Call {
    pub turn: String,
    pub codex_key: Option<String>,
    pub lifi_key: Option<String>,
    pub wallet: Option<String>,
    pub gaps: Vec<String>,
}

impl Call {
    pub fn new(ctx: &DynToolCallCtx) -> Self {
        let secret = |name: &str| {
            aomi_sdk::resolve_secret_value(ctx, None, name, "")
                .ok()
                .map(|v| v.trim().trim_matches(|c| c == '"' || c == '\'').to_string())
                .filter(|v| !v.is_empty())
        };
        let turn = ctx
            .attribute_path(&["hosted", "turn_id"])
            .and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| Some(v.to_string()))
            })
            .unwrap_or_else(|| ctx.call_id.clone());
        let wallet = ctx
            .attribute_path(&["domain", "evm", "address"])
            .and_then(Value::as_str)
            .and_then(|a| crate::shape::address(a).ok());
        Self {
            turn,
            codex_key: secret(CODEX_KEY),
            lifi_key: secret(LIFI_KEY),
            wallet,
            gaps: vec![],
        }
    }

    pub fn gap(&mut self, gap: impl Into<String>) {
        let gap = gap.into();
        if !self.gaps.contains(&gap) {
            self.gaps.push(gap);
        }
    }
}

#[derive(Clone, Default)]
pub struct HooditApp {
    runtime: Arc<OnceLock<Result<Arc<Runtime>, String>>>,
}

impl HooditApp {
    pub fn runtime(&self) -> Result<Arc<Runtime>, String> {
        self.runtime
            .get_or_init(|| Runtime::new().map(Arc::new))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn budgets_cap_paid_requests_per_turn() {
        let rt = Runtime::new().unwrap();
        for _ in 0..TURN_REQUEST_BUDGET {
            assert!(rt.spend("t1").is_ok());
        }
        assert!(rt.spend("t1").is_err());
        assert!(rt.spend("t2").is_ok());
    }

    #[test]
    fn cache_expires_but_stays_stale() {
        let rt = Runtime::new().unwrap();
        rt.store("k".into(), json!(1), Ttl::Live);
        assert_eq!(rt.cached("k"), Some(json!(1)));
        assert_eq!(rt.stale("k"), Some(json!(1)));
    }
}
