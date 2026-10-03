use super::{codex_id, exec};
use crate::app::{HooditApp, Ttl};
use crate::market::pons_curve;
use crate::providers::{self, codex};
use crate::shape::{self, field, ok};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckArgs {
    /// Exact 0x token contract.
    pub token: String,
}

pub struct Check;

impl DynAomiTool for Check {
    type App = HooditApp;
    type Args = CheckArgs;
    const NAME: &'static str = "hoodit_check";
    const DESCRIPTION: &'static str = "Flat numbers for one token, built for watchers (wake_on_condition polls one field with an operator and a value): price_usd, fdv_usd, liquidity_usd, holders, top10_pct, dev_pct, snipers_pct (percent 0–100), buy_usd_5m, sell_usd_5m, change_1h_pct, curve_pct (Pons curve only: funds raised ÷ graduation target, 0–100), graduated (1 = graduated, 0 = still on the curve; launchpad tokens only). as_of is the current unix time. Costs one data request per poll.";

    fn run(app: &HooditApp, args: CheckArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let query = format!(
                "{{ s: filterTokens(tokens: [\"{}\"], limit: 1) {{ results {{ priceUSD marketCap liquidity holders top10HoldersPercent devHeldPercentage sniperHeldPercentage buyVolume5m sellVolume5m change1 token {{ symbol launchpad {{ launchpadName completed migrated }} }} }} }} }}",
                codex_id(&token)
            );
            let (data, note) = match codex(&rt, &call, &query, json!({}), Ttl::Live).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let Some(r) = data.pointer("/s/results/0") else {
                return shape::error(
                    "NOT_FOUND",
                    "no Robinhood Chain market indexed for this exact contract",
                    None,
                );
            };
            let f = |k: &str| field(r, &[k]);
            let pad = r
                .pointer("/token/launchpad/launchpadName")
                .and_then(Value::as_str);
            let flag = |k: &str| {
                r.pointer(&format!("/token/launchpad/{k}"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            };
            let mut graduated = pad.map(|_| flag("completed") || flag("migrated"));
            let mut curve_pct = Value::Null;
            if pad == Some("pons") {
                match providers::pons_launch(&rt, &token).await {
                    Some(launch) => {
                        graduated = Some(!launch.on_curve());
                        if launch.on_curve() {
                            curve_pct = pons_curve(&rt, &launch).await["pct"].clone();
                        }
                    }
                    None => call.gap("curve progress unavailable"),
                }
            }
            ok(
                json!({
                    "symbol": shape::label(r.pointer("/token/symbol"), 20),
                    "token": token,
                    "price_usd": shape::price(f("priceUSD")),
                    "fdv_usd": shape::usd(f("marketCap")),
                    "liquidity_usd": shape::usd(f("liquidity")),
                    "holders": shape::int(f("holders")),
                    "top10_pct": shape::pct(f("top10HoldersPercent")),
                    "dev_pct": shape::pct(f("devHeldPercentage")),
                    "snipers_pct": shape::pct(f("sniperHeldPercentage")),
                    "buy_usd_5m": shape::usd(f("buyVolume5m").or(Some(0.0))),
                    "sell_usd_5m": shape::usd(f("sellVolume5m").or(Some(0.0))),
                    "change_1h_pct": shape::change_pct(f("change1")),
                    "curve_pct": curve_pct,
                    "graduated": graduated.map(u8::from),
                }),
                &call.gaps,
            )
        })
    }
}
