use aomi_sdk::{DynAomiApp, DynAomiTool, DynToolCallCtx};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use hoodit::app::{HooditApp, ProviderOrigins, Runtime};
use hoodit::tools::{
    CandlesArgs, DiscoverArgs, DiscoverPools, GetCandles, GetHolding, GetMarketOptions,
    GetPortfolio, GetToken, GetTokenPools, GetTrades, HoldingArgs, MarketOptionsArgs,
    PortfolioArgs, SearchArgs, SearchTokens, TokenArgs, TokenPoolsArgs, TradesArgs,
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
    let discover: DiscoverArgs = serde_json::from_value(json!({"feed":null,"duration":null,"page":null,"min_liquidity_usd":null,"min_volume_24h_usd":null,"filters":null,"sort":null,"direction":null,"limit":null,"max_pages":null,"deduplicate_tokens":null,"cursor":null,"refresh":null})).unwrap();
    assert!(
        discover.feed.is_none()
            && discover.duration.is_none()
            && discover.page.is_none()
            && discover.min_liquidity_usd.is_none()
            && discover.min_volume_24h_usd.is_none()
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
    let portfolio: PortfolioArgs = serde_json::from_value(
        json!({"wallet_address":wallet,"cursor":null,"include_quotes":null,"valuation":null,"security":null,"sort":null,"min_value_usd":null,"include_unpriced":null,"refresh":null}),
    )
    .unwrap();
    assert!(
        portfolio.cursor.is_none()
            && portfolio.include_quotes.is_none()
            && portfolio.refresh.is_none()
    );
    let holding: HoldingArgs = serde_json::from_value(json!({"wallet_address":wallet,"token":"native","quote_balance_bps":null,"include_quote":null,"security":null,"refresh":null})).unwrap();
    assert!(
        holding.quote_balance_bps.is_none()
            && holding.include_quote.is_none()
            && holding.refresh.is_none()
    );
    for cursor in [json!("null"), json!(" NULL "), json!(""), json!("   ")] {
        let args: PortfolioArgs =
            serde_json::from_value(json!({"wallet_address":wallet,"cursor":cursor})).unwrap();
        assert!(args.cursor.is_none());
    }
    rejects::<SearchArgs>(json!({"query":null}));
    rejects::<PortfolioArgs>(json!({"wallet_address":null}));
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
    assert_eq!(tools.len(), 9);
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
    let portfolio = &tools["hoodit_get_portfolio"].parameters_schema;
    assert!(portfolio["properties"].get("page").is_none());
    assert!(portfolio["properties"].get("page_size").is_none());
    assert_eq!(portfolio["properties"]["include_quotes"]["default"], false);
    let cursor_schema = &portfolio["properties"]["cursor"];
    assert!(
        allows_null(cursor_schema),
        "strict tool schemas require a nullable first-page cursor: {cursor_schema:#}"
    );
    let cursor_types = cursor_schema["type"]
        .as_array()
        .expect("cursor must use a provider-compatible string/null type union");
    assert!(cursor_types.iter().any(|kind| kind == "string"));
    assert!(cursor_types.iter().any(|kind| kind == "null"));
    assert_eq!(cursor_schema["minLength"], 1);
    assert_eq!(cursor_schema["maxLength"], 4096);
    assert_eq!(cursor_schema["pattern"], "^[A-Za-z0-9_-]+$");
    assert!(
        cursor_schema["description"]
            .as_str()
            .unwrap()
            .contains("first page"),
        "{cursor_schema:#}"
    );
    assert!(cursor_schema.get("default").is_none(), "{cursor_schema:#}");
    let holding = &tools["hoodit_get_holding"].parameters_schema;
    assert_eq!(holding["properties"]["quote_balance_bps"]["minimum"], 1);
    assert_eq!(
        holding["properties"]["quote_balance_bps"]["maximum"],
        10_000
    );
    assert_eq!(holding["properties"]["quote_balance_bps"]["default"], 100);
    assert_eq!(holding["properties"]["include_quote"]["default"], false);
    let trades = &tools["hoodit_get_trades"].parameters_schema;
    assert_eq!(trades["properties"]["side"]["default"], "both");
}

fn mock_body(path: &str) -> Value {
    let token = "0x1111111111111111111111111111111111111111";
    let other = "0x2222222222222222222222222222222222222222";
    let pool = "0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let pool_resource = json!({"type":"pool","id":format!("robinhood_{pool}"),"attributes":{"address":pool,"name":"EXAMPLE / USDG","base_token_price_usd":"0.123456789012345678901234567890123456","quote_token_price_usd":"1","reserve_in_usd":"100000","pool_created_at":"2026-09-17T00:00:00Z","price_change_percentage":{"m5":"1","h1":"2","h6":"3","h24":"4"},"volume_usd":{"m5":"5","h1":"6","h6":"7","h24":"8"},"transactions":{"m5":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h1":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h6":{"buys":1,"sells":2,"buyers":1,"sellers":2},"h24":{"buys":1,"sells":2,"buyers":1,"sellers":2}}},"relationships":{"base_token":{"data":{"type":"token","id":format!("robinhood_{token}")}},"quote_token":{"data":{"type":"token","id":format!("robinhood_{other}")}},"dex":{"data":{"type":"dex","id":"example-dex"}}}});
    let included = json!([
        {"type":"token","id":format!("robinhood_{token}"),"attributes":{"address":token,"symbol":"EX","name":"Example","decimals":18}},
        {"type":"token","id":format!("robinhood_{other}"),"attributes":{"address":other,"symbol":"USDG","name":"USDG","decimals":6}},
        {"type":"dex","id":"example-dex","attributes":{"name":"Example DEX"}}
    ]);
    if path.contains("/api/v2/addresses/") && path.contains("/tokens") {
        if path.contains("id=7") {
            return json!({"items":[{"token":{"address_hash":token,"symbol":"EX","name":"Example","decimals":"18","icon_url":null},"value":"1000000000000000000","token_id":null}],"next_page_params":null});
        }
        return json!({"items":[{"token":{"address_hash":token,"symbol":"EX","name":"Example","decimals":"18","icon_url":null},"value":"1000000000000000000","token_id":null}],"next_page_params":null});
    }
    if path.contains("action=tokenbalance") {
        return json!({"status":"1","message":"OK","result":"1000000000000000000"});
    }
    if path.contains("action=balance") {
        return json!({"status":"1","message":"OK","result":"1000000000000000000"});
    }
    if path.contains("/api/v2/tokens/") {
        if path
            .to_ascii_lowercase()
            .contains("0x5fc5360d0400a0fd4f2af552add042d716f1d168")
        {
            return json!({"address_hash":"0x5fc5360d0400a0fd4f2af552add042d716f1d168","symbol":"USDG","name":"Global Dollar","decimals":"6"});
        }
        if path.contains("0x4444444444444444444444444444444444444444") {
            return json!({"address_hash":"0x4444444444444444444444444444444444444444","symbol":"UNKNOWN","name":"Unknown Decimals","decimals":null});
        }
        return json!({"address_hash":token,"symbol":"EX","name":"Example","decimals":"18"});
    }
    if path.contains("/ohlcv/") {
        return serde_json::from_str(r#"{"data":{"attributes":{"ohlcv_list":[[1700000000,0.123456789012345678901234567890123456,0.2,0.1,0.15,123.456789012345678901234567890123456]]}}}"#).unwrap();
    }
    if path.contains("/networks/robinhood/dexes") {
        return json!({"data":[{"type":"dex","id":"example-dex","attributes":{"name":"Example DEX"}}]});
    }
    if path.contains("/trades") {
        return json!({"data":[{"type":"trade","id":"trade-1","attributes":{"tx_hash":format!("0x{}", "a".repeat(64)),"block_timestamp":Utc::now().to_rfc3339(),"kind":"buy","from_token_address":other,"to_token_address":token,"from_token_amount":"1","to_token_amount":"2","price_to_in_usd":"0.5","volume_in_usd":"1"}}]});
    }
    if path.starts_with("/quote?") {
        let params = url::Url::parse(&format!("http://fixture{path}"))
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect::<HashMap<_, _>>();
        let from = params["fromToken"].clone();
        let amount = params["fromAmount"].clone();
        let wallet = params["fromAddress"].clone();
        return json!({
            "tool":"fixture-route",
            "action":{
                "fromChainId":4663,"toChainId":4663,"fromAmount":amount,
                "fromAddress":wallet,"toAddress":wallet,
                "fromToken":{"chainId":4663,"address":from,"decimals":18},
                "toToken":{"chainId":4663,"address":"0x5fc5360d0400a0fd4f2af552add042d716f1d168","decimals":6}
            },
            "estimate":{"fromAmount":amount,"toAmount":"1000000","toAmountMin":"995000","gasCosts":[{"amountUSD":"0.001"}]},
            "transactionRequest":{"data":"excluded-by-adapter"}
        });
    }
    if path.contains("/tokens/") && path.ends_with("/info") {
        return json!({"data":{"type":"token_info","id":format!("robinhood_{token}"),"attributes":{"websites":[],"description":"Synthetic"}}});
    }
    if path.contains("/networks/robinhood/tokens/") && !path.contains("/pools") {
        return json!({"data":{"type":"token","id":format!("robinhood_{token}"),"attributes":{"address":token,"symbol":"EX","name":"Example","decimals":18,"price_usd":"0.123456789012345678901234567890123456","market_cap_usd":null,"fdv_usd":"1000","volume_usd":{"h24":"8"}}}});
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
            blockscout: base.clone(),
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
        secrets: HashMap::from([("BLOCKSCOUT_API_KEY".into(), "fixture-key".into())]),
    }
}

fn ctx_without_secrets(name: &str) -> DynToolCallCtx {
    let mut context = ctx(name);
    context.secrets.clear();
    context
}

#[test]
fn emits_one_success_envelope_for_every_tool() {
    let app = mock_app();
    let token = "0x1111111111111111111111111111111111111111";
    let wallet = "0x3333333333333333333333333333333333333333";
    let cursor = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "v":1,"chain":4663,"wallet":wallet,"hop":1,
            "next":{"id":7,"value":"1000000000000000000","fiat_value":null,"items_count":50}
        }))
        .unwrap(),
    );
    let mut cases = vec![
        json!({"tool":"hoodit_search_tokens","input":{"query":"EX"},"output":SearchTokens::run(&app, serde_json::from_value(json!({"query":"EX"})).unwrap(), ctx("hoodit_search_tokens")).unwrap()}),
        json!({"tool":"hoodit_discover_pools","input":{},"output":DiscoverPools::run(&app, serde_json::from_value(json!({})).unwrap(), ctx("hoodit_discover_pools")).unwrap()}),
        json!({"tool":"hoodit_get_token","input":{"token":token,"include_metadata":true},"output":GetToken::run(&app, serde_json::from_value(json!({"token":token,"include_metadata":true})).unwrap(), ctx("hoodit_get_token")).unwrap()}),
        json!({"tool":"hoodit_get_token_pools","input":{"token":token},"output":GetTokenPools::run(&app, serde_json::from_value::<TokenPoolsArgs>(json!({"token":token})).unwrap(), ctx("hoodit_get_token_pools")).unwrap()}),
        json!({"tool":"hoodit_get_market_options","input":{},"output":GetMarketOptions::run(&app, serde_json::from_value::<MarketOptionsArgs>(json!({})).unwrap(), ctx("hoodit_get_market_options")).unwrap()}),
        json!({"tool":"hoodit_get_candles","input":{"token":token,"before":1700000100},"output":GetCandles::run(&app, serde_json::from_value(json!({"token":token,"before":1700000100})).unwrap(), ctx("hoodit_get_candles")).unwrap()}),
        json!({"tool":"hoodit_get_trades","input":{"token":token},"output":GetTrades::run(&app, serde_json::from_value(json!({"token":token})).unwrap(), ctx("hoodit_get_trades")).unwrap()}),
        json!({"tool":"hoodit_get_portfolio","input":{"wallet_address":wallet},"output":GetPortfolio::run(&app, serde_json::from_value(json!({"wallet_address":wallet})).unwrap(), ctx("hoodit_get_portfolio")).unwrap()}),
        json!({"tool":"hoodit_get_holding","input":{"wallet_address":wallet,"token":token,"include_quote":false},"output":GetHolding::run(&app, serde_json::from_value(json!({"wallet_address":wallet,"token":token,"include_quote":false})).unwrap(), ctx("hoodit_get_holding")).unwrap()}),
    ];
    for case in &cases {
        assert_eq!(case["output"]["status"], "ok", "{}", case["tool"]);
    }
    assert_eq!(
        cases[0]["output"]["data"]["pagination"]["next_page"],
        Value::Null
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
        cases[7]["output"]["data"]["pagination"],
        json!({"provider_rows_returned":1,"displayed":2,"next_cursor":null})
    );
    assert_eq!(cases[8]["output"]["data"]["requested_balance_bps"], 100);
    assert_eq!(
        cases[8]["output"]["data"]["sell_amount"]["atomic"],
        "10000000000000000"
    );
    let missing_provider = json!({
        "tool":"hoodit_get_portfolio",
        "input":{"wallet_address":wallet},
        "output":GetPortfolio::run(
            &app,
            serde_json::from_value(json!({"wallet_address":wallet})).unwrap(),
            ctx_without_secrets("hoodit_get_portfolio"),
        ).unwrap()
    });
    assert_eq!(missing_provider["output"]["status"], "error");
    assert_eq!(
        missing_provider["output"]["error"]["code"],
        "PROVIDER_NOT_CONFIGURED"
    );
    assert!(
        missing_provider["output"]["error"]["message"]
            .as_str()
            .unwrap()
            .contains("operator-managed provider configuration is missing")
    );
    cases.push(missing_provider);
    let quoted_app = mock_app();
    let quoted = json!({
        "tool":"hoodit_get_holding",
        "input":{"wallet_address":wallet,"token":token,"quote_balance_bps":5000,"include_quote":true},
        "output":GetHolding::run(
            &quoted_app,
            serde_json::from_value(json!({"wallet_address":wallet,"token":token,"quote_balance_bps":5000,"include_quote":true})).unwrap(),
            ctx("hoodit_get_holding"),
        ).unwrap()
    });
    assert_eq!(
        quoted["output"]["data"]["holding"]["valuation"]["status"], "quoted",
        "{quoted:#}"
    );
    assert_eq!(
        quoted["output"]["data"]["holding"]["valuation"]["quote"]["input_amount"]["atomic"],
        "500000000000000000"
    );
    cases.push(quoted);
    let continuation_app = mock_app();
    let continuation = json!({
        "tool":"hoodit_get_portfolio",
        "input":{"wallet_address":wallet,"cursor":cursor},
        "output":GetPortfolio::run(
            &continuation_app,
            serde_json::from_value(json!({"wallet_address":wallet,"cursor":cursor})).unwrap(),
            ctx("hoodit_get_portfolio"),
        ).unwrap()
    });
    assert_eq!(continuation["output"]["status"], "partial");
    assert_eq!(continuation["output"]["data"]["native_included"], false);
    assert_eq!(continuation["output"]["data"]["summary"]["scope"], "page");
    cases.push(continuation);
    let null_string_cursor = json!({
        "tool":"hoodit_get_portfolio",
        "input":{"wallet_address":wallet,"cursor":"null"},
        "output":GetPortfolio::run(
            &app,
            serde_json::from_value(json!({"wallet_address":wallet,"cursor":"null"})).unwrap(),
            ctx("hoodit_get_portfolio"),
        ).unwrap()
    });
    assert_eq!(null_string_cursor["output"]["status"], "ok");
    assert_eq!(
        null_string_cursor["output"]["data"]["native_included"],
        true
    );
    cases.push(null_string_cursor);
    let wrong_wallet_cursor = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&json!({
            "v":1,"chain":4663,
            "wallet":"0x5555555555555555555555555555555555555555",
            "hop":1,
            "next":{"id":7,"value":"1000000000000000000","fiat_value":null,"items_count":50}
        }))
        .unwrap(),
    );
    let wrong_wallet = json!({
        "tool":"hoodit_get_portfolio",
        "input":{"wallet_address":wallet,"cursor":wrong_wallet_cursor},
        "output":GetPortfolio::run(
            &app,
            serde_json::from_value(json!({"wallet_address":wallet,"cursor":wrong_wallet_cursor})).unwrap(),
            ctx("hoodit_get_portfolio"),
        ).unwrap()
    });
    assert_eq!(wrong_wallet["output"]["status"], "error");
    assert_eq!(wrong_wallet["output"]["error"]["code"], "INVALID_ARGUMENT");
    cases.push(wrong_wallet);
    let native_app = mock_app();
    let native = json!({
        "tool":"hoodit_get_holding",
        "input":{"wallet_address":wallet,"token":"native","include_quote":false},
        "output":GetHolding::run(
            &native_app,
            serde_json::from_value(json!({"wallet_address":wallet,"token":"native","include_quote":false})).unwrap(),
            ctx("hoodit_get_holding"),
        ).unwrap()
    });
    assert_eq!(
        native["output"]["data"]["holding"]["token"]["kind"],
        "native"
    );
    cases.push(native);
    let unknown_decimals_token = "0x4444444444444444444444444444444444444444";
    let unknown_decimals_app = mock_app();
    let unknown_decimals = json!({
        "tool":"hoodit_get_holding",
        "input":{"wallet_address":wallet,"token":unknown_decimals_token,"include_quote":false},
        "output":GetHolding::run(
            &unknown_decimals_app,
            serde_json::from_value(json!({"wallet_address":wallet,"token":unknown_decimals_token,"include_quote":false})).unwrap(),
            ctx("hoodit_get_holding"),
        ).unwrap()
    });
    assert_eq!(unknown_decimals["output"]["status"], "partial");
    assert_eq!(
        unknown_decimals["output"]["data"]["holding"]["valuation"]["reason"],
        "unknown_decimals"
    );
    cases.push(unknown_decimals);
    let invalid_argument = json!({
        "tool":"hoodit_get_holding",
        "output":GetHolding::run(
            &app,
            serde_json::from_value(json!({"wallet_address":"not-an-address","token":token})).unwrap(),
            ctx("hoodit_get_holding"),
        ).unwrap()
    });
    assert_eq!(invalid_argument["output"]["status"], "error");
    assert_eq!(
        invalid_argument["output"]["error"]["code"],
        "INVALID_ARGUMENT"
    );
    cases.push(invalid_argument);
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
