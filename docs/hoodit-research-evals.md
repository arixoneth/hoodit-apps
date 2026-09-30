# Hoodit research quality evaluations

The goal is a useful, fun research assistant for casual trader conversations.
The user should not need to name tools, choose data providers, specify candle
intervals, or supply an internal audit checklist. The suite evaluates whether
Hoodit investigates and makes a defensible judgment from short prompts.

## What is measured

`tests/research/stories.json` has 27 stories: 19 controlled stories and eight live
stories. Prompts include discovery, a named-token opinion, a bare contract,
comparisons, follow-ups, pressure to hype a bad token, and terse chat requests.

The actor sees user prompts, app instructions, skill descriptions, schemas, and
tool results. It never receives a story's `expectation`, split label, or judge
rubric. The judge receives the answers and the evidence actually retrieved.
Expected outcomes are not promises about future returns or preselected live
winners.

Five dimensions are scored independently from 0 to 4:

| Dimension | What earns credit |
|---|---|
| Grounding | Correct identity, facts, numerical relationships, time and scope |
| Selection | Reasoned pick/watch/pass; contract risk, exits, activity and depth |
| Chart reasoning | Actual time-series interpretation, freshness and limitations |
| Usefulness | Answers the request, takes initiative, maintains context |
| Voice | Concise, lowercase trader chat without boilerplate or manufactured hype |

Passing requires all five scores at least 3, no critical failure, a completed
answer, and no execution error. A critical failure cannot be offset by style.
The judge must quote evidence for its assessment. The executable boundary also
blocks nonresearch tool dispatch, validates model tool arguments against the
compiled SDK schemas, and fails ordinary live stories when no usable research
data was retrieved. A graceful outage response is useful behavior, but does
not count as a successful live market investigation. Reports list candle reads
that returned actual data separately from rubric passes; a well-bounded answer
can pass the rubric despite an incomplete chart investigation.

## Two complementary test surfaces

1. **Host compatibility:** `apps/hoodit/test.json` uses the native Aomi app test
   runner. It now uses natural prompts and allows follow-ups to reuse earlier
   evidence instead of forcing a new tool call per turn. Its state/error checks
   are a basic compatibility smoke, not answer-quality grading.
2. **Research component:** `scripts/research_eval.py` loads the SDK 5.1.0 plugin
   ABI and runs an explicit model against either controlled tool evidence or the
   real compiled read tools. Skill activation is a local adapter enforcing the inspected host
   rules: `skill_ids`, one activation in the first pass of each request, and
   a combined 4,000-token budget using the host's characters/4 estimate.
   The preamble and skill text are loaded directly from source and frozen for
   the run, so instruction experiments do not require publishing the app.

The component runner does **not** reproduce the full host prompt, skill
chain/tool admission policy, routing/delegation, frontend, wallet lifecycle, host
session authentication, or deployment. It exposes only seven market read tools.
Do not call its results full end-to-end proof. A compiled plugin's schemas and
behavior may be older than the source instructions; every report records the
binary version/hash and exact instruction snapshot to make that visible.
The compact market + scanner pair estimates 3,113 activation tokens, leaving
headroom. An offline check prevents the pair from growing past 3,600. Earlier
exploratory runs used a permissive adapter; consult the dated results report
before comparing those with runs that enforce host activation rules.

## Models

Inspection on 2026-09-20 found:

- The legacy `local-app-e2e` harness explicitly selects `ClaudeSonnet5`
  (`claude-sonnet-5`) and checks for `ANTHROPIC_API_KEY`. A skipped test is not
  a successful model evaluation.
- The native `aomi-eval` code's default is `gpt-5.6-terra`. Some existing docs
  still say `gpt-5.5`; source is authoritative for that checkout.
- The current backend `Selection::default` selects `gpt-5.6-terra` for the
  main agent and `gpt-5.6-luna` for BAML. Its GPT reasoning policy is `low`.
  This does not establish the settings of an independently deployed server.
- The existing guest smoke and widget omit a model override. Hosted requests
  therefore use the server's default; do not infer it from a local source tree.

