# Proactive loop

Everything the companion starts on its own passes through one loop (#92):
hourly check-ins, explicit schedules, connected-computer events, manual
triggers, and the future commitment (#85) and handoff (#82) triggers. There is
one execution record, one policy, and one place where duplicates, quiet hours,
cooldowns, and the attention budget are enforced.

## Execution record

Each run is one JSON file under `instances/companion/activity/{id}.json`,
format version 1:

| Field | Meaning |
| --- | --- |
| `id` | `run_<unix seconds>_<8 hex>` |
| `trigger` | `heartbeat`, `schedule`, `machine_connected`, `manual`, `commitment`, `handoff` with their identifying field |
| `reason` | stated, user-readable reason, at most 200 characters |
| `target` | `companion`, `chat` (with `chat_id`), or `machine` (with `machine_id`) |
| `dedupe_key` | derived from the trigger; runs sharing a key never execute concurrently |
| `status` | `running`, `completed`, `failed` (`error`, `retryable`), `cancelled`, or `skipped` (`quiet_hours`, `cooldown`, `duplicate`, `disabled`) |
| `attempt`, `retry_of` | attempt number and the id this attempt retries |
| `approvals` | each side-effect decision: `reach_out`, allowed or not, reason, time |
| `outcome` | receipts only: tool names with short summaries, `messages_sent`, `tokens` |

Records never contain model text, hidden reasoning, or tool traces. Skips are
recorded too, so a quiet or rate-limited period stays explainable.

## Lifecycle

1. A trigger calls `begin`. The loop admits or skips it under the policy:
   disabled → `skipped/disabled`; same `dedupe_key` already running →
   `skipped/duplicate`; spontaneous trigger inside quiet hours →
   `skipped/quiet_hours`; event trigger within `cooldown_secs` of its last
   finish → `skipped/cooldown`.
2. The worker holds a handle with a cancellation token and ends the run with
   exactly one of `complete`, `fail`, or `cancel`.
3. Side effects that leave companion storage (today: `reach_out`) call
   `approve_side_effect`, which denies during quiet hours or once the rolling
   24-hour `daily_reach_out_budget` is spent, and records the decision on the
   run. Denials are returned to the model as tool errors.
4. On startup, runs left `running` by a dead process are marked
   `failed` (`interrupted by server restart`, retryable), then retention
   removes finished runs beyond `retention_max` or older than
   `retention_days`. Running runs are never removed.

Explicit schedules ignore quiet hours and cooldown (the user or companion
asked for that time) but still cannot message the user during quiet hours.

## Policy

`instances/companion/proactive_policy.json`, editable through
`GET/PUT /api/instances/companion/proactive`:

| Field | Default | Effect |
| --- | --- | --- |
| `enabled` | `true` | master switch for spontaneous behavior |
| `quiet_hours` | none | `{start_hour, end_hour}` in the companion's timezone; may wrap midnight |
| `cooldown_secs` | 600 | minimum gap between runs of the same event trigger |
| `daily_reach_out_budget` | 6 | spontaneous messages per rolling 24 hours (the attention budget) |
| `retention_max` | 200 | finished records kept |
| `retention_days` | 30 | finished records older than this are removed |
| `check_in_interval_hours` | 1 | hours between check-ins (0.25 to 720) |
| `reflection_enabled` | `false` | opt in to the reflection routine |
| `reflection_interval_hours` | 72 | hours between reflections when enabled |

## API

| Route | Purpose |
| --- | --- |
| `GET /api/instances/companion/activity?limit=` | newest records first |
| `GET /api/instances/companion/activity/{id}` | one record |
| `POST /api/instances/companion/activity/{id}/cancel` | signal a running run; `409` when nothing is running |
| `POST /api/instances/companion/activity/{id}/retry` | new attempt of a retryable failure or a cancelled run; `409` otherwise |
| `GET/PUT /api/instances/companion/proactive` | policy |

## Routines

The companion has two routines (#93). There is no configurable agent set, no
agent TOML, no tool groups, and no `call_agent` tool.

| Routine | Trigger `agent` | Default interval | Tools |
| --- | --- | --- | --- |
| Check-in | `companion` | `check_in_interval_hours` = 1 | `reach_out`, `create_drop`, `memory_write`, `memory_read`, `memory_list`, `memory_search`, `read_email` (only with a configured account) |
| Reflection (opt-in) | `reflection` | `reflection_interval_hours` = 72 | `memory_write`, `memory_read`, `memory_list`, `memory_search`, `memory_connect` |

Neither routine can run commands, touch files or connected computers, send
email, or call `memory_forget`; bulk memory rewriting by a background job is
gone with the retired night-maintenance agent. Reflection is off until
`reflection_enabled` is set, and it can add or connect memories but never
delete them. A user-written `instances/companion/heartbeat.md` is appended to
the check-in prompt as guidance.

The next run of a routine is derived from its last finished activity record,
so schedules survive restarts without marker files. A machine-connect event
runs the check-in once with a connection task.

Retired child-agent state (`agents/`, `agent_runs/`) is removed from the
companion directory once at startup; it is never executed.

## Receipts instead of thoughts (#94)

The raw Thoughts store, its route, tab, and `heartbeat_thought` event are
gone. Routines return only their tool trace, which the loop reduces to
receipts; the model's private text is dropped in memory and never written
anywhere. Historical `thoughts/` files are removed from the companion
directory once at startup and cannot influence behavior. Mood remains
bounded companion state in `mood.json`.

The client's **Activity tab** lists the records newest first (trigger, stated
reason, status, receipts, held messages) with cancel and retry, and updates
live from the `activity_updated` server event, which carries the same
record shape as the API. Initiative controls (on/off, check-in interval,
quiet hours, daily message budget, reflection) live under Settings →
Companion.

## Migration hooks

#85 and #82 add the commitment and handoff triggers.
