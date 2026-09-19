// @ts-check
/**
 * Pure helpers for activity receipts (#94). The server's proactive run record
 * is the only source: trigger, stated reason, target, status, approvals, and
 * a receipt of actions. No model text ever reaches this module.
 */

/** @typedef {{ kind: string; [key: string]: unknown }} Tagged */
/**
 * @typedef {{
 *   version: number;
 *   id: string;
 *   dedupe_key: string;
 *   trigger: Tagged;
 *   reason: string;
 *   target: Tagged;
 *   status: Tagged;
 *   attempt: number;
 *   started_at: number;
 *   finished_at?: number;
 *   approvals: { side_effect: string; allowed: boolean; reason?: string; at: number }[];
 *   outcome?: { actions: { tool: string; summary: string }[]; messages_sent: number; tokens: number } | null;
 * }} ProactiveRun
 */

/** @param {Tagged} trigger */
export function triggerLabel(trigger) {
	switch (trigger.kind) {
		case "heartbeat":
			return trigger.agent === "reflection" ? "Reflection" : "Check-in";
		case "schedule":
			return "Scheduled";
		case "machine_connected":
			return `Computer connected${trigger.machine_id ? ` · ${trigger.machine_id}` : ""}`;
		case "manual":
			return "Run by you";
		case "commitment":
			return "Commitment";
		case "handoff":
			return "Handoff";
		default:
			return "Activity";
	}
}

/** @param {Tagged} status */
export function statusLabel(status) {
	switch (status.kind) {
		case "running":
			return "Running";
		case "completed":
			return "Done";
		case "cancelled":
			return "Cancelled";
		case "failed":
			return status.retryable ? "Failed, can retry" : "Failed";
		case "skipped": {
			const reason = /** @type {Tagged | undefined} */ (status.reason);
			switch (reason?.kind) {
				case "quiet_hours":
					return "Held for quiet hours";
				case "cooldown":
					return "Held, too soon";
				case "duplicate":
					return "Already running";
				case "disabled":
					return "Initiative is off";
				default:
					return "Skipped";
			}
		}
		default:
			return "Unknown";
	}
}

/** @param {ProactiveRun} run */
export function canCancel(run) {
	return run.status.kind === "running";
}

/** @param {ProactiveRun} run */
export function canRetry(run) {
	return run.status.kind === "cancelled" || (run.status.kind === "failed" && run.status.retryable === true);
}

/**
 * Human summary of what a run did, from receipts only.
 * @param {ProactiveRun} run
 */
export function outcomeSummary(run) {
	const outcome = run.outcome;
	if (!outcome) return "";
	const parts = [];
	if (outcome.messages_sent > 0) parts.push(outcome.messages_sent === 1 ? "sent you a message" : `sent you ${outcome.messages_sent} messages`);
	const other = outcome.actions.filter((a) => a.tool !== "reach_out").length;
	if (other > 0) parts.push(other === 1 ? "1 action" : `${other} actions`);
	if (parts.length === 0) return "nothing to do";
	return parts.join(", ");
}

/**
 * Insert or replace a run, keeping newest first by start time.
 * @param {ProactiveRun[]} runs
 * @param {ProactiveRun} run
 */
export function upsertRun(runs, run) {
	const next = runs.filter((r) => r.id !== run.id);
	next.push(run);
	next.sort((a, b) => b.started_at - a.started_at || (b.id < a.id ? -1 : 1));
	return next;
}

/**
 * @param {number} unixSeconds
 * @param {number} nowSeconds
 */
export function relativeTime(unixSeconds, nowSeconds) {
	const diff = Math.max(0, nowSeconds - unixSeconds);
	if (diff < 60) return "just now";
	if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
	if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
	if (diff < 7 * 86400) return `${Math.floor(diff / 86400)}d ago`;
	return new Date(unixSeconds * 1000).toLocaleDateString([], { month: "short", day: "numeric" });
}
