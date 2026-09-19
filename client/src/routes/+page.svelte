<script lang="ts">
	import { goto } from "$app/navigation";
	import { getCompanion } from "$lib/stores/companion.svelte.js";
	import { companionHome } from "$lib/companion/context.js";

	// One companion per server (#104): there is nothing to choose between here.
	// The root opens the companion; its layout offers onboarding when it does
	// not exist yet.
	const companion = getCompanion();

	$effect(() => {
		if (!companion.loading && !companion.error) {
			goto(companionHome(companion.slug), { replaceState: true });
		}
	});
</script>

<div class="home">
	{#if companion.error}
		<div class="connection-state">
			<h1>Cannot reach Nolune</h1>
			<p role="alert">{companion.error}</p>
			<button class="nl-button" onclick={() => companion.refresh().catch(() => {})}>Retry connection</button>
		</div>
	{:else}
		<p role="status" class="opening">Opening your companion…</p>
	{/if}
</div>

<style>
	.home {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 100%;
		height: 100%;
		padding: 0 20px;
		background: var(--background);
	}
	.opening {
		font: 400 16px/1.6 var(--font-body);
		color: var(--text-secondary);
	}
	.connection-state {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 16px;
		max-width: 420px;
		text-align: center;
	}
	.connection-state h1 {
		font: 400 32px/1.2 var(--font-display);
		letter-spacing: -0.02em;
		color: var(--foreground);
	}
	.connection-state p {
		font: 400 16px/1.6 var(--font-body);
		color: var(--text-secondary);
	}
</style>
