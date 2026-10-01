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
    for name, invalid_input in [
        ("hoodit_get_candles", {"token": token, "interval": "1s"}),
        ("hoodit_get_candles", {"token": token, "limit": 1001}),
        ("hoodit_get_candles", {"token": token, "limit": 0}),
        ("hoodit_get_token", {"token": "NVDA"}),
        ("hoodit_get_token", {"token": "0x" + "g" * 40}),
        ("hoodit_check_exit", {"token": token, "amount": "1", "fraction_bps": 10001}),
        ("hoodit_check_exit", {"token": token, "amount": "1", "fraction_bps": 0}),
        ("hoodit_check_exit", {"token": token, "amount": "1e5"}),
        ("hoodit_check_exit", {"token": token, "mode": "buy"}),
        ("hoodit_check_exit", {"token": token, "wallet_address": "me"}),
        ("hoodit_discover_pools", {"min_liquidity_usd": 100}),
        ("hoodit_discover_pools", {"min_liquidity_usd": "1e4"}),
        ("hoodit_search_tokens", {"query": "coin", "page": 11}),
        ("hoodit_get_trades", {"token": token, "min_volume_usd": "-1"}),
    ]:
        check(name, "input", invalid_input, valid=False)

    # Lifecycle states keep curve and graduation evidence apart.
    name = "hoodit_get_token"
    source = successes[name]["output"]
    for lifecycle, valid in [
        ({"state": "graduated", "graduation_pct": "100", "graduated_at": "2026-09-07T16:17:38Z", "destination_pool_id": "0x" + "b" * 64, "source": "geckoterminal"}, True),
        ({"state": "not_reported", "graduation_pct": None, "graduated_at": None, "destination_pool_id": None, "source": "geckoterminal"}, True),
        ({"state": "bonding_curve", "graduation_pct": "50", "graduated_at": None, "destination_pool_id": "0x" + "b" * 64, "source": "geckoterminal"}, False),
        ({"state": "not_reported", "graduation_pct": "0", "graduated_at": "2026-09-07T16:17:38Z", "destination_pool_id": None, "source": "geckoterminal"}, False),
    ]:
        result = copy.deepcopy(source)
        result["data"]["lifecycle"] = lifecycle
        check(name, "output", result, valid=valid)

    # Exit checks are never executable and a missing route carries no amounts.
    name = "hoodit_check_exit"
    source = next(
        case["output"]
        for case in fixtures["cases"]
        if case["tool"] == name and case["output"]["status"] == "ok" and case["output"]["data"]["mode"] == "sell"
    )
    invalid = copy.deepcopy(source)
    invalid["data"]["coverage"]["executable"] = True
    check(name, "output", invalid, valid=False)
    invalid = copy.deepcopy(source)
    invalid["data"]["sell"]["route_found"] = False
    check(name, "output", invalid, valid=False)
    invalid = copy.deepcopy(source)
    invalid["data"]["sell"]["transactionRequest"] = {"data": "0x"}
    check(name, "output", invalid, valid=False)
    invalid = copy.deepcopy(source)
    invalid["data"]["buy"] = copy.deepcopy(source["data"]["sell"])
    check(name, "output", invalid, valid=False)  # A sell-mode check has no buy leg.

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
    for stale in ('etherscan', 'plan_required', 'blockscout'):
        if stale in serialized:
            raise AssertionError(f"Superseded contract term remains: {stale}")
    for removed in ("HooditGetPortfolioInput", "HooditGetHoldingInput"):
        if removed in definitions:
            raise AssertionError(f"Removed wallet tool contract remains: {removed}")

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
