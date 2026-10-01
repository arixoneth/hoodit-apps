use aomi_sdk::{DynAomiApp, DynAomiTool, DynToolCallCtx};
use chrono::Utc;
use hoodit::app::{HooditApp, ProviderOrigins, Runtime};
use hoodit::tools::{
    CandlesArgs, CheckExit, DiscoverArgs, DiscoverPools, ExitArgs, GetCandles, GetMarketOptions,
    GetToken, GetTokenPools, GetTrades, MarketOptionsArgs, SearchArgs, SearchTokens, TokenArgs,
    TokenPoolsArgs, TradesArgs,
};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

fn rejects<T: DeserializeOwned>(value: Value) {
    assert!(serde_json::from_value::<T>(value).is_err());
}

fn allows_null(schema: &Value) -> bool {
    schema == "null"
        || schema.get("type").is_some_and(|value| {
            value == "null"
                || value
                    .as_array()
                    .is_some_and(|types| types.iter().any(|ty| ty == "null"))
        })
        || ["anyOf", "oneOf"]
            .iter()
            .filter_map(|key| schema.get(key).and_then(Value::as_array))
            .flatten()
            .any(allows_null)
}

fn assert_object_schemas_have_properties(path: &str, schema: &Value) {
    if schema.get("default").is_some_and(Value::is_null) {
        assert!(
            allows_null(schema),
            "{path} advertises default=null but rejects JSON null: {schema:#}"
        );
    }
    if schema.get("type") == Some(&Value::String("object".into())) {
        assert!(
            schema.get("properties").is_some_and(Value::is_object),
            "{path} has type=object without object-valued properties: {schema:#}"
        );
    }
    match schema {
        Value::Object(fields) => {
            for (name, child) in fields {
                assert_object_schemas_have_properties(&format!("{path}.{name}"), child);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                assert_object_schemas_have_properties(&format!("{path}[{index}]"), child);
            }
        }
        _ => {}
    }
}

#[test]
fn optional_nulls_are_omissions_but_unknown_and_required_nulls_are_rejected() {
    let token = "0x1111111111111111111111111111111111111111";
    let wallet = "0x3333333333333333333333333333333333333333";
    assert!(
        serde_json::from_value::<SearchArgs>(json!({"query":"PONS","page":null}))
            .unwrap()
            .page
            .is_none()
    );
    let discover: DiscoverArgs = serde_json::from_value(json!({"feed":null,"source_feed":null,"duration":null,"page":null,"min_liquidity_usd":null,"min_volume_24h_usd":null,"filters":null,"sort":null,"direction":null,"limit":null,"max_pages":null,"enrichment_limit":null,"deduplicate_tokens":null,"cursor":null,"refresh":null})).unwrap();
    assert!(
        discover.feed.is_none()
            && discover.duration.is_none()
            && discover.source_feed.is_none()
            && discover.page.is_none()
            && discover.min_liquidity_usd.is_none()
            && discover.min_volume_24h_usd.is_none()
            && discover.enrichment_limit.is_none()
    );
    let token_args: TokenArgs = serde_json::from_value(json!({"token":token,"pool_id":null,"include_metadata":null,"security":null,"include_holders":null,"refresh":null})).unwrap();
    assert!(token_args.pool_id.is_none() && token_args.include_metadata.is_none());
    let candles: CandlesArgs = serde_json::from_value(json!({"token":token,"pool_id":null,"interval":null,"before":null,"limit":null,"include_open":null})).unwrap();
    assert!(
        candles.pool_id.is_none()
            && candles.interval.is_none()
            && candles.before.is_none()
            && candles.limit.is_none()
            && candles.include_open.is_none()
    );
    let trades: TradesArgs = serde_json::from_value(
        json!({"token":token,"pool_id":null,"limit":null,"min_volume_usd":null,"side":null}),
    )
    .unwrap();
    assert!(trades.pool_id.is_none() && trades.limit.is_none() && trades.min_volume_usd.is_none());
    let exit: ExitArgs = serde_json::from_value(json!({"token":token,"mode":null,"amount":null,"fraction_bps":null,"eth_amount":null,"receive":null,"wallet_address":null})).unwrap();
    assert!(
        exit.mode.is_none()
            && exit.amount.is_none()
            && exit.fraction_bps.is_none()
            && exit.wallet_address.is_none()
    );
    rejects::<SearchArgs>(json!({"query":null}));
    rejects::<ExitArgs>(json!({"token":null}));
    rejects::<ExitArgs>(json!({"token":token,"wallet_address":wallet,"execute":true}));
    rejects::<SearchArgs>(json!({"query":"PONS","execute_now":true}));
}

