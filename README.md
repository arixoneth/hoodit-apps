# Hoodit

Hoodit is a Robinhood Chain trading assistant for token discovery, market research, wallet reads, and user-authorized trades. Canonical Stock Token trading remains available through the host's resolver and execution flow.

## Repository layout

- `app/` and `public/` — Next.js landing page and product UI
- `apps/hoodit/` — Rust v1.2 dynamic application loaded by Aomi; `src/tools.rs`
  is the public facade, while `src/tools/markets/` and `src/tools/portfolio/`
  own the market and wallet reads respectively
- `contracts/hoodit-v1/` — canonical JSON Schemas, examples, and independent validator
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

The workspace pins `aomi-sdk = "=5.1.0"`, matching the Aomi backend runtime. GeckoTerminal market data, GoPlus security evidence, CoinGecko native-asset pricing, and LI.FI read-only sample quotes use public keyless APIs. Wallet reads use a free Blockscout key configured only in Hoodit's Builder Environment. Provider credentials are delivered by the host and never exposed as tool arguments, requested from end users, or handled by the frontend relay.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
aomi-build sdk check --path . --required-version 5.1.0
```

The deterministic application scenario lives at `apps/hoodit/test.json`.
The app owns two skills: `hoodit/markets` with seven read tools and
`hoodit/portfolio` with two read tools. All nine are hidden until their owning
skill is activated. Actual swaps use the inherited host execution lifecycle.

The amended public schemas and synthetic fixtures live in
`contracts/hoodit-v1/`. Validate them with
`python3 contracts/hoodit-v1/validate_contracts.py`.
CI also runs all nine tools against a deterministic local provider transport,
emits their actual Rust JSON, and validates those envelopes with
`--implementation-fixtures`. These are source-level read tests; they do not
claim a deployment or a completed wallet transaction.

## Connect to Aomi Build

In **Deployments → New app → Connect an existing repository**, enter the fork
that Aomi Build should read. For this checkout, use:

```text
arixoneth/hoodit-apps
```

The root Project manifest selects the `community` platform and publishes `apps/hoodit/aomi.toml`. Commit and push changes before importing so Build can read the same revision.