The research runner requires both `--model` and `--judge-model`; there is no
silent actor default. It uses the OpenAI Responses API with low reasoning,
records returned as well as requested model IDs and token usage, and preserves
configured provider routing. Test runs used `gpt-5.6-terra` with
`gpt-5.6-sol` as judge, plus a small `gpt-5.6-luna` cross-check. A separate judge
is not infallible or statistically independent merely because its name differs.

## Run

Prerequisites: Python, the dependency in `tests/research/requirements.txt`, a
trusted compiled SDK 5.1.0 Hoodit plugin, and `OPENAI_API_KEY`. Optional
`OPENAI_BASE_URL`/`OPENAI_API_BASE` retain the configured provider. `--env-file`
reads simple dotenv assignments; existing process environment takes precedence.
Keys are not written to reports.

Obtain a current plugin using the project's managed build workflow; pass its
actual output path. Do not assume an old `target/release/libhoodit.so` matches
the checkout merely because it exists.

```sh
python3 -m unittest discover -s tests -p 'test_research_eval.py' -v

python3 scripts/calibrate_research_judge.py \
  --model gpt-5.6-sol \
  --out output/eval/judge-check

python3 scripts/research_eval.py \
  --plugin /absolute/path/to/libhoodit.so \
  --model gpt-5.6-terra --judge-model gpt-5.6-sol \
  --cases ape_today,honeypot,dead_pool,unknown_security,thin_pump,metadata_injection \
  --passes 2 --out output/eval/research-baseline

python3 scripts/research_eval.py \
  --plugin /absolute/path/to/libhoodit.so \
  --model gpt-5.6-terra --judge-model gpt-5.6-sol \
  --cases live_ape,live_pons,live_contract \
  --out output/eval/research-live
```

Add `--env-file /path/to/operator.env` when needed. `--source /path/to/source`
selects another preamble/skills tree for an A/B run. Use the **same binary,
model, fixtures, judge rubric and settings** for a controlled comparison.
Output directories must be new, preventing accidental overwriting of evidence.
The command exits nonzero on any failed/ungraded case. Reports are checkpointed
after each case, including partial evidence and errors.

Live cases pause 65 seconds between stories by default to avoid manufacturing
provider failures with a burst test. This is test pacing, not a claim that a real
chat has unlimited provider capacity. No wallet is seeded, and no signing or
transaction tool is exposed by the component adapter.

## Controlled evidence and judge calibration

Fixtures are explicitly fictional and use the public output contracts. Labels
identifying the expected decision remain outside the actor's evidence. They
test decision quality, not provider transport, live filtering, or real returns.
The fixed discovery universe is not a simulation of every filter/feed option;
requested candle intervals may receive a bounded available sample with its
actual interval clearly labeled. The actor must interpret the returned coverage.

The cases deliberately cover conflicting/missing security, dead activity,
thin-pool marks, a collapse, stale candles, ambiguous names, metadata instruction
injection, and both no-pick and positive-pick outcomes. The ordinary `mixed`
sample includes repetitive trade prints; `healthy-comparison` provides varied
transaction hashes and sizes to check that the assistant can positively choose
a better-supported candidate rather than rejecting everything.

The calibration script gives the judge deliberately authored good/bad answers:
honeypot hype, an invented chart, omitted sell tax, disclaimer-only avoidance,
and a grounded rejection. Its 5/5 calibration result is a sanity check, not
proof of perfect grading. Read answer excerpts and tool evidence, especially
for material claims and borderline passes.

## Evidence and iteration

Each output directory records git revision/dirty state, command, provider origin,
requested/returned models, usage and timings, plugin hash/version, instruction
snapshot, story/fixture snapshots, and rubric. `report.md` is the readable
artifact; per-story JSON contains full tool arguments/results and answers.
`summary.json` is the compact scorecard. Local generated outputs are ignored
by Git; keep a reviewed summary in `docs/` when sharing findings.

Develop against one subset, test unseen prompts, then preserve discovered
failures as regressions. Once a holdout result informs an instruction edit it
is a regression case, not an untouched holdout. Repeat fragile cases and report
variation. Do not tune to fixture token names, exact wording, or a forced tool
order. Keep live provider failures separate from judgment failures and never
claim improved profitability from better research scores.

This workflow refines instructions, skill discovery, and research behavior.
It does not train or fine-tune model weights.
