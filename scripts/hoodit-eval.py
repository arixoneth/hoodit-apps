#!/usr/bin/env python3
"""Run Hoodit's conversation evals against a deployed app on Aomi chat.

Each case is a fresh guest conversation. Mechanical checks (completion,
required tools, tool errors, no signing actions) are scored here; transcripts
are saved so answers can be graded against the evidence the tools returned.

  scripts/hoodit-eval.py --model gpt-6-luna --split dev --out /tmp/hoodit-eval
"""
from __future__ import annotations

import argparse
import importlib.util
import json
from pathlib import Path
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("smoke", ROOT / "scripts" / "hoodit-staging-smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


def tool_calls(events: list[dict], seen: set) -> list[dict]:
    """New tool calls in order, one entry per call, with its parsed result.
    `seen` spans the conversation: later turns replay earlier messages."""
    calls = []
    for event in events:
        for message in event.get("messages", []):
            name = message.get("toolName")
            if not name or message.get("toolResult") is None or message.get("id") in seen:
                continue
            seen.add(message.get("id"))
            result = message.get("toolResult")
            for value in smoke.decoded_values(result):
                if isinstance(value, dict) and "status" in value:
                    result = value
                    break
            calls.append({"tool": name, "args": message.get("toolArguments"), "result": result})
    return calls


def answer(events: list[dict], seen: set) -> str:
    final = ""
    for event in events:
        for message in event.get("messages", []):
            if message.get("role") == "agent" and not message.get("toolName") and str(message.get("content", "")).strip():
                if message.get("id") in seen:
                    continue
                final = message["content"]
    for event in events:
        for message in event.get("messages", []):
            if message.get("role") == "agent" and not message.get("toolName"):
                seen.add(message.get("id"))
    return final


def check(case: dict, turns: list[dict]) -> list[str]:
    failures = []
    calls = [call for turn in turns for call in turn["calls"]]
    names = [call["tool"] for call in calls]
    rules = case.get("checks", {})
    for tool in rules.get("must_call", []):
        if tool not in names:
            failures.append(f"never called {tool}")
    if rules.get("must_call_any") and not any(any(t in n for n in names) for t in rules["must_call_any"]):
        failures.append(f"called none of {rules['must_call_any']}")
    for tool in rules.get("must_not_call", []):
        if tool in names:
            failures.append(f"called {tool}")
    errors = [c for c in calls if isinstance(c["result"], dict) and c["result"].get("status") == "error"]
    if errors:
        failures.append("tool errors: " + ", ".join(f"{c['tool']}={c['result'].get('error', {}).get('code')}" for c in errors))
    if any(not turn["answer"] for turn in turns):
        failures.append("empty answer")
    return failures


def run_case(args, case: dict) -> dict:
    base = args.base.rstrip("/")
    guest = smoke.request_json(f"{base}/api/auth/widget/guest", origin=args.origin, method="POST", body={})
    token = guest["access_token"]
    session_id, turns, seen = None, [], set()
    for prompt in case["prompts"]:
        payload = {"applicationId": args.application_id, "message": prompt, "model": args.model}
        if session_id:
            payload["sessionId"] = session_id
        started = time.monotonic()
        try:
            for attempt in range(3):
                try:
                    delta = smoke.normalize_delta(smoke.request_json(f"{base}/v1/agent/chat", token=token, origin=args.origin, method="POST", body=payload))
                    break
                except RuntimeError as error:
                    # 409 while the platform is busy with the app (e.g. a deploy in flight).
                    if "HTTP 409" not in str(error) or attempt == 2:
                        raise
                    time.sleep(30)
            session_id = delta.get("sessionId") or session_id
            _, events = smoke.settle(base, args.origin, token, delta, args.timeout, [token])
            turns.append({"prompt": prompt, "seconds": round(time.monotonic() - started), "calls": tool_calls(events, seen), "answer": answer(events, seen)})
        except Exception as error:  # a failed turn is a result, not a crash
            turns.append({"prompt": prompt, "seconds": round(time.monotonic() - started), "calls": [], "answer": "", "error": str(error)[:500]})
            break
    result = {"id": case["id"], "split": case["split"], "model": args.model, "turns": turns}
    result["failures"] = check(case, turns) + [t["error"] for t in turns if t.get("error")]
    return result


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="https://chat-staging.aomi.dev")
    parser.add_argument("--origin", default="https://chat-staging.aomi.dev")
    parser.add_argument("--application-id", type=int, default=2937810)
    parser.add_argument("--model", default="gpt-6-luna")
    parser.add_argument("--split", choices=["dev", "holdout", "all"], default="dev")
    parser.add_argument("--case", action="append", help="run only these case ids")
    parser.add_argument("--cases", default=str(ROOT / "tests" / "evals" / "cases.json"))
    parser.add_argument("--out", required=True)
    parser.add_argument("--timeout", type=int, default=400)
    parser.add_argument("--pause", type=int, default=45, help="seconds between cases, to respect shared provider limits")
    args = parser.parse_args()
    cases = [
        case for case in json.loads(Path(args.cases).read_text())["cases"]
        if (args.split == "all" or case["split"] == args.split) and (not args.case or case["id"] in args.case)
    ]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    for index, case in enumerate(cases):
        if index:
            time.sleep(args.pause)
        result = run_case(args, case)
        (out / f"{case['id']}.json").write_text(json.dumps(result, indent=2))
        tools = " > ".join(c["tool"].replace("hoodit_", "") for t in result["turns"] for c in t["calls"])
        seconds = sum(t["seconds"] for t in result["turns"])
        print(f"{'PASS' if not result['failures'] else 'FAIL'} {case['id']} ({seconds}s) [{tools}] {'; '.join(result['failures'])}", flush=True)


if __name__ == "__main__":
    main()
