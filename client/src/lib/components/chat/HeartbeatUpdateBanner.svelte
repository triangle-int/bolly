<script lang="ts">
	import { applyHeartbeatUpdate, dismissHeartbeatUpdate, fetchHeartbeatUpdates } from "$lib/api/client.js";
	import type { HeartbeatUpdate } from "$lib/api/types.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import CreatureBubble from "./CreatureBubble.svelte";

	const toast = getToasts();

	let { slug }: { slug: string } = $props();

	let updates = $state<HeartbeatUpdate[]>([]);
	let expanded = $state<string | null>(null);
	let applying = $state(false);

	$effect(() => {
		fetchHeartbeatUpdates(slug)
			.then((u) => (updates = u))
			.catch(() => {});
	});

	async function apply(id: string) {
		applying = true;
		try {
			await applyHeartbeatUpdate(slug, id);
			updates = updates.filter((u) => u.id !== id);
			toast.success("heartbeat updated");
		} catch {
			toast.error("failed to apply update");
		}
		applying = false;
	}

	async function dismiss(id: string) {
		updates = updates.filter((u) => u.id !== id);
		try {
			await dismissHeartbeatUpdate(slug, id);
		} catch {}
	}
</script>

{#each updates as update (update.id)}
	<CreatureBubble ondismiss={() => dismiss(update.id)}>
		<div class="hb-inner">
			<span class="hb-label">Heartbeat update</span>
			<span class="hb-desc">{update.description}</span>
			<div class="hb-actions">
				<button class="hb-toggle" onclick={() => (expanded = expanded === update.id ? null : update.id)}>
					{expanded === update.id ? "Hide preview" : "Preview"}
				</button>
				<button class="hb-apply" disabled={applying} onclick={() => apply(update.id)}>Apply</button>
			</div>
			{#if expanded === update.id}
				<pre class="hb-preview">{update.preview}</pre>
			{/if}
		</div>
	</CreatureBubble>
{/each}

<style>
	.hb-inner {
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
	}

	.hb-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.06em;
		text-transform: none;
		color: var(--text-secondary);
	}

	.hb-desc {
		font-size: 0.8125rem;
		color: var(--text-secondary);
		line-height: 1.35;
	}

	.hb-actions {
		display: flex;
		gap: 0.375rem;
		margin-top: 0.25rem;
	}

	.hb-toggle, .hb-apply {
		padding: 0.15rem 0.5rem;
		border-radius: 0.25rem;
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.04em;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.hb-toggle {
		background: none;
		border: 1px solid var(--border);
		color: var(--text-secondary);
	}
	.hb-toggle:hover {
		border-color: var(--border);
		color: var(--text-secondary);
	}

	.hb-apply {
		background: var(--card);
		border: 1px solid var(--border);
		color: var(--text-secondary);
	}
	.hb-apply:hover:not(:disabled) {
		background: var(--card);
	}
	.hb-apply:disabled {
		opacity: 0.4;
		cursor: default;
	}

	.hb-preview {
		margin: 0.25rem 0 0;
		padding: 0.5rem;
		border-radius: 0.375rem;
		background: var(--card);
		font-family: var(--font-body);
		font-size: 0.8125rem;
		line-height: 1.5;
		color: var(--text-secondary);
		white-space: pre-wrap;
		word-break: break-word;
	}

 .hb-inner {gap:8px;min-width:0}
 .hb-label {color:var(--primary);letter-spacing:0}
 .hb-desc {font-size:14px;line-height:1.5}
 .hb-actions {flex-wrap:wrap}
 .hb-toggle,.hb-apply {min-height:44px;padding:8px 12px;border-radius:8px;font-size:14px;letter-spacing:0}
 .hb-apply,.hb-apply:hover:not(:disabled) {background:var(--primary);color:var(--primary-foreground);border-color:var(--primary)}
 .hb-preview {font-family:var(--font-mono);background:var(--background);font-size:12px}

</style>
