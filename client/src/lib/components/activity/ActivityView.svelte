<script lang="ts">
	import { cancelActivity, fetchActivity, retryActivity } from "$lib/api/client.js";
	import type { ProactiveRun, ServerEvent } from "$lib/api/types.js";
	import { canCancel, canRetry, outcomeSummary, relativeTime, statusLabel, triggerLabel } from "$lib/activity/receipts.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { upsertRun } from "$lib/activity/receipts.js";

	const toast = getToasts();
	let { slug }: { slug: string } = $props();

	let runs = $state<ProactiveRun[]>([]);
	let loading = $state(true);
	let loadError = $state("");
	let busy = $state<string | null>(null);
	let now = $state(Math.floor(Date.now() / 1000));

	const ws = getWebSocket();

	async function load() {
		loading = true;
		loadError = "";
		try {
			runs = await fetchActivity(slug, 100);
		} catch {
			loadError = "Could not load activity. Please try again.";
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "activity_updated" && event.instance_slug === slug) {
				runs = upsertRun(runs, event.run);
			}
		});
		const tick = setInterval(() => (now = Math.floor(Date.now() / 1000)), 30_000);
		return () => {
			unsub();
			clearInterval(tick);
		};
	});

	async function cancel(run: ProactiveRun) {
		busy = run.id;
		try {
			await cancelActivity(slug, run.id);
		} catch {
			toast.error("Could not cancel that run.");
		} finally {
			busy = null;
		}
	}

	async function retry(run: ProactiveRun) {
		busy = run.id;
		try {
			const next = await retryActivity(slug, run.id);
			runs = upsertRun(runs, next);
		} catch {
			toast.error("Could not retry that run.");
		} finally {
			busy = null;
		}
	}

	function approvalNote(run: ProactiveRun): string {
		const denied = run.approvals.filter((a) => !a.allowed);
		if (denied.length === 0) return "";
		const reason = denied[0].reason ?? "not allowed right now";
		return `Message held: ${reason}`;
	}
</script>

<div class="activity-page">
	<header class="activity-header">
		<h2>Activity</h2>
		<p>What your companion did on its own, why, and what it was allowed to do. Nothing here is private reasoning.</p>
	</header>

	{#if loading}
		<p role="status" class="activity-center">Loading activity…</p>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if runs.length === 0}
		<div class="activity-center">
			<p class="empty-text">Nothing yet</p>
			<p class="empty-sub">Check-ins, scheduled wake-ups, and connected-computer events will show up here.</p>
		</div>
	{:else}
		<ul class="activity-list">
			{#each runs as run (run.id)}
				{@const status = run.status.kind}
				<li class="activity-item" class:activity-running={status === "running"} class:activity-failed={status === "failed"} class:activity-skipped={status === "skipped"}>
					<div class="activity-row">
						<span class="activity-trigger">{triggerLabel(run.trigger)}</span>
						<span class="activity-time">{relativeTime(run.started_at, now)}</span>
					</div>
					<p class="activity-reason">{run.reason}{run.attempt > 1 ? ` · attempt ${run.attempt}` : ""}</p>
					<div class="activity-row">
						<span class="activity-status">{statusLabel(run.status)}</span>
						{#if run.outcome}<span class="activity-outcome">{outcomeSummary(run)}</span>{/if}
					</div>
					{#if status === "failed" && "error" in run.status}
						<p class="activity-error">{String(run.status.error)}</p>
					{/if}
					{#if approvalNote(run)}
						<p class="activity-note">{approvalNote(run)}</p>
					{/if}
					{#if run.outcome && run.outcome.actions.length > 0}
						<ul class="activity-actions">
							{#each run.outcome.actions as action, i (i)}
								<li><span class="activity-tool">{action.tool}</span> {action.summary}</li>
							{/each}
						</ul>
					{/if}
					{#if canCancel(run) || canRetry(run)}
						<div class="activity-controls">
							{#if canCancel(run)}
								<button class="nl-button-secondary" disabled={busy === run.id} onclick={() => cancel(run)}>Cancel</button>
							{/if}
							{#if canRetry(run)}
								<button class="nl-button-secondary" disabled={busy === run.id} onclick={() => retry(run)}>Retry</button>
							{/if}
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	{/if}
</div>

<style>
	.activity-page { height: 100%; overflow-y: auto; padding: 24px 20px 48px; }
	.activity-header { max-width: 720px; margin: 0 auto 24px; }
	.activity-header h2 { font: 400 28px/1.2 var(--font-display); letter-spacing: -0.02em; color: var(--foreground); margin: 0 0 8px; }
	.activity-header p { font: 400 14px/1.6 var(--font-body); color: var(--text-secondary); margin: 0; }
	.activity-center { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; min-height: 220px; text-align: center; color: var(--text-secondary); font: 400 16px/1.6 var(--font-body); }
	.empty-text { color: var(--foreground); }
	.empty-sub { font-size: 14px; }
	.load-error { display: flex; flex-direction: column; align-items: center; gap: 16px; min-height: 220px; justify-content: center; color: var(--text-secondary); text-align: center; }
	.activity-list { list-style: none; margin: 0 auto; padding: 0; max-width: 720px; display: flex; flex-direction: column; gap: 12px; }
	.activity-item { background: var(--card); border: 1px solid var(--border); border-radius: 16px; padding: 16px 20px; display: flex; flex-direction: column; gap: 6px; }
	.activity-running { border-color: var(--primary); }
	.activity-failed { border-color: var(--destructive); }
	.activity-skipped { opacity: 0.8; }
	.activity-row { display: flex; align-items: baseline; justify-content: space-between; gap: 12px; flex-wrap: wrap; }
	.activity-trigger { font: 500 14px var(--font-body); color: var(--foreground); }
	.activity-time { font: 400 12px var(--font-body); color: var(--text-muted); }
	.activity-reason { margin: 0; font: 400 14px/1.5 var(--font-body); color: var(--text-secondary); }
	.activity-status { font: 500 13px var(--font-body); color: var(--primary); }
	.activity-failed .activity-status { color: var(--destructive); }
	.activity-outcome { font: 400 13px var(--font-body); color: var(--text-secondary); }
	.activity-error, .activity-note { margin: 0; font: 400 13px/1.5 var(--font-body); color: var(--text-muted); }
	.activity-error { color: var(--destructive); }
	.activity-actions { list-style: none; margin: 4px 0 0; padding: 0; display: flex; flex-direction: column; gap: 4px; font: 400 13px/1.5 var(--font-body); color: var(--text-secondary); }
	.activity-tool { font: 500 12px var(--font-mono); color: var(--foreground); }
	.activity-controls { display: flex; gap: 8px; margin-top: 8px; }
	@media (max-width: 720px) { .activity-page { padding: 16px 16px 40px; } .activity-item { padding: 14px 16px; } }
</style>
