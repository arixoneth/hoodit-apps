# Hoodit watch

Use when the user wants to hear when something happens to a coin.

1. Read `hoodit_check` for the token once, so the threshold is relative to now and you can quote the current value.
2. Turn the ask into one numeric condition on a `hoodit_check` field: `curve_pct`, `price_usd`, `fdv_usd`, `liquidity_usd`, `holders`, `dev_pct`, `top10_pct`, `snipers_pct`, `buy_usd_5m`, `sell_usd_5m`, `change_1h_pct`, `graduated` (1 or 0). Operators are `>=`, `<=`, `>`, `<` only.
   - "ping me at 80% curve" → curve_pct >= 80
   - "tell me if the dev sells" → dev_pct < (current dev_pct − 0.5)
   - "if it dumps 30%" → price_usd <= current × 0.7
   - "when it graduates" → graduated >= 1
3. Arm it with the host `wake_on_condition`:
   - `condition`: a JSON string, e.g. `{"read":"hoodit_check","args":{"token":"0x…"},"path":"curve_pct","op":">=","value":80}`
   - `intent`: what to do when it fires, standalone: "Hoodit alert: $TICKER (0x…) curve_pct reached 80. Read hoodit_token for 0x… and give the news plus a fresh two-line take. Don't trade."
   - `poll_seconds`: 300 (never below 300). `expires_at`: `as_of` from the check + 86400 unless the user asks for longer.
   - Keep it to 5 active watches per user; each poll costs a data request.
4. Confirm in one line: what you're watching, the current value, the trigger, how long, and that the alert arrives in this chat and on Telegram if linked.

When a watch fires: read `hoodit_token`, then give the news and a fresh take in two lines. Never trade automatically.

`list_scheduled` / `cancel_scheduled` show and stop watches. If arming says the thread needs a signed-in user, tell guests to sign in first.
