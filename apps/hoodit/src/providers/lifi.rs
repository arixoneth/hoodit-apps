//! LI.FI read-only same-chain quotes. Calldata is discarded; execution belongs
//! to the host's swap flow.

use super::{Body, ProviderError, fetch};
use crate::app::{Call, Runtime};
use crate::model::{self, CHAIN_ID};
use serde_json::Value;
use std::time::Duration;

pub struct Lifi<'a> {
    rt: &'a Runtime,
}
impl<'a> Lifi<'a> {
    pub fn new(rt: &'a Runtime) -> Self {
        Self { rt }
    }
    pub fn quote(
        &self,
        call: &Call,
        wallet: &str,
        from: &str,
        to: &str,
        amount: &str,
        slippage_bps: u32,
    ) -> Result<Value, ProviderError> {
        if amount.is_empty() || amount.len() > 78 || !amount.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ProviderError::new("BAD_REQUEST", "invalid quote amount"));
        }
        let query = [
            ("fromChain", CHAIN_ID.to_string()),
            ("toChain", CHAIN_ID.to_string()),
            ("fromToken", from.to_string()),
            ("toToken", to.to_string()),
            ("fromAmount", amount.to_string()),
            ("fromAddress", wallet.to_string()),
            ("toAddress", wallet.to_string()),
            ("order", "RECOMMENDED".into()),
            (
                "slippage",
                format!("{}", f64::from(slippage_bps) / 10_000.0),
            ),
        ];
        let url = format!("{}/quote", self.rt.origins.lifi);
        let mut value = fetch(
            self.rt,
            call,
            "lifi",
            &url,
            Body::Get(&query),
            1,
            Some(Duration::from_secs(5)),
        )?;
        if !echoes(&value, wallet, from, to, amount) {
            return Err(ProviderError::new(
                "BAD_RESPONSE",
                "LI.FI quote does not match the request",
            ));
        }
        if let Some(object) = value.as_object_mut() {
            object.remove("transactionRequest");
        }
        Ok(value)
    }
}

fn echoes(value: &Value, wallet: &str, from: &str, to: &str, amount: &str) -> bool {
    let chain = |path: &[&str]| {
        model::get(value, path).and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
    };
    let same = |path: &[&str], expected: &str| {
        model::string(value, path).is_some_and(|v| v.eq_ignore_ascii_case(expected))
    };
    chain(&["action", "fromChainId"]) == Some(CHAIN_ID)
        && chain(&["action", "toChainId"]) == Some(CHAIN_ID)
        && same(&["action", "fromToken", "address"], from)
        && same(&["action", "toToken", "address"], to)
        && same(&["action", "fromAddress"], wallet)
        && same(&["action", "toAddress"], wallet)
        && same(&["action", "fromAmount"], amount)
        && model::string(value, &["estimate", "toAmount"])
            .is_some_and(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn quote_must_echo_the_request() {
        let wallet = "0xb202bb725c85b90bd847d350ebc7f16ff8408ed8";
        let from = "0x39dbed3a2bd333467115de45665cc57f813c4571";
        let quote = json!({"action":{"fromChainId":4663,"toChainId":4663,"fromAmount":"1000","fromAddress":wallet,"toAddress":wallet,"fromToken":{"address":from},"toToken":{"address":model::USDG}},"estimate":{"toAmount":"636098"}});
        assert!(echoes(&quote, wallet, from, model::USDG, "1000"));
        let mut wrong = quote;
        wrong["action"]["toChainId"] = json!(1);
        assert!(!echoes(&wrong, wallet, from, model::USDG, "1000"));
    }
}
