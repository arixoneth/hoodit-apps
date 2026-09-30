# Hoodit research tuning — 2026-09-20

Hoodit now has natural trader stories, an evidence-based LLM judge, and a more
conversational research personality. Exploratory controlled comparisons improved
voice and usefulness; final validation also enforces host activation limits. The work is local and has not
been deployed. This is instruction refinement, not model-weight training.

## Exploratory before/after evidence

These early runs used a permissive activation adapter, before the host gate
was reproduced. They show instruction potential, **not deployable before/after
performance**. The original market/scanner pair exceeded the host budget.

Six identical controlled stories used the same fixture hashes, compiled plugin,
actor (`gpt-5.6-terra`), judge (`gpt-5.6-sol`), and rubric. Both requested and
returned model IDs were recorded; the API returned those model names. Reasoning
effort was `low`, matching the current local backend's GPT policy.

| Measure | Original instructions | First revised instructions |
|---|---:|---:|
| Passed all quality gates | 2/6 | 6/6 |
| Grounding, mean /4 | 4.00 | 4.00 |
| Selection, mean /4 | 4.00 | 4.00 |
| Chart reasoning, mean /4 | 3.83 | 4.00 |
| Usefulness, mean /4 | 3.50 | 3.83 |
| Voice, mean /4 | 2.33 | 4.00 |
| Mean answer words | 206 | 126 |

The largest measured gain was conversational quality. The original already
rejected obvious adverse contracts in these controlled cases; these results
do not establish improved future returns or a general six-case safety guarantee.

Matched cases: `ape_today`, `honeypot`, `dead_pool`, `unknown_security`,
`thin_pump`, and `metadata_injection`. Evidence:

- [Original report](../output/eval/research-20260920/baseline-final/report.md)
- [First revised report](../output/eval/research-20260920/tuned-development/report.md)

## Host activation correction

Final source inspection found one activation allowed in the first pass of each
request, with a 4,000-token combined budget. The original pair estimated 4,839
tokens; the expanded draft reached 6,409. A permissive component harness could
therefore reward guidance the actual host would trim.

Both skills were condensed to **3,113 estimated activation tokens combined**.
The catalog asks for both IDs in one call. The runner now uses the actual
`skill_ids` argument, enforces the first-pass/once-per-request gate, and rejects
oversized activation. Offline regression tests cover those boundaries and
reserve budget headroom. These checks still do not reproduce the full host.

## Final component checks with activation gates

The compact final instructions passed **8/8 controlled stories** on Terra with
Sol grading: informal discovery, a follow-up pick, honeypot rejection, unknown
security, metadata injection, ambiguous identity, stale charts, and a positive
relative pick. Both research skills activated together within the budget; no
activation-gate failure occurred. This is one final pass per story, not a
statistical reliability estimate or a claim that the bot always works.

The two final live stories also passed the **answer-quality rubric**, but
**0/2 retrieved candle data**: both chart reads were rate-limited. The exact
contract assessment disclosed missing chart/trade data and rejected an active
but collapsing setup using returned market/security facts. The discovery
answer offered a conditional PONS watch with unknown taxes and unavailable
candles. Neither counts as completed live chart research.

Human review still found wording to improve despite passing scores: one answer
called a mixed series the “eighth green candle,” another conflated separate
reference pools, and the live discovery answer omitted source-conflicted holder
concentration. These are reasons to review raw evidence and improve the judge,
not to read 8/8 as factual perfection. Reports now show successful candle reads
separately from rubric passes.

- [Final controlled report and answers](../output/eval/research-20260920/host-gated-final/report.md)
- [Final live report and coverage](../output/eval/research-20260920/host-gated-live/report.md)

## What changed

- The preamble now asks for lowercase, compact, opinionated trader chat with
  natural humor. It removes routine volatility/disclaimer speeches while
  keeping concrete exit problems, unknowns, and provenance visible.
- The scanner's skill description recognizes informal discovery and token
  opinions. Users need not request a full audit or name the workflow.
- Research guidance distinguishes a relative favorite, an incomplete watchlist
  lead, and an entry judgment. A trending row is not a completed investigation.
- The scanner now prioritizes bounded discovery, cached evidence, and request
  budget for the finalist's chart and recent activity. It guards against stale
  candles, token/pool volume confusion, unsupported identity labels, and chart
  claims inferred only from percentage-change windows.
- The native compatibility scenario uses casual prompts and permits follow-ups
  to reuse evidence. The separate quality suite contains **27 stories**,
  including **19 controlled stories** and **eight live stories**.
- The component evaluator keeps expectations away from the actor, saves full
  evidence, and grades grounding, selection, chart reasoning, usefulness, and
  voice. Critical failures cannot be averaged away by entertaining prose.

For example, a fictional-token rejection became:

> hard pass on FIZZ — this looks like an exit trap, not momentum.
> … both GeckoTerminal and GoPlus flag honeypot behavior, and GoPlus reports a
> **95% sell tax**. that’s basically a velvet-rope exit.

This excerpt comes from a synthetic evidence pack, not a claim about a real
token. The positive-pick case also succeeded: the model chose the better-supported
candidate over a flagged token and a dead pool, while separating token quality
from entry quality. It did not need to manufacture a guaranteed winner.

## Exploratory generalization checks and honest failures

