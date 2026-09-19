<script lang="ts">
	import {
		fetchSkills,
		deleteSkill,
		fetchRegistry,
		installRegistrySkill,
	} from "$lib/api/client.js";
	import type { Skill, RegistryEntry } from "$lib/api/types.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import SkillCard from "./SkillCard.svelte";
	import RegistryCard from "./RegistryCard.svelte";
	import CreateSkillModal from "./CreateSkillModal.svelte";

	const toast = getToasts();

	let skills = $state<Skill[]>([]);
	let loading = $state(true);
	let loadError = $state("");
	let showCreate = $state(false);

	let mode = $state<"installed" | "browse">("installed");
	let registry = $state<RegistryEntry[]>([]);
	let registryLoading = $state(false);
	let registryError = $state("");
	let installingId = $state<string | null>(null);

	async function load() {
		loading = true;
		loadError = "";
		try {
			skills = await fetchSkills();
		} catch {
			loadError = "Could not load skills. Please try again.";
			toast.error("failed to load skills");
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
	});

	async function loadRegistry() {
		if (registry.length > 0) return;
		registryLoading = true;
		registryError = "";
		try {
			registry = await fetchRegistry();
		} catch (e) {
			console.error("failed to load registry", e);
			registryError =
				e instanceof Error ? e.message : "failed to load registry";
		} finally {
			registryLoading = false;
		}
	}

	function switchMode(m: "installed" | "browse") {
		mode = m;
		if (m === "browse") loadRegistry();
	}

	async function handleDelete(skillId: string) {
		try {
			await deleteSkill(skillId);
			skills = skills.filter((s) => s.id !== skillId);
			// Update registry installed status
			registry = registry.map((e) =>
				e.id === skillId ? { ...e, installed: false } : e,
			);
		} catch {
			toast.error("failed to delete skill");
		}
	}

	function handleCreated(skill: Skill) {
		skills = [...skills, skill];
		showCreate = false;
	}

	async function handleInstall(id: string) {
		installingId = id;
		try {
			const skill = await installRegistrySkill(id);
			skills = [...skills, skill];
			registry = registry.map((e) =>
				e.id === id ? { ...e, installed: true } : e,
			);
		} catch {
			toast.error("failed to install skill");
		} finally {
			installingId = null;
		}
	}
</script>

