# Hoodit

Hoodit is a Robinhood Chain trading assistant for token and launchpad discovery, market research, exit checks, and user-authorized trades. Canonical Stock Token trading remains available through the host's resolver and execution flow.

## Repository layout

- `app/` and `public/` — Next.js landing page and product UI
- `apps/hoodit/` — Rust v2 dynamic application loaded by Aomi: `src/tools/`
  holds the nine read tools, `src/market.rs` the shared token logic,
  `src/providers.rs` the Codex, LI.FI and chain transport, `src/shape.rs` the
  model-facing JSON rules
- `tests/evals/` and `scripts/hoodit-eval.py` — conversation evals run on Aomi chat
- `docs/hoodit-v1-validation.md` — local and sanitized provider evidence, with deployment work called out separately
- `.aomi/config.json` — Aomi Project manifest used by Build's community repository import
- `Cargo.toml` — shared Rust workspace and backend-compatible Aomi SDK pin

The repository name remains plural so separately permissioned products can live
beside Hoodit without expanding this app's trust boundary.

## Landing page

Requires Node.js 24.3 or newer and npm 10.9.2.

```bash
npm ci
npm run dev
```

Use `npm test`, `npm run lint` and `npm run build` before publishing frontend changes.

### In-page chat

`/app` mounts the native Aomi widget, pinned to Hoodit application `2938613`.
Guest and wallet sessions are issued directly by `chat.aomi.dev` and bound to
the browser origin. Agent requests use `/api/agent/*` on this site because the
hosted `/v1/agent/*` endpoint does not currently supply cross-origin CORS headers.
The relay forwards the caller's bearer unchanged with this site's origin; Aomi
still validates identity, scope and session ownership. It does not forward
cookies, mint credentials, follow redirects, or use a shared server API key.
Guest credentials are page-scoped, so reloading starts a fresh conversation;
local thread-ID persistence is disabled to avoid restoring another guest's session.

Assistant UI dependencies are pinned through `overrides` to avoid the render
loop and incompatible Markdown peer dependency in the freely resolved versions.
When upgrading, verify rendering and a real read-only chat response in the
browser, not just a successful build. Regression tests cover the relay's route,
origin, credential and error boundaries. Wallet signing requires separate tests.

## Aomi application

The workspace pins `aomi-sdk = "=5.1.1"`, matching the Aomi backend runtime.

| Source | Used for |
|---|---|
| [Codex](https://docs.codex.io) GraphQL | Boards, token cards, candles (curve phase included), trades with the real wallet behind ERC-4337 bundles, holders, top traders, wallet records. Every request is paid with MPP: $0.001 in USDC.e on Tempo from the operator wallet, several aliased queries billed once. |
| Robinhood Chain RPC | The Pons V2 launch record (true stage, pair token, graduation target, deployer) and curve reserve, so `curve_pct` matches what Pons shows; token decimals and symbols; contract checks for holder labels. |
| LI.FI | Exit quotes for `hoodit_exit`: a round trip at the user's size, loss measured in ETH. The host's own LI.FI tools still do the trading. |
| GoPlus | Contract security, only for tokens deployed outside a launchpad. |

Two operator secrets, set per application in Aomi Build → Environment:

| Secret | Value |
|---|---|
| `CODEX_MPP_KEY` | Private key of the wallet that pays for Codex requests on Tempo (chain 4217, USDC.e). Keep only a few dollars on it. |
| `LIFI_API_KEY` | LI.FI integrator key. Without it quotes fall back to the keyless allowance (about 40 an hour). |

Missing secrets make the affected tools return `UNCONFIGURED` instead of failing
the app. Spend is capped in-process: ten paid requests per answer and 5,000 per
host per UTC day, with stale cache served when a provider fails.

Nine read tools, none owned by a skill so they work in every thread:

| Tool | Answers |
|---|---|
| `hoodit_scan` | What's moving, new, on a curve, or just graduated, with filters |
| `hoodit_find` | Which contract a ticker means (ranked by holders; copycats listed) |
| `hoodit_token` | The card: stage, curve %, trade support, market, flow, holder mix, dev, security |
| `hoodit_chart` | Candles for a range plus facts computed from the same bars |
| `hoodit_trades` | Buy and sell dollars by window and the largest trades with wallets |
| `hoodit_holders` | Top holders with contract labels and top traders, or the dev's launches |
| `hoodit_wallet` | A wallet's record and bag |
| `hoodit_exit` | Whether a size gets back out, in ETH |
| `hoodit_check` | Flat numbers for watchers (`wake_on_condition`) |

The always-on preamble sets the voice and evidence rules; three short skills
(`hoodit/research`, `hoodit/trade`, `hoodit/watch`) are playbooks. Every reply
stays under 2,500 characters because guest chats share a 64 kB model input with
the whole history.

```bash
cargo test -p hoodit
cargo clippy --workspace --all-targets -- -D warnings
aomi-build sdk check --path . --required-version 5.1.1
```

A live run calls real tools with real secrets and fails on any reply over the
size limit (costs about $0.02):

```bash
set -a; . ~/.config/hoodit/mpp-test-wallet.env; set +a   # CODEX_MPP_KEY
export LIFI_API_KEY=...
cargo run -p hoodit --example live -- apps/hoodit/examples/live-plan.json
cargo run -p hoodit --example gql -- '{ ... }'           # one raw Codex query
```

### Conversation evals

`tests/evals/cases.json` holds casual trader conversations split into `dev`
(used while tuning) and `holdout` (run once per release candidate, never copied
into skills). `scripts/hoodit-eval.py` runs them as fresh guest chats against a
deployed app with a chosen model, checks mechanics (completion, required tools,
tool errors, no signing actions), and saves transcripts for grading the
answers against the evidence the tools returned:

```bash
scripts/hoodit-eval.py --model gpt-6-luna --split dev --out /tmp/hoodit-eval
```

## Connect to Aomi Build

In **Deployments → New app → Connect an existing repository**, enter the fork
that Aomi Build should read. For this checkout, use:

```text
arixoneth/hoodit-apps
```

The root Project manifest selects the `community` platform and publishes `apps/hoodit/aomi.toml`. Commit and push changes before importing so Build can read the same revision.
