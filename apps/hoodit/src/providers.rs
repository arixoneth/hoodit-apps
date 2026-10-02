//! Keyless public data sources. Every request is cached, rate-budgeted per
//! provider, and bounded by the calling tool's deadline.

use crate::app::{Call, Runtime};
use reqwest::{StatusCode, header::RETRY_AFTER};
use serde_json::Value;
use std::time::Duration;

pub mod dex;
pub mod gecko;
pub mod goplus;
pub mod lifi;
pub mod rpc;

#[derive(Debug)]
pub struct ProviderError {
    pub code: &'static str,
    pub message: String,
}
impl ProviderError {
    pub(crate) fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn retryable(&self) -> bool {
        matches!(self.code, "RATE_LIMITED" | "UNAVAILABLE" | "DEADLINE")
    }
}

pub(crate) enum Body<'a> {
    Get(&'a [(&'a str, String)]),
    Post(&'a Value),
}

fn display(provider: &str) -> &str {
    match provider {
        "dexscreener" => "DexScreener",
        "geckoterminal" => "GeckoTerminal",
        "goplus" => "GoPlus",
        "rpc" | "rpc-fast" => "Robinhood RPC",
        "lifi" => "LI.FI",
        other => other,
    }
}

/// One JSON request. `cost` counts batched RPC calls against the allowance;
/// `ttl` of None skips the cache.
pub(crate) fn fetch(
    rt: &Runtime,
    call: &Call,
    provider: &'static str,
    url: &str,
    body: Body,
    cost: u32,
    ttl: Option<Duration>,
) -> Result<Value, ProviderError> {
    let name = display(provider);
    let key = match &body {
        Body::Get(query) => format!("{provider}|{url}|{query:?}"),
        Body::Post(value) => format!("{provider}|{url}|{value}"),
    };
    if ttl.is_some()
        && let Some(value) = rt.cached(&key)
    {
        return Ok(value);
    }
    for attempt in 0..2 {
        let Some(remaining) = call.remaining() else {
            return Err(ProviderError::new(
                "DEADLINE",
                format!("ran out of time before {name} answered"),
            ));
        };
        if !rt.take(provider, cost) {
            return Err(ProviderError::new(
                "RATE_LIMITED",
                format!("{name} request allowance is used up for now"),
            ));
        }
        let request = match &body {
            Body::Get(query) => rt.http.get(url).query(query),
            Body::Post(value) => rt.http.post(url).json(value),
        };
        let started = std::time::Instant::now();
        let sent = request
            .timeout(remaining.min(Duration::from_secs(12)))
            .send();
        if std::env::var_os("HOODIT_DEBUG").is_some() {
            let what = match &body {
                Body::Post(v) => v
                    .get("method")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("batch x{cost}")),
                Body::Get(_) => url
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .chars()
                    .take(30)
                    .collect(),
            };
            eprintln!(
                "  {name} {what}: {:?} {:?}",
                started.elapsed(),
                sent.as_ref().map(|r| r.status().as_u16())
            );
        }
        let response = match sent {
            Ok(response) => response,
            Err(_) if attempt == 0 => continue,
            Err(_) => {
                return Err(ProviderError::new(
                    "UNAVAILABLE",
                    format!("{name} did not respond"),
                ));
            }
        };
        let status = response.status();
        if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
            let delay = retry_after(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok()),
            );
            if attempt == 0
                && delay <= Duration::from_secs(2)
                && call
                    .remaining()
                    .is_some_and(|left| left > delay + Duration::from_secs(2))
            {
                std::thread::sleep(delay);
                continue;
            }
            return Err(if status == StatusCode::TOO_MANY_REQUESTS {
                ProviderError::new("RATE_LIMITED", format!("{name} is rate limiting us"))
            } else {
                ProviderError::new("UNAVAILABLE", format!("{name} returned {status}"))
            });
        }
        if !status.is_success() {
            let body = response.json::<Value>().ok();
            if provider == "lifi" && body.as_ref().is_some_and(no_route) {
                return Err(ProviderError::new(
                    "NO_ROUTE",
                    "LI.FI found no route at this size",
                ));
            }
            return Err(if status == StatusCode::NOT_FOUND {
                ProviderError::new("NOT_FOUND", format!("{name} has no record of this"))
            } else {
                ProviderError::new("UNAVAILABLE", format!("{name} returned {status}"))
            });
        }
        let value = response.json::<Value>().map_err(|_| {
            ProviderError::new("BAD_RESPONSE", format!("{name} returned unreadable data"))
        })?;
        if let Some(ttl) = ttl {
            rt.store(key, value.clone(), ttl);
        }
        return Ok(value);
    }
    Err(ProviderError::new(
        "UNAVAILABLE",
        format!("{name} did not respond"),
    ))
}

fn no_route(body: &Value) -> bool {
    body.get("code").is_some_and(|code| {
        code.as_u64() == Some(1002)
            || code
                .as_str()
                .is_some_and(|c| matches!(c, "NO_QUOTE" | "NO_ROUTE" | "NoQuoteError"))
    })
}

fn retry_after(value: Option<&str>) -> Duration {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(3600)))
        .unwrap_or(Duration::from_millis(400))
}

#[cfg(test)]
pub(crate) mod testing {
    use crate::app::{Origins, Runtime};
    use reqwest::blocking::Client;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    /// Serves canned HTTP responses in order and counts requests.
    pub fn server(responses: Vec<String>) -> (Runtime, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        std::thread::spawn(move || {
            for response in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = [0_u8; 16384];
                let _ = stream.read(&mut request);
                seen.fetch_add(1, Ordering::SeqCst);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        let origins = Origins {
            dexscreener: base.clone(),
            gecko: base.clone(),
            goplus: base.clone(),
            rpc: base.clone(),
            rpc_fast: base.clone(),
            lifi: base,
        };
        (Runtime::new(Client::new(), origins), count)
    }

    pub fn reply(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::{reply, server};
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn caches_and_reports_rate_limits() {
        let (rt, count) = server(vec![
            reply("200 OK", "{\"ok\":true}"),
            reply("429 Too Many Requests", "{}"),
            reply("429 Too Many Requests", "{}"),
        ]);
        let call = Call::new(10);
        let url = format!("{}/a", rt.origins.dexscreener);
        let ttl = Some(Duration::from_secs(30));
        assert!(fetch(&rt, &call, "dexscreener", &url, Body::Get(&[]), 1, ttl).is_ok());
        assert!(fetch(&rt, &call, "dexscreener", &url, Body::Get(&[]), 1, ttl).is_ok());
        assert_eq!(count.load(Ordering::SeqCst), 1);
        let url = format!("{}/b", rt.origins.dexscreener);
        let error = fetch(&rt, &call, "dexscreener", &url, Body::Get(&[]), 1, ttl).unwrap_err();
        assert_eq!(error.code, "RATE_LIMITED");
        assert!(error.message.contains("DexScreener"));
    }

    #[test]
    fn expired_deadline_sends_nothing() {
        let (rt, count) = server(vec![]);
        let mut call = Call::new(10);
        call.deadline = std::time::Instant::now();
        let url = format!("{}/a", rt.origins.gecko);
        let error = fetch(&rt, &call, "geckoterminal", &url, Body::Get(&[]), 1, None).unwrap_err();
        assert_eq!(error.code, "DEADLINE");
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
