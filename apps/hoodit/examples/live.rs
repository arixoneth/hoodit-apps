//! Runs a plan of real tool calls against live providers and prints each
//! reply with its size: `cargo run --example live -- plan.json`.
//! Reads CODEX_MPP_KEY and LIFI_API_KEY from the environment. Costs money.
use aomi_sdk::testing::{TestCtxBuilder, run_tool};
use hoodit::app::HooditApp;
use hoodit::tools::*;
use serde_json::Value;

const MAX_REPLY: usize = 2500;

fn main() {
    let path = std::env::args().nth(1).expect("plan.json");
    let plan: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let app = HooditApp::default();
    let mut oversized = vec![];
    for (i, step) in plan.iter().enumerate() {
        let tool = step["tool"].as_str().unwrap();
        let args = step["args"].clone();
        let mut ctx = TestCtxBuilder::new(tool)
            .call_id(format!("live-{i}"))
            .attribute(
                "hosted",
                serde_json::json!({ "turn_id": format!("live-turn-{i}") }),
            );
        if let Some(wallet) = step.get("wallet").and_then(Value::as_str) {
            ctx = ctx.attribute(
                "domain",
                serde_json::json!({ "evm": { "address": wallet } }),
            );
        }
        for name in ["CODEX_MPP_KEY", "LIFI_API_KEY"] {
            if let Ok(v) = std::env::var(name) {
                ctx = ctx.secret(name, v);
            }
        }
        let ctx = ctx.build();
        let started = std::time::Instant::now();
        let out = match tool {
            "hoodit_scan" => run_tool::<Scan>(&app, args, ctx),
            "hoodit_find" => run_tool::<Find>(&app, args, ctx),
            "hoodit_token" => run_tool::<Token>(&app, args, ctx),
            "hoodit_chart" => run_tool::<Chart>(&app, args, ctx),
            "hoodit_trades" => run_tool::<Trades>(&app, args, ctx),
            "hoodit_holders" => run_tool::<Holders>(&app, args, ctx),
            "hoodit_wallet" => run_tool::<Wallet>(&app, args, ctx),
            "hoodit_exit" => run_tool::<Exit>(&app, args, ctx),
            "hoodit_check" => run_tool::<Check>(&app, args, ctx),
            other => Err(format!("unknown tool {other}")),
        };
        let text = match out {
            Ok(r) => serde_json::to_string(&r.value).unwrap(),
            Err(e) => format!("ERR {e}"),
        };
        let size = text.len();
        println!(
            "=== {tool} {} ({size} chars, {:.1}s)\n{text}\n",
            step["args"],
            started.elapsed().as_secs_f64()
        );
        if size > MAX_REPLY {
            oversized.push(format!("{tool} {}: {size} chars", step["args"]));
        }
    }
    if !oversized.is_empty() {
        eprintln!("replies over {MAX_REPLY} chars:\n{}", oversized.join("\n"));
        std::process::exit(1);
    }
}
