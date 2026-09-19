<script lang="ts">
	import { fetchThoughts } from "$lib/api/client.js";
	import type { Thought, ServerEvent } from "$lib/api/types.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";

	const toast = getToasts();
	let { slug }: { slug: string } = $props();

	let thoughts = $state<Thought[]>([]);
	let loading = $state(true);
	let loadError = $state("");
	let expandedIds = $state<Set<string>>(new Set());

	const ws = getWebSocket();

	const moodColors: Record<string, string> = {
		calm: "var(--primary)", curious: "var(--primary)",
		excited: "var(--primary)", warm: "var(--primary)",
		happy: "var(--primary)", joyful: "var(--primary)",
		reflective: "var(--primary)", contemplative: "var(--primary)",
		melancholy: "var(--primary)", sad: "var(--primary)",
		worried: "var(--primary)", anxious: "var(--primary)",
		playful: "var(--primary)", mischievous: "var(--primary)",
		focused: "var(--primary)", tired: "var(--primary)",
		peaceful: "var(--primary)", loving: "var(--primary)",
		tender: "var(--primary)", creative: "var(--primary)",
		energetic: "var(--primary)",
	};

	async function load() {
		loading = true;
		loadError = "";
		try {
			thoughts = await fetchThoughts(slug);
		} catch {
			loadError = "Could not load thoughts. Please try again.";
			toast.error("failed to load thoughts");
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "heartbeat_thought" && event.instance_slug === slug) {
				thoughts = [event.thought, ...thoughts];
			}
		});
		return unsub;
	});

	function formatTime(ts: string): string {
		const ms = parseInt(ts);
		if (isNaN(ms)) return "";
		const d = new Date(ms);
		const diff = Date.now() - d.getTime();
		const mins = Math.floor(diff / 60000);
		const hours = Math.floor(diff / 3600000);
		const days = Math.floor(diff / 86400000);
		if (mins < 1) return "just now";
		if (mins < 60) return `${mins}m ago`;
		if (hours < 24) return `${hours}h ago`;
		if (days < 7) return `${days}d ago`;
		return d.toLocaleDateString([], { month: "short", day: "numeric" });
	}

	function parseAction(action: string): { kind: string; label: string } {
		const i = action.indexOf(":");
		if (i === -1) return { kind: action.trim(), label: "" };
		return { kind: action.substring(0, i).trim(), label: action.substring(i + 1).trim() };
	}

	function isQuiet(t: Thought): boolean {
		return t.actions.length === 1 && t.actions[0].startsWith("quiet");
	}

	function cleanRaw(raw: string): string {
		if (!raw) return "";
		// Extract thought/reason from structured JSON triage output
		try {
			const j = JSON.parse(raw);
			if (j) {
				const text = j.thought ?? j.reason;
				if (typeof text === "string" && text.trim()) return text.trim();
			}
			return "";
		} catch {}
		return raw.trim();
	}

	function primaryKind(t: Thought): string {
		for (const a of t.actions) {
			const p = parseAction(a);
			if (p.kind.startsWith("wake")) return "wake";
			if (p.kind !== "mood" && p.kind !== "quiet") return p.kind;
		}
		return "quiet";
	}

	function toggleExpand(id: string) {
		const next = new Set(expandedIds);
		if (next.has(id)) next.delete(id); else next.add(id);
		expandedIds = next;
	}

	let visibleThoughts = $derived(thoughts);
</script>

