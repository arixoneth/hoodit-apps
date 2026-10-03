use super::exec;
use crate::app::{HooditApp, Ttl};
use crate::providers::{self, codex};
use crate::shape::{self, field, int, min_ago, num, ok, pct, put_span, usd};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WalletArgs {
    /// 0x wallet to look at. null = the user's connected wallet.
    pub wallet: Option<String>,
}

pub struct Wallet;

const WINDOW: &str = "statsUsd { realizedProfitUsd volumeUsd realizedProfitPercentage } statsNonCurrency { swaps uniqueTokens wins losses avgHoldPeriodSec }";

fn window(w: &Value) -> Value {
    let wins = field(w, &["statsNonCurrency", "wins"]);
    let losses = field(w, &["statsNonCurrency", "losses"]);
    json!({
        "realized_profit_usd": usd(field(w, &["statsUsd", "realizedProfitUsd"])),
        "realized_profit_pct": pct(field(w, &["statsUsd", "realizedProfitPercentage"])),
        "volume_usd": usd(field(w, &["statsUsd", "volumeUsd"])),
        "swaps": int(field(w, &["statsNonCurrency", "swaps"])),
        "tokens": int(field(w, &["statsNonCurrency", "uniqueTokens"])),
        "win_rate_pct": match (wins, losses) { (Some(a), Some(b)) if a + b > 0.0 => pct(Some(a / (a + b) * 100.0)), _ => Value::Null },
        "avg_hold_min": int(field(w, &["statsNonCurrency", "avgHoldPeriodSec"]).map(|s| s / 60.0)),
    })
}

impl DynAomiTool for Wallet {
    type App = HooditApp;
    type Args = WalletArgs;
    const NAME: &'static str = "hoodit_wallet";
    const DESCRIPTION: &'static str = "A Robinhood Chain wallet's record and bag: realized profit, win rate, swaps and average hold time for 7 and 30 days, bot and scammer scores, labels, and current token balances with USD value and how long each has been held. Defaults to the user's connected wallet. Realized profit only counts tokens with a known purchase cost; average hold, not median.";

    fn run(app: &HooditApp, args: WalletArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let wallet = match args.wallet.as_deref().map(shape::address).transpose() {
                Ok(Some(w)) => w,
                Ok(None) => match call.wallet.clone() {
                    Some(w) => w,
                    None => {
                        return shape::error(
                            "NO_WALLET",
                            "no wallet given and none connected; ask for an address or to connect",
                            None,
                        );
                    }
                },
                Err(message) => return shape::error("INVALID_ARGUMENT", &message, None),
            };
            let q = format!(
                "{{ s: detailedWalletStats(input: {{walletAddress: \"{wallet}\", networkId: {n}}}) {{ labels botScore scammerScore lastTransactionAt statsWeek1 {{ {WINDOW} }} statsDay30 {{ {WINDOW} }} }} b: balances(input: {{walletAddress: \"{wallet}\", networks: [{n}], removeScams: true, limit: 12}}) {{ items {{ tokenAddress shiftedBalance balanceUsd firstHeldTimestamp token {{ symbol }} }} }} }}",
                n = providers::NETWORK
            );
            let (data, note) = match codex(&rt, &call, &q, json!({}), Ttl::Minute).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let s = &data["s"];
            let mut bag: Vec<(f64, Value)> = data
                .pointer("/b/items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|b| {
                    let value = b.get("balanceUsd").and_then(num).unwrap_or(0.0);
                    let mut item = json!({
                        "symbol": shape::label(b.pointer("/token/symbol"), 20),
                        "token": b.get("tokenAddress"),
                        "usd": usd(Some(value)),
                    });
                    put_span(&mut item, "held", b.get("firstHeldTimestamp").and_then(num));
                    (value, item)
                })
                .filter(|(v, _)| *v >= 1.0)
                .collect();
            bag.sort_by(|a, b| b.0.total_cmp(&a.0));
            ok(
                json!({
                    "wallet": wallet,
                    "is_user": call.wallet.as_deref() == Some(wallet.as_str()),
                    "labels": s.get("labels"),
                    "bot_score": s.get("botScore"),
                    "scammer_score": s.get("scammerScore"),
                    "last_trade_min_ago": min_ago(s.get("lastTransactionAt").and_then(num)),
                    "week": window(&s["statsWeek1"]),
                    "month": window(&s["statsDay30"]),
                    "bag": bag.into_iter().take(10).map(|b| b.1).collect::<Vec<_>>(),
                }),
                &call.gaps,
            )
        })
    }
}
