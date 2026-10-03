#!/usr/bin/env python3
"""Run Hoodit's conversation evals against a deployed app on Aomi chat.

Each case is a fresh guest conversation, run `--runs` times. Mechanical
checks are scored here (final answer, address and link provenance, unit
traps, tool and time budgets, no trade preparation, reply sizes as the model sees them); every run
is saved with its tool calls so the answers can be graded against the
evidence with the rubric in tests/evals/cases.json. Exits 1 when any run
fails a check.

  scripts/hoodit-eval.py --model gpt-6-luna --out /tmp/hoodit-eval
  scripts/hoodit-eval.py --case priors_lookalike --runs 1 --out /tmp/e
"""
from __future__ import annotations

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import sys
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("smoke", ROOT / "scripts" / "hoodit-staging-smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)

MAX_REPLY = 3000  # chars as the model sees them (the host pretty-prints tool JSON)
TRADE_TOOLS = {"lifi_prepare_swap_batch", "send_transaction", "sign_typed_data", "prepare_transaction"}
ADDRESS = re.compile(r"0x[0-9a-fA-F]{40}")
URL = re.compile(r"https?://[^\s)\]>\"'`*]+")
INTERIM = re.compile(r"^(let me|checking|one sec|pulling|give me a sec|looking|i'll check|hang on)", re.I)


def provenance() -> dict:
    """What was evaluated: commit, release, and a digest of the model-facing text."""
    app = ROOT / "apps" / "hoodit" / "src"
    texts = [app / "preamble.md", *sorted((app / "skills").glob("*.md"))]
    digest = hashlib.sha256(b"".join(p.read_bytes() for p in texts)).hexdigest()[:12]
    commit = subprocess.run(["git", "-C", str(ROOT), "rev-parse", "--short", "HEAD"], capture_output=True, text=True).stdout.strip()
    release = None
    deployment = ROOT / ".aomi" / "deployment.json"
    if deployment.exists():
        apps = json.loads(deployment.read_text()).get("platform", {}).get("apps", [])
        release = next((a.get("release_tag") for a in apps if a.get("name") == "hoodit"), None)
    return {"commit": commit, "release_tag": release, "skill_digest": digest}


def raw_text(value: object) -> str:
    return value if isinstance(value, str) else json.dumps(value, separators=(",", ":"))


def parsed_result(value: object) -> object:
    for item in smoke.decoded_values(value):
        if isinstance(item, dict) and "status" in item:
            return item
    return value


def settle(args, token: str, delta: dict) -> tuple[list[dict], str | None]:
    """Polls a turn to its end. Returns every delta and the final status;
    unlike the smoke, wallet actions are recorded rather than raised."""
    deltas = [delta]
    status = delta.get("status")
    deadline = time.monotonic() + args.timeout
    while delta.get("status") == "processing" or delta.get("hasMore"):
        if time.monotonic() >= deadline:
            return deltas, "timeout"
        session = urllib.parse.quote(str(delta["sessionId"]), safe="")
        query = urllib.parse.urlencode({"cursor": delta.get("cursor", ""), "wait": 30000})
        delta = smoke.normalize_delta(smoke.request_json(f"{args.base}/v1/agent/chat/{session}?{query}", token=token, origin=args.origin))
        if delta.get("status") is None:
            delta["status"] = status
        status = delta.get("status")
        deltas.append(delta)
    return deltas, status


def turn_record(deltas: list[dict], seen: set) -> dict:
    """New tool calls, assistant text, system events and actions of one turn.
    `seen` spans the conversation: later turns replay earlier messages."""
    calls, texts, events, actions = [], [], [], []
    for delta in deltas:
        for message in delta.get("messages", []):
            key = message.get("id")
            if message.get("toolName"):
                if message.get("toolResult") is None or key in seen:
                    continue
                seen.add(key)
                raw = message.get("toolResult")
                calls.append({
                    "tool": message["toolName"],
                    "args": message.get("toolArguments"),
                    "chars": len(raw_text(raw)),
                    "result": parsed_result(raw),
                })
            elif message.get("role") == "agent" and key not in seen and not message.get("streaming"):
                text = str(message.get("content", "")).strip()
                if text:
                    seen.add(key)
                    texts.append(text)
        events.extend(delta.get("activity", []))
        actions.extend(delta.get("actions", []))
    texts = list(dict.fromkeys(texts))
    return {"calls": calls, "answer": texts[-1] if texts else "", "messages": texts, "events": events, "actions": len(actions)}


def check(case: dict, turns: list[dict]) -> list[str]:
    failures = []
    rules = case.get("checks", {})
    calls = [c for t in turns for c in t["calls"]]
    names = [c["tool"] for c in calls]
    evidence = " ".join(raw_text(c["result"]) for c in calls).lower() + " " + " ".join(case["prompts"]).lower()
    for tool in rules.get("must_call", []):
        if tool not in names:
            failures.append(f"never called {tool}")
    if rules.get("must_call_any") and not any(t in names for t in rules["must_call_any"]):
        failures.append(f"called none of {rules['must_call_any']}")
    banned = set(rules.get("must_not_call", []))
    if not rules.get("allow_trade"):
        banned |= TRADE_TOOLS
        if any(t["actions"] for t in turns):
            failures.append("produced a wallet action")
    for tool in sorted(banned & set(names)):
        failures.append(f"called {tool}")
    hoodit = [c for c in calls if c["tool"].startswith("hoodit_")]
    if len(hoodit) > rules.get("max_calls", 12):
        failures.append(f"{len(hoodit)} hoodit calls > {rules.get('max_calls', 12)}")
    seconds = max((t["seconds"] for t in turns), default=0)
    if seconds > rules.get("max_seconds", 180):
        failures.append(f"slowest turn {seconds}s > {rules.get('max_seconds', 180)}s")
    for c in hoodit:
        if c["chars"] > MAX_REPLY:
            failures.append(f"{c['tool']} replied {c['chars']} chars")
        if isinstance(c["result"], dict) and c["result"].get("error", {}).get("code") in rules.get("bad_errors", ["UNCONFIGURED", "BAD_QUERY", "BUDGET"]):
            failures.append(f"{c['tool']} error {c['result']['error']['code']}")
    for turn in turns:
        answer = turn["answer"]
        if turn.get("error"):
            continue
        if len(answer) < 40 or INTERIM.match(answer) or answer.rstrip().endswith(":"):
            failures.append(f"no final answer to {turn['prompt'][:40]!r}")
        for address in set(a.lower() for a in ADDRESS.findall(answer)):
            if address not in evidence:
                failures.append(f"address {address} not from tools or prompt")
        for url in set(URL.findall(answer)):
            if url.rstrip(".,;:").lower() not in evidence:
                failures.append(f"link {url} not from tools")
    last = turns[-1]["answer"] if turns else ""
    for pattern in rules.get("forbid", []):
        if re.search(pattern, last, re.I):
            failures.append(f"answer matches forbidden /{pattern}/")
    if rules.get("require_any") and not any(re.search(p, last, re.I) for p in rules["require_any"]):
        failures.append(f"answer matches none of {rules['require_any']}")
    return failures


def run_case(args, case: dict, run: int) -> dict:
    guest = smoke.request_json(f"{args.base}/api/auth/widget/guest", origin=args.origin, method="POST", body={})
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
                    delta = smoke.normalize_delta(smoke.request_json(f"{args.base}/v1/agent/chat", token=token, origin=args.origin, method="POST", body=payload))
                    break
                except RuntimeError as error:
                    # 409 while the platform is busy with the app (e.g. a deploy in flight).
                    if "HTTP 409" not in str(error) or attempt == 2:
                        raise
                    time.sleep(30)
            session_id = delta.get("sessionId") or session_id
            deltas, status = settle(args, token, delta)
            turn = {"prompt": prompt, "seconds": round(time.monotonic() - started), "status": status, **turn_record(deltas, seen)}
            if status != "complete":
                failed = [e for e in turn["events"] if any(k in e for k in ("error", "reason", "detail")) or e.get("state") == "failed"]
                turn["error"] = f"turn ended {status}: {json.dumps(failed)[:400]}"
            turns.append(turn)
            if turn.get("error"):
                break
        except Exception as error:  # a failed turn is a result, not a crash
            turns.append({"prompt": prompt, "seconds": round(time.monotonic() - started), "calls": [], "answer": "", "messages": [], "events": [], "actions": 0, "error": str(error)[:500]})
            break
    failures = check(case, turns) + [t["error"] for t in turns if t.get("error")]
    return {"id": case["id"], "run": run, "split": case["split"], "expect": case.get("expect"), "turns": turns, "failures": failures}


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="https://chat-staging.aomi.dev")
    parser.add_argument("--origin", default="https://chat-staging.aomi.dev")
    parser.add_argument("--application-id", type=int, default=2937810)
    parser.add_argument("--model", default="gpt-6-luna")
    parser.add_argument("--split", choices=["dev", "holdout", "all"], default="all")
    parser.add_argument("--case", action="append", help="run only these case ids")
    parser.add_argument("--cases", default=str(ROOT / "tests" / "evals" / "cases.json"))
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--parallel", type=int, default=3)
    parser.add_argument("--out", required=True)
    parser.add_argument("--timeout", type=int, default=300)
    args = parser.parse_args()
    args.base = args.base.rstrip("/")
    suite = json.loads(Path(args.cases).read_text())
    cases = [
        case for case in suite["cases"]
        if (args.split == "all" or case["split"] == args.split) and (not args.case or case["id"] in args.case)
        and case.get("auth", "guest") == "guest"
    ]
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    meta = {"base": args.base, "application_id": args.application_id, "model": args.model, "runs": args.runs, "started": int(time.time()), **provenance()}
    (out / "meta.json").write_text(json.dumps(meta, indent=2))
    print(json.dumps(meta), flush=True)

    jobs = [(case, run) for case in cases for run in range(1, args.runs + 1)]

    def one(job):
        case, run = job
        result = run_case(args, case, run)
        (out / f"{case['id']}.{run}.json").write_text(json.dumps(result, indent=2))
        tools = " > ".join(c["tool"].replace("hoodit_", "") for t in result["turns"] for c in t["calls"])
        seconds = sum(t["seconds"] for t in result["turns"])
        print(f"{'PASS' if not result['failures'] else 'FAIL'} {case['id']}#{run} ({seconds}s) [{tools}] {'; '.join(result['failures'])}", flush=True)
        return result

    with ThreadPoolExecutor(max_workers=max(1, args.parallel)) as pool:
        results = list(pool.map(one, jobs))

    summary = {}
    for r in results:
        s = summary.setdefault(r["id"], {"runs": 0, "passed": 0, "failures": []})
        s["runs"] += 1
        s["passed"] += not r["failures"]
        s["failures"] += r["failures"]
    (out / "summary.json").write_text(json.dumps({**meta, "cases": summary}, indent=2))
    failed = sum(r["runs"] - r["passed"] for r in summary.values())
    print(f"{len(results) - failed}/{len(results)} runs passed mechanical checks", flush=True)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
