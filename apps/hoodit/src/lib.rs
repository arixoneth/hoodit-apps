use aomi_sdk::*;
mod amount;
pub mod app;
mod market;
mod providers;
mod shape;
pub mod tools;

const PREAMBLE: &str = include_str!("preamble.md");

const CODEX: Secret = Secret::new(
    app::CODEX_KEY,
    "Payment key for the operator's Codex market-data wallet (MPP on Tempo)",
    false,
);
const LIFI: Secret = Secret::new(
    app::LIFI_KEY,
    "LI.FI integrator API key for exit quotes",
    false,
);

dyn_aomi_app!(
    app = app::HooditApp,
    name = "hoodit",
    version = "2.0.0",
    preamble = PREAMBLE,
    tools = [
        tools::Scan,
        tools::Find,
        tools::Token,
        tools::Chart,
        tools::Trades,
        tools::Holders,
        tools::Wallet,
        tools::Exit,
        tools::Check
    ],
    secrets = [CODEX, LIFI],
    namespaces = ["aomi-core", "evm-core"],
    skills = [
        {
            id: "hoodit/research",
            description: "Scan Robinhood Chain memecoins and judge one: what's moving or launching, Pons curves near graduation, opinions on a ticker or contract, charts, who is buying or dumping, holders and the dev, bag checks, and whether a size can get out. Load for any coin question.",
            tags: ["memecoins", "robinhood", "scan", "pons", "chart", "holders", "exit"],
            sections: { instructions: "skills/research.md" },
        },
        {
            id: "hoodit/trade",
            description: "Buy or sell a Robinhood Chain memecoin the user explicitly asked to trade. Load together with the host lifi_swap skill.",
            tags: ["buy", "sell", "ape", "swap", "slippage"],
            sections: { instructions: "skills/trade.md" },
        },
        {
            id: "hoodit/watch",
            description: "Set, list or cancel alerts on a Robinhood Chain coin: tell me when the curve hits X%, the price crosses a level, the dev sells, liquidity drops.",
            tags: ["alert", "watch", "notify", "remind"],
            sections: { instructions: "skills/watch.md" },
        },
    ],
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_nine_unowned_tools_and_small_skills() {
        let manifest = app::HooditApp::default().manifest();
        assert_eq!(manifest.version, "2.0.0");
        let names: Vec<&str> = manifest.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "hoodit_scan",
                "hoodit_find",
                "hoodit_token",
                "hoodit_chart",
                "hoodit_trades",
                "hoodit_holders",
                "hoodit_wallet",
                "hoodit_exit",
                "hoodit_check"
            ]
        );
        assert!(
            manifest
                .tools
                .iter()
                .all(|t| t.parameters_schema["additionalProperties"] == false)
        );
        for skill in &manifest.skills {
            let chars: usize = skill.sections.iter().map(|s| s.content.len()).sum();
            assert!(chars < 7000, "{} is {chars} chars", skill.id);
        }
        assert!(manifest.preamble.len() < 3000);
    }

    /// The host makes every property required, so optional ones must accept null.
    #[test]
    fn optional_arguments_accept_null() {
        let manifest = app::HooditApp::default().manifest();
        for tool in &manifest.tools {
            let schema = &tool.parameters_schema;
            let required: Vec<&str> = schema["required"]
                .as_array()
                .map(|r| r.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();
            for (name, prop) in schema["properties"].as_object().unwrap() {
                if required.contains(&name.as_str()) {
                    continue;
                }
                let text = prop.to_string();
                assert!(
                    text.contains("\"null\""),
                    "{}.{name} is optional but not nullable: {text}",
                    tool.name
                );
            }
        }
    }
}
