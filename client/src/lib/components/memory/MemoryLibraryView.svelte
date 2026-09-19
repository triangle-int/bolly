<script lang="ts">
	import { resourceMedia, resourceFromUrl, issueResource } from "$lib/api/resource-media.js";
	import { deleteMemoryFile, fetchMemory, fetchMemoryContent, fetchMemoryGraph, searchMemory, type MemorySearchResult } from "$lib/api/client.js";
	import type { MemoryEntry, MemoryGraph } from "$lib/api/types.js";
	import { displayName, filterEntries, formatSize, groupByFolder, mediaKind, relatedPaths } from "$lib/memory/library.js";
	import { openFile } from "$lib/stores/fileviewer.svelte.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";

	const toast = getToasts();
	let { slug }: { slug: string } = $props();

	let entries = $state<MemoryEntry[]>([]);
	let graph = $state<MemoryGraph>({ edges: [] });
	let loading = $state(true);
	let loadError = $state("");

	let query = $state("");
	let results = $state<MemorySearchResult[]>([]);
	let searching = $state(false);
	let searchError = $state("");
	let debounce: ReturnType<typeof setTimeout> | null = null;

	let viewing = $state<MemoryEntry | null>(null);
	let content = $state("");
	let contentLoading = $state(false);
	let confirmingDelete = $state(false);
	let deleting = $state(false);

	let groups = $derived(groupByFolder(filterEntries(entries, query)));
	let totalSize = $derived(entries.reduce((sum, e) => sum + e.size, 0));
	let related = $derived(viewing ? relatedPaths(graph, viewing.path) : []);

	async function load() {
		loading = true;
		loadError = "";
		try {
			const [e, g] = await Promise.all([fetchMemory(slug), fetchMemoryGraph(slug)]);
			entries = e;
			graph = g;
		} catch {
			loadError = "Could not load memories. Please try again.";
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		load();
	});

	// Server-side search (keyword + semantic) for queries of two characters or more.
	$effect(() => {
		const q = query.trim();
		searchError = "";
		if (q.length < 2) {
			results = [];
			searching = false;
			return;
		}
		searching = true;
		if (debounce) clearTimeout(debounce);
		debounce = setTimeout(async () => {
			try {
				results = await searchMemory(slug, q, 20);
			} catch {
				results = [];
				searchError = "Could not search memories. The library below still works.";
			} finally {
				searching = false;
			}
		}, 250);
	});

	async function open(entry: MemoryEntry) {
		viewing = entry;
		confirmingDelete = false;
		if (mediaKind(entry.path) !== "text") {
			content = "";
			contentLoading = false;
			return;
		}
		contentLoading = true;
		try {
			content = await fetchMemoryContent(slug, entry.path);
		} catch {
			content = "(could not load this memory)";
		} finally {
			contentLoading = false;
		}
	}

	function openByPath(path: string) {
		const entry = entries.find((e) => e.path === path);
		if (entry) open(entry);
	}

	async function openResult(result: MemorySearchResult) {
		const isMedia = result.source_type?.startsWith("media_") ?? false;
		const basePath = isMedia ? result.path : result.path.split("#")[0];
		const resource = resourceFromUrl(result.media_url ?? "", slug);
		if (isMedia && resource) {
			openFile((await issueResource(resource)).url, basePath || "media");
			return;
		}
		openByPath(basePath);
	}

	function back() {
		viewing = null;
		content = "";
		confirmingDelete = false;
	}

	async function forget() {
		if (!viewing || deleting) return;
		deleting = true;
		try {
			await deleteMemoryFile(slug, viewing.path);
			toast.success("Forgotten");
			back();
			await load();
		} catch {
			toast.error("Could not delete that memory.");
		} finally {
			deleting = false;
		}
	}
</script>

