# Hoodit trade

Use when the user asks to buy or sell a specific Robinhood Chain coin. Activate the host `lifi_swap` skill too and follow its prepare → simulate → sign flow. These rules come first.

## Before preparing
- Resolve the exact contract and pass the 0x address to the swap tool, never a ticker. If a ticker matches several contracts, ask which one (show the top two with holders and liquidity).
- Read `hoodit_token` unless you have a card from the last ~5 minutes. Don't prepare when `trade_support` isn't `full`, or security shows honeypot, cannot_sell_all or a sell tax over 10%. Say why and offer a smaller size, waiting for graduation, or another coin.
- Never buy what can't be sold. Run `hoodit_exit` round_trip at the user's size first, unless one ran in the last few minutes. Sell leg `no_route`: don't buy. `loss_pct` above 10%: say so and suggest a smaller size before going on.

## Slippage
The fill lands a minute or two after the quote (your reply plus the wallet confirmation), so memecoins need room.
- Always pass `slippage_bps` explicitly: the user's number if given, otherwise 300 (3%) for liquidity over $50k and 500 (5%) below that or on a curve. Above 500 only if the user agrees; never above 1000.
- Say it in a few words ("5% slippage").

## Executing
- Use chain_id 4663 and 0x addresses (native ETH is 0x0000000000000000000000000000000000000000).
- Before the wallet opens, give one line: coin, contract, amount in, expected and minimum out, slippage, route.
- Pair a buy with an exit plan in one line: the level where the idea is wrong (from the chart) and a time stop. Offer a watcher (`hoodit/watch`).
- Minimum output not met: say how far price moved if known, re-quote at the same tolerance and ask before reopening the wallet. Raise the tolerance only if the user asks.
- Gas, balance, approval and stale-simulation failures aren't slippage: name the real cause and the stage (prepare, simulation, wallet, on-chain).
- A trade is done only when the host reports the transaction.
