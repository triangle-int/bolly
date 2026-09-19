<script lang="ts">
	import type { Drop } from "$lib/api/types.js";
	let {
		drop,
		icon,
		time,
		expanded,
		onexpand,
		ondelete,
	}: {
		drop: Drop;
		icon: string;
		time: string;
		expanded: boolean;
		onexpand: () => void;
		ondelete: () => void;
	} = $props();

	const moodColors: Record<string, string> = {
		calm: "var(--primary)",
		curious: "var(--primary)",
		excited: "var(--primary)",
		warm: "var(--primary)",
		happy: "var(--primary)",
		joyful: "var(--primary)",
		reflective: "var(--primary)",
		contemplative: "var(--primary)",
		melancholy: "var(--primary)",
		sad: "var(--primary)",
		worried: "var(--primary)",
		anxious: "var(--primary)",
		playful: "var(--primary)",
		mischievous: "var(--primary)",
		focused: "var(--primary)",
		tired: "var(--primary)",
		peaceful: "var(--primary)",
		loving: "var(--primary)",
		tender: "var(--primary)",
		creative: "var(--primary)",
		energetic: "var(--primary)",
	};

	const accentColor = $derived(moodColors[drop.mood] ?? "var(--primary)");
</script>

<article
	class="drop-card"
	class:drop-card-expanded={expanded}
	style="--drop-accent: {accentColor}"
>
	<div class="drop-card-header">
		<span class="drop-card-icon">{icon}</span>
		<span class="drop-card-kind">{drop.kind}</span>
		<span class="drop-card-time">{time}</span>
	</div>

	<h3 class="drop-card-title"><button class="card-expand" aria-expanded={expanded} onclick={onexpand}>{drop.title}</button></h3>

	{#if drop.image_url}
		<img class="drop-card-image" src={drop.image_url} alt={drop.title} loading="lazy" />
	{/if}

	<div class="drop-card-content" class:drop-card-content-expanded={expanded}>
		{drop.content}
	</div>

	{#if drop.mood}
		<div class="drop-card-mood">
			<span class="drop-card-mood-dot" style="background: {accentColor}"></span>
			{drop.mood}
		</div>
	{/if}

	{#if expanded}
		<button
			type="button"
			class="drop-card-delete"
			onclick={(e) => { e.stopPropagation(); ondelete(); }}
		>
			Delete
		</button>
	{/if}
</article>

<style>
	.drop-card {
		position: relative;
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		padding: 1rem 1.125rem;
		border-radius: 0.75rem;
		background: var(--card);
		border: 1px solid var(--border);
		cursor: pointer;
		transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
		text-align: left;
		width: 100%;
		animation: none;
	}

	@keyframes drop-emerge {
		from {
			opacity: 1;
			transform: translateY(8px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.drop-card:hover {
		background: var(--card);
		border-color: var(--border);
		box-shadow: none;
	}

	.drop-card-expanded {
		border-color: var(--border);
		box-shadow: none;
	}

	.drop-card-header {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.drop-card-icon {
		font-family: var(--font-body);
		font-size: 0.8rem;
		color: var(--text-secondary);
		opacity: 0.7;
	}

	.drop-card-kind {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-muted);
		letter-spacing: 0.06em;
		text-transform: none;
	}

	.drop-card-time {
		margin-left: auto;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-muted);
	}

	.drop-card-title {
		font-family: var(--font-display);
		font-size: 0.875rem;
		font-weight: 500;
		color: var(--foreground);
		line-height: 1.35;
	}

	.drop-card-image {
		width: 100%;
		border-radius: 0.5rem;
		max-height: 280px;
		object-fit: cover;
		opacity: 0.85;
		transition: opacity 0.3s ease;
	}

	.drop-card-expanded .drop-card-image {
		max-height: none;
		opacity: 1;
	}

	.drop-card-content {
		font-size: 0.78rem;
		color: var(--text-secondary);
		line-height: 1.55;
		overflow: hidden;
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
		white-space: pre-wrap;
	}

	.drop-card-content-expanded {
		-webkit-line-clamp: unset;
		line-clamp: unset;
		color: var(--foreground);
	}

	.drop-card-mood {
		display: flex;
		align-items: center;
		gap: 0.35rem;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-muted);
		margin-top: 0.25rem;
	}

	.drop-card-mood-dot {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		opacity: 0.6;
	}

	.drop-card-delete {
		align-self: flex-end;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--destructive);
		background: none;
		border: none;
		cursor: pointer;
		padding: 0.25rem 0.5rem;
		border-radius: 0.25rem;
		transition: all 0.2s ease;
	}

	.drop-card-delete:hover {
		color: var(--destructive);
		background: var(--card);
	}

/* Little Moon surfaces, controls, and readable content. */

.drop-card { padding: 24px; border-radius: 16px; background: var(--card); border-color: var(--border); }
.drop-card:hover, .drop-card-expanded { background: var(--card); border-color: var(--primary); box-shadow: none; }
.drop-card-title { font: 500 18px var(--font-body); }
.drop-card-content { font-size: 14px; line-height: 1.65; }
.drop-card-icon, .drop-card-mood-dot, .drop-card-image { opacity: 1; }
.drop-card-delete { min-height: 44px; display: inline-flex; align-items: center; padding: 8px 12px; color: var(--destructive); }
.drop-card-delete:hover { color: var(--destructive); background: var(--accent); }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.card-expand { color: var(--foreground); text-align: left; background: none; border: none; padding: 0; cursor: pointer; font: inherit; }
</style>
