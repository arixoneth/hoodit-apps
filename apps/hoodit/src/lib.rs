use aomi_sdk::*;
mod amount;
pub mod app;
mod model;
mod providers;
pub mod tools;

const PREAMBLE: &str = include_str!("preamble.md");
dyn_aomi_app!(
    app = app::HooditApp, name = "hoodit", version = "1.3.0", preamble = PREAMBLE,
    tools = [], secrets = [], namespaces = ["aomi-core", "evm-core"],
    skills = [
        { id: "hoodit/markets", description: "Research Robinhood Chain token identity, security and ownership evidence, pool discovery and comparison, prices, liquidity, candles, and public trades", tags: ["markets", "tokens", "security", "pools", "research", "discovery"], tools: [tools::SearchTokens, tools::DiscoverPools, tools::GetToken, tools::GetTokenPools, tools::GetMarketOptions, tools::GetCandles, tools::GetTrades], sections: { instructions: "skills/markets.md" }, },
        { id: "hoodit/coin-scanner", description: "Find a play, a coin to ape or watch, audit a bag, judge a comeback, or give an opinion on a ticker or contract. Investigate Robinhood Chain candidates using charts, contract and exit risk, holders, liquidity, and activity; activate hoodit/coin-scanner and hoodit/markets together in the same call", tags: ["coin scanner", "coin audit", "token audit", "pick", "suggestions", "chart patterns", "honeypot", "holders", "risk"], sections: { instructions: "skills/coin-scanner.md" }, },
        { id: "hoodit/portfolio", description: "Inspect exact Robinhood Chain wallet holdings, valuations, exposures, token risks, and fractional sell sizing without executing a trade", tags: ["wallet", "portfolio", "balances", "valuation", "exposure", "risk"], tools: [tools::GetPortfolio, tools::GetHolding], sections: { instructions: "skills/portfolio.md" }, },
    ],
);

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    #[test]
    fn manifest_has_only_skill_owned_v1_tools() {
        let manifest = app::HooditApp::default().manifest();
        assert_eq!(manifest.version, "1.3.0");
        assert_eq!(manifest.skills.len(), 3);
        assert_eq!(manifest.tools.len(), 9);
        let names = manifest
            .tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<HashSet<_>>();
        assert_eq!(names.len(), 9);
        assert!(
            !names
                .iter()
                .any(|n| n.contains("stock") || n.contains("probe"))
        );
        assert!(
            manifest
                .tools
                .iter()
                .all(|t| t.parameters_schema["additionalProperties"] == false)
        );
        assert!(manifest.secrets.as_ref().is_none_or(Vec::is_empty));
        for forbidden in [
            "hoodit_",
            "GeckoTerminal",
            "Blockscout",
            "LI.FI",
            "cursor",
            "basis points",
            "activate",
        ] {
            assert!(
                !manifest.preamble.contains(forbidden),
                "tool-specific guidance leaked into preamble: {forbidden}"
            );
        }
        assert!(manifest.preamble.contains("operator-managed"));
        assert!(manifest.preamble.contains("host authorization"));
        let scanner = manifest
            .skills
            .iter()
            .find(|skill| skill.id == "hoodit/coin-scanner")
            .expect("coin scanner skill is registered");
        assert!(scanner.injected_tools.is_empty());
        assert!(
            scanner
                .sections
                .iter()
                .any(|section| section.content.contains("hoodit_get_candles"))
        );
    }
}
