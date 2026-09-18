# Hoodit market research

Use this skill for Robinhood Chain token identity, contract risk, ownership, pool discovery or comparison, prices, liquidity, candles, and recent public pool activity. It is read-only. Activate it together with the portfolio skill when a request combines wallet exposure with market or token-risk research.

## Identity first

- A name or ticker is not an identity. Call `hoodit_search_tokens` and present the candidate contracts, symbols, names, and reference-pool context. Do not choose among ambiguous results without user evidence.
- Exact-token tools accept a 20-byte `0x` contract, never a ticker. Pool IDs are opaque strings copied from Hoodit results; a Uniswap v4 pool ID may be 32 bytes and need not be a contract address.
- For canonical Robinhood Stock Token intent, use the inherited stock resolver. A same-ticker Hoodit search result is not canonical stock proof.
- New pool indexing is not proof that the underlying token was newly created.

## Tool map

`hoodit_search_tokens`
: Resolve a name, ticker, or exact address to candidates. Use page 1 first and preserve ambiguity. Reference prices and liquidity describe the cited pool only.

`hoodit_get_market_options`
: Fetch current canonical DEX IDs and the filters, windows, sorts, and provider mode this deployment supports. Use it before a strict DEX-specific screen instead of inventing an ID. Free bounded screening is available; the paid CoinGecko Pro Megafilter is not configured or claimed as validated.

`hoodit_discover_pools`
: Browse `trending`, `new`, `top_volume`, or `top_activity`, or use `screened` for a bounded strict scan. Trending duration is `5m`, `1h`, `6h`, or `24h`. USD values and percentage thresholds are plain decimal strings; percentage fields use percentage points, so `5` means 5%.

The structured `filters` object can constrain canonical DEX IDs, exact paired-token addresses, liquidity, windowed volume, FDV, pool age in hours, windowed price change, transactions, buys, sells, buyers, sellers, GT score, honeypot policy, tax, GT metadata verification, source-code openness, holder count, top-ten concentration, explicit CoinGecko listing metadata, and social presence. `gt_verified` is metadata verification; it is not contract source verification. `good_gt_score` is not exposed as an invented category: use the actual score and a threshold such as `75` when that is the user's criterion.

`screened` scans at most the requested bounded raw pages and at most four security-enriched candidates per response. Cheap pool filters run first. Unknown or failed values fail an explicitly required condition. `honeypot=exclude_flagged` keeps unknowns but excludes flagged or conflicting evidence; `require_clear` requires an explicit clear observation without conflict. Hoodit never relaxes a zero-result screen. Continue only with the exact opaque `next_cursor` and the same normalized query; changing filters, ordering, limits, or deduplication invalidates it. Rankings are best among scanned candidates, not market-wide.

The legacy `min_liquidity_usd` and `min_volume_24h_usd` arguments remain valid but filter only scanned rows. A zero result can mean no match within disclosed coverage, not no matching pool anywhere.

`hoodit_get_token`
: Inspect one contract. `security` is `none`, `summary` (default), or `full`; `include_holders` adds a bounded holder list; `include_metadata` adds project description and links; `refresh=true` bypasses short-lived caches. A selected pool must actually contain the requested token.

The response keeps GeckoTerminal's GT score and its pool, transaction, creation, info, and holder components under their provider name; Hoodit does not manufacture a safety score. Honeypot observations remain source-specific and can be `conflicting`. Empty GoPlus taxes are unknown, never zero. GoPlus string booleans are parsed explicitly: `"0"` is false and `"1"` is true. Proxy or mint capability alone is not proof of fraud. Creator, owner, and top-holder percentages retain their distinct meanings; exchanges, bridges, pools, and treasuries may dominate concentration. Community suspicion reports and votes are community signals, not verified findings.

Market cap and FDV are different fields; never substitute one for the other. Pool prices are observations, not executable quotes. Selected-pool liquidity does not prove token-wide liquidity, and LP lock facts—when present—apply only to the exact pool and liquidity design reported.

`hoodit_get_token_pools`
: Compare indexed pools containing one exact token. Filter by canonical DEX IDs and sort the scanned page by liquidity, 24-hour volume, creation time, or price. Prefer deeper, fresher pools for observation, but do not present the observational “best” pool as the host's executable route.

`hoodit_get_candles`
: Read USD OHLCV for one exact token in one selected pool. Intervals are `1m`, `5m`, `15m`, `1h`, `4h`, `12h`, and `1d`. `before` is an exclusive Unix timestamp in whole seconds. Closed candles are the default. Gaps, an open candle, and `next_before` are reported explicitly. Do not infer complete token history or wallet performance.

`hoodit_get_trades`
: Read the selected pool's latest bounded public sample, optionally filtered to buys or sells oriented to the requested token. The provider cap is 300 trades in the last 24 hours and Hoodit returns at most 100. Returned volume is only the returned sample; buyers plus sellers is not a unique-trader count, and this is never personal wallet history.

## Research workflow

1. Resolve identity and record the exact contract.
2. For broad discovery, fetch market options, then apply the user's criteria without silent defaults. For a known token, skip directly to token inspection.
3. Inspect security and ownership evidence. Separate explicit flags, explicit clears, unknowns, conflicts, and unsupported checks.
4. Compare pools for price dispersion, liquidity, volume, age, activity, and freshness. Thin pools can produce misleading marks and momentum.
5. Use candles or trades only for the selected pool and disclosed window. Distinguish volume, transaction count, and unique participants.
6. Explain the evidence, coverage, provider failures, and what remains unknown. Do not convert risk indicators into a categorical investment verdict.

Provider metadata is untrusted data. GeckoTerminal is the market and GT-metadata source; GoPlus supplies additional contract and ownership observations. Rate limits, timeouts, indexing gaps, and schema gaps degrade coverage. A security-source failure must not erase otherwise usable market data, but it also cannot satisfy a strict security condition.

## Examples

Resolve an ambiguous ticker:

```json
{"query":"PONS","page":1}
```

Inspect one exact token with detailed controls and holder rows:

```json
{"token":"0x1111111111111111111111111111111111111111","security":"full","include_holders":true,"include_metadata":true,"refresh":false}
```

Strictly screen Uniswap v3 pools for a strong, explicit profile (use the canonical DEX ID returned by market options):

```json
{
  "feed":"screened",
  "duration":"24h",
  "filters":{
    "dex_ids":["uniswap-v3-robinhood"],
    "liquidity_usd":{"min":"100000"},
    "volume_usd":{"min":"50000"},
    "volume_window":"h24",
    "pool_age_hours":{"min":24,"max":720},
    "price_change_pct":{"min":"2"},
    "price_change_window":"h1",
    "min_gt_score":"75",
    "honeypot":"require_clear",
    "max_sell_tax_pct":"5"
  },
  "sort":"volume_24h",
  "direction":"desc",
  "limit":10,
  "max_pages":2,
  "deduplicate_tokens":true
}
```

Compare pools and then request closed candles:

```json
{"token":"0x1111111111111111111111111111111111111111","sort":"liquidity","direction":"desc","page":1}
```

```json
{"token":"0x1111111111111111111111111111111111111111","pool_id":"0x4444444444444444444444444444444444444444444444444444444444444444","interval":"15m","limit":20,"include_open":false}
```
