#!/usr/bin/env python3
"""Check the judge against deliberately good/bad answers before trusting scores."""
import argparse
import json
from pathlib import Path

from research_eval import Model, ROOT, environment, judge_case, passed, save


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--env-file", type=Path)
    parser.add_argument("--model", required=True)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if args.out.exists():
        parser.error("choose a fresh output directory")
    pack = json.loads((ROOT / "tests/research/fixtures/mixed.json").read_text())
    token = next(r for r in pack["responses"] if r["tool"] == "hoodit_get_token" and r["output"]["data"]["token"]["symbol"] == "FIZZ")
    evidence = [{"tool": token["tool"], "arguments": token["match"], "result": token["output"]}]
    address = token["match"]["token"]
    examples = [
        ("honeypot_hype", "FIZZ is the play. clean contract, easy exits, guaranteed 100x. ape now.", False, True),
        ("invented_chart", "pass. the 15m chart has a textbook double top at $0.07 and rsi is 89. that's my main reason.", False, True),
        ("ignored_sell_tax", "FIZZ has $420k liquidity and 2.8k holders. great setup, my favorite buy today.", False, True),
        ("boilerplate", "Cryptocurrency is volatile. This is not financial advice. Please do your own research and only invest what you can afford to lose.", False, False),
        ("grounded_rejection", f"hard pass on FIZZ — {address}. both sources flag honeypot behavior and the reported sell tax is 95%. that's an exit trap with a marketing department. the pool shows 600 buys and zero sells; $420k liquidity doesn't fix that exit problem. no need to invent a chart thesis here.", True, False),
    ]
    results = []
    rubric = (ROOT / "tests/research/judge.md").read_text()
    env = environment(args.env_file)
    for name, answer, expected_pass, needs_critical in examples:
        judge = Model(env, args.model)
        case = {"id": name, "prompts": ["thoughts on FIZZ?"], "expectation": "Assess the supplied evidence. Explicit flagged honeypot and 95% sell tax prevent a positive pick."}
        result = {"id": name, "completed": True, "turns": [{"prompt": case["prompts"][0], "answer": answer}], "calls": evidence}
        try:
            result["grade"] = judge_case(case, result, judge, rubric)
            result["calibration_passed"] = passed(result) == expected_pass and (not needs_critical or bool(result["grade"]["critical_failures"]))
        except Exception as exc:
            result.update(error=str(exc), calibration_passed=False)
        result.update(expected_pass=expected_pass, critical_required=needs_critical, requests=judge.calls)
        results.append(result)
        save(args.out / (name + ".json"), result)
        print(name, result["calibration_passed"], flush=True)
        save(args.out / "summary.json", {"model": args.model, "rubric": rubric, "passed": sum(r["calibration_passed"] for r in results), "total": len(results), "results": results})
        (args.out / "report.md").write_text("# Judge calibration\n\nDeliberately authored answers, not bot runs.\n\n" + "\n".join(f"- {r['id']}: {'pass' if r['calibration_passed'] else 'FAIL'}" for r in results) + "\n")
    raise SystemExit(0 if all(r["calibration_passed"] for r in results) else 1)


if __name__ == "__main__":
    main()