<div class="memory-page">
	{#if loading}
		<p role="status" class="memory-center">Loading memories…</p>
	{:else if loadError}
		<div class="memory-center" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if viewing}
		{@const kind = mediaKind(viewing.path)}
		<header class="memory-toolbar">
			<button class="nl-button-secondary" onclick={back}>← Library</button>
			<span class="memory-path">{viewing.path}</span>
			<span class="memory-meta">{formatSize(viewing.size)}</span>
		</header>
		<article class="memory-doc">
			{#if contentLoading}
				<p role="status" class="memory-meta">Loading…</p>
			{:else if kind === "image"}
				<img use:resourceMedia={{ slug, kind: "memory", path: viewing.path }} alt={viewing.path} class="doc-media" />
			{:else if kind === "video"}
				<video use:resourceMedia={{ slug, kind: "memory", path: viewing.path }} controls playsinline class="doc-media"><track kind="captions" /></video>
			{:else if kind === "audio"}
				<audio use:resourceMedia={{ slug, kind: "memory", path: viewing.path }} controls class="doc-audio"></audio>
			{:else if kind === "pdf"}
				<iframe use:resourceMedia={{ slug, kind: "memory", path: viewing.path }} class="doc-pdf" title="PDF viewer"></iframe>
			{:else}
				<pre class="doc-content">{content}</pre>
			{/if}
		</article>
		{#if related.length > 0}
			<section class="memory-related" aria-label="Related memories">
				<h3>Linked memories</h3>
				<ul>
					{#each related as path (path)}
						<li><button class="link-btn" onclick={() => openByPath(path)}>{path}</button></li>
					{/each}
				</ul>
			</section>
		{/if}
		<footer class="memory-danger">
			{#if confirmingDelete}
				<p class="memory-meta">Forget this memory permanently? Its file and search entries are removed.</p>
				<div class="memory-actions">
					<button class="nl-button-secondary" disabled={deleting} onclick={() => (confirmingDelete = false)}>Keep it</button>
					<button class="nl-button nl-button-destructive" disabled={deleting} onclick={forget}>{deleting ? "Forgetting…" : "Forget"}</button>
				</div>
			{:else}
				<button class="nl-button-secondary" onclick={() => (confirmingDelete = true)}>Forget this memory</button>
			{/if}
		</footer>
	{:else}
		<header class="memory-header">
			<div>
				<h2>Memory</h2>
				<p class="memory-meta">{entries.length} {entries.length === 1 ? "memory" : "memories"} · {formatSize(totalSize)}. Stored as plain files you can read, search, and forget.</p>
			</div>
			<label class="memory-search">
				<span class="sr-only">Search memories</span>
				<input class="nl-input" type="search" placeholder="Search memories" bind:value={query} />
			</label>
		</header>

		{#if query.trim().length >= 2}
			<section class="memory-results" aria-label="Search results">
				{#if searching}
					<p role="status" class="memory-meta">Searching…</p>
				{:else if searchError}
					<p role="alert" class="memory-meta">{searchError}</p>
				{:else if results.length === 0}
					<p class="memory-meta">No matches for “{query.trim()}”.</p>
				{:else}
					<ul class="memory-list">
						{#each results as result (result.path)}
							{@const isMedia = result.source_type?.startsWith("media_") ?? false}
							{@const basePath = isMedia ? result.path : result.path.split("#")[0]}
							<li>
								<button class="memory-row" onclick={() => openResult(result)}>
									<span class="memory-name">{isMedia ? basePath : displayName(basePath)}</span>
									<span class="memory-summary">{result.text.trim().slice(0, 160)}</span>
								</button>
							</li>
						{/each}
					</ul>
				{/if}
			</section>
		{/if}

		{#if entries.length === 0}
			<div class="memory-center">
				<p>No memories yet</p>
				<p class="memory-meta">Your companion writes memories as you talk. They show up here as files.</p>
			</div>
		{:else}
			{#each groups as group (group.folder)}
				<section class="memory-group">
					<h3>{group.folder || "Loose notes"} <span class="memory-meta">· {group.entries.length} · {formatSize(group.size)}</span></h3>
					<ul class="memory-list">
						{#each group.entries as entry (entry.path)}
							{@const kind = mediaKind(entry.path)}
							<li>
								<button class="memory-row" onclick={() => open(entry)}>
									<span class="memory-name">{displayName(entry.path)}{kind !== "text" ? ` · ${kind}` : ""}</span>
									<span class="memory-summary">{entry.summary}</span>
									<span class="memory-size">{formatSize(entry.size)}</span>
								</button>
							</li>
						{/each}
					</ul>
				</section>
			{/each}
		{/if}
	{/if}
</div>

<style>
	.memory-page { height: 100%; overflow-y: auto; padding: 24px 20px 48px; }
	.memory-header, .memory-toolbar { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; max-width: 840px; margin: 0 auto 20px; flex-wrap: wrap; }
	.memory-header h2 { font: 400 28px/1.2 var(--font-display); letter-spacing: -0.02em; color: var(--foreground); margin: 0 0 6px; }
	.memory-meta { font: 400 13px/1.5 var(--font-body); color: var(--text-secondary); margin: 0; }
	.memory-search { flex: 1 1 260px; max-width: 360px; }
	.memory-search input { width: 100%; min-height: 44px; }
	.memory-center { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 8px; min-height: 220px; text-align: center; color: var(--foreground); font: 400 16px/1.6 var(--font-body); }
	.memory-group, .memory-results, .memory-related, .memory-danger { max-width: 840px; margin: 0 auto 20px; }
	.memory-group h3, .memory-related h3 { font: 500 14px var(--font-body); color: var(--foreground); margin: 0 0 8px; text-transform: capitalize; }
	.memory-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
	.memory-row { display: grid; grid-template-columns: minmax(120px, 1fr) 2fr auto; gap: 12px; align-items: baseline; width: 100%; min-height: 44px; padding: 10px 14px; text-align: left; background: var(--card); border: 1px solid var(--border); border-radius: 12px; color: var(--foreground); cursor: pointer; }
	.memory-row:hover { background: var(--accent); }
	.memory-name { font: 500 14px var(--font-body); overflow-wrap: anywhere; }
	.memory-summary { font: 400 13px/1.5 var(--font-body); color: var(--text-secondary); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
	.memory-size { font: 400 12px var(--font-body); color: var(--text-muted); white-space: nowrap; }
	.memory-toolbar { align-items: center; }
	.memory-path { font: 400 13px var(--font-mono); color: var(--text-secondary); overflow-wrap: anywhere; flex: 1; }
	.memory-doc { max-width: 840px; margin: 0 auto 20px; background: var(--card); border: 1px solid var(--border); border-radius: 16px; padding: 20px; }
	.doc-content { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font: 400 14px/1.7 var(--font-body); color: var(--foreground); }
	.doc-media { max-width: 100%; max-height: 70vh; border-radius: 12px; display: block; margin: 0 auto; }
	.doc-audio { width: 100%; }
	.doc-pdf { width: 100%; height: 70vh; border: 0; border-radius: 12px; background: var(--background); }
	.memory-related ul { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
	.link-btn { background: none; border: 0; padding: 6px 0; min-height: 44px; color: var(--primary); font: 400 14px var(--font-body); cursor: pointer; text-align: left; }
	.memory-actions { display: flex; gap: 8px; margin-top: 8px; }
	@media (max-width: 720px) {
		.memory-page { padding: 16px 16px 40px; }
		.memory-row { grid-template-columns: 1fr; gap: 4px; }
		.memory-summary { white-space: normal; }
	}
</style>
