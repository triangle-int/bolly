<script lang="ts">
	import { deleteDrop, fetchDrops } from "$lib/api/client.js";
	import type { Drop, ServerEvent } from "$lib/api/types.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { play } from "$lib/sounds.js";
	import { hapticDouble } from "$lib/haptics.js";
	import DropCard from "./DropCard.svelte";

	const toast = getToasts();

	let { slug }: { slug: string } = $props();

	let drops = $state<Drop[]>([]);
	let loading = $state(true);
	let loadError = $state("");
	let expandedId = $state<string | null>(null);

	const ws = getWebSocket();

	async function load() {
		loading = true;
		loadError = "";
		try {
			drops = await fetchDrops(slug);
		} catch {
			loadError = "Could not load drops. Please try again.";
			toast.error("failed to load drops");
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();

		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "drop_created" && event.instance_slug === slug) {
				drops = [event.drop, ...drops];
				play("drop_received");
				hapticDouble();
			}
		});

		return unsub;
	});

	async function handleDelete(dropId: string) {
		try {
			await deleteDrop(slug, dropId);
			drops = drops.filter((d) => d.id !== dropId);
			if (expandedId === dropId) expandedId = null;
		} catch {
			toast.error("failed to delete drop");
		}
	}

	function toggleExpand(dropId: string) {
		expandedId = expandedId === dropId ? null : dropId;
	}

	const kindIcon: Record<string, string> = {
		thought: "~",
		idea: "*",
		poem: "\"",
		observation: "o",
		reflection: ".",
		recommendation: ">",
		story: "#",
		question: "?",
		note: "-",
	};

	function formatTime(ts: string): string {
		const ms = parseInt(ts);
		if (isNaN(ms)) return "";
		const d = new Date(ms);
		const now = new Date();
		const diff = now.getTime() - d.getTime();
		const mins = Math.floor(diff / 60000);
		const hours = Math.floor(diff / 3600000);
		const days = Math.floor(diff / 86400000);

		if (mins < 1) return "just now";
		if (mins < 60) return `${mins}m ago`;
		if (hours < 24) return `${hours}h ago`;
		if (days < 7) return `${days}d ago`;
		return d.toLocaleDateString([], { month: "short", day: "numeric" });
	}
</script>

<div class="drops-container">
	{#if loading}
		<span class="sr-only" role="status">Loading drops…</span>
		<div class="drops-loading">
			<div class="drops-loading-dot"></div>
		</div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if drops.length === 0}
		<div class="drops-empty">
			<img class="drops-empty-icon" src="/skins/moon/character.svg" alt="" width="64" height="64" />
			<p class="drops-empty-text">No drops yet</p>
			<p class="drops-empty-hint">
				your companion creates drops autonomously — ideas, poems, observations, reflections.
				they appear here as they come.
			</p>
		</div>
	{:else}
		<div class="drops-header">
			<span class="drops-count">{drops.length} drop{drops.length !== 1 ? "s" : ""}</span>
		</div>
		<div class="drops-grid">
			{#each drops as drop (drop.id)}
				<DropCard
					{drop}
					icon={kindIcon[drop.kind] ?? "~"}
					time={formatTime(drop.created_at)}
					expanded={expandedId === drop.id}
					onexpand={() => toggleExpand(drop.id)}
					ondelete={() => handleDelete(drop.id)}
				/>
			{/each}
		</div>
	{/if}
</div>

<style>
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.drops-container {
		height: 100%;
		overflow-y: auto;
		padding: 2rem 1.5rem;
	}

	.drops-loading {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
	}

	.drops-loading-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--text-muted);
		animation:none;
	}

	.drops-empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: 100%;
		gap: 0.75rem;
		text-align: center;
	}

	.drops-empty-icon {
		font-family: var(--font-body);
		font-size: 1.5rem;
		color: var(--text-muted);
		animation:none;
	}

	.drops-empty-text {
		font-family: var(--font-display);
		font-size: 0.95rem;
		color: var(--text-secondary);
	}

	.drops-empty-hint {
		font-size: 0.75rem;
		color: var(--text-muted);
		max-width: 28ch;
		line-height: 1.5;
	}

	.drops-header {
		margin-bottom: 1.25rem;
	}

	.drops-count {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-muted);
		letter-spacing: 0.05em;
	}

	.drops-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		gap: 0.75rem;
	}

	@media (max-width: 640px) {
		.drops-grid {
			grid-template-columns: 1fr;
		}
		.drops-container {
			padding: 1.5rem 1rem;
		}
	}

/* Little Moon surfaces, controls, and readable content. */

.drops-container { padding: 32px; }
.drops-empty-text { font: 400 28px var(--font-display); color: var(--foreground); }
.drops-empty-hint { font-size: 14px; max-width: 42ch; }
.drops-count { font-size: 20px; color: var(--foreground); }
@media (max-width: 640px) { .drops-container { padding: 20px; } }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.drops-loading-dot { background: var(--primary); }
</style>
