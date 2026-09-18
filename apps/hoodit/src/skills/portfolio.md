# Hoodit portfolio analysis

Use this skill for Robinhood Chain wallet inventory, exact balances, market or quote-sample valuation, exposure and concentration, compact token-risk context, and exact fractional sell sizing. It reads public chain data and never stages, signs, or broadcasts. Activate the market skill too when the request needs broader token or pool research.

## Wallet identity

- For an explicit public address, use that exact 20-byte `0x` address.
- For “my wallet,” first use the inherited account-information capability on chain `4663`. Prefer the funded executor when the host reports one; do not assume a display, signer, or smart-account address holds the assets.
- Native ETH is `token="native"`. Wrapped ETH is a distinct ERC-20 contract and must not be merged with native ETH.

## Inventory and valuation

Call `hoodit_get_portfolio` for a wallet inventory page. The first-page `cursor` is JSON null or omitted when the client permits omission. Later pages must use the exact opaque `next_cursor` from the preceding response for the same wallet. It is bound to that wallet and traversal; never invent, edit, or reuse it for another query.

Each balance preserves exact atomic integer units and, when decimals are known, a formatted decimal amount. Native ETH is included only on the first page. A response covers the whole wallet only when it began on the first page, native balance was established, and no continuation remains. Otherwise summaries and allocations are page-scoped. Do not extrapolate a page into a complete-wallet total.

`valuation` modes:

- `none`: balances only; fastest and default. Legacy `include_quotes=false` maps here.
- `market`: observed USD marks from public market sources. Native ETH is mapped to the chain's native ETH asset and priced as ETH; ERC-20s use Robinhood Chain token marks. Prices are not executable.
- `quote_sample`: bounded LI.FI read quotes into USDG. Legacy `include_quotes=true` maps here. The default sample is 1% of a holding, floor-sized in atomic units, then extrapolated. This is not guaranteed full-liquidation proceeds.

USD and USDG are separate units. USDG's unit value of `1` in a USDG-denominated quote does not establish a USD peg. Never add USD and USDG values or relabel one as the other.

Use `sort=value_desc` for exposures or `sort=symbol` for inventory. `min_value_usd` applies only to observed USD values. Unpriced assets remain visible by default; `include_unpriced=false` is an explicit display filter and the response reports how many holdings it removed. Allocation percentages use the disclosed denominator `displayed_priced_holdings`, never missing or unpriced assets. `priced_value_*` is a subtotal; `total_value_*` is emitted only for complete-wallet coverage with no unpriced assets.

`security=summary` enriches at most four ERC-20 holdings per response with source-labelled GT and GoPlus facts, and adds selected-pool liquidity context to a bounded subset. The position-to-pool ratio is a rough size/context heuristic, not slippage, depth at each tick, or an executable route. Native ETH does not have ERC-20 contract-security fields. Use the market skill for full controls, holder rows, conflicting evidence, or additional pools.

## Exact holding and sell sizing

Call `hoodit_get_holding` for one exact ERC-20 contract or `native`. It refreshes by default. `quote_balance_bps` is the share of the balance in basis points: `1 = 0.01%`, `100 = 1%`, `5000 = 50%`, and `10000 = 100%`. Sizing floors in atomic units, so tiny fractions can become zero. `include_quote=true` requests a read-only USDG estimate for exactly that sized amount. `security=summary` optionally attaches compact token-risk and selected-pool liquidity context.

For a user-authorized fractional sale:

1. Resolve the funded executor on chain 4663.
2. Call `hoodit_get_holding` with the exact token and requested basis points, usually with `include_quote=false` when only exact sizing is needed.
3. Read `sell_amount.atomic` and `sell_amount.formatted`; do not recompute with floating point.
4. Route execution to the inherited same-chain swap skill using the exact contract and formatted amount. Preserve the host's preparation, staging, signing, and commit policy.
5. Do not claim completion without the host's verified transaction or receipt state.

Canonical Robinhood Stock Token intent must first use the inherited stock resolver. A ticker match from general market search is not sufficient. Hoodit's portfolio tools never grant trade authorization.

## Interpretation

- Large allocation can indicate concentration, but the denominator and missing prices matter.
- A high position-to-pool-liquidity ratio is a warning to obtain an executable quote; it is not a slippage prediction.
- Contract flags, proxy status, holder concentration, and creator ownership need context. Unknown is not clear, and a provider conflict must remain visible.
- Missing decimals preserve the raw balance but prevent formatted valuation. Missing routes or prices keep the asset visible unless the user explicitly filters it out.
- Cost basis, realized or unrealized P&L, historical equity curves, and tax lots are not available from current inventory reads. Do not invent them.

## Examples

Balance-only first page:

```json
{"wallet_address":"0x3333333333333333333333333333333333333333","cursor":null,"valuation":"none","security":"none","sort":"symbol","refresh":false}
```

USD exposure and bounded security context, keeping unpriced assets:

```json
{"wallet_address":"0x3333333333333333333333333333333333333333","cursor":null,"valuation":"market","security":"summary","sort":"value_desc","include_unpriced":true,"refresh":false}
```

Exact half-balance sell sizing without a quote:

```json
{"wallet_address":"0x3333333333333333333333333333333333333333","token":"0x1111111111111111111111111111111111111111","quote_balance_bps":5000,"include_quote":false,"security":"none","refresh":true}
```

Read-only 5% quote sample with compact risk context:

```json
{"wallet_address":"0x3333333333333333333333333333333333333333","token":"0x1111111111111111111111111111111111111111","quote_balance_bps":500,"include_quote":true,"security":"summary","refresh":true}
```
