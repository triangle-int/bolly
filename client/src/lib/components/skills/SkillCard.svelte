<script lang="ts">
	import type { Skill } from "$lib/api/types.js";

	let {
		skill,
		ondelete,
	}: {
		skill: Skill;
		ondelete: () => void;
	} = $props();

	let expanded = $state(false);
</script>

<article
	class="skill-card"
	class:skill-card-expanded={expanded}
	class:skill-card-builtin={skill.builtin}
>
	<div class="skill-card-header">
		<span class="skill-card-icon">{skill.icon || "~"}</span>
		<button class="skill-card-name card-expand" aria-expanded={expanded} onclick={() => expanded = !expanded}>{skill.name}</button>
		{#if skill.kind === "anthropic"}
			<span class="skill-card-badge skill-card-badge-anthropic">Anthropic</span>
		{:else if skill.builtin}
			<span class="skill-card-badge">Built-in</span>
		{:else if skill.source}
			<span class="skill-card-badge skill-card-badge-community">Community</span>
		{/if}
	</div>

	<p class="skill-card-desc">{skill.description}</p>

	{#if expanded && skill.instructions}
		<div class="skill-card-instructions">
			<span class="skill-card-instructions-label">Instructions</span>
			<p class="skill-card-instructions-text">{skill.instructions}</p>
		</div>
	{/if}

	{#if expanded && skill.resources && skill.resources.length > 0}
		<div class="skill-card-instructions">
			<span class="skill-card-instructions-label">Resources</span>
			<ul class="skill-card-resources">
				{#each skill.resources as resource}
					<li class="skill-card-resource">{resource}</li>
				{/each}
			</ul>
		</div>
	{/if}

	{#if expanded && !skill.builtin}
		<button
			type="button"
			class="skill-card-delete"
			onclick={(e) => {
				e.stopPropagation();
				ondelete();
			}}
		>
			Delete
		</button>
	{/if}
</article>

<style>
	.skill-card {
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

	@keyframes skill-emerge {
		from {
			opacity: 1;
			transform: translateY(8px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.skill-card:hover {
		background: var(--card);
		border-color: var(--border);
		box-shadow: none;
	}

	.skill-card-expanded {
		border-color: var(--border);
		box-shadow: none;
	}

	.skill-card-builtin {
		border-color: var(--border);
	}

	.skill-card-header {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.skill-card-icon {
		font-family: var(--font-body);
		font-size: 0.85rem;
		color: var(--text-secondary);
		width: 1.25rem;
		text-align: center;
		flex-shrink: 0;
	}

	.skill-card-name {
		font-family: var(--font-display);
		font-size: 0.85rem;
		font-weight: 500;
		color: var(--foreground);
	}

	.skill-card-badge {
		margin-left: auto;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: var(--card);
		padding: 0.15rem 0.45rem;
		border-radius: 0.25rem;
		letter-spacing: 0.06em;
		text-transform: none;
	}

	.skill-card-desc {
		font-size: 0.78rem;
		color: var(--text-secondary);
		line-height: 1.5;
	}

	.skill-card-instructions {
		display: flex;
		flex-direction: column;
		gap: 0.35rem;
		padding-top: 0.5rem;
		border-top: 1px solid var(--border);
	}

	.skill-card-instructions-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.06em;
		text-transform: none;
	}

	.skill-card-instructions-text {
		font-size: 0.75rem;
		color: var(--text-secondary);
		line-height: 1.55;
		white-space: pre-wrap;
	}

	.skill-card-resources {
		list-style: none;
		padding: 0;
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
	}

	.skill-card-resource {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.skill-card-badge-community {
		color: var(--text-secondary);
		background: var(--card);
	}

	.skill-card-badge-anthropic {
		color: var(--text-secondary);
		background: var(--card);
	}

	.skill-card-delete {
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

	.skill-card-delete:hover {
		color: var(--destructive);
		background: var(--card);
	}

/* Little Moon surfaces, controls, and readable content. */

.skill-card { background: var(--card); border-color: var(--border); border-radius: 16px; padding: 24px; }
.skill-card:hover, .skill-card-expanded { background: var(--card); border-color: var(--primary); }
.skill-card-name { font: 500 18px var(--font-body); }
.skill-card-desc, .skill-card-instructions-text { font-size: 14px; line-height: 1.6; }
.skill-card-header { flex-wrap: wrap; }
.skill-card-badge { color: var(--primary); background: var(--accent); }
.skill-card-delete { min-height: 44px; display: inline-flex; align-items: center; padding: 8px 12px; color: var(--destructive); }
.skill-card-delete:hover { color: var(--destructive); background: var(--accent); }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.card-expand { color: var(--foreground); text-align: left; background: none; border: none; padding: 0; cursor: pointer; font: inherit; }
</style>
