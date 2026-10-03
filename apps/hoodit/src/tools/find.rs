use super::{codex_id, exec};
use crate::app::{HooditApp, Ttl};
use crate::market::{RESULT_FIELDS, row};
use crate::providers::{self, codex};
use crate::shape::{self, ok};
use aomi_sdk::schemars::JsonSchema;
use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FindArgs {
    /// Ticker ("PRIORS", "$PRIORS"), name, or an exact 0x contract.
    pub query: String,
}

pub struct Find;

/// The fields a candidate keeps from a scan row.
const CANDIDATE: &[&str] = &[
    "symbol",
    "token",
    "launchpad",
    "stage",
    "age_h",
    "age_min",
    "fdv_usd",
    "liquidity_usd",
    "volume_24h_usd",
    "holders",
];

impl DynAomiTool for Find {
    type App = HooditApp;
    type Args = FindArgs;
    const NAME: &'static str = "hoodit_find";
    const DESCRIPTION: &'static str = "Resolve a ticker, name or exact 0x contract to Robinhood Chain tokens. A 0x address returns exactly that token or NOT_FOUND, never a substitute. A ticker returns candidate contracts ranked by holder count, with launchpad, stage, age, FDV, liquidity, 24h volume and holders. Ranking is not proof of which one is official: the same ticker on different contracts means different tokens.";

    fn run(app: &HooditApp, args: FindArgs, ctx: DynToolCallCtx) -> Result<Value, String> {
        exec(app, &ctx, |rt, mut call| async move {
            let q = args.query.trim().trim_start_matches('$').to_string();
            if q.is_empty() || q.len() > 64 {
                return shape::error("INVALID_ARGUMENT", "query must be 1–64 characters", None);
            }
            if q.starts_with("0x") {
                let token = super::arg!(shape::address(&q));
                let query = format!(
                    "{{ s: filterTokens(tokens: [\"{}\"], limit: 1) {{ results {{ {RESULT_FIELDS} }} }} }}",
                    codex_id(&token)
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
                return ok(
                    json!({ "match": "exact_address", "token": pick(&row(hit)) }),
                    &call.gaps,
                );
            }
            let query = format!(
                "query($q: String, $f: TokenFilters) {{ s: filterTokens(phrase: $q, filters: $f, rankings: [{{attribute: holders, direction: DESC}}], limit: 8) {{ results {{ {RESULT_FIELDS} }} }} }}"
            );
            let vars = json!({ "q": q, "f": { "network": [providers::NETWORK] } });
            let (data, note) = match codex(&rt, &call, &query, vars, Ttl::Search).await {
                Ok(found) => found,
                Err(fail) => return fail.to_value(),
            };
            if let Some(note) = note {
                call.gap(note);
            }
            let upper = q.to_ascii_uppercase();
            let candidates: Vec<Value> = data
                .pointer("/s/results")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|r| {
                    let mut c = pick(&row(r));
                    let exact = c["symbol"]
                        .as_str()
                        .is_some_and(|s| s.to_ascii_uppercase() == upper);
                    c["match"] = json!(if exact { "exact_symbol" } else { "partial" });
                    c
                })
                .collect();
            if candidates.is_empty() {
                return shape::error(
                    "NOT_FOUND",
                    "no Robinhood Chain token matches that name",
                    None,
                );
            }
            let same_symbol = candidates
                .iter()
                .filter(|c| c["match"] == "exact_symbol")
                .count();
            ok(
                json!({
                    "query": q,
                    "same_symbol_contracts": same_symbol,
                    "ranked_by": "holders (not proof of the official contract)",
                    "candidates": candidates,
                }),
                &call.gaps,
            )
        })
    }
}

fn pick(row: &Value) -> Value {
    let mut out = json!({});
    for key in CANDIDATE {
        if let Some(v) = row.get(*key) {
            out[*key] = v.clone();
        }
    }
    out
}
