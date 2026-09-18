<script lang="ts">
	import "./layout.css";
	import favicon from "$lib/assets/favicon.png";
	import { getInstances } from "$lib/stores/instances.svelte.js";
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

	const instances = getInstances();
	const ws = getWebSocket();
	const sceneStore = createSceneStore();
	setSceneStore(sceneStore);
	const skinStore = createSkinStore();
	setSkinStore(skinStore);

	// Sync theme class on <html> with active skin
	const isMint = $derived(skinStore.skinId === 'mint');
	$effect(() => {
		const el = typeof document !== 'undefined' ? document.documentElement : null;
		if (!el) return;
		el.classList.toggle('dark', !isMint);
		el.classList.toggle('mint', isMint);
		const meta = document.querySelector('meta[name="theme-color"]');
		if (meta) meta.setAttribute('content', isMint ? '#f0f7f4' : '#0d0b09');
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
		instances.refresh().catch((e: unknown) => {
			if (e instanceof AuthError) needsAuth = true;
		});
		ws.connect();
	}

	$effect(() => {
		init();

		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "instance_discovered") {
				instances.upsert(event.instance);
			} else if (event.type === "secret_request") {
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
	<link rel="manifest" href="/manifest.webmanifest" />
	<title>bolly</title>
</svelte:head>

<div class="relative h-dvh w-full overflow-hidden" style="padding-top: env(safe-area-inset-top); padding-left: env(safe-area-inset-left); padding-right: env(safe-area-inset-right);">
	<SharedScene />

	{#if needsAuth}
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
	{#if ws.reconnecting}
		<div class="island">
			<div class="island-pill" class:island-expanded={ws.retryCount > 2}>
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
	@keyframes pulse-warn {
		0%, 100% { opacity: 1; transform: scale(1); }
		50% { opacity: 0.3; transform: scale(1.4); }
	}

	/* ── Dynamic Island ── */

	.island {
		position: fixed;
		top: calc(0.5rem + env(safe-area-inset-top, 0px));
		left: 50%;
		transform: translateX(-50%);
		z-index: 200;
		pointer-events: none;
	}

	.island-pill {
		position: relative;
		display: flex;
		align-items: center;
		justify-content: center;
		min-width: 120px;
		height: 32px;
		border-radius: 50px;
		background: var(--surface-overlay);
		backdrop-filter: blur(40px) saturate(180%);
		-webkit-backdrop-filter: blur(40px) saturate(180%);
		border: 1px solid oklch(var(--ink) / 8%);
		border-top-color: oklch(var(--ink) / 14%);
		box-shadow:
			0 2px 20px oklch(var(--shade) / 40%),
			0 8px 40px oklch(var(--shade) / 20%),
			inset 0 1px 0 oklch(var(--ink) / 6%),
			inset 0 -1px 0 oklch(var(--shade) / 10%);
		overflow: hidden;
		animation: island-enter 0.6s cubic-bezier(0.34, 1.56, 0.64, 1) both;
		transition: min-width 0.5s cubic-bezier(0.34, 1.56, 0.64, 1),
		            height 0.4s cubic-bezier(0.34, 1.56, 0.64, 1),
		            border-radius 0.4s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.island-expanded {
		min-width: 200px;
		height: 36px;
	}

	/* Specular highlight */
	.island-pill::before {
		content: "";
		position: absolute;
		top: 0;
		left: 15%;
		right: 15%;
		height: 1px;
		background: linear-gradient(90deg, transparent, oklch(var(--ink) / 18%), transparent);
		pointer-events: none;
	}

	/* Inner refraction glow */
	.island-pill::after {
		content: "";
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		height: 50%;
		background: linear-gradient(180deg, oklch(var(--ink) / 3%) 0%, transparent 100%);
		pointer-events: none;
		border-radius: 50px 50px 0 0;
	}

	@keyframes island-enter {
		0% { transform: scaleX(0.3) scaleY(0.5); opacity: 0; border-radius: 50%; }
		50% { transform: scaleX(1.08) scaleY(1.05); opacity: 1; }
		100% { transform: scaleX(1) scaleY(1); opacity: 1; }
	}

	.island-content {
		position: relative;
		z-index: 1;
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0 0.875rem;
	}

	.island-activity {
		position: relative;
		width: 12px;
		height: 12px;
		flex-shrink: 0;
	}

	.island-dot {
		position: absolute;
		top: 50%;
		left: 50%;
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: oklch(0.75 0.15 55);
		transform: translate(-50%, -50%);
		animation: island-dot-pulse 1.2s ease-in-out infinite;
	}

	.island-ring {
		position: absolute;
		inset: 0;
		border-radius: 50%;
		border: 1.5px solid oklch(0.75 0.15 55 / 50%);
		animation: island-ring-expand 1.2s ease-out infinite;
	}

	@keyframes island-dot-pulse {
		0%, 100% { transform: translate(-50%, -50%) scale(1); opacity: 1; }
		50% { transform: translate(-50%, -50%) scale(0.7); opacity: 0.5; }
	}

	@keyframes island-ring-expand {
		0% { transform: scale(0.5); opacity: 0.8; }
		100% { transform: scale(1.8); opacity: 0; }
	}

	.island-label {
		font-family: var(--font-mono);
		font-size: 0.65rem;
		letter-spacing: 0.05em;
		color: oklch(0.85 0.02 75 / 75%);
		white-space: nowrap;
	}
</style>
