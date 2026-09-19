<script lang="ts">
	import { onDestroy } from "svelte";
	import { checkUpdate, applyUpdate, getUpdateChannel, setUpdateChannel, type UpdateCheck } from "$lib/api/client.js";
	import { play } from "$lib/sounds.js";
	import { getSkinStore } from "$lib/stores/skin.svelte.js";

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
			<span class="update-label">Update available</span>
		</button>
	</div>
{:else if updating}
	<div class="update-bar">
		<div class="update-pill update-pill-active">
			<span class="update-spinner"></span>
			<span class="update-label">Updating…</span>
		</div>
	</div>
{/if}

{#if showReborn}
	<div class="reborn-overlay">
		<img class="reborn-video" src={skinStore.skin.avatar.idle} alt="Nolune is ready" />
		<div class="reborn-text">A little more possibility.</div>
	</div>
{/if}

<style>
.update-bar{display:flex;justify-content:center;padding:8px;background:var(--card);border-bottom:1px solid var(--border)}.update-pill{display:flex;align-items:center;gap:8px;min-height:44px;padding:8px 16px;border-radius:8px;background:var(--primary);color:var(--primary-foreground);cursor:pointer}.update-pill:hover{filter:brightness(1.08)}.update-pill-active{background:var(--accent);color:var(--foreground);cursor:default}.update-dot{width:6px;height:6px;border-radius:50%;background:currentColor}.update-spinner{width:16px;height:16px;border:2px solid var(--border);border-top-color:var(--primary);border-radius:50%;animation:spin .8s linear infinite}.update-label{font:500 14px var(--font-body)}@keyframes spin{to{transform:rotate(360deg)}}.reborn-overlay{position:fixed;inset:0;z-index:9999;background:var(--background);display:flex;flex-direction:column;gap:32px;align-items:center;justify-content:center}.reborn-video{width:min(48vw,320px);height:min(48vw,320px);object-fit:contain}.reborn-text{font:400 clamp(28px,6vw,48px)/1.15 var(--font-display);color:var(--foreground);text-align:center}
</style>
