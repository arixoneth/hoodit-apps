use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_CAPACITY: usize = 2048;

#[derive(Clone)]
pub struct Origins {
    pub dexscreener: String,
    pub gecko: String,
    pub goplus: String,
    pub rpc: String,
    /// Low-latency public endpoint for cheap reads; `rpc` is the fallback and
    /// serves wide log ranges, which this one rejects.
    pub rpc_fast: String,
    pub lifi: String,
}
impl Default for Origins {
    fn default() -> Self {
        Self {
            dexscreener: "https://api.dexscreener.com".into(),
            gecko: "https://api.geckoterminal.com/api/v2".into(),
            goplus: "https://api.gopluslabs.io/api/v1".into(),
            rpc: "https://rpc.mainnet.chain.robinhood.com".into(),
            rpc_fast: "https://rpc.ordofi.network".into(),
            lifi: "https://li.quest/v1".into(),
        }
    }
}

/// Requests allowed per window for each provider, below the published public
/// allowances so one process never trips them on its own.
fn allowance(provider: &str) -> Option<(u32, Duration)> {
    Some(match provider {
        "geckoterminal" => (9, Duration::from_secs(60)),
        "goplus" => (30, Duration::from_secs(60)),
        "dexscreener" => (240, Duration::from_secs(60)),
        "rpc" => (120, Duration::from_secs(60)),
        "rpc-fast" => (1500, Duration::from_secs(60)),
        "lifi" => (70, Duration::from_secs(7200)),
        _ => return None,
    })
}

pub struct Runtime {
    pub http: Client,
    pub origins: Origins,
    cache: Mutex<HashMap<String, (Instant, Value)>>,
    rates: Mutex<HashMap<String, (Instant, u32)>>,
}
impl Runtime {
    pub fn new(http: Client, origins: Origins) -> Self {
        Self {
            http,
            origins,
            cache: Mutex::new(HashMap::new()),
            rates: Mutex::new(HashMap::new()),
        }
    }
    pub(crate) fn cached(&self, key: &str) -> Option<Value> {
        let mut cache = self.cache.lock().ok()?;
        match cache.get(key) {
            Some((expires, value)) if *expires > Instant::now() => Some(value.clone()),
            Some(_) => {
                cache.remove(key);
                None
            }
            None => None,
        }
    }
    pub(crate) fn store(&self, key: String, value: Value, ttl: Duration) {
        let Ok(mut cache) = self.cache.lock() else {
            return;
        };
        if cache.len() >= CACHE_CAPACITY {
            let now = Instant::now();
            cache.retain(|_, (expires, _)| *expires > now);
            if cache.len() >= CACHE_CAPACITY
                && let Some(soonest) = cache
                    .iter()
                    .min_by_key(|(_, (expires, _))| *expires)
                    .map(|(key, _)| key.clone())
            {
                cache.remove(&soonest);
            }
        }
        cache.insert(key, (Instant::now() + ttl, value));
    }
    /// Spends `cost` requests from the provider's window; false when exhausted.
    pub(crate) fn take(&self, provider: &str, cost: u32) -> bool {
        let Some((limit, window)) = allowance(provider) else {
            return true;
        };
        let Ok(mut rates) = self.rates.lock() else {
            return false;
        };
        let now = Instant::now();
        let (started, used) = rates.entry(provider.into()).or_insert((now, 0));
        if now.duration_since(*started) >= window {
            *started = now;
            *used = 0;
        }
        if *used + cost > limit {
            return false;
        }
        *used += cost;
        true
    }
}

/// Per-tool-call deadline and the coverage notes the answer must disclose.
pub struct Call {
    pub deadline: Instant,
    pub notes: Vec<String>,
}
impl Call {
    pub fn new(seconds: u64) -> Self {
        Self {
            deadline: Instant::now() + Duration::from_secs(seconds),
            notes: vec![],
        }
    }
    pub fn remaining(&self) -> Option<Duration> {
        let left = self.deadline.saturating_duration_since(Instant::now());
        (!left.is_zero()).then_some(left)
    }
    pub fn note(&mut self, note: impl Into<String>) {
        let note = note.into();
        if !self.notes.contains(&note) {
            self.notes.push(note);
        }
    }
}

#[derive(Clone, Default)]
pub struct HooditApp {
    runtime: Arc<OnceLock<Result<Arc<Runtime>, String>>>,
}
impl HooditApp {
    pub fn runtime(&self) -> Result<Arc<Runtime>, String> {
        self.runtime.get_or_init(build_runtime).clone()
    }
    #[doc(hidden)]
    pub fn with_runtime(runtime: Runtime) -> Self {
        let slot = OnceLock::new();
        let _ = slot.set(Ok(Arc::new(runtime)));
        Self {
            runtime: Arc::new(slot),
        }
    }
}

fn build_runtime() -> Result<Arc<Runtime>, String> {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("hoodit/1.5"));
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(12))
        .default_headers(headers)
        .build()
        .map(|http| Arc::new(Runtime::new(http, Origins::default())))
        .map_err(|_| "HTTP client initialization failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cache_expires_and_stays_bounded() {
        let runtime = Runtime::new(Client::new(), Origins::default());
        runtime.store("a".into(), json!(1), Duration::from_secs(5));
        assert_eq!(runtime.cached("a"), Some(json!(1)));
        runtime.store("b".into(), json!(2), Duration::ZERO);
        assert_eq!(runtime.cached("b"), None);
        for index in 0..(CACHE_CAPACITY + 10) {
            runtime.store(format!("k{index}"), json!(index), Duration::from_secs(5));
        }
        assert!(runtime.cache.lock().unwrap().len() <= CACHE_CAPACITY);
    }

    #[test]
    fn provider_windows_cap_requests() {
        let runtime = Runtime::new(Client::new(), Origins::default());
        for _ in 0..9 {
            assert!(runtime.take("geckoterminal", 1));
        }
        assert!(!runtime.take("geckoterminal", 1));
        assert!(runtime.take("dexscreener", 1));
        assert!(!runtime.take("rpc", 121));
    }
}