<div class="thoughts-page">
	{#if loading}
		<span class="sr-only" role="status">Loading thoughts…</span>
		<div class="thoughts-center">
			<div class="pulse-dot"></div>
		</div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if thoughts.length === 0}
		<div class="thoughts-center">
			<p class="empty-text">No thoughts yet</p>
			<p class="empty-sub">Thoughts appear when your companion reflects on their own.</p>
		</div>
	{:else}
		<div class="thoughts-flow">
			{#each visibleThoughts as thought, i (thought.id)}
				{@const mood = thought.actions.find(a => a.startsWith("mood:"))?.substring(5).trim() ?? thought.mood}
				{@const color = moodColors[mood] ?? "var(--primary)"}
				{@const kind = primaryKind(thought)}
				{@const raw = cleanRaw(thought.raw)}
				{@const isExpanded = expandedIds.has(thought.id)}
				{@const isLong = raw.length > 180}

				<div
					class="thought"
					class:thought-reach={kind === "reach_out"}
					class:thought-drop={kind === "drop"}
					class:thought-wake={kind === "wake"}
					style="--c: {color}; --i: {Math.min(i, 10)};"
				>
					<!-- Mood glow -->
					<div class="thought-glow" style="background: {color};"></div>

					<!-- Mood + time -->
					<div class="thought-meta">
						<span class="thought-mood" style="color: {color}">{mood}</span>
						<span class="thought-time">{formatTime(thought.created_at)}</span>
					</div>

					<!-- Action label -->
					{#each thought.actions as action}
						{@const p = parseAction(action)}
						{#if p.kind === "reach_out"}
							<span class="thought-action thought-action-reach">Reached out</span>
						{:else if p.kind === "drop"}
							<span class="thought-action thought-action-drop">Created a drop</span>
						{:else if p.kind.startsWith("wake")}
							<span class="thought-action thought-action-wake">Woke up</span>
						{:else if p.kind === "mood"}
							<span class="thought-action thought-action-mood">Mood shift</span>
						{/if}
					{/each}

					<!-- Content -->
					{#if raw}
						<!-- svelte-ignore a11y_click_events_have_key_events -->
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="thought-body"
							class:thought-body-collapsed={isLong && !isExpanded}
						>
							{raw}
						</div>
						{#if isLong}
							<button aria-expanded={isExpanded} class="thought-more" onclick={() => toggleExpand(thought.id)}>
								{isExpanded ? "Show less" : "Read more"}
							</button>
						{/if}
					{/if}
				</div>
			{/each}

		</div>
	{/if}
</div>

<style>
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.thoughts-page {
		height: 100%;
		overflow-y: auto;
		padding: 2rem 1.5rem;
	}

	.thoughts-center {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: 100%;
		gap: 0.75rem;
	}

	.pulse-dot {
		width: 6px; height: 6px; border-radius: 50%;
		background: var(--card);
		animation:none;
	}
	@keyframes pulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.3; transform: scale(0.7); } }

	.empty-text {
		font-family: var(--font-display);
		font-style: normal;
		font-size: 0.9rem;
		color: var(--text-secondary);
	}
	.empty-sub {
		font-size: 0.75rem;
		color: var(--text-secondary);
		max-width: 26ch;
		text-align: center;
		line-height: 1.5;
	}

	/* ── Flow layout ── */
	.thoughts-flow {
		max-width: 480px;
		margin: 0 auto;
		display: flex;
		flex-direction: column;
		gap: 1.5rem;
	}

	/* ── Individual thought ── */
	.thought {
		position: relative;
		padding: 0.875rem 1rem;
		opacity: 1;
		animation: none;
		animation-delay: calc(var(--i) * 50ms);
	}

	@keyframes thought-in {
		from { opacity: 1; transform: translateY(12px); filter: blur(2px); }
		to { opacity: 1; transform: translateY(0); filter: blur(0); }
	}

	/* Ambient glow from mood */
	.thought-glow {
		position: absolute;
		left: -8px;
		top: 0.5rem;
		width: 3px;
		height: 1.5rem;
		border-radius: 2px;
		opacity: 0.35;
		filter: blur(1px);
		transition: opacity 0.3s ease, height 0.3s ease;
	}

	.thought:hover .thought-glow {
		opacity: 0.6;
		height: 100%;
	}

	/* Meta line */
	.thought-meta {
		display: flex;
		align-items: baseline;
		gap: 0.5rem;
		margin-bottom: 0.375rem;
	}

	.thought-mood {
		font-family: var(--font-display);
		font-style: normal;
		font-size: 0.75rem;
		opacity: 0.7;
	}

	.thought-time {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	/* Action labels */
	.thought-action {
		display: inline-block;
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.04em;
		margin-bottom: 0.375rem;
		padding: 0.125rem 0.4rem;
		border-radius: 0.75rem;
		background: var(--card);
		color: var(--text-secondary);
	}
	.thought-action-reach { color: var(--text-secondary); background: var(--card); }
	.thought-action-drop { color: var(--text-secondary); background: var(--card); }
	.thought-action-wake { color: var(--text-secondary); background: var(--card); }
	.thought-action-mood { color: var(--text-secondary); background: var(--card); }

	/* Body text */
	.thought-body {
		font-family: var(--font-body);
		font-size: 0.78rem;
		line-height: 1.7;
		color: var(--text-secondary);
		white-space: pre-line;
		cursor: default;
	}

	.thought-body-collapsed {
		max-height: 4.5em;
		overflow: hidden;
		mask-image: linear-gradient(to bottom, black 50%, transparent 100%);
		-webkit-mask-image: linear-gradient(to bottom, black 50%, transparent 100%);
		cursor: pointer;
	}

	.thought-more {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: none;
		border: none;
		padding: 0;
		margin-top: 0.25rem;
		cursor: pointer;
		letter-spacing: 0.05em;
		transition: color 0.2s ease;
	}
	.thought-more:hover { color: var(--text-secondary); }

	@media (max-width: 640px) {
		.thoughts-page { padding: 1.5rem 1rem; }
	}

/* Little Moon surfaces, controls, and readable content. */

.thoughts-page { padding: 32px; }
.thoughts-flow { max-width: 720px; gap: 16px; }
.thought { background: var(--card); border: 1px solid var(--border); border-radius: 16px; padding: 24px; }
.thought-glow { display: none; }
.thought-mood { font: 500 14px var(--font-body); opacity: 1; }
.thought-body { font-size: 16px; line-height: 1.7; color: var(--foreground); }
.thought-action { color: var(--primary); background: var(--accent); padding: 4px 8px; margin-right: 4px; }
.thought-more { min-height: 44px; font-size: 14px; color: var(--primary); padding: 8px 0; }
.empty-text { font: 400 28px var(--font-display); color: var(--foreground); }
.empty-sub { font-size: 14px; max-width: 42ch; }
@media (max-width: 640px) { .thoughts-page { padding: 20px; } }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.pulse-dot { background: var(--primary); }
</style>
