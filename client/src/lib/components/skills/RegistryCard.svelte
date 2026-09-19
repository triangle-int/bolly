<script lang="ts">
	import type { RegistryEntry } from "$lib/api/types.js";

	let {
		entry,
		installing,
		oninstall,
	}: {
		entry: RegistryEntry;
		installing: boolean;
		oninstall: (id: string) => void;
	} = $props();
</script>

<div class="registry-card" class:registry-card-installed={entry.installed}>
	<div class="registry-card-header">
		<span class="registry-card-icon">{entry.icon || "~"}</span>
		<span class="registry-card-name">{entry.name}</span>
		{#if entry.installed}
			<span class="registry-card-badge">Installed</span>
		{/if}
	</div>

	<p class="registry-card-desc">{entry.description}</p>

	<div class="registry-card-footer">
		{#if entry.author}
			<span class="registry-card-author">{entry.author}</span>
		{/if}
		<span class="registry-card-repo">{entry.repo}</span>

		{#if !entry.installed}
			<button
				class="registry-card-install"
				disabled={installing}
				onclick={(e) => {
					e.stopPropagation();
					oninstall(entry.id);
				}}
			>
				{installing ? "Installing…" : "Install"}
			</button>
		{/if}
	</div>
</div>

<style>
	.registry-card {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		padding: 1rem 1.125rem;
		border-radius: 0.75rem;
		background: var(--card);
		border: 1px solid var(--border);
		transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
		animation: none;
	}

	@keyframes registry-emerge {
		from {
			opacity: 1;
			transform: translateY(8px);
		}
		to {
			opacity: 1;
			transform: translateY(0);
		}
	}

	.registry-card:hover {
		background: var(--card);
		border-color: var(--border);
		box-shadow: none;
	}

	.registry-card-installed {
		opacity: 0.6;
	}

	.registry-card-header {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.registry-card-icon {
		font-family: var(--font-body);
		font-size: 0.85rem;
		color: var(--text-secondary);
		width: 1.25rem;
		text-align: center;
		flex-shrink: 0;
	}

	.registry-card-name {
		font-family: var(--font-display);
		font-size: 0.85rem;
		font-weight: 500;
		color: var(--foreground);
	}

	.registry-card-badge {
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

	.registry-card-desc {
		font-size: 0.78rem;
		color: var(--text-secondary);
		line-height: 1.5;
	}

	.registry-card-footer {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding-top: 0.4rem;
		border-top: 1px solid var(--border);
	}

	.registry-card-author {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.registry-card-repo {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.registry-card-install {
		margin-left: auto;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: var(--card);
		border: 1px solid var(--border);
		padding: 0.25rem 0.6rem;
		border-radius: 0.35rem;
		cursor: pointer;
		letter-spacing: 0.04em;
		transition: all 0.2s ease;
		flex-shrink: 0;
	}

	.registry-card-install:hover:not(:disabled) {
		color: var(--foreground);
		background: var(--card);
		border-color: var(--border);
	}

	.registry-card-install:disabled {
		opacity: 0.5;
		cursor: not-allowed;
	}

/* Little Moon surfaces, controls, and readable content. */

.registry-card { background: var(--card); border-color: var(--border); border-radius: 16px; padding: 24px; }
.registry-card:hover { background: var(--card); border-color: var(--primary); }
.registry-card-installed { opacity: 1; }
.registry-card-header, .registry-card-footer { flex-wrap: wrap; }
.registry-card-name { font: 500 18px var(--font-body); }
.registry-card-desc { font-size: 14px; line-height: 1.6; }
.registry-card-badge { color: var(--primary); background: var(--accent); }
.registry-card-install { min-height: 44px; padding: 8px 16px; background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); }
.registry-card-install:hover:not(:disabled) { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }
</style>
