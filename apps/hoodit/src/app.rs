use chrono::Utc;
use reqwest::blocking::Client;
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, USER_AGENT};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct ProviderOrigins {
    pub gecko: String,
    pub goplus: String,
    pub coingecko: String,
    pub blockscout: String,
    pub lifi: String,
}
impl Default for ProviderOrigins {
    fn default() -> Self {
        Self {
            gecko: "https://api.geckoterminal.com/api/v2".into(),
            goplus: "https://api.gopluslabs.io/api/v1".into(),
            coingecko: "https://api.coingecko.com/api/v3".into(),
            blockscout: "https://api.blockscout.com/4663".into(),
            lifi: "https://li.quest/v1".into(),
        }
    }
}

#[derive(Clone)]
struct CacheEntry {
    value: Value,
    fetched_at: String,
    inserted: Instant,
    expires: Instant,
}

struct RateState {
    window_started: Instant,
    used: u32,
    window: Duration,
}
pub struct Runtime {
    pub http: Client,
    pub origins: ProviderOrigins,
    cache: Mutex<HashMap<String, CacheEntry>>,
    rates: Mutex<HashMap<String, RateState>>,
}
impl Runtime {
    fn create(http: Client, origins: ProviderOrigins) -> Self {
        Self {
            http,
            origins,
            cache: Mutex::new(HashMap::new()),
            rates: Mutex::new(HashMap::new()),
        }
    }
    #[doc(hidden)]
    pub fn fixture(http: Client, origins: ProviderOrigins) -> Self {
        Self::create(http, origins)
    }
    pub(crate) fn cached(&self, key: &str, refresh: bool) -> Option<(Value, String)> {
        if refresh {
            return None;
        }
        let mut cache = self.cache.lock().ok()?;
        let entry = cache.get(key)?.clone();
        if entry.expires <= Instant::now() {
            cache.remove(key);
            return None;
        }
        Some((entry.value, entry.fetched_at))
    }
    pub(crate) fn cache(&self, key: String, value: Value, ttl: Duration) -> String {
        let fetched_at = Utc::now().to_rfc3339();
        if let Ok(mut cache) = self.cache.lock() {
            if cache.len() >= CACHE_CAPACITY
                && let Some(oldest) = cache
                    .iter()
                    .min_by_key(|(_, entry)| entry.inserted)
                    .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
            let now = Instant::now();
            cache.insert(
                key,
                CacheEntry {
                    value,
                    fetched_at: fetched_at.clone(),
                    inserted: now,
                    expires: now + ttl,
                },
            );
        }
        fetched_at
    }
    pub(crate) fn spend_rate(&self, provider: &str, credential: Option<&str>) -> Option<Duration> {
        let (limit, window) = match provider {
            "blockscout" => (5, Duration::from_secs(1)),
            "geckoterminal" => (10, Duration::from_secs(60)),
            "goplus" => (30, Duration::from_secs(60)),
            "coingecko" => (10, Duration::from_secs(60)),
            "lifi" if credential.is_some() => (200, Duration::from_secs(7200)),
            "lifi" => (75, Duration::from_secs(7200)),
            _ => return None,
        };
        let mut hasher = DefaultHasher::new();
        credential.unwrap_or("").hash(&mut hasher);
        let key = format!("{provider}:{:016x}", hasher.finish());
        let mut rates = self.rates.lock().ok()?;
        let now = Instant::now();
        rates.retain(|_, state| now.duration_since(state.window_started) < state.window);
        if rates.len() >= CACHE_CAPACITY
            && let Some(oldest) = rates
                .iter()
                .min_by_key(|(_, state)| state.window_started)
                .map(|(key, _)| key.clone())
        {
            rates.remove(&oldest);
        }
        let state = rates.entry(key).or_insert(RateState {
            window_started: now,
            used: 0,
            window,
        });
        if now.duration_since(state.window_started) >= window {
            state.window_started = now;
            state.used = 0;
        }
        if state.used >= limit {
            return Some(window.saturating_sub(now.duration_since(state.window_started)));
        }
        state.used += 1;
        None
    }
}

