#!/usr/bin/env python3
"""Read-only Hoodit staging guest-chat smoke. Never prints the guest token."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


DEFAULT_TURNS = [
    ("Hello. Briefly identify Hoodit and what read-only research you can do.", None),
    (
        "Activate Hoodit's markets skill and show the currently trending pools on Robinhood Chain. This is read-only; do not prepare or request any transaction.",
        "hoodit_discover_pools",
    ),
    (
        "Activate Hoodit's portfolio skill and show balances only for public Robinhood Chain address 0xb202bb725c85b90bd847d350ebc7f16ff8408ed8. Do not request valuation quotes and do not prepare or request any transaction.",
        "hoodit_get_portfolio",
    ),
]


def request_json(url: str, *, token: str | None = None, origin: str, method: str = "GET", body: object | None = None) -> dict:
    headers = {
        "Accept": "application/json",
        "Origin": origin,
        "User-Agent": "Mozilla/5.0 (compatible; HooditStagingSmoke/1.0)",
    }
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"
    if method == "POST":
        headers["Idempotency-Key"] = f"hoodit-smoke-{uuid.uuid4().hex}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=40) as response:
            return json.load(response)
    except urllib.error.HTTPError as exc:
        detail = ""
        try:
            error = json.loads(exc.read())
            if isinstance(error, dict):
                safe = {
                    key: error[key]
                    for key in ("code", "error", "message", "status")
                    if isinstance(error.get(key), (str, int, float, bool))
                }
                if safe:
                    detail = f": {json.dumps(safe, sort_keys=True)}"
        except (json.JSONDecodeError, UnicodeDecodeError):
            pass
        raise RuntimeError(
            f"HTTP {exc.code} from {urllib.parse.urlsplit(url).path}{detail}"
        ) from exc


def redact(value: object, secrets: list[str]) -> object:
    if isinstance(value, dict):
        return {key: redact(item, secrets) for key, item in value.items()}
    if isinstance(value, list):
        return [redact(item, secrets) for item in value]
    if isinstance(value, str):
        for secret in secrets:
            value = value.replace(secret, "[REDACTED]")
    return value


def normalize_delta(delta: dict) -> dict:
    if "events" not in delta:
        return delta
    states = [
        event.get("state")
        for event in delta.get("events", [])
        if event.get("type") == "turn_state_changed" and event.get("state")
    ]
    messages = []
    for event in delta.get("events", []):
        if event.get("type") != "message":
            continue
        message = {
            "id": event.get("message_key") or event.get("event_id"),
            "role": "agent" if event.get("sender") == "assistant" else event.get("sender"),
            "content": event.get("content", ""),
            "streaming": event.get("is_streaming", False),
        }
        if event.get("tool_name") is not None:
            message["toolName"] = event.get("tool_name")
        if event.get("tool_arguments") is not None:
            message["toolArguments"] = event.get("tool_arguments")
        if event.get("tool_result") is not None:
            message["toolResult"] = event.get("tool_result")
        messages.append(message)
    return {
        "sessionId": delta.get("session_id"),
        "cursor": delta.get("cursor"),
        "hasMore": delta.get("has_more", False),
        "status": states[-1] if states else None,
        "messages": messages,
        "activity": [
            {"type": event.get("type"), "state": event.get("state")}
            for event in delta.get("events", [])
            if event.get("type") != "message"
        ],
        "actions": [
            event for event in delta.get("events", []) if "action" in str(event.get("type", ""))
        ],
    }


def sanitize(delta: dict, secrets: list[str]) -> dict:
    delta = normalize_delta(delta)
    messages = []
    for message in delta.get("messages", []):
        messages.append({key: message.get(key) for key in ("id", "role", "content", "streaming", "toolName", "toolArguments", "toolResult") if key in message})
    return redact({
        "sessionId": delta.get("sessionId"),
        "status": delta.get("status"),
        "cursor": delta.get("cursor"),
        "messages": messages,
        "activity": delta.get("activity", []),
        "actionCount": len(delta.get("actions", [])),
    }, secrets)


def settle(base: str, origin: str, token: str, delta: dict, timeout: int, secrets: list[str]) -> tuple[list[dict], list[dict]]:
    delta = normalize_delta(delta)
    last_status = delta.get("status")
    raw = [delta]
    observed = [sanitize(delta, secrets)]
    if delta.get("actions"):
        raise RuntimeError("Read-only smoke unexpectedly produced a wallet/signing action")
    deadline = time.monotonic() + timeout
    while delta.get("status") == "processing" or delta.get("hasMore"):
        if time.monotonic() >= deadline:
            raise TimeoutError(f"Agent session did not settle within {timeout}s")
        session = urllib.parse.quote(str(delta["sessionId"]), safe="")
        query = urllib.parse.urlencode({"cursor": delta.get("cursor", ""), "wait": 30000})
        delta = normalize_delta(request_json(f"{base}/v1/agent/chat/{session}?{query}", token=token, origin=origin))
        if delta.get("status") is None:
            delta["status"] = last_status
        else:
            last_status = delta.get("status")
        raw.append(delta)
        if delta.get("actions"):
            raise RuntimeError("Read-only smoke unexpectedly produced a wallet/signing action")
        observed.append(sanitize(delta, secrets))
    if delta.get("status") != "complete":
        raise RuntimeError(f"Agent turn ended with non-complete status: {delta.get('status')}")
    if not any(m.get("role") == "agent" and str(m.get("content", "")).strip() for event in observed for m in event["messages"]):
        raise RuntimeError("Agent turn completed without a non-empty assistant message")
    return raw, observed


def decoded_values(value: object):
    yield value
    if isinstance(value, dict):
        for item in value.values():
            yield from decoded_values(item)
    elif isinstance(value, (list, tuple)):
        for item in value:
            yield from decoded_values(item)
    elif isinstance(value, str) and value[:1] in "[{":
        try:
            decoded = json.loads(value)
        except json.JSONDecodeError:
            return
        yield from decoded_values(decoded)


def require_tool_result(events: list[dict], expected_tool: str) -> None:
    matching = [
        message
        for event in events
        for message in event.get("messages", [])
        if message.get("toolName") == expected_tool and message.get("toolResult") is not None
    ]
    if not matching:
        raise RuntimeError(f"Turn completed without an {expected_tool} result")
    for message in matching:
        for value in decoded_values(message.get("toolResult")):
            if isinstance(value, dict) and value.get("status") in {"ok", "partial"} and value.get("data") is not None:
                return
    raise RuntimeError(f"{expected_tool} returned no successful non-empty Hoodit envelope")


def secret_values(paths: list[str]) -> list[str]:
    values: list[str] = []
    for name in paths:
        path = Path(name)
        if not path.exists():
            continue
        text = path.read_text()
        try:
            parsed = json.loads(text)
            candidates = decoded_values(parsed)
        except json.JSONDecodeError:
            candidates = (line.strip().split("=", 1)[-1] for line in text.splitlines())
        values.extend(value for value in candidates if isinstance(value, str) and len(value) >= 4)
    return sorted(set(values), key=len, reverse=True)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="https://chat-staging.aomi.dev")
    parser.add_argument("--origin", default="https://chat-staging.aomi.dev")
    parser.add_argument("--application-id", type=int, default=2937810)
    parser.add_argument("--prompt", action="append", dest="prompts")
    parser.add_argument("--expected-tool", action="append", dest="expected_tools")
    parser.add_argument("--secret-file", action="append", default=[])
    parser.add_argument("--timeout", type=int, default=180)
    args = parser.parse_args()
    base = args.base.rstrip("/")
    guest = request_json(f"{base}/api/auth/widget/guest", origin=args.origin, method="POST", body={})
    token = guest.get("access_token")
    if not isinstance(token, str) or not token:
        raise RuntimeError("Guest bootstrap returned no access_token")
    session_id = None
    transcript = []
    secrets = secret_values(args.secret_file)
    if args.prompts:
        expected = args.expected_tools or []
        if len(expected) > len(args.prompts):
            raise RuntimeError("More --expected-tool values than --prompt values")
        turns = list(zip(args.prompts, expected + [None] * (len(args.prompts) - len(expected))))
    else:
        turns = DEFAULT_TURNS
    for prompt, expected_tool in turns:
        payload = {"applicationId": args.application_id, "message": prompt}
        if session_id:
            payload["sessionId"] = session_id
        delta = normalize_delta(request_json(f"{base}/v1/agent/chat", token=token, origin=args.origin, method="POST", body=payload))
        session_id = delta.get("sessionId") or session_id
        raw_events, events = settle(base, args.origin, token, delta, args.timeout, secrets)
        transcript.append({"prompt": prompt, "events": events})
        if expected_tool:
            try:
                require_tool_result(raw_events, expected_tool)
            except RuntimeError as exc:
                transcript[-1]["validationError"] = str(exc)
                print(json.dumps({"base": base, "origin": args.origin, "applicationId": args.application_id, "turns": transcript}, indent=2))
                raise SystemExit(1) from exc
    print(json.dumps({"base": base, "origin": args.origin, "applicationId": args.application_id, "turns": transcript}, indent=2))


if __name__ == "__main__":
    main()
