# Behavioral metrics

Nolune does not ship an engagement dashboard (#95). The former Stats tab with
streaks, contribution heatmap, peak-hour and weekday charts, mood distribution,
message-length and response-interval widgets is gone, together with the
per-day `stats/*.json` aggregate store that fed it. Leftover `stats/`
directories are removed once at startup.

One behavioral metric remains because it changes what the companion does.

## Interaction rhythm

| | |
| --- | --- |
| Store | `instances/companion/rhythm.json`, format version 2 |
| Contents | message count per hour of day and per weekday, total messages and characters, average message length, average within-session response interval, timestamp of the last user message |
| Updated | incrementally, one user message at a time; no history rescans |
| Consumer | the heartbeat companion loop, which turns the aggregate into short prompt hints such as "usually most active around 9:00" or "responding slower than usual" |
| Not stored | per-message timestamps, per-day counts, message text, streaks |

### Retention

The aggregate is bounded by construction: fixed-size histograms and running
totals. The only per-event datum is `last_message_at`, kept to measure the
next within-session interval. Nothing else accumulates, so there is no time
based retention window to configure. Deleting `rhythm.json` resets it.

### Opt out

Rhythm tracking is on by default. You can opt out per companion:

- Settings → Companion → "Learn my rhythm".
- API: `GET /api/instances/companion/rhythm` returns `{"enabled": true|false}`;
  `PUT` with `{"enabled": false}` stops recording **and deletes**
  `rhythm.json`. Turning it back on starts from an empty aggregate.
- Config: `rhythm_tracking = false` in `instances/companion/instance.toml`.

While off, the heartbeat receives no rhythm hints.

## Operational metrics that are not behavioral

Per-chat context statistics (prompt, tool, and history token estimates) remain
available from the chat toolbar. They describe the request being built, not
the user, and are computed on demand without persistence.
