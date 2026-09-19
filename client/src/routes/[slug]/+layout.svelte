<script lang="ts">
	import * as AlertDialog from "$lib/components/ui/alert-dialog/index.js";
	import { goto } from "$app/navigation";
	import { page } from "$app/state";
	import { deleteInstance, machineHello, machineBye } from "$lib/api/client.js";
	import { getInstances } from "$lib/stores/instances.svelte.js";
	import { getPresentationState } from "$lib/stores/presentation.svelte.js";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { getSkinStore } from "$lib/stores/skin.svelte.js";
	import { getVoiceState } from "$lib/stores/voice.svelte.js";
	import InstanceOnboarding from "$lib/components/onboarding/InstanceOnboarding.svelte";
	import UpdateBanner from "$lib/components/UpdateBanner.svelte";
	let { children } = $props();

	const slug = $derived(page.params.slug!);
	const instances = getInstances();
	const presentation = getPresentationState();
	const scene = getSceneStore();
	const skinStore = getSkinStore();
	const voice = getVoiceState();
	const isNew = $derived(
		!instances.loading && !instances.error && !instances.list.some((i) => i.slug === slug)
	);
	const checking = $derived(instances.loading);

	// Fetch instance settings and enter chat mode when ready
	$effect(() => {
		if (!checking && !instances.error && !isNew) {
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

	const tabs = ["chat", "drops", "thoughts", "memory", "stats", "agents", "skills", "settings"] as const;
	const activeTab = $derived(
		tabs.find((t) => page.url.pathname.includes(`/${slug}/${t}`)) ?? "chat"
	);

	let showDeleteConfirm = $state(false);
	let confirmSlug = $state("");
	let deleting = $state(false);
	let deleteError = $state("");

	function handleOnboardingComplete() {
		instances.refresh();
	}

	async function handleDelete() {
		if (confirmSlug !== slug || deleting) return;
		deleteError = "";
		deleting = true;
		try {
			await deleteInstance(slug);
			instances.remove(slug);
			showDeleteConfirm = false;
			goto("/");
		} catch (e) {
			deleteError = e instanceof Error ? e.message : "Could not delete companion. Try again.";
		} finally {
			deleting = false;
		}
	}

	function openDeleteConfirm() {
		confirmSlug = "";
		deleteError = "";
		showDeleteConfirm = true;
	}

	function closeDeleteConfirm() {
		if (deleting) return;
		showDeleteConfirm = false;
		confirmSlug = "";
	}
</script>

{#if checking}
	<div class="flex h-full items-center justify-center">
		<p role="status">Loading your companion…</p>
	</div>
{:else if instances.error}
	<div class="connection-state"><h1>Cannot reach Nolune</h1><p role="alert">{instances.error}</p><button class="nl-button" onclick={() => instances.refresh()}>Retry connection</button><a href="/">Back to home</a></div>
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
			<a
				href="/"
				class="instance-tab instance-tab-home"
				title="All companions" aria-label="All companions"
			>
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="instance-tab-icon">
					<path d="M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2z" stroke-linecap="round" stroke-linejoin="round"/>
				</svg>
			</a>
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
			<div class="instance-tab-spacer"></div>
			<button
				class="instance-tab instance-tab-delete"
				onclick={openDeleteConfirm}
				title="Delete companion" aria-label="Delete companion"
			>
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="instance-tab-icon">
					<path d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" stroke-linecap="round" stroke-linejoin="round"/>
				</svg>
			</button>
		</nav>
		{/if}

		<div class="instance-content" class:instance-content-backdrop={activeTab !== "chat"}>
			{@render children()}
		</div>
	</div>
	</div>

	<AlertDialog.Root bind:open={showDeleteConfirm}>
		<AlertDialog.Content class="max-h-[90dvh] overflow-y-auto rounded-2xl border-border bg-card p-6" escapeKeydownBehavior={deleting ? "ignore" : "close"}>
			<AlertDialog.Header>
				<AlertDialog.Title>Delete “{slug}”?</AlertDialog.Title>
				<AlertDialog.Description>This permanently deletes this companion’s conversations, personality, memory, drops, uploaded files, and configuration. This cannot be undone.</AlertDialog.Description>
			</AlertDialog.Header>
			<label for="confirm-slug" class="text-sm">Type <strong>{slug}</strong> to confirm</label>
			<input id="confirm-slug" class="nl-input" bind:value={confirmSlug} placeholder={slug} disabled={deleting} autocomplete="off" onkeydown={(e) => e.key === 'Enter' && handleDelete()} />
			{#if deleteError}<p class="text-sm text-destructive" role="alert">{deleteError}</p>{/if}
			<AlertDialog.Footer>
				<button class="nl-button-secondary" onclick={closeDeleteConfirm} disabled={deleting}>Cancel</button>
				<button class="nl-button nl-button-destructive" disabled={confirmSlug !== slug || deleting} onclick={handleDelete}>{deleting ? "Deleting…" : "Delete permanently"}</button>
			</AlertDialog.Footer>
		</AlertDialog.Content>
	</AlertDialog.Root>
{/if}

<style>
.connection-state{display:flex;flex-direction:column;align-items:center;justify-content:center;gap:16px;height:100%;padding:24px;text-align:center}.connection-state h1{font-size:28px}.connection-state p{color:var(--text-muted)}
.instance-outer,.instance-view{display:flex;flex-direction:column;min-height:0;max-width:100%;overflow:hidden}.instance-outer{height:100%}.instance-view{flex:1}
.instance-tabs{display:flex;gap:4px;padding:8px 24px 0;border-bottom:1px solid var(--border);flex-shrink:0;z-index:10;overflow-x:auto;background:var(--background);scrollbar-width:thin}
.instance-tab{display:flex;align-items:center;justify-content:center;min-height:44px;min-width:44px;padding:10px 12px;border-radius:8px 8px 0 0;flex-shrink:0;position:relative;white-space:nowrap;font:500 14px var(--font-body);text-transform:capitalize;color:var(--text-muted);text-decoration:none;cursor:pointer}
.instance-tab:hover{background:var(--card);color:var(--foreground)}.instance-tab-active{background:var(--accent);color:var(--accent-foreground)}.instance-tab-active::after{content:"";position:absolute;bottom:0;left:12px;right:12px;height:2px;background:var(--primary)}.instance-tab-icon{width:18px;height:18px}.instance-tab-spacer{flex:1}.instance-tab-delete{color:var(--destructive)}.instance-content{position:relative;flex:1;min-width:0;min-height:0;overflow:hidden}.instance-content-backdrop{background:var(--background)}
@media(max-width:720px){.instance-tabs{padding:8px 12px 0}}
</style>
