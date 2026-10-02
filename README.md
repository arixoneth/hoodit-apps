# Hoodit

Hoodit is a Robinhood Chain trading assistant for token and launchpad discovery, market research, exit checks, and user-authorized trades. Canonical Stock Token trading remains available through the host's resolver and execution flow.

## Repository layout

- `app/` and `public/` — Next.js landing page and product UI
- `apps/hoodit/` — Rust v1.5 dynamic application loaded by Aomi: `src/tools/`
  holds the five read tools, `src/market/` decodes swaps into charts and flow,
  `src/providers/` talks to the public data sources
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

The workspace pins `aomi-sdk = "=5.1.1"`, matching the Aomi backend runtime. Every data source is public and keyless, so the app declares no secrets:

| Source | Used for |
|---|---|
| Robinhood Chain RPC | Charts, order flow and trading wallets, decoded from swap logs (Uniswap v2/v3/v4, PancakeSwap v3, Pons curves). Wide log ranges use the official endpoint; cheap batched reads use a faster public endpoint with the official one as fallback. |
| DexScreener | Pool snapshots: price, liquidity, FDV, volume and buy/sell counts by window, pool age, search. |
| GeckoTerminal | Discovery feeds and launchpad stage only, because its shared public allowance is about ten requests a minute. |
| GoPlus | Honeypot simulation, taxes, owner powers, and labelled top holders. |
| LI.FI | Read-only exit quotes at the trade's slippage tolerance. |

Wallet balances come from the host's `get_erc20_holdings`, and trades use the host's execution flow.

```bash
cargo test -p hoodit --lib
cargo clippy -p hoodit --lib --tests -- -D warnings
aomi-build sdk check --path . --required-version 5.1.1
```

The app exposes one skill, `hoodit/research`, with five read tools:
`hoodit_discover` (trending, new, Pons launchpad and volume feeds with setup
flags), `hoodit_search`, `hoodit_get_token` (snapshot, launchpad stage,
security), `hoodit_get_chart` (candles, structure, order flow and wallets from
on-chain swaps) and `hoodit_check_exit`. The tools compute setup flags such as
`extended`, `fading`, `churn` and `thin_exit`, so the model weighs a blow-off
top as late rather than reading momentum as quality.

Trades fill a minute or two after the host simulates them, so `hoodit_get_token`
and `hoodit_check_exit` size a slippage tolerance for that wait: one typical
5-minute move (from DexScreener's 5m and 1h changes), floored by pool depth
(0.5% at $250k+ liquidity up to 3% for thin pools and Pons curves). Hoodit
suggests at most 5%, uses up to 10% only when the user explicitly agrees, and
flags anything needing more as `too_volatile`. The skill tells the model to pass
`slippage_bps` explicitly to the host's LI.FI tools, re-quote at the same
tolerance after a slippage failure, and never widen it unasked.

A live read-only probe runs any tool against public providers:

```bash
HOODIT_TOOL=hoodit_get_chart HOODIT_ARGS='{"token":"0x..."}' \
  cargo test -p hoodit --test live_read_smoke -- --ignored --nocapture
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
