<script lang="ts">
	import { onDestroy } from "svelte";
	import { checkUpdate, applyUpdate, getUpdateChannel, setUpdateChannel, type UpdateCheck } from "$lib/api/client.js";
	import { play } from "$lib/sounds.js";
	import { getSkinStore, clipSrc } from "$lib/stores/skin.svelte.js";

	let updateInfo = $state<UpdateCheck | null>(null);
	let updating = $state(false);
	let showReborn = $state(false);
	let frozenCommit = '';
	const skinStore = getSkinStore();
	let channel = $state("stable");

	// Check on mount
	$effect(() => {
		checkUpdate().then(u => updateInfo = u).catch(() => {});
		getUpdateChannel().then(r => channel = r.channel).catch(() => {});
	});

	// Poll every 5 minutes in background
	const pollInterval = setInterval(() => {
		if (!updating) {
			checkUpdate().then(u => updateInfo = u).catch(() => {});
		}
	}, 5 * 60 * 1000);
	onDestroy(() => clearInterval(pollInterval));

	let hasUpdate = $derived(updateInfo?.update_available ?? false);

	async function switchChannel(val: string) {
		channel = val;
		await setUpdateChannel(val);
		updateInfo = await checkUpdate();
	}

	async function doUpdate() {
		if (!updateInfo) return;
		updating = true;
		frozenCommit = updateInfo.commit ?? '';
		try {
			await applyUpdate();
		} catch {}
		// Wait for server to go DOWN (fast poll — server restarts quickly)
		let sawDown = false;
		for (let i = 0; i < 15; i++) {
			await new Promise(r => setTimeout(r, 400));
			try {
				const info = await checkUpdate();
				// Server came back with new commit before we saw it go down
				if (info.commit !== frozenCommit) {
					updateInfo = info;
					updating = false;
					handleReborn();
					return;
				}
			} catch {
				sawDown = true;
				break;
			}
		}
		// Poll until it comes back with a new commit
		for (let i = 0; i < 30; i++) {
			await new Promise(r => setTimeout(r, 800));
			try {
				const info = await checkUpdate();
				if (info.commit !== frozenCommit) {
					updateInfo = info;
					updating = false;
					handleReborn();
					return;
				}
			} catch {}
		}
		updating = false;
	}

	function handleReborn() {
		showReborn = true;
		play('reborn');
		setTimeout(() => location.reload(), 5200);
	}
</script>

{#if hasUpdate && !updating && !showReborn}
	<div class="update-bar">
		<button class="update-pill" onclick={doUpdate}>
			<span class="update-dot"></span>
			<span class="update-label">update available</span>
		</button>
	</div>
{:else if updating}
	<div class="update-bar">
		<div class="update-pill update-pill-active">
			<span class="update-spinner"></span>
			<span class="update-label">updating...</span>
		</div>
	</div>
{/if}

{#if showReborn}
	<div class="reborn-overlay">
		{#if skinStore.skin.avatar}
			<img class="reborn-video" src={skinStore.skin.avatar.idle} alt="Nolune is ready" />
		{:else}
			<video class="reborn-video" src={clipSrc(skinStore.skin.clips.reborn)} autoplay muted playsinline></video>
		{/if}
		<div class="reborn-text">meet the new me</div>
	</div>
{/if}

<style>
	.update-bar {
		display: flex;
		justify-content: center;
		padding: 0.4rem 0;
		background: oklch(0.65 0.15 145 / 4%);
		border-bottom: 1px solid oklch(0.65 0.15 145 / 10%);
		animation: bar-in 0.4s ease both;
	}

	@keyframes bar-in {
		from { opacity: 0; transform: translateY(-100%); }
		to { opacity: 1; transform: translateY(0); }
	}

	.update-pill {
		display: flex;
		align-items: center;
		gap: 0.4rem;
		padding: 0.3rem 0.75rem;
		border-radius: 999px;
		border: 1px solid oklch(0.65 0.15 145 / 25%);
		background: oklch(0.65 0.15 145 / 8%);
		cursor: pointer;
		transition: all 0.25s ease;
		flex-shrink: 0;
	}

	.update-pill:hover {
		background: oklch(0.65 0.15 145 / 15%);
		border-color: oklch(0.65 0.15 145 / 40%);
	}

	.update-pill-active {
		cursor: default;
		border-color: oklch(0.6 0.08 240 / 20%);
		background: oklch(0.6 0.08 240 / 6%);
	}

	.update-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: oklch(0.65 0.15 145);
		animation: pulse-dot 2s ease-in-out infinite;
	}

	@keyframes pulse-dot {
		0%, 100% { opacity: 1; transform: scale(1); }
		50% { opacity: 0.5; transform: scale(0.8); }
	}

	.update-spinner {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		border: 1.5px solid oklch(0.6 0.08 240 / 20%);
		border-top-color: oklch(0.6 0.08 240 / 60%);
		animation: spin 0.8s linear infinite;
	}

	@keyframes spin {
		to { transform: rotate(360deg); }
	}

	.update-label {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		letter-spacing: 0.03em;
		color: oklch(0.75 0.04 240 / 60%);
	}

	/* Reborn overlay */
	.reborn-overlay {
		position: fixed;
		inset: 0;
		z-index: 9999;
		background: black;
		display: flex;
		align-items: center;
		justify-content: center;
		animation: reborn-fade-in 0.6s ease both;
	}

	@keyframes reborn-fade-in {
		from { opacity: 0; }
		to { opacity: 1; }
	}

	.reborn-video {
		position: absolute;
		width: min(80vw, 80vh);
		height: min(80vw, 80vh);
		object-fit: contain;
		opacity: 0.5;
		animation: reborn-video-in 1.5s ease both;
	}

	@keyframes reborn-video-in {
		from { opacity: 0; transform: scale(0.8); }
		to { opacity: 0.5; transform: scale(1); }
	}

	.reborn-text {
		position: relative;
		font-family: var(--font-display, 'Georgia', serif);
		font-size: clamp(2rem, 6vw, 4.5rem);
		font-weight: 300;
		letter-spacing: 0.08em;
		color: oklch(0.85 0.08 75);
		text-align: center;
		animation: reborn-text-in 1.2s cubic-bezier(0.16, 1, 0.3, 1) 0.6s both;
	}

	@keyframes reborn-text-in {
		from {
			opacity: 0;
			transform: translateY(30px) scale(0.92);
			filter: blur(8px);
		}
		to {
			opacity: 1;
			transform: translateY(0) scale(1);
			filter: blur(0);
		}
	}
</style>
