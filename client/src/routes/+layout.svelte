<script lang="ts">
	import "./layout.css";
	import { page } from "$app/state";
	import favicon from "$lib/assets/favicon.svg";
	import { getCompanion } from "$lib/stores/companion.svelte.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import { createSceneStore, setSceneStore } from "$lib/stores/scene.svelte.js";
	import { createSkinStore, setSkinStore } from "$lib/stores/skin.svelte.js";
	import { AuthError } from "$lib/api/client.js";
	import type { ServerEvent } from "$lib/api/types.js";
	import { onMount } from "svelte";
	import AuthGate from "$lib/components/auth/AuthGate.svelte";
	import Toast from "$lib/components/layout/Toast.svelte";
	import SecretDialog from "$lib/components/layout/SecretDialog.svelte";
	import SharedScene from "$lib/components/SharedScene.svelte";
	import FileViewer from "$lib/components/FileViewer.svelte";

	let { children } = $props();

	const companion = getCompanion();
	const ws = getWebSocket();
	const sceneStore = createSceneStore();
	setSceneStore(sceneStore);
	const skinStore = createSkinStore();
	setSkinStore(skinStore);

	// The client uses the Little Moon theme.
	const isDesignSystem = $derived(page.url.pathname === "/design-system");
	$effect(() => {
		const el = typeof document !== 'undefined' ? document.documentElement : null;
		if (!el) return;
		el.classList.add('dark');
		const meta = document.querySelector('meta[name="theme-color"]');
		if (meta) meta.setAttribute('content', '#201d29');
	});

	let needsAuth = $state(false);

	// Secret request state
	let secretRequest = $state<{
		instanceSlug: string;
		id: string;
		prompt: string;
		target: string;
	} | null>(null);

	function init() {
		needsAuth = false;
		companion.refresh().catch((e: unknown) => {
			if (e instanceof AuthError) needsAuth = true;
		});
		ws.connect();
	}

	$effect(() => {
		if (isDesignSystem) return;
		init();

		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "secret_request") {
				secretRequest = {
					instanceSlug: event.instance_slug,
					id: event.id,
					prompt: event.prompt,
					target: event.target,
				};
			}
		});

		return () => {
			unsub();
			ws.disconnect();
		};
	});

	function handleAuth() {
		// Re-init with new token
		ws.disconnect();
		init();
	}


</script>

<svelte:head>
	<link rel="icon" href={favicon} />
	<title>nolune</title>
</svelte:head>

<div class="relative h-dvh w-full overflow-hidden" style="padding-top: env(safe-area-inset-top); padding-left: env(safe-area-inset-left); padding-right: env(safe-area-inset-right);">
	{#if !isDesignSystem && sceneStore.mode !== "home"}<SharedScene />{/if}

	{#if needsAuth && !isDesignSystem}
		<AuthGate onauth={handleAuth} />
	{:else}
		{@render children()}
	{/if}

	<Toast />
	<FileViewer />

	{#if secretRequest}
		<SecretDialog
			instanceSlug={secretRequest.instanceSlug}
			requestId={secretRequest.id}
			prompt={secretRequest.prompt}
			target={secretRequest.target}
			onclose={() => (secretRequest = null)}
		/>
	{/if}

	<!-- Dynamic Island — connection status -->
	{#if ws.reconnecting && !isDesignSystem && page.url.pathname !== "/"}
		<div class="island" role="status">
			<div class="island-pill">
				<div class="island-content">
					<div class="island-activity">
						<span class="island-ring"></span>
						<span class="island-dot"></span>
					</div>
					<span class="island-label">
						{#if ws.retryCount > 2}
							reconnecting · attempt {ws.retryCount}
						{:else}
							reconnecting
						{/if}
					</span>
				</div>
			</div>
		</div>
	{/if}
</div>

<style>
.island{position:fixed;top:calc(8px + env(safe-area-inset-top,0px));left:50%;transform:translateX(-50%);z-index:200;pointer-events:none}.island-pill{display:flex;align-items:center;justify-content:center;min-height:44px;background:var(--popover);border:1px solid var(--border);border-radius:12px;padding:8px 16px}.island-content{display:flex;align-items:center;gap:12px}.island-activity{width:8px;height:8px;flex-shrink:0}.island-dot{display:block;width:8px;height:8px;border-radius:50%;background:var(--primary)}.island-ring{display:none}.island-label{font:14px var(--font-body);color:var(--text-secondary)}
</style>
