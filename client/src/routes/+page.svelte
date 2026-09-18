<script lang="ts">
	import { goto } from "$app/navigation";
	import { onMount } from "svelte";
	import { getInstances } from "$lib/stores/instances.svelte.js";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { fetchMeta, fetchChangelog, getUpdateChannel, setUpdateChannel, type ChangelogEntry } from "$lib/api/client.js";
	import { Marked } from "marked";

	const instances = getInstances();
	const scene = getSceneStore();

	let version = $state("");
	let commit = $state("");
	let changelog = $state<ChangelogEntry[]>([]);
	let showChangelog = $state(false);
	let channel = $state("stable");
	const md = new Marked({ breaks: true, gfm: true });

	onMount(async () => {
		scene.enterHome();
		try {
			const meta = await fetchMeta();
			version = meta.version;
			commit = meta.commit;
		} catch {}
		fetchChangelog().then(c => changelog = c).catch(() => {});
		getUpdateChannel().then(r => channel = r.channel).catch(() => {});
	});

	// Keep scene instances in sync
	$effect(() => { scene.setInstances(instances.list); });

	// Navigate when a sphere is clicked (scene sets pendingSelect)
	$effect(() => {
		const slug = scene.pendingSelect;
		if (slug) {
			scene.pendingSelect = null;
			// Navigate after sphere starts moving to center
			setTimeout(() => goto(`/${slug}`), 350);
		}
	});

	let newSlug = $state("");
	let showCreate = $state(false);

	function create() {
		const name = newSlug.trim();
		if (!name) return;
		const slug = name
			.toLowerCase()
			.replace(/[^a-z0-9_-]/g, "-")
			.replace(/-+/g, "-")
			.replace(/^-|-$/g, "");
		if (!slug) return;
		localStorage.setItem("nolune:preferredName", name);
		goto(`/${slug}`);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter") { e.preventDefault(); create(); }
		if (e.key === "Escape") { showCreate = false; newSlug = ""; }
	}

	function getGreeting(): string {
		const hour = new Date().getHours();
		if (hour < 6) return "still up?";
		if (hour < 12) return "good morning";
		if (hour < 17) return "good afternoon";
		if (hour < 22) return "good evening";
		return "late night?";
	}

	const uiVisible = $derived(scene.mode === "home");
</script>