#[test]
fn generated_manifest_exposes_strict_compatible_skill_inputs() {
    let manifest = HooditApp::default().manifest();
    let tools: HashMap<_, _> = manifest
        .tools
        .iter()
        .map(|tool| (tool.name.as_str(), tool))
        .collect();
    assert_eq!(tools.len(), 8);
    for (name, tool) in &tools {
        assert!(
            tool.description.len() >= 80,
            "{name} has an underspecified tool description"
        );
        assert_eq!(
            tool.parameters_schema["additionalProperties"], false,
            "{name}"
        );
        assert_object_schemas_have_properties(name, &tool.parameters_schema);
        for (property, schema) in tool.parameters_schema["properties"].as_object().unwrap() {
            assert!(
                schema["description"]
                    .as_str()
                    .is_some_and(|description| description.len() >= 20),
                "{name}.{property} needs a model-facing description: {schema:#}"
            );
        }
    }
    assert!(!tools.contains_key("hoodit_get_portfolio"));
    assert!(!tools.contains_key("hoodit_get_holding"));
    let exit = &tools["hoodit_check_exit"].parameters_schema;
    assert_eq!(exit["properties"]["fraction_bps"]["minimum"], 1);
    assert_eq!(exit["properties"]["fraction_bps"]["maximum"], 10_000);
    assert_eq!(exit["properties"]["fraction_bps"]["default"], 10_000);
    assert_eq!(exit["properties"]["mode"]["default"], "sell");
    assert!(allows_null(&exit["properties"]["wallet_address"]));
    let trades = &tools["hoodit_get_trades"].parameters_schema;
    assert_eq!(trades["properties"]["side"]["default"], "both");
}

