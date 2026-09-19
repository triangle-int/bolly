<script lang="ts">
	import { page } from "$app/stores";
	import { onMount } from "svelte";
	import { fetchSkin } from "$lib/api/client.js";

	const slug = $derived($page.params.slug!);

	import { SKINS } from "$lib/stores/skin.svelte.js";

	let skinId = $state("moon");
	let thinking = $state(false);
	let recording = $state(false);
	const skin = $derived(SKINS.find(s => s.id === skinId) ?? SKINS[0]);

	onMount(() => {
		let mounted = true;
		// Fetch skin from server
		void fetchSkin(slug).then((res) => {
			if (mounted && res.skin && SKINS.some(s => s.id === res.skin)) skinId = res.skin;
		}).catch(() => {});

		// Listen for SSE/WebSocket events for thinking state
		// For now, poll the agent_running status
		const poll = setInterval(async () => {
			try {
				const res = await fetch(`/api/instances/${slug}/chat/default`);
				if (res.ok) {
					const data = await res.json();
					if (mounted) thinking = data.agent_running ?? false;
				}
			} catch {}
		}, 2000);

		return () => {
			mounted = false;
			clearInterval(poll);
		};
	});
</script>

<div class="overlay">
	<div class="pip">
		<img class="pip-video" src={thinking ? skin.avatar.thinking : skin.avatar.idle} alt={thinking ? "Nolune is thinking" : "Nolune"} />
		{#if recording}
			<div class="pip-rec"></div>
		{/if}
	</div>
</div>

<style>
	:global(html), :global(body) {
		background: transparent !important;
		margin: 0;
		padding: 0;
		overflow: hidden;
	}

	.overlay {
		position: fixed;
		inset: 0;
		pointer-events: none;
	}

	.pip {
		position: absolute;
		bottom: 16px;
		right: 16px;
		width: 56px;
		height: 56px;
		border-radius: 50%;
		overflow: hidden;
		background: var(--card);
		border: 2px solid var(--primary);
		box-shadow:none;
		animation: breathe 4s ease-in-out infinite;
	}

	@keyframes breathe {
		0%, 100% { transform: scale(1); }
		50% { transform: scale(1.03); }
	}

	.pip-video {
		width: 100%;
		height: 100%;
		object-fit: contain;
	}

	.pip-rec {
		position: absolute;
		top: -1px;
		right: -1px;
		width: 12px;
		height: 12px;
		border-radius: 50%;
		background: var(--destructive);
		box-shadow:none;
		animation: rec-pulse 1.5s ease-in-out infinite;
		border: 2px solid var(--card);
	}

	@keyframes rec-pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.4; }
	}
</style>