pub struct ReadContext {
    pub deadline: Instant,
    pub refresh: bool,
    pub sources: Vec<Value>,
    pub warnings: Vec<Value>,
    budgets: HashMap<&'static str, u16>,
}
impl ReadContext {
    pub fn markets(refresh: bool) -> Self {
        Self::new(Duration::from_secs(15), refresh, 0)
    }
    pub fn portfolio(refresh: bool) -> Self {
        Self::new(Duration::from_secs(30), refresh, 20)
    }
    fn new(duration: Duration, refresh: bool, lifi_budget: u16) -> Self {
        Self {
            deadline: Instant::now() + duration,
            refresh,
            sources: vec![],
            warnings: vec![],
            budgets: HashMap::from([
                ("geckoterminal", 10),
                ("goplus", 10),
                ("coingecko", 2),
                ("blockscout", 25),
                ("lifi", lifi_budget),
            ]),
        }
    }
    pub fn remaining(&self) -> Option<Duration> {
        let remaining = self.deadline.saturating_duration_since(Instant::now());
        (!remaining.is_zero()).then_some(remaining)
    }
    pub(crate) fn spend(&mut self, provider: &'static str) -> bool {
        let Some(remaining) = self.budgets.get_mut(provider) else {
            return false;
        };
        if *remaining == 0 {
            return false;
        }
        *remaining -= 1;
        true
    }
    pub(crate) fn source(
        &mut self,
        provider: &str,
        resource: &str,
        fetched_at: String,
        cached: bool,
    ) {
        self.sources.push(json!({"provider":provider,"resource":resource,"fetched_at":fetched_at,"provider_updated_at":null,"cached":cached}));
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
    headers.insert(USER_AGENT, HeaderValue::from_static("hoodit/1.2"));
    Client::builder()
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(10))
        .default_headers(headers)
        .build()
        .map(|http| Arc::new(Runtime::create(http, ProviderOrigins::default())))
        .map_err(|_| "HTTP client initialization failed".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_is_bounded_and_refresh_bypasses_it() {
        let runtime = Runtime::fixture(Client::new(), ProviderOrigins::default());
        runtime.cache("answer".into(), json!({"ok":true}), Duration::from_secs(5));
        assert!(runtime.cached("answer", false).is_some());
        assert!(runtime.cached("answer", true).is_none());
        runtime.cache("expired".into(), json!(1), Duration::ZERO);
        assert!(runtime.cached("expired", false).is_none());
        for index in 0..300 {
            runtime.cache(format!("key-{index}"), json!(index), Duration::from_secs(5));
        }
        assert_eq!(runtime.cache.lock().unwrap().len(), CACHE_CAPACITY);
    }

    #[test]
    fn shared_rate_limits_are_credential_scoped() {
        let runtime = Runtime::fixture(Client::new(), ProviderOrigins::default());
        for _ in 0..5 {
            assert!(runtime.spend_rate("blockscout", Some("first")).is_none());
        }
        assert!(runtime.spend_rate("blockscout", Some("first")).is_some());
        assert!(runtime.spend_rate("blockscout", Some("second")).is_none());
        for index in 0..300 {
            assert!(
                runtime
                    .spend_rate("blockscout", Some(&format!("scope-{index}")))
                    .is_none()
            );
        }
        assert!(runtime.rates.lock().unwrap().len() <= CACHE_CAPACITY);
    }

    #[test]
    fn operation_budget_counts_attempts() {
        let mut read = ReadContext::portfolio(false);
        for _ in 0..20 {
            assert!(read.spend("lifi"));
        }
        assert!(!read.spend("lifi"));
        assert!(read.remaining().is_some());
    }
}
