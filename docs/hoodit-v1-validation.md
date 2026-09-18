# Hoodit v1.2 validation record

Validated on 2026-09-18. This record separates deterministic source checks,
direct read-only provider checks, and deployment acceptance. None of these
checks prepared, signed, broadcast, or simulated a wallet transaction.

## Source and contract checks

Run from the repository root:

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
aomi-build sdk check --path .
aomi-build compile --app hoodit --release
aomi-build manifest --lib plugins/hoodit.so
python3 contracts/hoodit-v1/validate_contracts.py

fixture_dir=$(mktemp -d)
HOODIT_CONTRACT_FIXTURE_DIR="$fixture_dir" \
  cargo test --test contracts emits_one_success_envelope_for_every_tool
python3 contracts/hoodit-v1/validate_contracts.py \
  --implementation-fixtures "$fixture_dir"
```

Results:

- Rust: 27 unit tests and three contract tests passed; the live-provider test
  remains explicitly ignored during ordinary test runs. Formatting and clippy
  with warnings denied passed.
- SDK: the live staging manager requires `aomi-sdk` 5.1.0; `Cargo.toml` is
  exactly pinned to 5.1.0 and `Cargo.lock` matches.
- Build: the optimized dynamic library compiled, and its generated manifest
  reports Hoodit 1.2.0, exactly nine skill-owned tools, two skills, and no
  user-declared secret slots.
- Canonical contract validator: 83 schemas and 95 synthetic fixture assertions
  passed across all nine tools.
- Rust-emitted fixtures: 33 input/output assertions passed across all nine
  tools. These use deterministic local provider responses and make no live
  network calls.
- Strict input coverage includes closed top-level and nested objects, bounded
  pages and result counts, exact addresses, opaque pool identifiers, signed
  price-change ranges, non-negative market ranges, query-bound cursors, and
  omission-equivalent optional nulls. The trade-side default is explicitly
  `both`, matching runtime behavior.

The managed Aomi workspace runner expects a product-mono-style `aomi/`
directory and cannot orchestrate this standalone community repository. The
documented raw Cargo commands above are the repository-appropriate fallback;
the SDK, optimized plugin, generated manifest, and emitted contract fixtures
are still checked independently.

## Sanitized live provider checks

The ignored Rust smoke was run directly against the public providers. Outputs
were written to temporary local files and summarized without credentials or raw
wallet data.

- `hoodit_get_token` returned `status=ok`, schema `1.2.0`, an exact selected
  pool, eight alternate pools, full security coverage, and holder evidence.
  Its observed sources were GeckoTerminal and GoPlus.
- `hoodit_get_market_options` returned 42 current DEX choices.
- `hoodit_get_token_pools` returned 20 pools with provider-page coverage and
  explicit `executable_route=false`.
- A strict `screened` discovery read scanned one 20-row page, enriched the
  disclosed maximum of four candidates, returned one result, did not relax any
  filter, and stopped at the scan bound. The partial status and bound warnings
  are expected completeness signals, not a silent filter relaxation.

The local shell did not contain `HOODIT_BLOCKSCOUT_API_KEY`, so no local live
portfolio probe was claimed in this validation pass. Wallet coverage must be
proved after deployment using the host-managed secret slot; secret presence is
checked without reading or printing its value.

Provider reads prove shape and availability only at the recorded time. They do
not prove future uptime, market-wide ranking, complete wallet history,
execution-route quality, or successful transaction broadcast.

## Deployment acceptance

Local source success is not deployment success. Release acceptance requires an
immutable pushed source commit, successful Project preflight and candidate CI,
the expected release asset and digest, host-managed Blockscout secret presence,
promotion of the exact release, artifact-ready/runtime-loaded state, a settled
no-tool chat, and a read-only Hoodit tool turn on the deployed application.

Record the deployment id, release tag, active application id, artifact digest,
and smoke evidence here only after those checks complete. A candidate pull
request or initial HTTP 200 is insufficient by itself.