<div class="home">
	<div class="home-ui" class:home-ui-hidden={!uiVisible}>
		{#if instances.loading}
			<div class="empty-state">
				<div class="loading-dot"></div>
			</div>
		{:else if instances.list.length === 0}
			<!-- Empty state: big centered get started -->
			<div class="empty-state">
				<div class="empty-glow"></div>
				<p class="empty-greeting">{getGreeting()}</p>
				<h1 class="empty-title">get started</h1>
				<p class="empty-sub">create your first companion</p>

				{#if !showCreate}
					<button class="empty-cta" onclick={() => showCreate = true}>
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="empty-cta-icon"><path d="M12 5v14M5 12h14" stroke-linecap="round"/></svg>
						<span>new companion</span>
					</button>
				{:else}
					<div class="create-field">
						<!-- svelte-ignore a11y_autofocus -->
						<input bind:value={newSlug} onkeydown={handleKeydown} placeholder="what's your name?" autofocus class="create-input" />
						{#if newSlug.trim()}
							<button onclick={create} class="create-go" aria-label="Create">
								<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-4 h-4"><path d="M5 12h14" stroke-linecap="round"/><path d="m12 5 7 7-7 7" stroke-linecap="round" stroke-linejoin="round"/></svg>
							</button>
						{/if}
					</div>
				{/if}

				<div class="empty-hints">
					<span>helps you study</span>
					<span class="sep">·</span>
					<span>thinks with you</span>
					<span class="sep">·</span>
					<span>feels your mood</span>
				</div>
			</div>
		{:else}
			<!-- Normal state: hero + instances -->
			<div class="hero">
				<p class="greeting">{getGreeting()}</p>
				<h1 class="title">
					your friend that<br/>
					<span class="title-accent">actually gets you</span>
				</h1>
			</div>

			<!-- Mobile: list of instances (3D spheres don't fit on small screens) -->
			{#if !instances.loading && instances.list.length > 0}
				<div class="mobile-list">
					{#each instances.list as inst (inst.slug)}
						<button class="mobile-card" onclick={() => goto(`/${inst.slug}`)}>
							<div class="mobile-card-orb"></div>
							<div class="mobile-card-info">
								<span class="mobile-card-name">{inst.companion_name || inst.slug}</span>
								<span class="mobile-card-slug">{inst.slug}</span>
							</div>
							<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="mobile-card-arrow"><path d="m9 18 6-6-6-6" stroke-linecap="round" stroke-linejoin="round"/></svg>
						</button>
					{/each}
				</div>
			{/if}

			<div class="bottom">
				{#if !showCreate}
					<button class="new-btn" onclick={() => showCreate = true}>
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="new-icon"><path d="M12 5v14M5 12h14" stroke-linecap="round"/></svg>
						<span>new companion</span>
					</button>
				{/if}

				{#if showCreate}
					<div class="create-field">
						<!-- svelte-ignore a11y_autofocus -->
						<input bind:value={newSlug} onkeydown={handleKeydown} placeholder="what's your name?" autofocus class="create-input" />
						{#if newSlug.trim()}
							<button onclick={create} class="create-go" aria-label="Create">
								<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" class="w-4 h-4"><path d="M5 12h14" stroke-linecap="round"/><path d="m12 5 7 7-7 7" stroke-linecap="round" stroke-linejoin="round"/></svg>
							</button>
						{/if}
					</div>
				{/if}

				<!-- Hover label from 3D scene (desktop only) -->
				{#if scene.hoveredSlug}
					<div class="hover-name">{instances.list.find(i => i.slug === scene.hoveredSlug)?.companion_name || scene.hoveredSlug}</div>
				{/if}

				<div class="hints">
					<span>helps you study</span>
					<span class="sep">·</span>
					<span>thinks with you</span>
					<span class="sep">·</span>
					<span>feels your mood</span>
				</div>
			</div>
		{/if}
	</div>

	{#if version && uiVisible}
		<button class="version" onclick={() => showChangelog = !showChangelog}>
			v{version}{commit && commit !== "dev" ? ` · ${commit.slice(0, 7)}` : ""}
			{#if changelog.length > 0}
				<span class="version-dot"></span>
			{/if}
		</button>
	{/if}

	{#if showChangelog && uiVisible}
		<div class="changelog-panel">
			<div class="changelog-header">
				<span class="changelog-title">what's new</span>
				<select
					class="changelog-channel"
					value={channel}
					onchange={async (e) => {
						const val = (e.target as HTMLSelectElement).value;
						channel = val;
						await setUpdateChannel(val);
					}}
				>
					<option value="stable">stable</option>
					<option value="nightly">nightly</option>
				</select>
				<button class="changelog-close" aria-label="Close" onclick={() => showChangelog = false}>
					<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" width="16" height="16"><path d="M18 6L6 18M6 6l12 12"/></svg>
				</button>
			</div>
			<div class="changelog-scroll">
				{#each changelog.slice(0, 5) as entry}
					<div class="changelog-entry">
						<div class="changelog-version">{entry.version}</div>
						<div class="changelog-body">{@html md.parse(entry.body)}</div>
					</div>
				{/each}
			</div>
		</div>
	{/if}
</div>

<style>
	.home { position: relative; width: 100%; height: 100%; overflow: hidden; pointer-events: none; }

	.home-ui {
		position: relative; z-index: 10;
		display: flex; flex-direction: column; align-items: center; justify-content: space-between;
		height: 100%; padding: 0 1.5rem;
		pointer-events: none;
		transition: opacity 0.5s ease, transform 0.5s ease;
	}
	.home-ui > * { pointer-events: auto; }
	.home-ui-hidden { opacity: 0; transform: translateY(-12px); pointer-events: none !important; }
	.home-ui-hidden > * { pointer-events: none !important; }

	/* ── Empty state (no instances) ── */
	.empty-state {
		position: absolute; inset: 0;
		display: flex; flex-direction: column; align-items: center; justify-content: center;
		gap: 0; padding: 2rem;
		animation: empty-enter 1s cubic-bezier(0.16, 1, 0.3, 1) both;
	}
	@keyframes empty-enter {
		from { opacity: 0; transform: scale(0.96) translateY(20px); }
		to { opacity: 1; transform: scale(1) translateY(0); }
	}

	.empty-glow {
		position: absolute;
		width: clamp(280px, 50vw, 500px); height: clamp(280px, 50vw, 500px);
		border-radius: 50%;
		background: radial-gradient(
			circle,
			oklch(0.50 0.12 240 / 12%) 0%,
			oklch(0.45 0.08 260 / 6%) 40%,
			transparent 70%
		);
		pointer-events: none;
		animation: glow-breathe 6s ease-in-out infinite;
	}
	@keyframes glow-breathe {
		0%, 100% { opacity: 0.6; transform: scale(1); }
		50% { opacity: 1; transform: scale(1.08); }
	}

	.empty-greeting {
		font-family: var(--font-display); font-size: 0.8rem; font-weight: 300;
		font-style: italic; letter-spacing: 0.18em; text-transform: lowercase;
		color: oklch(0.55 0.06 240 / 40%);
		margin-bottom: 1rem;
		animation: empty-enter 0.8s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 200ms;
	}

	.empty-title {
		font-family: var(--font-display); font-style: italic; font-weight: 300;
		font-size: clamp(3rem, 10vw, 6rem);
		letter-spacing: -0.03em; line-height: 1;
		color: var(--foreground);
		text-align: center;
		margin-bottom: 0.5rem;
		animation: empty-enter 0.9s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 300ms;
	}

	.empty-sub {
		font-family: var(--font-body); font-size: 0.85rem; font-weight: 300;
		letter-spacing: 0.06em;
		color: oklch(0.60 0.04 240 / 40%);
		margin-bottom: 2.5rem;
		animation: empty-enter 0.9s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 400ms;
	}

	.empty-cta {
		display: flex; align-items: center; gap: 0.75rem;
		padding: 1rem 2.5rem; border-radius: 3rem;
		background: oklch(0.50 0.10 240 / 20%);
		border: 1px solid oklch(0.55 0.10 240 / 25%);
		color: oklch(0.85 0.04 240 / 90%);
		font-family: var(--font-display); font-style: italic;
		font-size: 1.1rem; font-weight: 400;
		cursor: pointer;
		transition: all 0.35s cubic-bezier(0.16, 1, 0.3, 1);
		animation: empty-enter 1s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 550ms;
		box-shadow: 0 0 40px oklch(0.45 0.08 240 / 12%);
	}
	.empty-cta:hover {
		background: oklch(0.55 0.12 240 / 30%);
		border-color: oklch(0.60 0.12 240 / 35%);
		color: oklch(0.92 0.03 75);
		transform: translateY(-2px);
		box-shadow: 0 4px 50px oklch(0.50 0.10 240 / 20%);
	}
	.empty-cta:active {
		transform: translateY(0) scale(0.98);
	}
	.empty-cta-icon { width: 20px; height: 20px; }

	.empty-state .create-field {
		animation: empty-enter 0.4s cubic-bezier(0.16, 1, 0.3, 1) both;
	}
	.empty-state .create-input {
		width: 320px; padding: 1rem 1.5rem; border-radius: 3rem;
		font-size: 1rem;
	}

	.empty-hints {
		display: flex; align-items: center; gap: 0.625rem;
		font-family: var(--font-body); font-size: 0.72rem; letter-spacing: 0.04em;
		color: var(--foreground);
		margin-top: 3rem;
		animation: empty-enter 1s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 700ms;
	}
	.empty-hints .sep { font-size: 0.5rem; color: oklch(0.50 0.06 240 / 10%); }
	@media (max-width: 540px) {
		.empty-hints { flex-direction: column; gap: 0.3rem; }
		.empty-hints .sep { display: none; }
	}

	.hero {
		text-align: center; padding-top: clamp(3rem, 14vh, 10rem);
		animation: enter-up 0.7s cubic-bezier(0.16, 1, 0.3, 1) both;
	}
	@keyframes enter-up {
		from { opacity: 0; transform: translateY(16px); }
		to { opacity: 1; transform: translateY(0); }
	}
	.greeting {
		font-family: var(--font-display); font-size: 0.72rem; font-weight: 300;
		font-style: italic; letter-spacing: 0.15em; text-transform: lowercase;
		color: oklch(0.50 0.06 240 / 40%); margin-bottom: 0.75rem;
		animation: enter-up 0.6s cubic-bezier(0.16, 1, 0.3, 1) both; animation-delay: 100ms;
	}
	.title {
		font-family: var(--font-display); font-size: clamp(1.5rem, 4.5vw, 2.3rem);
		font-weight: 300; line-height: 1.25; letter-spacing: -0.01em;
		color: var(--foreground);
		animation: enter-up 0.7s cubic-bezier(0.16, 1, 0.3, 1) both; animation-delay: 150ms;
	}
	.title-accent { font-style: italic; font-weight: 400; color: oklch(0.50 0.06 240 / 70%); }

	.bottom {
		display: flex; flex-direction: column; align-items: center; gap: 1.25rem;
		padding-bottom: calc(clamp(2rem, 5vh, 4rem) + env(safe-area-inset-bottom, 0px));
		animation: enter-up 0.7s cubic-bezier(0.16, 1, 0.3, 1) both; animation-delay: 400ms;
	}

	.new-btn {
		display: flex; align-items: center; gap: 0.5rem;
		padding: 0.5rem 1.25rem; border-radius: 2rem;
		background: var(--glass-bg); backdrop-filter: var(--glass-blur);
		border: 1px solid var(--glass-border); border-top-color: var(--glass-border-top);
		color: oklch(var(--ink) / 35%); font-family: var(--font-mono);
		font-size: 0.72rem; letter-spacing: 0.05em; cursor: pointer; transition: all 0.3s ease;
	}
	.new-btn:hover { background: oklch(var(--ink) / 8%); color: oklch(var(--ink) / 55%); }
	.new-icon { width: 14px; height: 14px; }

	.create-field { position: relative; animation: enter-up 0.4s cubic-bezier(0.16, 1, 0.3, 1) both; }
	.create-input {
		width: 280px; padding: 0.75rem 1.25rem; border-radius: 2rem;
		border: 1px solid var(--glass-border); border-top-color: var(--glass-border-top);
		background: var(--glass-bg); backdrop-filter: var(--glass-blur);
		color: var(--foreground); font-family: var(--font-display);
		font-size: 0.875rem; font-style: italic; outline: none; transition: all 0.4s ease;
	}
	.create-input::placeholder { color: oklch(0.50 0.05 240 / 30%); font-style: italic; }
	.create-input:focus { border-color: oklch(var(--ink) / 16%); box-shadow: 0 0 0 4px oklch(0.40 0.06 var(--accent-hue) / 8%); }
	.create-go {
		position: absolute; right: 0.375rem; top: 50%; transform: translateY(-50%);
		display: flex; align-items: center; justify-content: center;
		width: 2rem; height: 2rem; border-radius: 50%;
		background: oklch(0.50 0.08 240 / 50%); color: oklch(0.065 0.015 280); transition: all 0.2s ease;
	}
	.create-go:hover { background: oklch(0.50 0.08 240 / 70%); }

	.hover-name {
		font-family: var(--font-display); font-size: 0.85rem; font-weight: 300;
		font-style: italic; letter-spacing: 0.04em; color: oklch(var(--ink) / 45%);
		animation: enter-up 0.3s ease both;
	}

	.hints {
		display: flex; align-items: center; gap: 0.625rem;
		font-family: var(--font-body); font-size: 0.72rem; letter-spacing: 0.04em;
		color: var(--foreground);
	}
	.sep { font-size: 0.5rem; color: oklch(0.50 0.06 240 / 12%); }
	@media (max-width: 540px) { .hints { flex-direction: column; gap: 0.3rem; } .sep { display: none; } }

	/* Mobile instance list — hidden on desktop where 3D spheres work */
	.mobile-list {
		display: none;
		flex-direction: column;
		gap: 0.5rem;
		width: 100%;
		max-width: 320px;
		padding: 0 0.5rem;
		animation: enter-up 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 300ms;
	}
	@media (max-width: 640px) {
		.mobile-list { display: flex; }
	}

	.mobile-card {
		display: flex;
		align-items: center;
		gap: 0.75rem;
		width: 100%;
		padding: 0.75rem 1rem;
		border-radius: 1rem;
		border: 1px solid oklch(0.5 0.06 220 / 10%);
		border-top-color: oklch(0.6 0.08 220 / 15%);
		background: linear-gradient(
			165deg,
			oklch(0.5 0.04 220 / 7%) 0%,
			oklch(0.4 0.03 230 / 4%) 100%
		);
		backdrop-filter: blur(16px) saturate(140%);
		-webkit-backdrop-filter: blur(16px) saturate(140%);
		cursor: pointer;
		transition: all 0.25s ease;
		text-align: left;
	}
	.mobile-card:active {
		transform: scale(0.97);
		background: oklch(0.5 0.06 220 / 12%);
	}
	.mobile-card-orb {
		width: 2.25rem;
		height: 2.25rem;
		border-radius: 50%;
		flex-shrink: 0;
		background: radial-gradient(
			circle at 35% 35%,
			oklch(0.55 0.10 220 / 30%) 0%,
			oklch(0.40 0.06 var(--accent-hue) / 15%) 60%,
			oklch(0.30 0.04 250 / 10%) 100%
		);
		border: 1px solid oklch(0.5 0.08 220 / 12%);
		box-shadow: 0 0 12px oklch(0.45 0.08 220 / 10%);
	}
	.mobile-card-info {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 0.1rem;
	}
	.mobile-card-name {
		font-family: var(--font-display);
		font-size: 0.9rem;
		font-weight: 400;
		color: oklch(0.88 0.02 220 / 80%);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.mobile-card-slug {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: oklch(0.60 0.04 220 / 35%);
		letter-spacing: 0.04em;
	}
	.mobile-card-arrow {
		width: 1rem;
		height: 1rem;
		flex-shrink: 0;
		color: oklch(0.55 0.04 220 / 25%);
		transition: transform 0.2s ease;
	}
	.mobile-card:active .mobile-card-arrow {
		transform: translateX(2px);
	}

	.loading-dot { width: 6px; height: 6px; border-radius: 50%; background: oklch(0.50 0.06 240 / 30%); animation: pulse 2s ease-in-out infinite; }
	@keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.25; } }

	.version {
		position: fixed; bottom: calc(0.5rem + env(safe-area-inset-bottom, 0px));
		left: 0; right: 0; text-align: center;
		font-family: var(--font-body); font-size: 0.68rem; letter-spacing: 0.05em;
		color: var(--foreground); cursor: pointer; z-index: 100;
		animation: enter-up 0.5s cubic-bezier(0.16, 1, 0.3, 1) both; animation-delay: 1s;
		display: flex; align-items: center; justify-content: center; gap: 0.375rem;
		pointer-events: auto;
		background: none; border: none; font: inherit;
		transition: color 0.2s ease;
	}
	.version:hover { color: var(--foreground); }
	.version-dot {
		width: 4px; height: 4px; border-radius: 50%;
		background: oklch(0.78 0.12 75);
		box-shadow: 0 0 6px oklch(0.78 0.12 75 / 50%);
	}

	.changelog-panel {
		position: fixed; bottom: calc(2.5rem + env(safe-area-inset-bottom, 0px));
		left: 50%; transform: translateX(-50%);
		width: 380px; max-width: calc(100vw - 2rem); max-height: 50vh;
		z-index: 200; pointer-events: auto;
		background: oklch(0.06 0.015 260 / 90%);
		backdrop-filter: blur(32px) saturate(160%);
		-webkit-backdrop-filter: blur(32px) saturate(160%);
		border: 1px solid oklch(var(--ink) / 8%);
		border-top-color: oklch(var(--ink) / 14%);
		border-radius: 1rem;
		box-shadow: 0 8px 40px oklch(var(--shade) / 40%);
		animation: changelog-in 0.3s cubic-bezier(0.16, 1, 0.3, 1) both;
		display: flex; flex-direction: column;
	}
	@keyframes changelog-in {
		from { opacity: 0; transform: translateX(-50%) translateY(12px) scale(0.96); }
		to { opacity: 1; transform: translateX(-50%) translateY(0) scale(1); }
	}

	.changelog-header {
		display: flex; align-items: center; justify-content: space-between;
		padding: 0.875rem 1rem 0.5rem;
		border-bottom: 1px solid oklch(var(--ink) / 6%);
	}
	.changelog-title {
		font-family: var(--font-display); font-style: italic;
		font-size: 0.9rem; color: var(--foreground);
	}
	.changelog-channel {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: oklch(var(--ink) / 35%);
		background: none;
		border: 1px solid oklch(var(--ink) / 8%);
		border-radius: 4px;
		padding: 0.15rem 0.35rem;
		cursor: pointer;
		outline: none;
		margin-left: auto;
		transition: all 0.2s ease;
	}
	.changelog-channel:hover {
		border-color: oklch(var(--ink) / 15%);
		color: oklch(var(--ink) / 55%);
	}
	.changelog-channel option {
		background: oklch(0.1 0.01 240);
		color: oklch(0.8 0.04 240);
	}
	.changelog-close {
		color: oklch(var(--ink) / 30%); cursor: pointer;
		background: none; border: none; padding: 0.25rem;
		transition: color 0.2s ease;
	}
	.changelog-close:hover { color: oklch(var(--ink) / 60%); }

	.changelog-scroll {
		overflow-y: auto; padding: 0.75rem 1rem; display: flex;
		flex-direction: column; gap: 0.75rem;
	}
	.changelog-entry {
		padding: 0.625rem 0.75rem; border-radius: 0.5rem;
		background: oklch(var(--ink) / 3%); border: 1px solid oklch(var(--ink) / 5%);
	}
	.changelog-version {
		font-family: var(--font-mono); font-size: 0.68rem;
		color: oklch(0.78 0.12 75 / 70%); margin-bottom: 0.25rem; letter-spacing: 0.03em;
	}
	.changelog-body {
		font-size: 0.75rem; line-height: 1.55; color: var(--foreground);
	}
	.changelog-body :global(h2) {
		font-size: 0.8rem; font-weight: 600; color: var(--foreground);
		margin: 0.75rem 0 0.35rem;
	}
	.changelog-body :global(h2:first-child) { margin-top: 0; }
	.changelog-body :global(ul) { padding-left: 1.2rem; margin: 0.25rem 0; }
	.changelog-body :global(li) { margin: 0.15rem 0; }
	.changelog-body :global(strong) { color: var(--foreground); }
	.changelog-body :global(img) {
		max-width: 100%; border-radius: 0.5rem; margin: 0.5rem 0;
	}
	.changelog-body :global(video) {
		max-width: 100%; border-radius: 0.5rem; margin: 0.5rem 0;
	}
	.changelog-body :global(a) {
		color: oklch(0.65 0.1 190 / 60%); text-decoration: none;
	}
	.changelog-body :global(a:hover) { text-decoration: underline; }
</style>
