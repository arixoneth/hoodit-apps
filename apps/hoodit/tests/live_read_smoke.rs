//! Live read-only probe against public providers. Ignored by default:
//!
//! HOODIT_TOOL=hoodit_get_chart HOODIT_ARGS='{"token":"0x..."}' \
//!   cargo test -p hoodit --test live_read_smoke -- --ignored --nocapture
//!
//! HOODIT_PLAN='[["hoodit_discover",{}],["hoodit_get_chart",{"token":"0x..."}]]'
//! runs several calls in one process, sharing caches like a chat session.

use aomi_sdk::{DynAomiTool, DynToolCallCtx};
use hoodit::{app::HooditApp, tools::*};
use serde_json::{Value, json};

fn ctx(name: &str) -> DynToolCallCtx {
    DynToolCallCtx {
        session_id: "hoodit-live-probe".into(),
        tool_name: name.into(),
        call_id: format!("{name}-1"),
        state_attributes: Default::default(),
        secrets: Default::default(),
    }
}

fn run<T: DynAomiTool<App = HooditApp>>(app: &HooditApp, args: Value) -> Value {
    T::run(
        app,
        serde_json::from_value(args).expect("valid args"),
        ctx(T::NAME),
    )
    .expect("tool ran")
}

fn call(app: &HooditApp, tool: &str, args: Value) -> Value {
    let started = std::time::Instant::now();
    let out = match tool {
        "hoodit_search" => run::<Search>(app, args),
        "hoodit_discover" => run::<Discover>(app, args),
        "hoodit_get_token" => run::<GetToken>(app, args),
        "hoodit_get_chart" => run::<GetChart>(app, args),
        "hoodit_check_exit" => run::<CheckExit>(app, args),
        other => panic!("unknown tool {other}"),
    };
    let text = serde_json::to_string(&out).unwrap();
    eprintln!(
        "{tool}: {} chars in {:?} status={}",
        text.len(),
        started.elapsed(),
        out["status"]
    );
    out
}

#[test]
#[ignore = "calls live public providers"]
fn live_probe() {
    let app = HooditApp::default();
    let plan: Vec<(String, Value)> = match std::env::var("HOODIT_PLAN") {
        Ok(plan) => {
            serde_json::from_str(&plan).expect("HOODIT_PLAN is a JSON list of [tool, args]")
        }
        Err(_) => vec![(
            std::env::var("HOODIT_TOOL").unwrap_or_else(|_| "hoodit_discover".into()),
            std::env::var("HOODIT_ARGS")
                .ok()
                .map(|a| serde_json::from_str(&a).expect("HOODIT_ARGS is JSON"))
                .unwrap_or_else(|| json!({})),
        )],
    };
    let outputs: Vec<Value> = plan
        .into_iter()
        .map(|(tool, args)| call(&app, &tool, args))
        .collect();
    println!("{}", serde_json::to_string_pretty(&outputs).unwrap());
    assert!(outputs.iter().all(|out| out["status"] != "error"));
}
