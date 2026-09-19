<script lang="ts">
	import { page } from "$app/stores";
	import { onMount } from "svelte";
	import { getAuthToken } from "$lib/api/client.js";

	const slug = $derived($page.params.slug!);

	let imgSrc = $state("");
	let connected = $state(false);
	let lastUpdate = $state("");
	let loading = $state(true);

	onMount(() => {
		let active = true;

		async function poll() {
			while (active) {
				try {
					const token = getAuthToken();
					const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
					const res = await fetch(`/api/instances/${slug}/live-frame`, { headers });
					if (res.ok && res.headers.get("content-type")?.includes("image/jpeg")) {
						const blob = await res.blob();
						const url = URL.createObjectURL(blob);
						if (imgSrc) URL.revokeObjectURL(imgSrc);
						imgSrc = url;
						connected = true;
						lastUpdate = new Date().toLocaleTimeString();
					} else if (res.status === 204) {
						connected = true; // connected but no frame yet
					} else {
						connected = false;
					}
				} catch {
					connected = false;
				}
				loading = false;
				await new Promise(r => setTimeout(r, 1000));
			}
		}

		poll();
		return () => { active = false; if (imgSrc) URL.revokeObjectURL(imgSrc); };
	});
</script>

<div class="live-page">
	<div class="live-header">
		<h1>Live screen</h1>
		<div class="live-indicator" class:live-active={connected && imgSrc}>
			<div class="live-dot"></div>
			<span class="live-label">{loading ? "Connecting" : connected ? (imgSrc ? "Live" : "Waiting for screen") : "Offline"}</span>
		</div>
		{#if lastUpdate}
			<span class="live-time">Last frame {lastUpdate}</span>
		{/if}
	</div>

	<div class="live-feed">
		{#if loading}
			<p role="status">Connecting to your desktop…</p>
		{:else if imgSrc}
			<img class="live-img" src={imgSrc} alt="Live screen" />
		{:else if connected}
			<div class="live-waiting">
				<div class="live-pulse"></div>
				<p>Waiting for the first frame…</p>
			</div>
		{:else}
			<div class="live-offline">
				<svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1" aria-hidden="true">
					<rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>
				</svg>
				<p>No screen available</p>
				<p class="live-hint">Check the connection, then open the desktop app and enable screen recording.</p>
			</div>
		{/if}
	</div>
</div>

<style>
.live-page{height:100%;display:flex;flex-direction:column;padding:32px;gap:24px;overflow:auto}.live-header{display:flex;align-items:center;gap:16px;flex-wrap:wrap}.live-header h1{font:500 28px/1.2 var(--font-body);color:var(--foreground);margin-right:auto}.live-indicator{display:flex;align-items:center;gap:8px;border:1px solid var(--border);padding:6px 12px;border-radius:8px;background:var(--card);font-size:13px;color:var(--text-muted)}.live-dot,.live-pulse{width:8px;height:8px;border-radius:50%;background:var(--text-muted)}.live-active .live-dot,.live-pulse{background:var(--primary)}.live-active{color:var(--primary)}.live-time{font-size:12px;color:var(--text-muted)}.live-feed{flex:1;min-height:240px;display:flex;align-items:center;justify-content:center;border:1px solid var(--border);border-radius:16px;background:var(--card);overflow:hidden;color:var(--text-muted)}.live-img{width:100%;height:100%;object-fit:contain}.live-waiting,.live-offline{display:flex;flex-direction:column;align-items:center;gap:16px;padding:32px;text-align:center;font-size:16px}.live-hint{font-size:14px;max-width:380px;color:var(--text-muted)}@media(max-width:640px){.live-page{padding:20px}.live-header h1{width:100%}}
</style>
