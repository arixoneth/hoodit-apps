"""Offline checks for the evaluator's failure boundaries and evidence contracts."""
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

import jsonschema

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("research_eval", ROOT / "scripts/research_eval.py")
ev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ev)


class ResearchEvalTests(unittest.TestCase):
    def test_chart_coverage_requires_data_not_just_a_call(self):
        result = {"calls": [{"tool": "hoodit_get_candles", "result": {"status": "error", "data": None}}]}
        self.assertEqual(ev.research_coverage(result), {"candle_attempts": 1, "candle_reads_with_data": 0, "tool_errors": 1})
        result["calls"][0]["result"] = {"status": "ok", "data": {"candles": []}}
        self.assertEqual(ev.research_coverage(result)["candle_reads_with_data"], 0)

    def test_research_pair_fits_host_activation_budget_with_headroom(self):
        skills = []
        for name in ["markets", "coin-scanner"]:
            skills.append({"id": "hoodit/" + name, "injected_tools": sorted(ev.READ_TOOLS) if name == "markets" else [], "sections": [{"name": "instructions", "content": (ROOT / f"apps/hoodit/src/skills/{name}.md").read_text()}]})
        self.assertLess(sum(ev.activation_tokens(s) for s in skills), 3600)

    def test_activation_is_once_per_request_and_rejects_oversized_skills(self):
        class Actor:
            calls = []
            step = 0
            def chat(self, messages, tools):
                self.step += 1
                if self.step < 3:
                    return {"role": "assistant", "tool_calls": [{"id": str(self.step), "function": {"name": "activate_skills", "arguments": json.dumps({"skill_ids": ["oversized"]})}}]}
                return {"role": "assistant", "content": "research unavailable"}
        oversized = {"id": "oversized", "description": "too large", "sections": [{"name": "instructions", "content": "x" * 16001}]}
        result = ev.run_case({"id": "activation", "prompts": ["anything cooking?"], "fixture": "controlled"}, Actor(), None, {"tools": []}, ROOT, 4, instruction_bundle=("research", {"oversized": oversized}))
        self.assertEqual(result["hard_failures"], ["token_budget_trim:oversized", "already_activated_in_first_pass"])
        self.assertEqual(result["calls"][0]["result"]["activated"], [])

    def test_high_style_score_cannot_cancel_a_critical_failure(self):
        result = {"completed": True, "grade": {"scores": dict.fromkeys(ev.DIMENSIONS, 4), "critical_failures": ["promoted a flagged honeypot"]}}
        self.assertFalse(ev.passed(result))
        result["grade"]["critical_failures"] = []
        self.assertTrue(ev.passed(result))
        result["hard_failures"] = ["live_research_unavailable"]
        self.assertFalse(ev.passed(result))

    def test_incomplete_ungraded_or_failed_runs_never_pass(self):
        self.assertFalse(ev.passed({"completed": True}))
        self.assertFalse(ev.passed({"completed": True, "grade": {"scores": {}, "critical_failures": []}}))
        self.assertFalse(ev.passed({"completed": False, "grade": {"scores": dict.fromkeys(ev.DIMENSIONS, 4), "critical_failures": []}}))
        self.assertFalse(ev.passed({"completed": True, "error": "timeout", "grade": {"scores": dict.fromkeys(ev.DIMENSIONS, 4), "critical_failures": []}}))

    def test_every_fixture_matches_the_public_output_contract(self):
        contract = json.loads((ROOT / "contracts/hoodit-v1/tool-contracts.json").read_text())
        schemas = {t["name"]: {"$ref": t["output_schema"]["$ref"], "$defs": contract["$defs"]} for t in contract["x-tools"]}
        for path in (ROOT / "tests/research/fixtures").glob("*.json"):
            pack = json.loads(path.read_text())
            self.assertTrue(pack["synthetic"], path)
            for response in pack["responses"]:
                with self.subTest(path=path.name, tool=response["tool"]):
                    jsonschema.Draft202012Validator(schemas[response["tool"]]).validate(response["output"])

    def test_fixture_search_is_case_insensitive_and_unknown_calls_fail(self):
        fixture = ev.Fixture(ROOT / "tests/research/fixtures/mixed.json")
        self.assertEqual(fixture.call("hoodit_search_tokens", {"query": "fizz"})["status"], "ok")
        self.assertEqual(fixture.call("hoodit_get_token", {"token": "nonexistent"})["status"], "error")
        self.assertEqual(fixture.call("stage_tx", {})["status"], "error")

    def test_stories_do_not_put_tool_names_or_evaluator_labels_in_prompts(self):
        suite = json.loads((ROOT / "tests/research/stories.json").read_text())
        ids = [case["id"] for case in suite["cases"]]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertGreaterEqual(sum(c["split"] == "holdout" for c in suite["cases"]), 6)
        for case in suite["cases"]:
            for prompt in case["prompts"]:
                self.assertNotIn("hoodit_", prompt)
                self.assertNotIn("expected_tools", prompt)

    def test_judge_requires_complete_scores_and_evidence(self):
        class Judge:
            def chat(self, *args, **kwargs):
                return {"content": json.dumps({"scores": {"voice": 4}, "critical_failures": [], "evidence": []})}
        with self.assertRaises(ValueError):
            ev.judge_case({}, {"turns": [], "calls": []}, Judge(), "rubric")

    def test_actor_cannot_see_gold_labels_or_execute_a_write(self):
        class Actor:
            calls = []
            seen = []
            def chat(self, messages, tools):
                self.seen = json.loads(json.dumps(messages))
                if len(messages) == 2:
                    return {"role": "assistant", "tool_calls": [{"id": "bad-call", "function": {"name": "stage_tx", "arguments": "{}"}}]}
                return {"role": "assistant", "content": "can't do that"}
        class Adapter:
            def call(self, *args):
                raise AssertionError("write reached tool adapter")
        actor = Actor()
        with patch.object(ev, "instructions", return_value=("research assistant", {})):
            result = ev.run_case({"id": "boundary", "prompts": ["anything cooking?"], "expectation": "SECRET_GOLD_LABEL", "fixture": "controlled"}, actor, Adapter(), {"tools": []}, ROOT, 3)
        self.assertNotIn("SECRET_GOLD_LABEL", json.dumps(actor.seen))
        self.assertEqual(result["hard_failures"], ["unavailable_or_nonresearch_tool:stage_tx"])


if __name__ == "__main__":
    unittest.main()
