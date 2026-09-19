<script lang="ts">
	import {
		fetchSoul,
		updateSoul,
		fetchSoulTemplates,
		applySoulTemplate,
	} from "$lib/api/client.js";
	import type { Soul, SoulTemplate } from "$lib/api/types.js";
	import X from "@lucide/svelte/icons/x";
	import Save from "@lucide/svelte/icons/save";
	import RotateCcw from "@lucide/svelte/icons/rotate-ccw";
	import Sparkles from "@lucide/svelte/icons/sparkles";
	import ChevronLeft from "@lucide/svelte/icons/chevron-left";

	let { slug, onclose }: { slug: string; onclose: () => void } = $props();

	let soul = $state<Soul | null>(null);
	let templates = $state<SoulTemplate[]>([]);
	let editContent = $state("");
	let loading = $state(true);
	let saving = $state(false);
	let saved = $state(false);
	let view = $state<"editor" | "templates">("editor");
	let dirty = $derived(soul !== null && editContent !== soul.content);

	$effect(() => {
		loading = true;
		Promise.all([fetchSoul(slug), fetchSoulTemplates()])
			.then(([s, t]) => {
				soul = s;
				editContent = s.content;
				templates = t;
				if (!s.exists) {
					view = "templates";
				}
			})
			.catch(() => {})
			.finally(() => {
				loading = false;
			});
	});

	async function save() {
		if (!dirty) return;
		saving = true;
		try {
			const updated = await updateSoul(slug, editContent);
			soul = updated;
			saved = true;
			setTimeout(() => (saved = false), 2000);
		} finally {
			saving = false;
		}
	}

	async function pickTemplate(template: SoulTemplate) {
		if (template.id === "custom") {
			editContent = template.content;
			soul = { content: template.content, exists: false };
			view = "editor";
			return;
		}

		saving = true;
		try {
			const updated = await applySoulTemplate(slug, template.id);
			soul = updated;
			editContent = updated.content;
			view = "editor";
		} finally {
			saving = false;
		}
	}

	function reset() {
		if (soul) {
			editContent = soul.content;
		}
	}

	function handleKeydown(e: KeyboardEvent) {
		if ((e.metaKey || e.ctrlKey) && e.key === "s") {
			e.preventDefault();
			save();
		}
		if (e.key === "Escape") {
			onclose();
		}
	}
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="soul-panel">
	<!-- header -->
	<div class="flex items-center gap-3 border-b border-border/60 px-5 py-3.5">
		{#if view === "templates"}
			<button
				onclick={() => (view = "editor")}
				class="flex h-11 w-11 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-muted/50 hover:text-foreground"
				disabled={!soul?.exists}
			>
				<ChevronLeft class="h-4 w-4" />
			</button>
		{:else}
			<div class="flex h-11 w-11 items-center justify-center rounded-lg bg-accent">
				<Sparkles class="h-3.5 w-3.5 text-primary" />
			</div>
		{/if}

		<div class="flex-1">
			<h2 class="font-display text-sm font-semibold tracking-tight">
				{view === "templates" ? "Choose a soul" : "Soul"}
			</h2>
			<p class="text-[13px] text-muted-foreground">
				{view === "templates"
					? "Pick a personality template"
					: soul?.exists
						? "Defines who your companion is"
						: "No soul yet"}
			</p>
		</div>

		<div class="flex items-center gap-1.5">
			{#if view === "editor" && soul?.exists}
				<button
					onclick={() => (view = "templates")}
					class="soul-header-btn"
					title="Browse templates"
				>
					<Sparkles class="h-3.5 w-3.5" />
				</button>
			{/if}

			{#if view === "editor" && dirty}
				<button onclick={reset} class="soul-header-btn" title="Discard changes">
					<RotateCcw class="h-3.5 w-3.5" />
				</button>
				<button
					onclick={save}
					disabled={saving}
					class="flex items-center gap-1.5 min-h-11 rounded-lg bg-primary px-4 py-2 text-sm font-medium text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-50"
				>
					<Save class="h-3 w-3" />
					{saving ? "saving..." : "save"}
				</button>
			{/if}

			{#if saved && !dirty}
				<span class="text-xs text-primary">saved</span>
			{/if}

			<button onclick={onclose} class="soul-header-btn ml-1" title="Close">
				<X class="h-3.5 w-3.5" />
			</button>
		</div>
	</div>

	<!-- body -->
	{#if loading}
		<div class="flex flex-1 items-center justify-center">
			<div
				class="h-5 w-5 animate-spin rounded-full border-2 border-warm/30 border-t-warm"
			></div>
		</div>
	{:else if view === "templates"}
		<div class="flex-1 overflow-y-auto p-5">
			<div class="grid gap-3">
				{#each templates as template (template.id)}
					<button
						onclick={() => pickTemplate(template)}
						class="soul-template-card"
						disabled={saving}
					>
						<div class="flex items-start gap-3">
							<div
								class="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-accent"
							>
								<Sparkles class="h-4 w-4 text-primary" />
							</div>
							<div class="text-left">
								<p class="font-display text-sm font-medium text-foreground">
									{template.name}
								</p>
								<p class="mt-0.5 text-xs text-muted-foreground">
									{template.description}
								</p>
							</div>
						</div>
					</button>
				{/each}
			</div>
		</div>
	{:else}
		<div class="flex flex-1 flex-col overflow-hidden">
			<textarea
				bind:value={editContent}
				placeholder={"# soul\n\ndefine who your companion is...\n\n## voice\nhow do they speak?\n\n## personality\nwhat drives them?"}
				spellcheck={false}
				class="soul-textarea"
			></textarea>

			<div
				class="flex items-center justify-between border-t border-border/40 px-4 py-2"
			>
				<span class="text-[13px] text-muted-foreground">
					markdown &middot; {editContent.length} chars
				</span>
				<span class="text-[13px] text-muted-foreground">
					{#if dirty}unsaved changes{:else}&nbsp;{/if}
				</span>
			</div>
		</div>
	{/if}
</div>

<style>
	.soul-panel {
		display: flex;
		flex-direction: column;
		height: 100%;
		animation: soul-enter 0.3s cubic-bezier(0.16, 1, 0.3, 1) both;
	}

	@keyframes soul-enter {
		from {
			opacity: 0;
			transform: translateX(12px);
		}
		to {
			opacity: 1;
			transform: translateX(0);
		}
	}

	.soul-header-btn {
		display: flex;
		height: 44px;
		width: 44px;
		align-items: center;
		justify-content: center;
		border-radius: 0.375rem;
		color: var(--text-secondary);
		transition: all 0.15s ease;
	}
	.soul-header-btn:hover {
		background: var(--accent);
		color: var(--foreground);
	}

	.soul-textarea {
		flex: 1;
		resize: none;
		padding: 1.25rem;
		font-family: ui-monospace, SFMono-Regular, "SF Mono", Menlo, monospace;
		font-size: 1rem;
		line-height: 1.7;
		color: var(--foreground);
		background: transparent;
		outline: none;
		tab-size: 2;
	}
	.soul-textarea::placeholder {
		color: var(--text-muted);
	}

	.soul-template-card {
		width: 100%;
		border-radius: 0.75rem;
		border: 1px solid var(--border);
		background: var(--card);
		padding: 0.875rem 1rem;
		cursor: pointer;
		transition: all 0.2s ease;
	}
	.soul-template-card:hover {
		border-color: var(--primary);
		background: var(--accent);
		box-shadow: none;
	}
	.soul-template-card:disabled {
		opacity: 0.5;
		cursor: wait;
	}

    .soul-panel { background: var(--card); min-width: 0; }
    .soul-panel > div:first-child { flex-wrap: wrap; gap: 8px; }
    .soul-textarea { background: var(--background); min-height: 160px; }
    .soul-textarea:focus-visible { outline: 2px solid var(--ring); outline-offset: -2px; }
    .soul-textarea::placeholder { opacity: 1; }
    .soul-template-card { padding: 16px; border-radius: 12px; }
</style>
