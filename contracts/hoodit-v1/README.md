# Hoodit v1.4.0 tool contracts

This bundle is the canonical public contract for Hoodit’s eight read-only tools on Robinhood Chain (`4663`), all owned by the `hoodit/markets` skill. Market reads use GeckoTerminal (or CoinGecko's keyed onchain API when an operator key is configured), security enrichment uses GeckoTerminal and GoPlus, and exit checks use read-only LI.FI quotes. Wallet balances come from the Aomi host's `get_erc20_holdings`; actual swaps remain in the inherited host execution flow.

## Amendments in 1.4.0

- `hoodit_get_portfolio`, `hoodit_get_holding`, and the Blockscout provider are removed; the host's holdings tool covers Robinhood Chain wallets.
- `hoodit_get_token` adds `lifecycle`: launchpad curve progress, graduation time, and destination pool. `not_reported` is not 0% bonded. Graduated tokens default to their destination pool.
- Discovery pools carry the same lifecycle when it could be read.
- Trades keep `sender` and `block_number`; the summary counts distinct senders and ranks the top three buyers and sellers in the returned sample.
- `hoodit_check_exit` quotes an exact-size sell or a buy-then-sell round trip. It is never executable and never evaluates balances.

## Amendments from 1.0.0

- `blockscout` replaces `etherscan`; no paid data subscription is required.
- `hoodit_get_portfolio` accepts an opaque `cursor` instead of `page`/`page_size`, consumes one Blockscout provider page, and includes native ETH only initially. Its first-page cursor is nullable because strict model tool schemas require every declared property; later pages accept only an exact returned continuation.
- Portfolio reads default to balances only (`include_quotes=false`). Requested valuation is limited to 20 non-USDG quote attempts and a 30-second overall deadline. Unscheduled holdings remain visible with `budget_exhausted` or `deadline_exceeded`.
- A response contains at most 50 ERC-20 rows plus initial native ETH. Complete-wallet totals require an initial page with no continuation and a successful native read.

## Amendments in 1.3.0

- Discovery now exposes verified market-cap and observed-price filters, independent transaction/buy/sell windows, richer sorting, selectable screened source feeds, and resumable cursors on every feed.
- Cursors preserve unprocessed rows when a result or conditional security-enrichment limit is reached. Responses distinguish raw scanned rows from security-enriched candidates.
- Pool normalization retains provider FDV and verified market-cap fields. Unknown market cap is never replaced with FDV.

## Amendments in 1.2.0

- `hoodit_get_token_pools` compares pools for one exact token contract and `hoodit_get_market_options` returns current canonical DEX choices and supported discovery dimensions.
- `hoodit_get_token` supports explicit `none`, `summary`, and `full` security levels, optional holder evidence, source disagreement, tri-state contract controls, and exact-pool liquidity-lock evidence.
- `hoodit_discover_pools` supports a bounded strict `screened` mode with structured market, activity, metadata, security, ownership, and concentration filters. Unknown required values fail the filter; no constraint is silently relaxed.
- Portfolio valuation separates keyless USD market estimates from USDG quote samples. Allocation percentages use only the disclosed priced USD denominator, and unpriced holdings remain visible by default.
- Every envelope uses schema version `1.2.0`; provider provenance now includes `goplus` and `coingecko`.

`examples.json` includes synthetic continuation, 50-row, quote-budget, and wrong-wallet cursor cases.

## Validate

```bash
python3 validate_contracts.py
```

Validation covers schemas, fixtures, and documentation consistency. It does not call providers, compile Rust, validate host skill gating, or execute trades.

To validate serialized Rust results independently from Rust's own tests, dump
`{ "tool": "...", "input": {...}, "output": {...} }` cases to a directory and run:

```bash
python3 validate_contracts.py --implementation-fixtures path/to/generated-contract-fixtures
```

This mode requires valid emitted outputs for all nine tools. It catches wire
drift such as omitted required nulls, stale schema versions, invalid warning
objects, and response fields that Rust's compile-time types do not constrain.
