<script lang="ts">
	import { fetchMemory, fetchMemoryContent, searchMemory, deleteMemoryFile, fetchVectors, fetchMemoryGraph, type MemorySearchResult, type VectorEntry } from "$lib/api/client.js";
	import { Play, Music, FileText } from "@lucide/svelte";
	import type { MemoryEntry, MemoryGraph } from "$lib/api/types.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { openFile } from "$lib/stores/fileviewer.svelte.js";

	const toast = getToasts();

	let { slug }: { slug: string } = $props();

	let entries = $state<MemoryEntry[]>([]);
	let graph = $state<MemoryGraph>({ edges: [] });
	let loading = $state(true);
	let loadError = $state("");
	let graphMode = $state(false);
	let focusedFolder = $state<string | null>(null);
	let hoveredNode = $state<string | null>(null);
	let containerEl = $state<HTMLDivElement | null>(null);
	let containerWidth = $state(600);
	let containerHeight = $state(500);
	let viewKey = $state(0);

	// Search state
	let searchQuery = $state("");
	let searchError = $state("");
	let searchOpen = $state(false);

	// Debug state
	let debugOpen = $state(false);
	let vectors = $state<VectorEntry[]>([]);
	let vectorsLoading = $state(false);

	async function loadVectors() {
		vectorsLoading = true;
		try {
			vectors = await fetchVectors(slug);
		} catch {
			toast.error("failed to load vectors");
		}
		vectorsLoading = false;
	}

	function toggleDebug() {
		debugOpen = !debugOpen;
		if (debugOpen && vectors.length === 0) loadVectors();
	}

	// Document viewer state
	let viewingEntry = $state<MemoryEntry | null>(null);
	let viewingContent = $state<string>("");
	let viewingLoading = $state(false);

	// Pan state
	let panX = $state(0);
	let panY = $state(0);
	let isPanning = $state(false);
	let panStartX = 0;
	let panStartY = 0;
	let panStartPanX = 0;
	let panStartPanY = 0;

	async function load() {
		loading = true;
		loadError = "";
		try {
			const [e, g] = await Promise.all([fetchMemory(slug), fetchMemoryGraph(slug)]);
			entries = e;
			graph = g;
		} catch {
			loadError = "Could not load memories. Please try again.";
			toast.error("failed to load memory");
		} finally {
			loading = false;
		}
	}

	$effect(() => { load(); });

	$effect(() => {
		if (!containerEl) return;
		const ro = new ResizeObserver((es) => {
			const e = es[0];
			if (e) {
				containerWidth = e.contentRect.width;
				containerHeight = e.contentRect.height;
			}
		});
		ro.observe(containerEl);
		return () => ro.disconnect();
	});

	// --- data ---

	interface FolderNode {
		name: string;
		files: MemoryEntry[];
		totalSize: number;
	}

	let folders = $derived.by(() => {
		const map = new Map<string, MemoryEntry[]>();
		for (const e of entries) {
			const slash = e.path.indexOf("/");
			const folder = slash !== -1 ? e.path.substring(0, slash) : "(root)";
			if (!map.has(folder)) map.set(folder, []);
			map.get(folder)!.push(e);
		}
		const result: FolderNode[] = [];
		for (const [name, files] of map) {
			result.push({ name, files, totalSize: files.reduce((s, f) => s + f.size, 0) });
		}
		result.sort((a, b) => b.totalSize - a.totalSize);
		return result;
	});

	let totalSize = $derived(folders.reduce((s, f) => s + f.totalSize, 0));

	// --- search (server-side BM25) ---

	let searchResults = $state<MemorySearchResult[]>([]);
	let searchLoading = $state(false);
	let searchDebounce: ReturnType<typeof setTimeout> | null = null;

	$effect(() => {
		const q = searchQuery.trim();
		searchError = "";
		if (q.length < 2) {
			searchResults = [];
			return;
		}
		searchLoading = true;
		if (searchDebounce) clearTimeout(searchDebounce);
		searchDebounce = setTimeout(async () => {
			try {
				searchResults = await searchMemory(slug, q, 20);
			} catch {
				searchResults = [];
				searchError = "Could not search memories. Try again.";
			} finally {
				searchLoading = false;
			}
		}, 250);
	});

	function toggleSearch() {
		searchOpen = !searchOpen;
		searchQuery = "";
		searchResults = [];
	}

	// --- colors ---

	const folderHex: Record<string, string> = {
		about: "var(--primary)", facts: "var(--primary)", moments: "var(--primary)",
		preferences: "var(--primary)", projects: "var(--primary)", interests: "var(--primary)",
		people: "var(--primary)", emotions: "var(--primary)", knowledge: "var(--primary)",
		technical: "var(--primary)", "(root)": "var(--primary)",
	};

	function getHex(folder: string): string {
		if (folderHex[folder]) return folderHex[folder];
		let hash = 0;
		for (let i = 0; i < folder.length; i++) hash = folder.charCodeAt(i) + ((hash << 5) - hash);
		return "var(--primary)";
	}

	// --- circle packing ---

	interface Circle {
		x: number; y: number; r: number;
		id: string; label: string; hex: string;
		size: number; fileCount?: number;
		entry?: MemoryEntry; folder?: FolderNode;
		floatSeed: number;
	}

	function packCircles(
		items: { id: string; weight: number }[],
		w: number, h: number,
	): { id: string; x: number; y: number; r: number }[] {
		if (items.length === 0) return [];
		if (items.length === 1) {
			const r = Math.min(w, h) * 0.25;
			return [{ id: items[0].id, x: w / 2, y: h / 2, r }];
		}

		const maxWeight = Math.max(...items.map((i) => i.weight));
		const minWeight = Math.min(...items.map((i) => i.weight));
		const range = Math.max(maxWeight - minWeight, 1);

		const normed = items.map((item) => {
			const norm = (item.weight - minWeight) / range;
			return { id: item.id, logW: Math.log1p(norm * 9) / Math.log(10) };
		});

		const targetArea = w * h * 0.40;
		const sumLogSq = normed.reduce((s, n) => s + (0.35 + 0.65 * n.logW) ** 2, 0);
		const scale = Math.sqrt(targetArea / (Math.PI * sumLogSq));
		const minR = 28;
		const maxR = Math.min(w, h) * 0.22;

		const circles = normed.map((n) => {
			const r = Math.max(minR, Math.min(maxR, (0.35 + 0.65 * n.logW) * scale));
			return { id: n.id, x: 0, y: 0, r };
		});

		circles.sort((a, b) => b.r - a.r);

		const pad = 14;
		circles[0].x = w / 2;
		circles[0].y = h / 2;

		for (let i = 1; i < circles.length; i++) {
			const c = circles[i];
			let bestX = w / 2;
			let bestY = h / 2;
			let bestDist = Infinity;

			for (let j = 0; j < i; j++) {
				const ref = circles[j];
				const touchDist = ref.r + c.r + pad;

				for (let ai = 0; ai < 36; ai++) {
					const a = (ai / 36) * Math.PI * 2;
					const tx = ref.x + Math.cos(a) * touchDist;
					const ty = ref.y + Math.sin(a) * touchDist;

					// Clamp within viewport with margin
					const margin = c.r + 16;
					if (tx < margin || tx > w - margin || ty < margin || ty > h - margin) continue;

					let ok = true;
					for (let k = 0; k < i; k++) {
						if (k === j) continue;
						const dx = tx - circles[k].x;
						const dy = ty - circles[k].y;
						const need = c.r + circles[k].r + pad;
						if (dx * dx + dy * dy < need * need) { ok = false; break; }
					}
					if (!ok) continue;

					const dx = tx - w / 2;
					const dy = ty - h / 2;
					const d = dx * dx + dy * dy;
					if (d < bestDist) { bestDist = d; bestX = tx; bestY = ty; }
				}
			}

			c.x = bestX;
			c.y = bestY;
		}

		// Re-center within viewport
		let bx0 = Infinity, bx1 = -Infinity, by0 = Infinity, by1 = -Infinity;
		for (const c of circles) {
			bx0 = Math.min(bx0, c.x - c.r);
			bx1 = Math.max(bx1, c.x + c.r);
			by0 = Math.min(by0, c.y - c.r);
			by1 = Math.max(by1, c.y + c.r);
		}
		const ox = (w - (bx1 - bx0)) / 2 - bx0;
		const oy = (h - (by1 - by0)) / 2 - by0;
		for (const c of circles) { c.x += ox; c.y += oy; }

		return circles;
	}

	let mapH = $derived(containerHeight - 44);

	let folderCircles = $derived.by((): Circle[] => {
		if (folders.length === 0) return [];
		const items = folders.map((f) => ({ id: f.name, weight: f.totalSize }));
		const packed = packCircles(items, containerWidth, mapH);
		return packed.map((p, i) => {
			const folder = folders.find((f) => f.name === p.id)!;
			return {
				...p, label: folder.name, hex: getHex(folder.name),
				size: folder.totalSize, fileCount: folder.files.length,
				folder, floatSeed: i * 1.7,
			};
		});
	});

	let fileCircles = $derived.by((): Circle[] => {
		if (!focusedFolder) return [];
		const folder = folders.find((f) => f.name === focusedFolder);
		if (!folder) return [];
		const items = folder.files.map((f) => ({ id: f.path, weight: Math.max(f.size, 20) }));
		const packed = packCircles(items, containerWidth, mapH);
		return packed.map((p, i) => {
			const entry = folder.files.find((f) => f.path === p.id)!;
			return {
				...p,
				label: entry.path.split("/").pop()?.replace(".md", "") ?? entry.path,
				hex: getHex(folder.name), size: entry.size, entry,
				floatSeed: i * 1.3,
			};
		});
	});

	let activeCircles = $derived(focusedFolder ? fileCircles : folderCircles);

	// ── Graph mode: force-directed layout ──

	interface GraphNode {
		id: string;
		x: number; y: number;
		vx: number; vy: number;
		r: number;
		label: string;
		hex: string;
		size: number;
		entry: MemoryEntry;
		floatSeed: number;
	}

	interface GraphEdge {
		from: string;
		to: string;
	}

	let graphNodes = $state<GraphNode[]>([]);
	let graphEdges = $state<GraphEdge[]>([]);

	// Build graph data when entering graph mode
	$effect(() => {
		if (!graphMode || entries.length === 0 || graph.edges.length === 0) {
			graphNodes = [];
			graphEdges = [];
			return;
		}

		// Collect all paths that appear in edges
		const edgePaths = new Set<string>();
		for (const [a, b] of graph.edges) {
			edgePaths.add(a);
			edgePaths.add(b);
		}

		// Create nodes for all entries that have connections
		const w = containerWidth;
		const h = mapH;
		const nodes: GraphNode[] = [];
		let i = 0;
		for (const entry of entries) {
			if (!edgePaths.has(entry.path)) continue;
			const slash = entry.path.indexOf("/");
			const folder = slash !== -1 ? entry.path.substring(0, slash) : "(root)";
			const logSize = Math.log1p(Math.max(entry.size, 20)) / Math.log(10);
			const r = Math.max(24, Math.min(60, logSize * 16));
			nodes.push({
				id: entry.path,
				x: w * 0.2 + Math.random() * w * 0.6,
				y: h * 0.2 + Math.random() * h * 0.6,
				vx: 0, vy: 0,
				r,
				label: entry.path.split("/").pop()?.replace(".md", "") ?? entry.path,
				hex: getHex(folder),
				size: entry.size,
				entry,
				floatSeed: i * 1.3,
			});
			i++;
		}

		// Build edge list (only edges where both nodes exist)
		const nodeIds = new Set(nodes.map(n => n.id));
		const edges: GraphEdge[] = [];
		for (const [a, b] of graph.edges) {
			if (nodeIds.has(a) && nodeIds.has(b)) {
				edges.push({ from: a, to: b });
			}
		}

		graphNodes = nodes;
		graphEdges = edges;
	});

	// Force simulation
	$effect(() => {
		if (!graphMode || graphNodes.length === 0) return;

		const nodes = graphNodes;
		const edges = graphEdges;
		const w = containerWidth;
		const h = mapH;
		let raf: number;
		let ticks = 0;
		const MAX_TICKS = 300;

		function tick() {
			if (ticks >= MAX_TICKS) return;
			ticks++;

			const alpha = Math.max(0.01, 1 - ticks / MAX_TICKS);
			const repulsion = 3000;
			const attraction = 0.005;
			const centerPull = 0.002;

			// Reset forces
			for (const n of nodes) { n.vx = 0; n.vy = 0; }

			// Repulsion (all pairs)
			for (let i = 0; i < nodes.length; i++) {
				for (let j = i + 1; j < nodes.length; j++) {
					const a = nodes[i], b = nodes[j];
					let dx = a.x - b.x;
					let dy = a.y - b.y;
					const dist = Math.sqrt(dx * dx + dy * dy) || 1;
					const minDist = a.r + b.r + 20;
					const force = repulsion / (dist * dist);
					// Extra push when overlapping
					const overlap = minDist - dist;
					const totalForce = force + (overlap > 0 ? overlap * 0.5 : 0);
					const fx = (dx / dist) * totalForce;
					const fy = (dy / dist) * totalForce;
					a.vx += fx; a.vy += fy;
					b.vx -= fx; b.vy -= fy;
				}
			}

			// Attraction (edges)
			for (const e of edges) {
				const a = nodes.find(n => n.id === e.from);
				const b = nodes.find(n => n.id === e.to);
				if (!a || !b) continue;
				const dx = b.x - a.x;
				const dy = b.y - a.y;
				const dist = Math.sqrt(dx * dx + dy * dy) || 1;
				const targetDist = a.r + b.r + 60;
				const force = (dist - targetDist) * attraction;
				const fx = (dx / dist) * force;
				const fy = (dy / dist) * force;
				a.vx += fx; a.vy += fy;
				b.vx -= fx; b.vy -= fy;
			}

			// Center gravity
			const cx = w / 2, cy = h / 2;
			for (const n of nodes) {
				n.vx += (cx - n.x) * centerPull;
				n.vy += (cy - n.y) * centerPull;
			}

			// Apply velocity with damping
			for (const n of nodes) {
				n.x += n.vx * alpha;
				n.y += n.vy * alpha;
				// Clamp to bounds
				n.x = Math.max(n.r + 16, Math.min(w - n.r - 16, n.x));
				n.y = Math.max(n.r + 16, Math.min(h - n.r - 16, n.y));
			}

			graphNodes = [...nodes]; // trigger reactivity
			if (ticks < MAX_TICKS) {
				raf = requestAnimationFrame(tick);
			}
		}

		raf = requestAnimationFrame(tick);
		return () => cancelAnimationFrame(raf);
	});

	// SVG edge lines for graph mode
	let graphSvgEdges = $derived.by(() => {
		if (!graphMode || graphNodes.length === 0) return [];
		const nodeMap = new Map(graphNodes.map(n => [n.id, n]));
		return graphEdges.map(e => {
			const from = nodeMap.get(e.from);
			const to = nodeMap.get(e.to);
			if (!from || !to) return null;
			return { x1: from.x, y1: from.y, x2: to.x, y2: to.y, hex: from.hex };
		}).filter(Boolean) as { x1: number; y1: number; x2: number; y2: number; hex: string }[];
	});

	function toggleGraphMode() {
		graphMode = !graphMode;
		if (graphMode) {
			focusedFolder = null;
			viewingEntry = null;
			resetView();
		}
	}

	function resetView() {
		panX = 0; panY = 0; zoom = 1;
	}

	function handleCircleClick(circle: Circle) {
		if (!focusedFolder && circle.folder) {
			focusedFolder = circle.folder.name;
			hoveredNode = null;
			resetView();
			viewKey++;
		} else if (focusedFolder && circle.entry) {
			openDocument(circle.entry);
		}
	}

	function handleBack() {
		if (viewingEntry) {
			viewingEntry = null;
			viewingContent = "";
		} else {
			focusedFolder = null;
			hoveredNode = null;
			resetView();
			viewKey++;
		}
	}

	const IMAGE_EXTS = ['.jpg', '.jpeg', '.png', '.gif', '.webp', '.svg', '.bmp'];
	const VIDEO_EXTS = ['.mp4', '.mov', '.webm'];
	const AUDIO_EXTS = ['.mp3', '.wav', '.ogg'];
	const PDF_EXTS = ['.pdf'];
	const MEDIA_EXTS = [...IMAGE_EXTS, ...VIDEO_EXTS, ...AUDIO_EXTS, ...PDF_EXTS];

	function isMediaFile(path: string): boolean {
		const lower = path.toLowerCase();
		return MEDIA_EXTS.some(ext => lower.endsWith(ext));
	}

	function mediaType(path: string): 'image' | 'video' | 'audio' | 'pdf' | 'text' {
		const lower = path.toLowerCase();
		if (IMAGE_EXTS.some(ext => lower.endsWith(ext))) return 'image';
		if (VIDEO_EXTS.some(ext => lower.endsWith(ext))) return 'video';
		if (AUDIO_EXTS.some(ext => lower.endsWith(ext))) return 'audio';
		if (PDF_EXTS.some(ext => lower.endsWith(ext))) return 'pdf';
		return 'text';
	}

	function mediaUrl(path: string): string {
		// Same-origin: the session cookie authenticates media requests.
		return `/api/instances/${encodeURIComponent(slug)}/memory/${path}`;
	}

	async function openDocument(entry: MemoryEntry) {
		viewingEntry = entry;
		if (isMediaFile(entry.path)) {
			// Media files rendered inline — no need to fetch content
			viewingContent = "";
			viewingLoading = false;
			return;
		}
		viewingLoading = true;
		try {
			viewingContent = await fetchMemoryContent(slug, entry.path);
		} catch {
			viewingContent = "(failed to load)";
		} finally {
			viewingLoading = false;
		}
	}

	async function handleDelete() {
		if (!viewingEntry) return;
		const path = viewingEntry.path;
		try {
			await deleteMemoryFile(slug, path);
			toast.success("deleted");
			viewingEntry = null;
			viewingContent = "";
			await load();
		} catch {
			toast.error("failed to delete");
		}
	}

	function fileName(path: string): string {
		return path.split("/").pop()?.replace(".md", "") ?? path;
	}

	function formatSize(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		return `${(bytes / 1024).toFixed(1)} KB`;
	}

	function truncLabel(label: string, r: number): string {
		const maxChars = Math.floor(r / 4.2);
		if (maxChars < 4) return "";
		if (label.length <= maxChars) return label;
		return label.slice(0, Math.max(maxChars - 2, 3)) + "..";
	}

	// Zoom
	let zoom = $state(1);
	const ZOOM_MIN = 0.3;
	const ZOOM_MAX = 3;

	function onWheel(e: WheelEvent) {
		e.preventDefault();
		const delta = e.deltaY > 0 ? 0.9 : 1.1;
		const newZoom = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom * delta));

		// Zoom toward cursor position
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		const cx = e.clientX - rect.left;
		const cy = e.clientY - rect.top;
		panX = cx - (cx - panX) * (newZoom / zoom);
		panY = cy - (cy - panY) * (newZoom / zoom);
		zoom = newZoom;
	}

	// Pan: mousedown on empty area starts drag
	function onMapMouseDown(e: MouseEvent) {
		if ((e.target as HTMLElement).closest(".bubble")) return;
		if (e.button !== 0) return;
		isPanning = true;
		panStartX = e.clientX;
		panStartY = e.clientY;
		panStartPanX = panX;
		panStartPanY = panY;

		const onMove = (me: MouseEvent) => {
			panX = panStartPanX + (me.clientX - panStartX);
			panY = panStartPanY + (me.clientY - panStartY);
		};
		const onUp = () => {
			isPanning = false;
			window.removeEventListener("mousemove", onMove);
			window.removeEventListener("mouseup", onUp);
		};
		window.addEventListener("mousemove", onMove);
		window.addEventListener("mouseup", onUp);
	}