<div class="skills-container">
	{#if loading}
		<span class="sr-only" role="status">Loading skills…</span>
		<div class="skills-loading">
			<div class="skills-loading-dot"></div>
		</div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else}
		<div class="skills-header">
			<div class="skills-tabs">
				<button
					class="skills-tab"
					class:skills-tab-active={mode === "installed"}
					onclick={() => switchMode("installed")}
				>
					Installed
					<span class="skills-tab-count">{skills.length}</span>
				</button>
				<button
					class="skills-tab"
					class:skills-tab-active={mode === "browse"}
					onclick={() => switchMode("browse")}
				>
					Browse
				</button>
			</div>
			{#if mode === "installed"}
				<button class="skills-add" onclick={() => (showCreate = true)}>
					+ New skill
				</button>
			{/if}
		</div>

		{#if mode === "installed"}
			{#if skills.length === 0}
				<div class="skills-empty">
					<div class="skills-empty-icon">+</div>
					<p class="skills-empty-text">No skills yet</p>
					<p class="skills-empty-hint">
						skills extend what your companion can do — teach it new
						behaviors, workflows, and abilities.
					</p>
				</div>
			{:else}
				<div class="skills-grid">
					{#each skills as skill (skill.id)}
						<SkillCard
							{skill}
							ondelete={() => handleDelete(skill.id)}
						/>
					{/each}
				</div>
			{/if}
		{:else if mode === "browse"}
			{#if registryLoading}
				<div class="skills-loading">
					<div class="skills-loading-dot"></div>
				</div>
			{:else if registryError}
				<div class="skills-empty">
					<p class="skills-empty-text">Could not load registry</p>
					<p class="skills-empty-hint">{registryError}</p>
					<button
						class="skills-add"
						onclick={() => {
							registry = [];
							loadRegistry();
						}}
					>
						Retry
					</button>
				</div>
			{:else if registry.length === 0}
				<div class="skills-empty">
					<p class="skills-empty-text">No community skills available</p>
					<p class="skills-empty-hint">
						the registry is empty — check back later or set a custom
						registry URL in config.toml
					</p>
				</div>
			{:else}
				<div class="skills-grid">
					{#each registry as entry (entry.id)}
						<RegistryCard
							{entry}
							installing={installingId === entry.id}
							oninstall={handleInstall}
						/>
					{/each}
				</div>
			{/if}
		{/if}
	{/if}

	{#if showCreate}
		<CreateSkillModal
			onclose={() => (showCreate = false)}
			oncreated={handleCreated}
		/>
	{/if}
</div>

<style>
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.skills-container {
		height: 100%;
		overflow-y: auto;
		padding: 2rem 1.5rem;
	}

	.skills-loading {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
	}

	.skills-loading-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--card);
		animation:none;
	}

	.skills-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 0.5rem;
		margin-bottom: 1.25rem;
	}

	.skills-tabs {
		display: flex;
		gap: 0.125rem;
		background: var(--card);
		border-radius: 0.5rem;
		padding: 0.15rem;
	}

	.skills-tab {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: none;
		border: none;
		padding: 0.3rem 0.65rem;
		border-radius: 0.375rem;
		cursor: pointer;
		letter-spacing: 0.04em;
		transition: all 0.2s ease;
		display: flex;
		align-items: center;
		gap: 0.35rem;
	}

	.skills-tab:hover {
		color: var(--text-secondary);
	}

	.skills-tab-active {
		color: var(--text-secondary);
		background: var(--card);
	}

	.skills-tab-count {
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.skills-add {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: var(--card);
		border: 1px solid var(--border);
		padding: 0.35rem 0.75rem;
		border-radius: 0.5rem;
		cursor: pointer;
		letter-spacing: 0.04em;
		transition: all 0.2s ease;
	}

	.skills-add:hover {
		color: var(--text-secondary);
		background: var(--card);
		border-color: var(--border);
	}

	.skills-empty {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: calc(100% - 3rem);
		gap: 0.75rem;
		text-align: center;
	}

	.skills-empty-icon {
		font-family: var(--font-body);
		font-size: 1.5rem;
		color: var(--text-secondary);
		animation:none;
	}

	.skills-empty-text {
		font-family: var(--font-display);
		font-size: 0.95rem;
		color: var(--text-secondary);
	}

	.skills-empty-hint {
		font-size: 0.75rem;
		color: var(--text-secondary);
		max-width: 30ch;
		line-height: 1.5;
	}

	.skills-grid {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(260px, 1fr));
		grid-auto-rows: min-content;
		align-items: start;
		gap: 0.75rem;
	}

	@media (max-width: 640px) {
		.skills-grid {
			grid-template-columns: 1fr;
		}
		.skills-container {
			padding: 1.5rem 1rem;
		}
	}

/* Little Moon surfaces, controls, and readable content. */

.skills-container { padding: 32px; }
.skills-header, .skills-grid { max-width: 1040px; margin-left: auto; margin-right: auto; }
.skills-add { background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); padding: 8px 16px; }
.skills-add:hover { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); }
.skills-tab { min-height: 44px; font-size: 14px; padding: 8px 16px; }
.skills-tab-active { color: var(--primary); background: var(--accent); }
.skills-empty-text { font: 400 28px var(--font-display); color: var(--foreground); }
.skills-empty-hint { font-size: 14px; max-width: 42ch; }
@media (max-width: 640px) { .skills-container { padding: 20px; } }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.skills-loading-dot { background: var(--primary); }
</style>