fn mock_body(path: &str) -> Value {
    let token = "0x1111111111111111111111111111111111111111";
    let other = "0x2222222222222222222222222222222222222222";
    let pool = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let pool_resource = json!({"type":"pool","id":format!("robinhood_{pool}"),"attributes":{"address":pool,"name":"EXAMPLE / USDG","base_token_price_usd":"0.123456789012345678901234567890123456","quote_token_price_usd":"1","reserve_in_usd":"100000","fdv_usd":"1000","market_cap_usd":"750","pool_created_at":"2026-09-17T00:00:00Z","price_change_percentage":{"m5":"1","h1":"2","h6":"3","h24":"4"},"volume_usd":{"m5":"5","h1":"6","h6":"7","h24":"8"},"transactions":{"m5":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h1":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h6":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h24":{"buys":1,"sells":2,"buyers":1,"sellers":2}}},"relationships":{"base_token":{"data":{"type":"token","id":format!("robinhood_{token}")}},"quote_token":{"data":{"type":"token","id":format!("robinhood_{other}")}},"dex":{"data":{"type":"dex","id":"example-dex"}}}});
    let included = json!([
        {"type":"token","id":format!("robinhood_{token}"),"attributes":{"address":token,"symbol":"EX","name":"Example","decimals":18}},
        {"type":"token","id":format!("robinhood_{other}"),"attributes":{"address":other,"symbol":"USDG","name":"USDG","decimals":6}},
        {"type":"dex","id":"example-dex","attributes":{"name":"Example DEX"}}
    ]);
    if path.contains("/ohlcv/") {
        return serde_json::from_str(r#"{"data":{"attributes":{"ohlcv_list":[[1700000000,0.123456789012345678901234567890123456,0.2,0.1,0.15,123.456789012345678901234567890123456]]}}}"#).unwrap();
    }
    if path.contains("/networks/robinhood/dexes") {
        return json!({"data":[{"type":"dex","id":"example-dex","attributes":{"name":"Example DEX"}}]});
    }
    if path.contains("/trades") {
        return json!({"data":[{"type":"trade","id":"trade-1","attributes":{"tx_hash":format!("0x{}", "a".repeat(64)),"block_timestamp":Utc::now().to_rfc3339(),"block_number":123,"tx_from_address":"0x00000000000000000000000000000000000000aa","kind":"buy","from_token_address":other,"to_token_address":token,"from_token_amount":"1","to_token_amount":"2","price_to_in_usd":"0.5","volume_in_usd":"1"}}]});
    }
    if path.starts_with("/quote?") {
        let params = url::Url::parse(&format!("http://fixture{path}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect::<HashMap<_, _>>();
        let from = params["fromToken"].clone();
        let to = params["toToken"].clone();
        let amount = params["fromAmount"].clone();
        let wallet = params["fromAddress"].clone();
        return json!({
            "tool":"fixture-route",
            "action":{
                "fromChainId":4663,"toChainId":4663,"fromAmount":amount,
                "fromAddress":wallet,"toAddress":wallet,
                "fromToken":{"chainId":4663,"address":from,"decimals":18},
                "toToken":{"chainId":4663,"address":to,"decimals":18}
            },
            "estimate":{"fromAmount":amount,"toAmount":"1000000","toAmountMin":"995000","fromAmountUSD":"10","toAmountUSD":"9.5","gasCosts":[{"amountUSD":"0.001"}],"feeCosts":[{"amountUSD":"0.0025"}]},
            "transactionRequest":{"data":"excluded-by-adapter"}
        });
    }
    if path.contains("/tokens/") && path.ends_with("/info") {
        return json!({"data":{"type":"token_info","id":format!("robinhood_{token}"),"attributes":{"websites":[],"description":"Synthetic"}}});
    }
    let token_resource = json!({"type":"token","id":format!("robinhood_{token}"),"attributes":{"address":token,"symbol":"EX","name":"Example","decimals":18,"price_usd":"0.123456789012345678901234567890123456","market_cap_usd":null,"fdv_usd":"1000","volume_usd":{"h24":"8"},"launchpad_details":{"graduation_percentage":100.0,"completed":true,"completed_at":"2026-09-07T16:17:38.000Z","migrated_destination_pool_address":pool}}});
    if path.contains("/networks/robinhood/tokens/multi/") {
        return json!({"data":[token_resource]});
    }
    if path.contains("/networks/robinhood/tokens/") && !path.contains("/pools") {
        return json!({"data":token_resource});
    }
    if path.contains("/search/pools")
        || path.contains("/new_pools")
        || path.contains("/trending_pools")
        || path.contains("/pools")
    {
        return json!({"data":[pool_resource],"included":included});
    }
    json!({"error":"unmatched synthetic route"})
}

fn mock_app() -> HooditApp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0_u8; 8192];
            let size = stream.read(&mut request).unwrap();
            let first = String::from_utf8_lossy(&request[..size]);
            let path = first.split_whitespace().nth(1).unwrap_or("/");
            let body = serde_json::to_vec(&mock_body(path)).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    let runtime = Runtime::fixture(
        Client::new(),
        ProviderOrigins {
            gecko: base.clone(),
            goplus: base.clone(),
            coingecko: base.clone(),
            coingecko_pro: base.clone(),
            lifi: base,
        },
    );
    HooditApp::with_runtime(runtime)
}

fn ctx(name: &str) -> DynToolCallCtx {
    DynToolCallCtx {
        session_id: "contract-fixture".into(),
        tool_name: name.into(),
        call_id: format!("{name}-1"),
        state_attributes: Default::default(),
        secrets: HashMap::new(),
    }
}

fn pagination_mock_app() -> HooditApp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let pools = (1_u8..=20)
        .map(|index| {
            let pool = format!("0x{index:064x}");
            let token = format!("0x{index:040x}");
            json!({
                "type":"pool",
                "id":format!("robinhood_{pool}"),
                "attributes":{
                    "address":pool,"name":format!("TOKEN{index} / USDG"),
                    "base_token_price_usd":"1","quote_token_price_usd":"1",
                    "reserve_in_usd":"1000","fdv_usd":"2000","market_cap_usd":"1500",
                    "pool_created_at":"2026-09-17T00:00:00Z",
                    "volume_usd":{"h24":"100"},
                    "transactions":{"h24":{"buys":10,"sells":5,"buyers":8,"sellers":4}}
                },
                "relationships":{
                    "base_token":{"data":{"type":"token","id":format!("robinhood_{token}")}},
                    "quote_token":{"data":{"type":"token","id":"robinhood_0x9999999999999999999999999999999999999999"}},
                    "dex":{"data":{"type":"dex","id":"example-dex"}}
                }
            })
        })
        .collect::<Vec<_>>();
    let mut included = (1_u8..=20)
        .map(|index| {
            let token = format!("0x{index:040x}");
            json!({"type":"token","id":format!("robinhood_{token}"),"attributes":{"address":token,"symbol":format!("T{index}"),"name":format!("Token {index}"),"decimals":18}})
        })
        .collect::<Vec<_>>();
    included.push(json!({"type":"token","id":"robinhood_0x9999999999999999999999999999999999999999","attributes":{"address":"0x9999999999999999999999999999999999999999","symbol":"USDG","name":"USDG","decimals":6}}));
    included.push(json!({"type":"dex","id":"example-dex","attributes":{"name":"Example DEX"}}));
    let body = serde_json::to_vec(&json!({"data":pools,"included":included})).unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = [0_u8; 8192];
            let _ = stream.read(&mut request).unwrap();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(&body).unwrap();
        }
    });
    let runtime = Runtime::fixture(
        Client::new(),
        ProviderOrigins {
            gecko: base.clone(),
            goplus: base.clone(),
            coingecko: base.clone(),
            coingecko_pro: base.clone(),
            lifi: base,
        },
    );
    HooditApp::with_runtime(runtime)
}

#[test]
fn discovery_cursor_resumes_inside_a_page_for_non_screened_feeds() {
    let app = pagination_mock_app();
    let input = json!({"feed":"new","limit":3,"max_pages":2});
    let first = DiscoverPools::run(
        &app,
        serde_json::from_value(input.clone()).unwrap(),
        ctx("hoodit_discover_pools"),
    )
    .unwrap();
    let cursor = first["data"]["pagination"]["next_cursor"].as_str().unwrap();
    assert_eq!(first["data"]["pagination"]["next_page"], Value::Null);
    assert_eq!(first["data"]["coverage"]["stop_reason"], "result_limit");
    assert_eq!(first["data"]["coverage"]["scanned"], 3);

    let mut continuation_input = input;
    continuation_input["cursor"] = Value::String(cursor.into());
    let second = DiscoverPools::run(
        &app,
        serde_json::from_value(continuation_input).unwrap(),
        ctx("hoodit_discover_pools"),
    )
    .unwrap();
    assert_ne!(
        first["data"]["pools"][0]["pool_id"],
        second["data"]["pools"][0]["pool_id"]
    );
    assert_eq!(second["data"]["pools"].as_array().unwrap().len(), 3);
    assert_eq!(second["data"]["source_feed"], "new");
    assert_eq!(second["data"]["pagination"]["start_page"], 1);
    assert_eq!(second["data"]["pagination"]["start_offset"], 3);
    assert_eq!(second["data"]["coverage"]["scanned"], 3);
}

#[test]
fn emits_one_success_envelope_for_every_tool() {
    let app = mock_app();
    let token = "0x1111111111111111111111111111111111111111";
    let mut cases = vec![
        json!({"tool":"hoodit_search_tokens","input":{"query":"EX"},"output":SearchTokens::run(&app, serde_json::from_value(json!({"query":"EX"})).unwrap(), ctx("hoodit_search_tokens")).unwrap()}),
        json!({"tool":"hoodit_discover_pools","input":{},"output":DiscoverPools::run(&app, serde_json::from_value(json!({})).unwrap(), ctx("hoodit_discover_pools")).unwrap()}),
        json!({"tool":"hoodit_get_token","input":{"token":token,"include_metadata":true},"output":GetToken::run(&app, serde_json::from_value(json!({"token":token,"include_metadata":true})).unwrap(), ctx("hoodit_get_token")).unwrap()}),
        json!({"tool":"hoodit_get_token_pools","input":{"token":token},"output":GetTokenPools::run(&app, serde_json::from_value::<TokenPoolsArgs>(json!({"token":token})).unwrap(), ctx("hoodit_get_token_pools")).unwrap()}),
        json!({"tool":"hoodit_get_market_options","input":{},"output":GetMarketOptions::run(&app, serde_json::from_value::<MarketOptionsArgs>(json!({})).unwrap(), ctx("hoodit_get_market_options")).unwrap()}),
        json!({"tool":"hoodit_get_candles","input":{"token":token,"before":1700000100},"output":GetCandles::run(&app, serde_json::from_value(json!({"token":token,"before":1700000100})).unwrap(), ctx("hoodit_get_candles")).unwrap()}),
        json!({"tool":"hoodit_get_trades","input":{"token":token},"output":GetTrades::run(&app, serde_json::from_value(json!({"token":token})).unwrap(), ctx("hoodit_get_trades")).unwrap()}),
        json!({"tool":"hoodit_check_exit","input":{"token":token,"amount":"1000","fraction_bps":5000},"output":CheckExit::run(&app, serde_json::from_value(json!({"token":token,"amount":"1000","fraction_bps":5000})).unwrap(), ctx("hoodit_check_exit")).unwrap()}),
    ];
    for case in &cases {
        assert_eq!(case["output"]["status"], "ok", "{}", case["tool"]);
    }
    assert_eq!(
        cases[0]["output"]["data"]["pagination"]["next_page"],
        Value::Null
    );
    assert_eq!(cases[1]["output"]["data"]["source_feed"], "trending");
    assert_eq!(cases[1]["output"]["data"]["pools"][0]["fdv_usd"], "1000");
    assert_eq!(
        cases[1]["output"]["data"]["pools"][0]["market_cap_usd"],
        "750"
    );
    assert_eq!(
        cases[2]["output"]["data"]["metadata"]["description"],
        "Synthetic"
    );
    assert_eq!(
        cases[5]["output"]["data"]["candles"][0]["open"],
        "0.123456789012345678901234567890123456"
    );
    assert_eq!(cases[5]["output"]["data"]["coverage"]["returned"], 1);
    assert_eq!(cases[6]["output"]["data"]["coverage"]["returned"], 1);
    assert_eq!(
        cases[2]["output"]["data"]["lifecycle"]["state"],
        "graduated"
    );
    assert_eq!(
        cases[2]["output"]["data"]["lifecycle"]["destination_pool_id"],
        cases[2]["output"]["data"]["selected_pool"]["pool_id"]
    );
    assert_eq!(
        cases[1]["output"]["data"]["pools"][0]["lifecycle"]["state"],
        "graduated"
    );
    let trade = &cases[6]["output"]["data"]["trades"][0];
    assert_eq!(
        trade["sender"],
        "0x00000000000000000000000000000000000000aa"
    );
    assert_eq!(trade["block_number"], 123);
    assert_eq!(cases[6]["output"]["data"]["summary"]["distinct_senders"], 1);
    let sell = &cases[7]["output"]["data"]["sell"];
    assert_eq!(sell["amount_in"]["formatted"], "500");
    assert_eq!(sell["amount_in"]["atomic"], "500000000000000000000");
    assert_eq!(sell["loss_pct"], "5");
    assert_eq!(cases[7]["output"]["data"]["sender_kind"], "placeholder");
    assert_eq!(cases[7]["output"]["data"]["coverage"]["executable"], false);
    let round_trip_app = mock_app();
    let round_trip = json!({
        "tool":"hoodit_check_exit",
        "input":{"token":token,"mode":"round_trip","eth_amount":"0.05","wallet_address":"0x3333333333333333333333333333333333333333"},
        "output":CheckExit::run(
            &round_trip_app,
            serde_json::from_value(json!({"token":token,"mode":"round_trip","eth_amount":"0.05","wallet_address":"0x3333333333333333333333333333333333333333"})).unwrap(),
            ctx("hoodit_check_exit"),
        ).unwrap()
    });
    assert_eq!(round_trip["output"]["status"], "ok", "{round_trip:#}");
    assert_eq!(
        round_trip["output"]["data"]["buy"]["amount_in"]["atomic"],
        "50000000000000000"
    );
    assert_eq!(
        round_trip["output"]["data"]["sell"]["amount_in"]["atomic"],
        "1000000"
    );
    assert!(round_trip["output"]["data"]["round_trip_loss_pct"].is_string());
    cases.push(round_trip);
    let missing_amount = json!({
        "tool":"hoodit_check_exit",
        "input":{"token":token},
        "output":CheckExit::run(
            &app,
            serde_json::from_value(json!({"token":token})).unwrap(),
            ctx("hoodit_check_exit"),
        ).unwrap()
    });
    assert_eq!(missing_amount["output"]["status"], "error");
    assert_eq!(
        missing_amount["output"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    cases.push(missing_amount);
    let too_precise = json!({
        "tool":"hoodit_check_exit",
        "output":CheckExit::run(
            &app,
            serde_json::from_value(json!({"token":token,"mode":"round_trip","eth_amount":"0.0000000000000000001"})).unwrap(),
            ctx("hoodit_check_exit"),
        ).unwrap()
    });
    assert_eq!(too_precise["output"]["error"]["code"], "INVALID_ARGUMENT");
    cases.push(too_precise);
    if let Some(directory) = std::env::var_os("HOODIT_CONTRACT_FIXTURE_DIR") {
        let directory = PathBuf::from(directory);
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("rust-emitted.json"),
            serde_json::to_vec_pretty(&json!({"cases":cases})).unwrap(),
        )
        .unwrap();
    }
}
