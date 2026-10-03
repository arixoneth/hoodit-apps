use super::{codex_id, exec};
use crate::app::{HooditApp, Ttl};
use crate::market::{HolderLabels, dev_wallet, top10_wallets_pct};
use crate::providers::{self, codex};
use crate::shape::{self, field, int, num, ok, pct, put_span, usd};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema, Clone, Copy, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum View {
    /// Top holders and the most profitable traders this week.
    Top,
    /// The dev: what they still hold, their other launches and how those went.
    Dev,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HoldersArgs {
    /// Exact 0x token contract.
    pub token: String,
    /// What to show. null = top.
    pub view: Option<View>,
}

pub struct Holders;

impl DynAomiTool for Holders {
    type App = HooditApp;
    type Args = HoldersArgs;
    const NAME: &'static str = "hoodit_holders";
    const DESCRIPTION: &'static str = "Who holds a Robinhood Chain token. view=top: holder count, the share of supply in the top 10 wallets (pool, curve, locker and burn contracts excluded), the largest holders with % of supply, USD value, how long held and a label for known contracts, and this week's top traders by realized profit. view=dev: the dev wallet and how it was identified, how much of this token it still holds, and its launches with whether each graduated, FDV now and at peak. Wallets are addresses, not people; size alone isn't skill.";

    fn run(app: &HooditApp, args: HoldersArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let token = super::arg!(shape::address(&args.token));
            let id = codex_id(&token);
            let n = providers::NETWORK;
            if args.view == Some(View::Dev) {
                let q = format!(
                    "{{ t: token(input: {{address: \"{token}\", networkId: {n}}}) {{ creatorAddress info {{ totalSupply }} }} }}"
                );
                let (launch, creator) = tokio::join!(
                    providers::pons_launch(&rt, &token),
                    codex(&rt, &call, &q, json!({}), Ttl::Slow)
                );
                let creator = match creator {
                    Ok((d, _)) => d,
                    Err(fail) => return fail.to_value(),
                };
                let sender = creator.pointer("/t/creatorAddress").and_then(Value::as_str);
                let supply = creator.pointer("/t/info/totalSupply").and_then(num);
                let Some((dev, source)) = dev_wallet(&rt, launch.as_ref(), sender).await else {
                    return shape::error("NOT_FOUND", "dev wallet unknown for this token", None);
                };
                let mut wallets = vec![dev.clone()];
                if let Some(s) = sender.map(str::to_ascii_lowercase).filter(|s| *s != dev) {
                    wallets.push(s);
                }
                let q = format!(
                    "{{ l: filterTokens(filters: {{network: [{n}], creatorAddresses: {}}}, rankings: [{{attribute: tokenCreatedAt, direction: DESC}}], limit: 10) {{ count results {{ marketCap volume24 athPrice token {{ address symbol createdAt info {{ totalSupply }} launchpad {{ completed migrated }} }} }} }} b: balances(input: {{walletAddress: \"{dev}\", networks: [{n}], tokens: [\"{id}\"]}}) {{ items {{ shiftedBalance balanceUsd }} }} }}",
                    json!(wallets)
                );
                let (data, note) = match codex(&rt, &call, &q, json!({}), Ttl::Minute).await {
                    Ok(found) => found,
                    Err(fail) => return fail.to_value(),
                };
                if let Some(note) = note {
                    call.gap(note);
                }
                let held = data
                    .pointer("/b/items/0/shiftedBalance")
                    .and_then(num)
                    .unwrap_or(0.0);
                let launches: Vec<Value> = data
                    .pointer("/l/results")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|r| {
                        let flag = |k: &str| {
                            r.pointer(&format!("/token/launchpad/{k}"))
                                .and_then(Value::as_bool)
                                .unwrap_or(false)
                        };
                        let address = r
                            .pointer("/token/address")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let peak = field(r, &["athPrice"])
                            .zip(field(r, &["token", "info", "totalSupply"]))
                            .map(|(p, s)| p * s);
                        let mut out = json!({
                            "symbol": shape::label(r.pointer("/token/symbol"), 20),
                            "token": address,
                            "this_token": (address.eq_ignore_ascii_case(&token)).then_some(true),
                            "graduated": flag("completed") || flag("migrated"),
                        });
                        put_span(&mut out, "age", r.pointer("/token/createdAt").and_then(num));
                        out["fdv_usd"] = usd(field(r, &["marketCap"]));
                        out["peak_fdv_usd"] = usd(peak);
                        out["volume_24h_usd"] = usd(field(r, &["volume24"]));
                        out
                    })
                    .collect();
                let graduated = launches.iter().filter(|l| l["graduated"] == true).count();
                call.gap("launches made from other wallets are not counted");
                return ok(
                    json!({
                        "token": token,
                        "dev": dev,
                        "dev_source": source,
                        "also_launched_by": wallets.get(1),
                        "dev_holds_pct": supply.filter(|s| *s > 0.0).map(|s| pct(Some(held / s * 100.0))),
                        "dev_holds_usd": usd(data.pointer("/b/items/0/balanceUsd").and_then(num).or(Some(0.0))),
                        "launches_total": data.pointer("/l/count"),
                        "graduated_of_listed": format!("{graduated} of {}", launches.len()),
                        "launches": launches,
                    }),
                    &call.gaps,
                );
            }
            let q = format!(
                "{{ h: holders(input: {{tokenId: \"{id}\", limit: 16}}) {{ count items {{ address shiftedBalance balanceUsd firstHeldTimestamp }} }} r: tokenTopTraders(input: {{tokenAddress: \"{token}\", networkId: {n}, tradingPeriod: WEEK, limit: 4}}) {{ items {{ walletAddress realizedProfitUsd amountBoughtUsd amountSoldUsd buys sells tokenBalance labels botScore }} }} p: filterTokens(tokens: [\"{id}\"], limit: 1) {{ results {{ top10HoldersPercent priceUSD pair {{ address }} token {{ symbol creatorAddress info {{ totalSupply }} }} }} }} }}"
            );
            let (launch, fetched) = tokio::join!(
                providers::pons_launch(&rt, &token),
                codex(&rt, &call, &q, json!({}), Ttl::Minute)
            );
            let (data, note) = match fetched {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let p = data.pointer("/p/results/0").cloned().unwrap_or(Value::Null);
            let supply = field(&p, &["token", "info", "totalSupply"]).filter(|s| *s > 0.0);
            let pool = p.pointer("/pair/address").and_then(Value::as_str);
            let sender = p.pointer("/token/creatorAddress").and_then(Value::as_str);
            let dev = dev_wallet(&rt, launch.as_ref(), sender).await.map(|d| d.0);
            let labels = HolderLabels::new(launch.as_ref(), pool, dev.as_deref());
            let items = data
                .pointer("/h/items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let shown: Vec<&Value> = items.iter().take(8).collect();
            let addresses: Vec<String> = shown
                .iter()
                .map(|h| {
                    h.get("address")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_ascii_lowercase()
                })
                .collect();
            let code = providers::has_code(&rt, &addresses).await;
            let holders: Vec<Value> = shown
                .iter()
                .zip(addresses.iter().zip(code))
                .map(|(h, (addr, contract))| {
                    let label = labels
                        .of(addr)
                        .or_else(|| (contract == Some(true)).then_some("contract or smart wallet"));
                    let mut out = json!({
                        "wallet": addr,
                        "label": label,
                        "pct": supply.map(|s| pct(h.get("shiftedBalance").and_then(num).map(|b| b / s * 100.0))),
                        "usd": usd(h.get("balanceUsd").and_then(num)),
                    });
                    put_span(&mut out, "held", h.get("firstHeldTimestamp").and_then(num));
                    out
                })
                .collect();
            let traders: Vec<Value> = data
                .pointer("/r/items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|t| {
                    json!({
                        "wallet": t.get("walletAddress"),
                        "realized_profit_usd": usd(field(t, &["realizedProfitUsd"])),
                        "bought_usd": usd(field(t, &["amountBoughtUsd"])),
                        "sold_usd": usd(field(t, &["amountSoldUsd"])),
                        "buys": int(field(t, &["buys"])), "sells": int(field(t, &["sells"])),
                        "holds_usd": usd(field(t, &["tokenBalance"]).zip(field(&p, &["priceUSD"])).map(|(b, px)| b * px)),
                        "labels": t.get("labels"),
                        "bot_score": t.get("botScore"),
                    })
                })
                .collect();
            ok(
                json!({
                    "token": token,
                    "symbol": shape::label(p.pointer("/token/symbol"), 20),
                    "holders": int(data.pointer("/h/count").and_then(num)),
                    "top10_wallets_pct": top10_wallets_pct(&items, &labels, supply),
                    "top_holders": holders,
                    "top_traders_7d": traders,
                }),
                &call.gaps,
            )
        })
    }
}
