use aomi_sdk::*;
mod amount;
pub mod app;
pub mod market;
mod model;
mod providers;
pub mod tools;

const PREAMBLE: &str = include_str!("preamble.md");
dyn_aomi_app!(
    app = app::HooditApp, name = "hoodit", version = "1.5.0", preamble = PREAMBLE,
    tools = [], secrets = [], namespaces = ["aomi-core", "evm-core"],
    skills = [
        { id: "hoodit/research", description: "Find, check, and judge Robinhood Chain coins: what to ape or watch, fresh Pons launches and curves near graduation, opinions on a ticker or contract, real charts and order flow from on-chain swaps, who is buying or dumping, contract and holder risk, and whether a size can actually be exited", tags: ["markets", "coins", "scanner", "launchpad", "pons", "chart", "flow", "security", "exit"], tools: [tools::Discover, tools::Search, tools::GetToken, tools::GetChart, tools::CheckExit], sections: { instructions: "skills/research.md" }, },
    ],
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_one_skill_owning_five_read_tools() {
        let manifest = app::HooditApp::default().manifest();
        assert_eq!(manifest.version, "1.5.0");
        assert_eq!(manifest.skills.len(), 1);
        let names: Vec<&str> = manifest.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "hoodit_discover",
                "hoodit_search",
                "hoodit_get_token",
                "hoodit_get_chart",
                "hoodit_check_exit"
            ]
        );
        assert!(
            manifest
                .tools
                .iter()
                .all(|t| t.parameters_schema["additionalProperties"] == false)
        );
        assert!(manifest.secrets.as_ref().is_none_or(Vec::is_empty));
        for leaked in ["hoodit_", "GeckoTerminal", "DexScreener", "LI.FI"] {
            assert!(
                !manifest.preamble.contains(leaked),
                "tool detail leaked into preamble: {leaked}"
            );
        }
        let skill = &manifest.skills[0];
        assert!(
            skill
                .sections
                .iter()
                .any(|s| s.content.contains("hoodit_get_chart"))
        );
    }
}
