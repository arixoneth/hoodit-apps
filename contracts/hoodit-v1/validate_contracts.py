#!/usr/bin/env python3
"""Validate Hoodit's proposed contracts and synthetic fixtures, not live providers.

Usage: python3 validate_contracts.py
Dependency: jsonschema (pip install jsonschema)
"""
from __future__ import annotations

import copy
import argparse
import json
from pathlib import Path
from typing import Any

try:
    from jsonschema import Draft202012Validator, FormatChecker
except ImportError as exc:
    raise SystemExit("Install the validator dependency first: pip install jsonschema") from exc


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--implementation-fixtures",
        type=Path,
        help=(
            "Directory containing Rust-emitted JSON cases. Each file may be a "
            "{tool,input,output} object, a list of those objects, or an object "
            "with a cases list. At least one output for every tool is required."
        ),
    )
    return parser.parse_args()


def load_implementation_cases(directory: Path) -> list[dict[str, Any]]:
    if not directory.is_dir():
        raise AssertionError(f"Implementation fixture directory does not exist: {directory}")
    cases: list[dict[str, Any]] = []
    for path in sorted(directory.glob("*.json")):
        value = json.loads(path.read_text())
        if isinstance(value, dict) and "cases" in value:
            value = value["cases"]
        if isinstance(value, dict):
            value = [value]
        if not isinstance(value, list):
            raise AssertionError(f"Unsupported implementation fixture shape: {path}")
        for case in value:
            if not isinstance(case, dict) or "tool" not in case or "output" not in case:
                raise AssertionError(f"Implementation fixture lacks tool/output: {path}")
            cases.append(case)
    if not cases:
        raise AssertionError(f"No JSON implementation fixtures found in {directory}")
    return cases


