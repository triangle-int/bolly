<script lang="ts">
	import { goto } from "$app/navigation";
	import { page } from "$app/state";
	import { machineHello, machineBye } from "$lib/api/client.js";
	import { needsOnboarding, redirectForStaleSlug } from "$lib/companion/context.js";
	import { getCompanion } from "$lib/stores/companion.svelte.js";
	import { getPresentationState } from "$lib/stores/presentation.svelte.js";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { getSkinStore } from "$lib/stores/skin.svelte.js";
	import { getVoiceState } from "$lib/stores/voice.svelte.js";
	import InstanceOnboarding from "$lib/components/onboarding/InstanceOnboarding.svelte";
	import UpdateBanner from "$lib/components/UpdateBanner.svelte";
	let { children } = $props();

	const slug = $derived(page.params.slug!);
	const companion = getCompanion();
	const presentation = getPresentationState();
	const scene = getSceneStore();
	const skinStore = getSkinStore();
	const voice = getVoiceState();

	// One companion per server (#104): a stale multi-instance URL opens the
	// canonical companion and keeps its chat or settings context.
	const stale = $derived(redirectForStaleSlug(companion.slug, slug, page.url.pathname));
	$effect(() => {
		if (stale) goto(`${stale}${page.url.search}`, { replaceState: true });
	});

	const checking = $derived(companion.loading);
	const isNew = $derived(!checking && !companion.error && needsOnboarding(companion.context));
	const ready = $derived(!checking && !companion.error && !isNew && !stale);

	// Fetch companion settings and enter chat mode when ready
	$effect(() => {
		if (ready) {
			const currentSlug = slug;
			Promise.all([
				voice.loadForInstance(currentSlug),
				skinStore.loadForInstance(currentSlug),
				machineHello(currentSlug).catch(() => {}),
			]).finally(() => scene.enterChat(currentSlug));

			return () => {
				machineBye(currentSlug).catch(() => {});
			};
		}
	});

	const tabs = ["chat", "drops", "thoughts", "memory", "skills", "settings"] as const;
	const activeTab = $derived(
		tabs.find((t) => page.url.pathname.includes(`/${slug}/${t}`)) ?? "chat"
	);

	function handleOnboardingComplete() {
		companion.refresh().catch(() => {});
	}
</script>

{#if checking || stale}
	<div class="flex h-full items-center justify-center">
		<p role="status">Loading your companion…</p>
	</div>
{:else if companion.error}
	<div class="connection-state"><h1>Cannot reach Nolune</h1><p role="alert">{companion.error}</p><button class="nl-button" onclick={() => companion.refresh().catch(() => {})}>Retry connection</button></div>
{:else if isNew}
	{#key slug}
		<InstanceOnboarding {slug} oncomplete={handleOnboardingComplete} />
	{/key}
{:else}
	<div class="instance-outer">
	<UpdateBanner />
	<div class="instance-view">
		{#if !presentation.active && (scene.mode === "chat" || activeTab !== "chat")}
		<nav class="instance-tabs" aria-label="Companion navigation">
			{#each tabs as tab}
				<a
					href="/{slug}/{tab}"
					class="instance-tab"
					class:instance-tab-active={activeTab === tab}
					aria-current={activeTab === tab ? "page" : undefined}
				>
					{tab}
				</a>
			{/each}
		</nav>
		{/if}

		<div class="instance-content" class:instance-content-backdrop={activeTab !== "chat"}>
			{@render children()}
		</div>
	</div>
	</div>
{/if}

<style>
.connection-state{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;height:100%;padding:24px;text-align:center}.connection-state h1{font-size:28px}.connection-state p{color:var(--text-muted)}
.instance-outer,.instance-view{display:flex;flex-direction:column;min-height:0;max-width:100%;overflow:hidden}.instance-outer{height:100%}.instance-view{flex:1}
.instance-tabs{display:flex;gap:4px;padding:8px 24px 0;border-bottom:1px solid var(--border);flex-shrink:0;z-index:10;overflow-x:auto;background:var(--background);scrollbar-width:thin}
.instance-tab{display:flex;align-items:center;justify-content:center;min-height:44px;min-width:44px;padding:10px 12px;border-radius:8px 8px 0 0;flex-shrink:0;position:relative;white-space:nowrap;font:500 14px var(--font-body);text-transform:capitalize;color:var(--text-muted);text-decoration:none;cursor:pointer}
.instance-tab:hover{background:var(--card);color:var(--foreground)}.instance-tab-active{background:var(--accent);color:var(--accent-foreground)}.instance-tab-active::after{content:"";position:absolute;bottom:0;left:12px;right:12px;height:2px;background:var(--primary)}.instance-content{position:relative;flex:1;min-width:0;min-height:0;overflow:hidden}.instance-content-backdrop{background:var(--background)}
@media(max-width:720px){.instance-tabs{padding:8px 12px 0}}
</style>