| Phase | Outcome | Meaning |
|---|---:|---|
| First revised development set | 11/11 | All selected development stories passed |
| Unseen phrasing / conversation set | 5/7 | Exposed skipped chart analysis on a follow-up and stale-chart framing |
| Targeted regression reruns, two passes | 7/8 | Follow-up issue fixed; one freshness answer still too generous |
| Final freshness + identity reruns | 4/4 | Both cases passed twice after the final wording change |
| Positive pick cross-check on `gpt-5.6-luna` | 2/2 | Honeypot rejection and reasoned relative pick both passed |
| Judge calibration | 5/5 | Rejected deliberately wrong answers and accepted grounded rejection |

These runs also predate enforcement of the host activation gates.
The formerly unseen cases became regression cases once they informed edits.
The rows above represent sequential development phases, not independent samples
to combine into one headline pass rate. Scores are LLM judgments, not ground
truth; answer excerpts and raw tool facts remain available for human review.

The final stale-chart answer led with “not seeing a current wake-up” and cited
the lack of current activity instead of using an old rise as current momentum.
The identity check stopped calling a higher-ranked search result “the likely
real one” and neutrally presented the ambiguous contracts.

- [Unseen conversation results](../output/eval/research-20260920/tuned-holdout/report.md)
- [Repeated regressions](../output/eval/research-20260920/tuned-regression/report.md)
- [Final freshness and identity](../output/eval/research-20260920/final-freshness-identity/report.md)
- [Luna cross-check](../output/eval/research-20260920/luna-crosscheck/report.md)
- [Judge calibration](../output/eval/research-20260920/judge-calibration/report.md)

## Exploratory live research findings

The original component baseline invented a “lower lows” chart description
without retrieving candles and misstated pool ages. The revised outputs were
shorter and more specific, but live checks still exposed missing-candle
language and provider-capacity limits. A paced three-story live run passed 1/3;
the two failures were unsupported chart language and an unsupported identity
inference. Those are retained as failures.

After the identity instruction change, the final two live cases passed their
rubric: an ambiguous symbol prompted neutral clarification, and a bare contract
produced a specific market/security assessment. The latter still could not
retrieve candles and disclosed that limitation. This is **not** a completed
chart-analysis verification or a claim that live research is fully reliable.

I also inspected the [ZFORGE/WETH GeckoTerminal chart](https://www.geckoterminal.com/robinhood/pools/0x6ac6b3a1e78c32ab48152ec079c12014d2a72e8c).
At observation time it showed a green one-hour window inside a roughly 93%
daily collapse. That illustrates why the eval requires context before treating
a short bounce as a recovery; it is not a fixed live-token verdict.

The existing compiled tools repeatedly reached their request limit before the
candle read. Current source has a 10-request/minute GeckoTerminal budget, and
a single full token read can make several upstream requests. Test pacing
reduces interference between stories but cannot remove the within-story
capacity constraint. The final instructions reduce unnecessary refreshes and
discovery breadth; provider-budget behavior still needs a fresh-build host run.

- [Paced live report, including failures](../output/eval/research-20260920/tuned-live-paced/report.md)
- [Final live identity / exact-contract report](../output/eval/research-20260920/final-live-identity/report.md)
- [Original live evidence](../output/eval/research-20260920/baseline-v2/live_ape-1.json)

## Execution scope and blockers

The user selected the deployed app for baseline and local testing for changes.
Production guest attempts against application `2938613` at `chat.aomi.dev`
repeatedly returned HTTP 409 `session_busy` before an answer. Consequently there
is **no successful deployed-app baseline** and the deployed model was not
confirmed. The saved failure is
[here](../output/eval/research-20260920/hosted-baseline-attempt.json).

The full backend benchmark build and a current app build were both refused by
the managed build memory guard. I did not stop unrelated services or bypass it.
Tests therefore used the existing **Hoodit 1.2.0 / SDK 5.1.0** release plugin for
schemas and real read tools, with the **current source instructions** supplied
by the component runner. The source checkout is 1.3.0 and already contained
uncommitted tool changes before this task. Those existing changes were not
represented by a newly compiled binary in these runs.

Local code inspection establishes:

- legacy app E2E: `claude-sonnet-5`, explicitly selected;
- newer `aomi-eval`: default `gpt-5.6-terra`;
- backend default selection: Terra main agent / Luna BAML;
- component experiments here: explicitly selected Terra or Luna, with Sol judge.

The component adapter does not reproduce host routing, full prompt composition,
full skill admission policy, wallet behavior, or deployment. It now models
the inspected activation timing and token budget. Passing results must not be
represented as full end-to-end or production verification. No deployment or
model-weight fine-tuning was performed.

Early API/fixture pilots are retained under the output directory but excluded
from comparisons. They include an unsupported API parameter combination, an
overly conspicuous illustrative-data label, and an empty judge-evidence response.
Those harness problems were corrected before the matched comparison.

## Local checks and next release gate

- Ten offline evaluator tests passed, including chart-data coverage, activation limits, budget
  headroom, hidden gold-label separation,
  nonresearch-tool blocking, grading failure boundaries, and every controlled
  output's public schema.
- Existing contract validation passed: 83 schemas / 95 synthetic assertions.
- Python compilation, Rust formatting, and whitespace checks passed.
- Current Rust compilation/native E2E and deployment remain unverified for the
  resource and hosted-session reasons above.

Before publishing, build the current checkout, rerun the native host scenario
with an explicit model, resolve the fresh guest-session conflict, and verify
that a casual live discovery story can complete the finalist's chart/security
investigation within the provider budget. Keep the failing cases in the suite.
The [run guide](hoodit-research-evals.md) documents the reusable workflow.