</script>

<div class="memory-container" bind:this={containerEl}>

	{#if loading}
		<span class="sr-only" role="status">Loading memories…</span>
		<div class="memory-loading">
			<div class="memory-loading-orb">
				<div class="memory-loading-ring"></div>
				<div class="memory-loading-ring memory-loading-ring-2"></div>
				<div class="memory-loading-dot"></div>
			</div>
		</div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if entries.length === 0}
		<div class="memory-empty">
			<img class="memory-moon" src="/skins/moon/character.svg" alt="" width="72" height="72" />
			<p class="memory-empty-text">No memories yet</p>
			<p class="memory-empty-hint">Memories form as you talk. Your companion learns and remembers.</p>
		</div>
	{:else if viewingEntry}
		<!-- Document viewer -->
		<div class="memory-header">
			<button class="memory-back" aria-label="Back" onclick={handleBack}>
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
					<path d="M19 12H5m0 0l7 7m-7-7l7-7" stroke-linecap="round" stroke-linejoin="round"/>
				</svg>
			</button>
			<span class="memory-breadcrumb">{viewingEntry.path}</span>
			<span class="memory-count">{formatSize(viewingEntry.size)}</span>
			<button class="memory-delete" onclick={handleDelete} title="delete memory">
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
					<path d="M3 6h18M8 6V4a2 2 0 012-2h4a2 2 0 012 2v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6h14" stroke-linecap="round" stroke-linejoin="round"/>
				</svg>
			</button>
		</div>
		<div class="doc-viewer">
			{#if viewingLoading}
				<div class="doc-loading">Loading…</div>
			{:else if mediaType(viewingEntry.path) === 'image'}
				<img src={mediaUrl(viewingEntry.path)} alt={viewingEntry.path} class="doc-media-img" />
			{:else if mediaType(viewingEntry.path) === 'video'}
				<video src={mediaUrl(viewingEntry.path)} controls playsinline class="doc-media-video">
					<track kind="captions" />
				</video>
			{:else if mediaType(viewingEntry.path) === 'audio'}
				<div class="doc-media-audio-wrap">
					<div class="doc-audio-icon">
						<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="32" height="32">
							<path d="M9 18V5l12-2v13" stroke-linecap="round" stroke-linejoin="round"/>
							<circle cx="6" cy="18" r="3"/><circle cx="18" cy="16" r="3"/>
						</svg>
					</div>
					<audio src={mediaUrl(viewingEntry.path)} controls class="doc-media-audio"></audio>
				</div>
			{:else if mediaType(viewingEntry.path) === 'pdf'}
				<iframe src={mediaUrl(viewingEntry.path)} class="doc-media-pdf" title="PDF viewer"></iframe>
			{:else}
				<pre class="doc-content">{viewingContent}</pre>
			{/if}
		</div>
	{:else}
		<!-- Map view -->
		<div class="memory-header">
			{#if focusedFolder}
				<button class="memory-back" aria-label="Back" onclick={handleBack}>
					<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
						<path d="M19 12H5m0 0l7 7m-7-7l7-7" stroke-linecap="round" stroke-linejoin="round"/>
					</svg>
				</button>
				<span class="memory-breadcrumb">{focusedFolder}/</span>
				<span class="memory-count">{folders.find(f => f.name === focusedFolder)?.files.length ?? 0} memories</span>
			{:else}
				<span class="memory-count">{entries.length} memories · {folders.length} folders · {formatSize(totalSize)}</span>
			{/if}
			<button class="search-toggle" class:search-toggle-active={searchOpen} onclick={toggleSearch} title="search memories">
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
					<circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" stroke-linecap="round"/>
				</svg>
			</button>
			{#if graph.edges.length > 0}
				<button class="search-toggle" class:search-toggle-active={graphMode} onclick={toggleGraphMode} title="memory graph ({graph.edges.length} connections)">
					<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
						<circle cx="6" cy="6" r="2.5"/><circle cx="18" cy="6" r="2.5"/><circle cx="12" cy="18" r="2.5"/>
						<path d="M8.5 7l3 8.5M15.5 7l-3 8.5" stroke-linecap="round"/>
					</svg>
				</button>
			{/if}
			<button class="search-toggle" class:search-toggle-active={debugOpen} onclick={toggleDebug} title="vector debug">
				<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" width="14" height="14">
					<path d="M12 20h9M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" stroke-linecap="round" stroke-linejoin="round"/>
				</svg>
			</button>
		</div>

		{#if searchOpen}
ttt<!-- svelte-ignore a11y_autofocus -->
			<div class="search-bar">
				<input
					class="search-input"
					type="text"
					placeholder="search memories..."
					bind:value={searchQuery}
					autofocus
					onkeydown={(e) => e.key === "Escape" && toggleSearch()}
				/>
				{#if searchQuery.length >= 2}
					<span class="search-count">{searchLoading ? "searching..." : `${searchResults.length} results`}</span>
				{/if}
			</div>

			{#if searchResults.length > 0}
				<div class="search-results">
					{#each searchResults as result}
						{@const isMedia = result.source_type?.startsWith("media_")}
						{@const basePath = result.path.split("#")[0]}
						{@const folder = isMedia ? result.source_type?.replace("media_", "") ?? "media" : (basePath.split("/")[0] ?? "(root)")}
						{@const preview = result.text.trim().slice(0, 200)}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<!-- svelte-ignore a11y_click_events_have_key_events -->
						<div
							class="search-result"
							class:search-result-media={isMedia}
							style="--c: {isMedia ? 'var(--primary)' : getHex(folder)}"
							onclick={() => {
								if (isMedia && result.media_url) {
									const ext = result.source_type === "media_image" ? "image.jpg" : result.source_type === "media_video" ? "video.mp4" : "audio.mp3";
									openFile(result.media_url, basePath || ext);
								} else {
									const entry = entries.find(e => e.path === basePath);
									if (entry) openDocument(entry);
								}
							}}
						>
							{#if isMedia && result.media_url && result.source_type === "media_image"}
								<img class="search-result-thumb" src={result.media_url} alt={basePath} />
							{/if}
							<div class="search-result-body">
								<div class="search-result-path">
									{#if isMedia}
										<span class="search-result-folder" style="color: var(--c)">{result.source_type === "media_image" ? "image" : result.source_type === "media_video" ? "video" : "audio"}</span>
										{preview || basePath}
									{:else}
										<span class="search-result-folder" style="color: {getHex(folder)}">{folder}/</span>{fileName(basePath)}
									{/if}
								</div>
								{#if !isMedia}
									<div class="search-result-summary">{preview}</div>
								{/if}
								<div class="search-result-meta">score: {result.score.toFixed(4)}</div>
							</div>
						</div>
					{/each}
				</div>
			{:else if searchError}
				<div class="search-empty" role="alert">{searchError}</div>
			{:else if searchQuery.length >= 2 && !searchLoading}
				<div class="search-empty">No matches</div>
			{/if}
		{/if}

		{#if debugOpen}
			<div class="debug-panel">
				<div class="debug-header">
					<span class="debug-title">vectors ({vectors.length})</span>
					<button class="debug-refresh" onclick={loadVectors}>{vectorsLoading ? "..." : "refresh"}</button>
				</div>
				<div class="debug-list">
					{#each vectors as v}
						<div class="debug-entry" class:debug-media={v.source_type.startsWith("media_")}>
							<div class="debug-path">{v.path}</div>
							<div class="debug-meta">
								<span class="debug-type">{v.source_type}</span>
								{#if v.upload_id && v.upload_id !== v.path}
									<span class="debug-upload">upload: {v.upload_id}</span>
								{/if}
							</div>
							{#if v.content_preview}
								<div class="debug-preview">{v.content_preview.slice(0, 150)}</div>
							{/if}
						</div>
					{/each}
				</div>
			</div>
		{/if}

		{#if !searchOpen && !debugOpen}
		{#if graphMode && graphNodes.length > 0}
			<!-- Graph mode: force-directed layout with edges -->
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="memory-map"
				style="height: {mapH}px"
				onmousedown={onMapMouseDown}
				onwheel={onWheel}
			>
				<div class="memory-map-inner" style="transform: translate({panX}px, {panY}px) scale({zoom}); transform-origin: 0 0">
					<!-- Edge lines -->
					<svg class="graph-edges" style="width: {containerWidth}px; height: {mapH}px">
						{#each graphSvgEdges as edge}
							<line
								x1={edge.x1} y1={edge.y1}
								x2={edge.x2} y2={edge.y2}
								stroke={edge.hex}
								stroke-opacity="0.25"
								stroke-width="1.5"
							/>
						{/each}
					</svg>
					<!-- Nodes -->
					{#each graphNodes as node (node.id)}
						{@const isHovered = hoveredNode === node.id}
						{@const diameter = node.r * 2}
						{@const showLabel = node.r > 18}
						{@const showSub = node.r > 36}
						{@const isImage = IMAGE_EXTS.some(ext => node.entry.path.toLowerCase().endsWith(ext))}
						{@const fileType = mediaType(node.entry.path)}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="bubble-anchor"
							style="left: {node.x}px; top: {node.y}px"
						>
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<div
								class="bubble bubble-clickable"
								class:bubble-hovered={isHovered}
								style="
									width: {diameter}px;
									height: {diameter}px;
									--c: {node.hex};
									--float-x: 0px;
									--float-y: 0px;
									--float-dur: 99s;
									--float-delay: 0s;
								"
								onmouseenter={() => hoveredNode = node.id}
								onmouseleave={() => hoveredNode = null}
								onclick={() => openDocument(node.entry)}
							>
								{#if isImage}
									<img class="bubble-thumb" src={mediaUrl(node.entry.path)} alt="" loading="lazy" />
								{:else if fileType === 'video'}
									<video class="bubble-thumb" src={mediaUrl(node.entry.path)} autoplay muted loop playsinline></video>
								{:else if fileType === 'audio'}
									<div class="bubble-type-icon"><Music size={20} /></div>
								{:else if fileType === 'pdf'}
									<div class="bubble-type-icon"><FileText size={20} /></div>
								{:else}
									<div class="bubble-core"></div>
								{/if}
								<div class="bubble-shine"></div>
								{#if isHovered}
									<div class="bubble-ring"></div>
								{/if}
								{#if showLabel}
									<span class="bubble-label">{truncLabel(node.label, node.r)}</span>
									{#if showSub}
										<span class="bubble-sub">{formatSize(node.size)}</span>
									{/if}
								{/if}
							</div>
						</div>
					{/each}
				</div>
			</div>

			<!-- Tooltip for graph mode -->
			{#if hoveredNode && !isPanning}
				{@const node = graphNodes.find(n => n.id === hoveredNode)}
				{#if node}
					{@const neighbors = graphEdges
						.filter(e => e.from === node.id || e.to === node.id)
						.map(e => e.from === node.id ? e.to : e.from)
						.map(p => p.split("/").pop()?.replace(".md", "") ?? p)}
					<div class="memory-tooltip" style="--c: {node.hex}">
						<div class="memory-tooltip-dot" style="background: {node.hex}"></div>
						<div class="memory-tooltip-body">
							<div class="memory-tooltip-name">{node.entry.path}</div>
							<div class="memory-tooltip-summary">{node.entry.summary}</div>
							{#if neighbors.length > 0}
								<div class="memory-tooltip-files">
									{#each neighbors as n}
										<span class="memory-tooltip-file">{n}</span>
									{/each}
								</div>
							{/if}
							<div class="memory-tooltip-meta">{formatSize(node.size)} · {neighbors.length} connections</div>
						</div>
					</div>
				{/if}
			{/if}
		{:else}
		{#key viewKey}
			<!-- svelte-ignore a11y_no_static_element_interactions -->
			<div
				class="memory-map"
				style="height: {mapH}px"
				onmousedown={onMapMouseDown}
				onwheel={onWheel}
			>
				<div class="memory-map-inner" style="transform: translate({panX}px, {panY}px) scale({zoom}); transform-origin: 0 0">
					{#each activeCircles as circle, i (circle.id)}
						{@const isHovered = hoveredNode === circle.id}
						{@const isFolderView = !focusedFolder}
						{@const diameter = circle.r * 2}
						{@const showLabel = circle.r > 20}
						{@const showSub = circle.r > 42}
						{@const isImage = !isFolderView && circle.entry && IMAGE_EXTS.some(ext => circle.entry!.path.toLowerCase().endsWith(ext))}
						{@const fileType = !isFolderView && circle.entry ? mediaType(circle.entry.path) : 'text'}
						<!-- svelte-ignore a11y_no_static_element_interactions -->
						<div
							class="bubble-anchor"
							style="
								left: {circle.x}px;
								top: {circle.y}px;
								animation-delay: {i * 60 + 50}ms;
							"
						>
							<!-- svelte-ignore a11y_no_static_element_interactions -->
							<!-- svelte-ignore a11y_click_events_have_key_events -->
							<div
								class="bubble bubble-clickable"
								class:bubble-hovered={isHovered}
								style="
									width: {diameter}px;
									height: {diameter}px;
									--c: {circle.hex};
									--float-x: {Math.sin(circle.floatSeed) * 5}px;
									--float-y: {Math.cos(circle.floatSeed * 0.7) * 6}px;
									--float-dur: {5 + circle.floatSeed % 3}s;
									--float-delay: {circle.floatSeed * -0.4}s;
								"
								onmouseenter={() => hoveredNode = circle.id}
								onmouseleave={() => hoveredNode = null}
								onclick={() => handleCircleClick(circle)}
							>
								{#if isImage}
									<img class="bubble-thumb" src={mediaUrl(circle.entry?.path ?? '')} alt="" loading="lazy" />
								{:else if fileType === 'video'}
									<video class="bubble-thumb" src={mediaUrl(circle.entry?.path ?? '')} autoplay muted loop playsinline></video>
								{:else if fileType === 'audio'}
									<div class="bubble-type-icon"><Music size={20} /></div>
								{:else if fileType === 'pdf'}
									<div class="bubble-type-icon"><FileText size={20} /></div>
								{:else}
									<div class="bubble-core"></div>
								{/if}
								<div class="bubble-shine"></div>
								{#if isHovered}
									<div class="bubble-ring"></div>
								{/if}
								{#if showLabel}
									<span class="bubble-label">{truncLabel(circle.label, circle.r)}</span>
									{#if showSub}
										<span class="bubble-sub">
											{isFolderView ? `${circle.fileCount} files` : formatSize(circle.size)}
										</span>
									{/if}
								{/if}
							</div>
						</div>
					{/each}
				</div>
			</div>
		{/key}

		<!-- Tooltip -->
		{#if hoveredNode && !isPanning}
			{@const circle = activeCircles.find(c => c.id === hoveredNode)}
			{#if circle}
				<div class="memory-tooltip" style="--c: {circle.hex}">
					<div class="memory-tooltip-dot" style="background: {circle.hex}"></div>
					<div class="memory-tooltip-body">
						<div class="memory-tooltip-name">{circle.label}</div>
						{#if circle.entry}
							<div class="memory-tooltip-summary">{circle.entry.summary}</div>
							<div class="memory-tooltip-meta">{formatSize(circle.entry.size)} · click to read</div>
						{:else if circle.folder}
							<div class="memory-tooltip-summary">{circle.folder.files.length} memories · {formatSize(circle.folder.totalSize)}</div>
							<div class="memory-tooltip-files">
								{#each circle.folder.files.slice(0, 5) as file}
									<span class="memory-tooltip-file">{fileName(file.path)}</span>
								{/each}
								{#if circle.folder.files.length > 5}
									<span class="memory-tooltip-file memory-tooltip-more">+{circle.folder.files.length - 5}</span>
								{/if}
							</div>
						{/if}
					</div>
				</div>
			{/if}
		{/if}
		{/if}
		{/if}
	{/if}
</div>

<style>
	.memory-moon { flex-shrink: 0; }
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.memory-container {
		height: 100%;
		display: flex;
		flex-direction: column;
		overflow: hidden;
		position: relative;
	}


	/* ═══════ Loading ═══════ */

	.memory-loading { display: flex; align-items: center; justify-content: center; height: 100%; z-index: 1; }
	.memory-loading-orb { position: relative; width: 48px; height: 48px; }
	.memory-loading-ring {
		position: absolute; inset: 0; border-radius: 50%;
		border: 1px solid var(--border);
		animation:none;
	}
	.memory-loading-ring::after {
		content: ""; position: absolute; top: -1px; left: 50%;
		width: 6px; height: 2px; background: var(--card);
		border-radius: 1px; transform: translateX(-50%);
	}
	.memory-loading-ring-2 { inset: 8px; animation-direction: reverse; animation-duration: 2s; border-color: var(--border); }
	.memory-loading-dot {
		position: absolute; top: 50%; left: 50%; width: 4px; height: 4px;
		border-radius: 50%; background: var(--card);
		transform: translate(-50%, -50%); animation:none;
	}
	@keyframes loading-spin { to { transform: rotate(360deg); } }

	/* ═══════ Empty ═══════ */

	.memory-empty {
		display: flex; flex-direction: column; align-items: center; justify-content: center;
		height: 100%; gap: 1rem; text-align: center; z-index: 1;
	}
	@keyframes breathe-slow { 0%, 100% { opacity: 0.4; transform: scale(1); } 50% { opacity: 0.8; transform: scale(1.08); } }
	.memory-empty-text { font-family: var(--font-display); font-size: 0.95rem; font-style: normal; color: var(--text-secondary); }
	.memory-empty-hint { font-size: 0.75rem; color: var(--text-secondary); max-width: 26ch; line-height: 1.5; }

	/* ═══════ Header ═══════ */

	.memory-header {
		display: flex; align-items: center; gap: 0.625rem;
		padding: 0.75rem 1.5rem 0; flex-shrink: 0; z-index: 2;
	}
	.memory-back {
		display: flex; align-items: center; justify-content: center;
		width: 28px; height: 28px; color: var(--text-secondary);
		background: var(--card);
		border: 1px solid var(--border); border-top-color: var(--border);
		border-radius: 50%; cursor: pointer; transition: all 0.25s ease;
		box-shadow: none;
	}
	.memory-back:hover {
		color: var(--text-secondary);
		border-color: var(--border);
		background: var(--card);
	}
	.memory-breadcrumb { font-family: var(--font-body); font-size: 0.75rem; color: var(--text-secondary); }
	.memory-count { font-family: var(--font-body); font-size: 0.75rem; color: var(--text-secondary); letter-spacing: 0.04em; margin-left: auto; }
	.memory-delete {
		display: flex; align-items: center; justify-content: center;
		width: 28px; height: 28px; color: var(--text-secondary);
		background: none; border: 1px solid var(--border);
		border-radius: 50%; cursor: pointer; transition: all 0.25s ease;
		margin-left: 6px;
	}
	.memory-delete:hover {
		color: var(--destructive);
		border-color: var(--border);
		background: var(--card);
	}

	/* ═══════ Search ═══════ */

	.search-toggle {
		display: flex; align-items: center; justify-content: center;
		width: 28px; height: 28px; color: var(--text-secondary);
		background: none; border: 1px solid var(--border);
		border-radius: 50%; cursor: pointer; transition: all 0.25s ease;
		flex-shrink: 0;
	}
	.search-toggle:hover, .search-toggle-active {
		color: var(--text-secondary);
		border-color: var(--border);
		background: var(--card);
	}

	.search-bar {
		display: flex; align-items: center; gap: 0.5rem;
		padding: 0.5rem 1.5rem; flex-shrink: 0; z-index: 2;
	}
	.search-input {
		flex: 1; font-family: var(--font-body); font-size: 0.75rem;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--border); border-top-color: var(--border);
		border-radius: 0.625rem; padding: 0.4rem 0.75rem;
		color: var(--foreground); outline: none;
		transition: border-color 0.2s ease;
		box-shadow: none;
	}
	.search-input:focus { border-color: var(--border); box-shadow: none; }
	.search-input::placeholder { color: var(--text-secondary); }
	.search-count {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); white-space: nowrap;
	}

	.search-results {
		flex: 1; min-height: 0; overflow-y: auto; z-index: 2;
		padding: 0.25rem 1.5rem 1.5rem; display: flex; flex-direction: column; gap: 0.35rem;
	}
	.search-result {
		position: relative;
		padding: 0.6rem 0.75rem; border-radius: 0.75rem;
		border: 1px solid var(--border); border-top-color: var(--border);
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		cursor: pointer; transition: all 0.2s ease;
		display: flex; flex-direction: column; gap: 0.15rem;
		box-shadow: none;
		overflow: hidden;
	}
	.search-result:hover {
		border-color: var(--border);
		background: var(--card);
	}
	.search-result-path {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--foreground);
	}
	.search-result-folder {
		font-size: 0.75rem; opacity: 0.7;
	}
	.search-result-summary {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--foreground); line-height: 1.4;
		overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
	}
	.search-result-media {
		flex-direction: row; align-items: center; gap: 0.6rem;
	}
	.search-result-thumb {
		width: 48px; height: 48px; border-radius: 0.5rem;
		object-fit: cover; flex-shrink: 0;
		border: 1px solid var(--border);
	}
	.search-result-body {
		flex: 1; min-width: 0;
		display: flex; flex-direction: column; gap: 0.15rem;
	}
	.search-result-meta {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary);
	}
	.search-empty {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); text-align: center;
		padding: 2rem; z-index: 2;
	}

	/* ═══════ Document Viewer ═══════ */

	.doc-viewer {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 1.25rem 1.5rem 2rem;
		z-index: 1;
	}

	.doc-loading {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		padding: 2rem;
		text-align: center;
	}

	.doc-media-img {
		max-width: 100%;
		max-height: 70vh;
		border-radius: 0.75rem;
		object-fit: contain;
	}

	.doc-media-video {
		max-width: 100%;
		max-height: 70vh;
		border-radius: 0.75rem;
	}

	.doc-media-audio-wrap {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1.5rem;
		padding: 3rem 0;
	}

	.doc-audio-icon {
		color: var(--text-secondary);
	}

	.doc-media-audio {
		width: 100%;
		max-width: 400px;
	}

	.doc-media-pdf {
		width: 100%;
		height: 80vh;
		border: none;
		border-radius: 0.75rem;
	}

	.doc-content {
		font-family: var(--font-body);
		font-size: 0.78rem;
		line-height: 1.7;
		color: var(--foreground);
		white-space: pre-wrap;
		word-wrap: break-word;
		margin: 0;
		max-width: 560px;
	}

	/* ═══════ Bubble Map ═══════ */

	.memory-map {
		position: relative;
		flex: 1;
		min-height: 0;
		z-index: 1;
		overflow: hidden;
		cursor: grab;
		touch-action: none;
	}

	.memory-map:active {
		cursor: grabbing;
	}

	.memory-map-inner {
		position: absolute;
		inset: 0;
		will-change: transform;
	}

	.graph-edges {
		position: absolute;
		top: 0;
		left: 0;
		pointer-events: none;
		z-index: 0;
	}

	.bubble-anchor {
		position: absolute;
		transform: translate(-50%, -50%);
		animation: none;
		z-index: 1;
	}

	@keyframes bubble-enter {
		0% { opacity: 1; transform: translate(-50%, -50%) scale(0); filter: blur(12px); }
		60% { opacity: 1; transform: translate(-50%, -50%) scale(1.06); filter: blur(1px); }
		100% { opacity: 1; transform: translate(-50%, -50%) scale(1); filter: blur(0); }
	}

	.bubble {
		position: relative;
		border-radius: 50%;
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		gap: 2px;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--border);
		border-top-color: var(--border);
		box-shadow: none;
		animation: none;
		transition: border-color 0.3s ease, box-shadow 0.3s ease;
		overflow: hidden;
	}

	/* Specular highlight on glass sphere */
	.bubble::before {
		content: "";
		position: absolute;
		top: 8%;
		left: 20%;
		width: 35%;
		height: 20%;
		border-radius: 50%;
		background: var(--card);
		transform: rotate(-20deg);
		pointer-events: none;
	}

	.bubble-clickable { cursor: pointer; }

	.bubble-hovered {
		border-color: var(--border);
		border-top-color: var(--border);
		box-shadow: none;
		z-index: 5;
	}

	@keyframes bubble-float {
		0%, 100% { transform: translate(0, 0); }
		33% { transform: translate(var(--float-x), var(--float-y)); }
		66% { transform: translate(calc(var(--float-x) * -0.6), calc(var(--float-y) * -0.4)); }
	}

	.bubble-thumb {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		object-fit: cover;
		border-radius: 50%;
		opacity: 0.85;
	}

	.bubble-type-icon {
		position: absolute;
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%);
		color: var(--text-secondary);
		opacity: 0.75;
	}

	.bubble-core {
		position: absolute; top: 50%; left: 50%; width: 30%; height: 30%;
		border-radius: 50%;
		background: var(--card);
		transform: translate(-50%, -50%);
		animation: none;
	}

	@keyframes core-pulse {
		0%, 100% { opacity: 0.5; transform: translate(-50%, -50%) scale(1); }
		50% { opacity: 1; transform: translate(-50%, -50%) scale(1.3); }
	}

	.bubble-shine {
		position: absolute; top: 12%; left: 22%; width: 32%; height: 18%;
		border-radius: 50%;
		background: var(--card);
		transform: rotate(-20deg); pointer-events: none;
	}

	.bubble-ring {
		position: absolute; inset: -6px; border-radius: 50%;
		border: 1px solid var(--border);
		animation: none;
		pointer-events: none;
	}

	@keyframes ring-expand {
		0% { opacity: 0.6; inset: -4px; }
		100% { opacity: 1; inset: -20px; }
	}

	.bubble-label {
		font-family: var(--font-body); font-size: 0.75rem; letter-spacing: 0.05em;
		font-weight: 600;
		color: var(--foreground);
		text-shadow: none;
		pointer-events: none; user-select: none; z-index: 2;
		text-align: center; line-height: 1; max-width: 85%;
		overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
	}

	.bubble-sub {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-muted);
		text-shadow: none;
		pointer-events: none; user-select: none; z-index: 2;
	}

	/* ═══════ Tooltip ═══════ */

	.memory-tooltip {
		position: absolute; bottom: 1.25rem; left: 50%;
		transform: translateX(-50%); display: flex; gap: 0.625rem;
		align-items: flex-start;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--border); border-top-color: var(--border);
		border-radius: 1rem;
		padding: 0.75rem 1rem; max-width: 340px; min-width: 180px;
		animation: none;
		pointer-events: none; z-index: 20;
		box-shadow: none;
		overflow: hidden;
	}
	@keyframes tooltip-enter {
		from { opacity: 1; transform: translateX(-50%) translateY(8px) scale(0.96); }
		to { opacity: 1; transform: translateX(-50%) translateY(0) scale(1); }
	}

	.memory-tooltip-dot { width: 6px; height: 6px; border-radius: 50%; flex-shrink: 0; margin-top: 4px; box-shadow: none; }
	.memory-tooltip-body { display: flex; flex-direction: column; gap: 0.2rem; min-width: 0; }
	.memory-tooltip-name { font-family: var(--font-body); font-size: 0.75rem; font-weight: 500; letter-spacing: 0.03em; color: var(--text-secondary); }
	.memory-tooltip-summary { font-family: var(--font-body); font-size: 0.75rem; color: var(--foreground); line-height: 1.4; }
	.memory-tooltip-meta { font-family: var(--font-body); font-size: 0.75rem; color: var(--text-secondary); }
	.memory-tooltip-files { margin-top: 0.2rem; display: flex; flex-wrap: wrap; gap: 0.25rem; }
	.memory-tooltip-file {
		font-family: var(--font-body); font-size: 0.75rem; color: var(--foreground);
		background: var(--card); padding: 0.1rem 0.35rem; border-radius: 0.25rem;
		border: 1px solid var(--border);
	}
	.memory-tooltip-more { color: var(--text-secondary); font-style: normal; border: none; background: none; padding: 0.1rem 0; }

	@media (max-width: 640px) {
		.memory-header { padding: 0.5rem 0.75rem 0; }
		.memory-tooltip { left: 0.75rem; right: 0.75rem; transform: none; max-width: none; }
		@keyframes tooltip-enter {
			from { opacity: 1; transform: translateY(8px) scale(0.96); }
			to { opacity: 1; transform: translateY(0) scale(1); }
		}
	}

	/* ═══════ Debug panel ═══════ */

	.debug-panel {
		flex: 1; overflow-y: auto; padding: 0.5rem 1rem;
	}
	.debug-header {
		display: flex; align-items: center; justify-content: space-between;
		margin-bottom: 0.5rem;
	}
	.debug-title {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); letter-spacing: 0.04em;
	}
	.debug-refresh {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); background: none; border: 1px solid var(--border);
		border-radius: 4px; padding: 0.15rem 0.5rem; cursor: pointer;
	}
	.debug-refresh:hover { border-color: var(--border); color: var(--text-secondary); }
	.debug-list { display: flex; flex-direction: column; gap: 0.3rem; }
	.debug-entry {
		padding: 0.4rem 0.5rem; border-radius: 0.35rem;
		background: var(--card); border: 1px solid var(--border);
	}
	.debug-media { border-left: 2px solid var(--border); }
	.debug-path {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); word-break: break-all;
	}
	.debug-meta {
		display: flex; gap: 0.5rem; margin-top: 0.15rem;
	}
	.debug-type {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary);
	}
	.debug-upload {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary);
	}
	.debug-preview {
		font-family: var(--font-body); font-size: 0.75rem;
		color: var(--text-secondary); margin-top: 0.2rem;
		line-height: 1.4; white-space: pre-wrap; word-break: break-word;
	}

/* Little Moon surfaces, controls, and readable content. */

.memory-container { background: var(--background); color: var(--foreground); }
.memory-empty { padding: 24px; }
.memory-empty-text { font: 400 28px var(--font-display); color: var(--foreground); }
.memory-empty-hint { font-size: 14px; max-width: 42ch; }
.memory-header { flex-wrap: wrap; gap: 8px; padding: 20px 24px 0; }
.memory-back, .memory-delete, .search-toggle { width: 44px; height: 44px; border-radius: 8px; background: var(--card); border: 1px solid var(--border); }
.memory-back:hover, .search-toggle:hover, .search-toggle-active { background: var(--accent); color: var(--primary); }
.memory-delete { color: var(--destructive); }
.memory-breadcrumb { overflow-wrap: anywhere; font-size: 14px; }
.search-input { font-size: 16px; min-height: 44px; background: var(--card); border-color: var(--input); min-width: 0; }
.search-result, .memory-tooltip, .debug-panel { background: var(--card); border-color: var(--border); }
.search-result { padding: 16px; min-height: 64px; }
.search-result:hover { background: var(--accent); border-color: var(--primary); }
.doc-content { font-family: var(--font-mono); font-size: 14px; color: var(--foreground); }
.bubble { background: var(--card); border: 1px solid var(--border); box-shadow: none; }
.bubble-hovered { background: var(--accent); border-color: var(--primary); }
.bubble-core, .bubble-shine, .bubble-ring { display: none; }
.bubble-label { font-size: 13px; color: var(--foreground); opacity: 1; }
.bubble-sub { color: var(--text-secondary); opacity: 1; }
.memory-tooltip { border: 1px solid var(--border); }
.debug-refresh { min-height: 44px; border-radius: 8px; padding: 8px 12px; background: var(--accent); color: var(--primary); }
@media (max-width: 640px) { .memory-header { padding: 16px 20px 0; } }

button:focus-visible, input:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.memory-loading-dot { background: var(--primary); }
</style>
