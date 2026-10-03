use super::{codex_id, exec};
use crate::app::{HooditApp, Runtime, Ttl};
use crate::market::{Basics, RESULT_FIELDS, card, enrich_card};
use crate::providers::codex;
use crate::shape::{self, ok};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TokenArgs {
    /// Exact 0x token contract. Resolve tickers with hoodit_find first.
    pub token: String,
}

pub struct Token;

impl DynAomiTool for Token {
    type App = HooditApp;
    type Args = TokenArgs;
    const NAME: &'static str = "hoodit_token";
    const DESCRIPTION: &'static str = "The card for one Robinhood Chain token: launchpad and stage (for Pons curves, curve.pct = funds raised ÷ graduation target); pair token; trade_support (full, after_graduation, research_only: whether LI.FI can trade it now); age; price, FDV, market cap, liquidity, 24h volume; % change 1h/4h/24h; buy/sell dollars and unique traders for 1h and 24h; holders and the share held by the top 10 wallets (pool, curve and locker excluded), dev, snipers, bundlers, insiders (percent 0–100); ATH and distance from it; minutes since the last trade; the dev wallet; a contract security check for tokens not from a launchpad; explorer link. Observations, not executable quotes.";

    fn run(app: &HooditApp, args: TokenArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let id = codex_id(&token);
            let query = format!(
                "{{ s: filterTokens(tokens: [\"{id}\"], limit: 1) {{ results {{ {RESULT_FIELDS} }} }} h: holders(input: {{tokenId: \"{id}\", limit: 16}}) {{ items {{ address shiftedBalance }} }} }}"
            );
            let (data, note) = match codex(&rt, &call, &query, json!({}), Ttl::Live).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let Some(hit) = data.pointer("/s/results/0") else {
                return shape::error(
                    "NOT_FOUND",
                    "no Robinhood Chain market indexed for this exact contract; check the address and chain",
                    None,
                );
            };
            let holders = data
                .pointer("/h/items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut out = card(hit);
            enrich_card(&rt, &mut call, &mut out, hit, &holders).await;
            if Basics::of(hit).launchpad.is_none() {
                out["security"] = security(&rt, &token).await.unwrap_or_else(|| {
                    call.gap("contract security check unavailable");
                    Value::Null
                });
            } else {
                out["security"] = json!("launchpad template contract; deep check skipped");
            }
            ok(out, &call.gaps)
        })
    }
}

/// GoPlus contract check, only for tokens deployed outside a launchpad.
/// "" from GoPlus means unknown and stays null.
async fn security(rt: &Runtime, token: &str) -> Option<Value> {
    let key = format!("goplus:{token}");
    if let Some(hit) = rt.cached(&key) {
        return Some(hit);
    }
    let url =
        format!("https://api.gopluslabs.io/api/v1/token_security/4663?contract_addresses={token}");
    let response = rt
        .http
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    let body: Value = response.json().await.ok()?;
    let r = body.pointer(&format!("/result/{token}"))?;
    let flag = |k: &str| match r.get(k).and_then(Value::as_str) {
        Some("1") => json!(true),
        Some("0") => json!(false),
        _ => Value::Null,
    };
    let tax = |k: &str| shape::pct(r.get(k).and_then(shape::num).map(|v| v * 100.0));
    let out = json!({
        "honeypot": flag("is_honeypot"),
        "cannot_sell_all": flag("cannot_sell_all"),
        "buy_tax_pct": tax("buy_tax"),
        "sell_tax_pct": tax("sell_tax"),
        "tax_changeable": flag("slippage_modifiable"),
        "mintable": flag("is_mintable"),
        "owner_can_change_balance": flag("owner_change_balance"),
        "blacklist": flag("is_blacklisted"),
        "pausable": flag("transfer_pausable"),
        "proxy": flag("is_proxy"),
        "hidden_owner": flag("hidden_owner"),
        "source": "GoPlus",
    });
    rt.store(key, out.clone(), Ttl::Slow);
    Some(out)
}