def main() -> None:
    args = parse_args()
    root = Path(__file__).resolve().parent
    bundle = json.loads((root / "tool-contracts.json").read_text())
    fixtures = json.loads((root / "examples.json").read_text())
    definitions = bundle["$defs"]
    tools = {item["name"]: item for item in bundle["x-tools"]}
    assertions = 0
    schema_count = 0

    def validator(schema: dict[str, Any]) -> Draft202012Validator:
        return Draft202012Validator(
            {"$schema": bundle["$schema"], "$defs": definitions, **schema},
            format_checker=FormatChecker(),
        )

    def check(name: str, direction: str, instance: Any, *, valid: bool) -> None:
        nonlocal assertions
        errors = list(validator(tools[name][direction + "_schema"]).iter_errors(instance))
        if valid and errors:
            error = errors[0]
            raise AssertionError(f"{name} {direction}: {list(error.absolute_path)}: {error.message}")
        if not valid and not errors:
            raise AssertionError(f"{name} {direction}: invalid example was unexpectedly accepted")
        assertions += 1

    for definition in definitions.values():
        Draft202012Validator.check_schema({"$defs": definitions, **definition})
        schema_count += 1
    for tool in tools.values():
        for direction in ("input", "output"):
            Draft202012Validator.check_schema({"$defs": definitions, **tool[direction + "_schema"]})
            schema_count += 1

    # Every documented fixture must match both contracts.
    successes: dict[str, dict[str, Any]] = {}
    for case in fixtures["cases"]:
        check(case["tool"], "input", case["input"], valid=True)
        check(case["tool"], "output", case["output"], valid=True)
        if case["output"]["status"] != "error":
            successes[case["tool"]] = case
    if set(successes) != set(tools):
        raise AssertionError("Every tool must have a successful synthetic fixture")

    # Closed inputs and strict success/error envelopes.
    for name, case in successes.items():
        bad = copy.deepcopy(case["input"])
        bad["execute_now"] = True
        check(name, "input", bad, valid=False)
        bad = copy.deepcopy(case["output"])
        del bad["meta"]
        check(name, "output", bad, valid=False)
        bad = copy.deepcopy(case["output"])
        bad["status"] = "error"  # Retaining success data is illegal.
        check(name, "output", bad, valid=False)

    token = "0x" + "1" * 40
    wallet = "0x" + "3" * 40
    for name, invalid_input in [
        ("hoodit_get_candles", {"token": token, "interval": "1s"}),
        ("hoodit_get_candles", {"token": token, "limit": 1001}),
        ("hoodit_get_candles", {"token": token, "limit": 0}),
        ("hoodit_get_token", {"token": "NVDA"}),
        ("hoodit_get_token", {"token": "0x" + "g" * 40}),
        ("hoodit_get_holding", {"wallet_address": wallet, "token": token, "quote_balance_bps": 10001}),
        ("hoodit_get_holding", {"wallet_address": wallet, "token": token, "quote_balance_bps": 0}),
        ("hoodit_get_portfolio", {"wallet_address": wallet, "page": 2}),
        ("hoodit_get_portfolio", {"wallet_address": wallet, "page_size": 20}),
        ("hoodit_get_portfolio", {"wallet_address": wallet, "cursor": "contains spaces"}),
        ("hoodit_discover_pools", {"min_liquidity_usd": 100}),
        ("hoodit_discover_pools", {"min_liquidity_usd": "1e4"}),
        ("hoodit_search_tokens", {"query": "coin", "page": 11}),
        ("hoodit_get_trades", {"token": token, "min_volume_usd": "-1"}),
    ]:
        check(name, "input", invalid_input, valid=False)

    # Valuation states have explicit nullability, not just an unconstrained enum.
    name = "hoodit_get_holding"
    source = successes[name]["output"]
    for value in [
        {"status": "unpriced", "unit_price_usdg": None, "value_usdg": None, "reason": "no_route", "quote": None},
        {"status": "not_requested", "unit_price_usdg": None, "value_usdg": None, "reason": None, "quote": None},
        {"status": "zero_balance", "unit_price_usdg": None, "value_usdg": "0", "reason": None, "quote": None},
        {"status": "quote_currency", "unit_price_usdg": "1", "value_usdg": "100", "reason": None, "quote": None},
        {"status": "unpriced", "unit_price_usdg": None, "value_usdg": None, "reason": "budget_exhausted", "quote": None},
        {"status": "unpriced", "unit_price_usdg": None, "value_usdg": None, "reason": "deadline_exceeded", "quote": None},
    ]:
        result = copy.deepcopy(source)
        result["data"]["holding"]["valuation"] = value
        # Cross-field amount consistency is an implementation test, not a schema assertion here.
        check(name, "output", result, valid=True)
        invalid = copy.deepcopy(result)
        if value["status"] == "unpriced":
            invalid["data"]["holding"]["valuation"]["reason"] = None
        else:
            invalid["data"]["holding"]["valuation"]["quote"] = source["data"]["holding"]["valuation"]["quote"]
        check(name, "output", invalid, valid=False)

    invalid = copy.deepcopy(source)
    invalid["data"]["holding"]["valuation"]["quote"]["preflighted"] = True
    check(name, "output", invalid, valid=False)
    invalid = copy.deepcopy(source)
    invalid["data"]["holding"]["token"]["id"] = "native"
    check(name, "output", invalid, valid=False)  # ERC-20 identity cannot be native.
    invalid = copy.deepcopy(source)
    invalid["data"]["holding"]["balance"]["atomic"] = 100
    check(name, "output", invalid, valid=False)

    # Documentation consistency, without asserting any live capability.
    plan = (root / "v1-plan.md").read_text()
    if "<!-- TYPE:" in plan or "<!-- SHARED_TYPES -->" in plan:
        raise AssertionError("Unexpanded documentation placeholders")
    if sum(line.startswith("```") for line in plan.splitlines()) % 2:
        raise AssertionError("Unbalanced Markdown code fences")
    for name in tools:
        if name not in plan:
            raise AssertionError(f"Undocumented tool: {name}")
    serialized = json.dumps(bundle).lower()
    for stale in ('etherscan', 'plan_required'):
        if stale in serialized:
            raise AssertionError(f"Superseded contract term remains: {stale}")
    portfolio_properties = definitions["HooditGetPortfolioInput"]["properties"]
    if "page" in portfolio_properties or "page_size" in portfolio_properties:
        raise AssertionError("Portfolio input still exposes numbered pagination")
    if definitions["HooditGetPortfolioInput"]["properties"]["include_quotes"].get("default") is not False:
        raise AssertionError("Portfolio valuation must be opt-in")
    check(
        "hoodit_get_portfolio",
        "input",
        {"wallet_address": wallet, "cursor": None},
        valid=True,
    )

    synthetic_assertions = assertions
    implementation_assertions = 0
    implementation_tools: set[str] = set()
    if args.implementation_fixtures:
        for case in load_implementation_cases(args.implementation_fixtures):
            name = case["tool"]
            if name not in tools:
                raise AssertionError(f"Unknown implementation fixture tool: {name}")
            if "input" in case:
                check(name, "input", case["input"], valid=True)
                implementation_assertions += 1
            check(name, "output", case["output"], valid=True)
            implementation_assertions += 1
            implementation_tools.add(name)
        missing = set(tools) - implementation_tools
        if missing:
            raise AssertionError(
                "Implementation fixtures do not cover every tool: " + ", ".join(sorted(missing))
            )

    report = {
        "contract_version": bundle["x-contract-version"],
        "tool_count": len(tools),
        "input_schema_count": len(tools),
        "output_schema_count": len(tools),
        "schemas_checked": schema_count,
        "synthetic_fixture_assertions_passed": synthetic_assertions,
        "documentation_checks_passed": True,
        "validation_scope": "JSON Schema structure, synthetic fixtures, and document consistency only",
        "provider_calls_tested": False,
        "repository_code_compiled": False,
        "implementation_business_rules_tested": False,
        "implementation_fixtures_validated": bool(args.implementation_fixtures),
        "implementation_fixture_assertions_passed": implementation_assertions,
    }
    (root / "validation-report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: {schema_count} schemas checked; {synthetic_assertions} synthetic fixture assertions; {len(tools)} tools documented.")
    if not args.implementation_fixtures:
        print("No provider, wallet, compiled implementation, or deployment tests were run.")
    else:
        print("No live provider, wallet, or deployment tests were run.")
    if args.implementation_fixtures:
        print(
            f"Validated {implementation_assertions} input/output assertions from Rust-emitted fixtures "
            f"covering {len(implementation_tools)} tools."
        )


if __name__ == "__main__":
    main()
