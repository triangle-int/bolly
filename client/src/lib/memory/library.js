// @ts-check
/**
 * Pure helpers for the memory library (#96): grouping, naming, media kinds,
 * and explicit links. No retrieval internals reach the UI through here.
 */

/** @typedef {{ path: string; summary: string; size: number }} MemoryEntry */
/** @typedef {{ edges: [string, string][] }} MemoryGraph */

const IMAGE = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"];
const VIDEO = [".mp4", ".mov", ".webm"];
const AUDIO = [".mp3", ".wav", ".m4a", ".ogg"];
const PDF = [".pdf"];

/**
 * @param {string} path
 * @returns {"image" | "video" | "audio" | "pdf" | "text"}
 */
export function mediaKind(path) {
	const lower = path.toLowerCase();
	if (IMAGE.some((ext) => lower.endsWith(ext))) return "image";
	if (VIDEO.some((ext) => lower.endsWith(ext))) return "video";
	if (AUDIO.some((ext) => lower.endsWith(ext))) return "audio";
	if (PDF.some((ext) => lower.endsWith(ext))) return "pdf";
	return "text";
}

/** @param {string} path */
export function folderOf(path) {
	const slash = path.indexOf("/");
	return slash === -1 ? "" : path.slice(0, slash);
}

/** @param {string} path */
export function displayName(path) {
	const name = path.split("/").pop() ?? path;
	return name.endsWith(".md") ? name.slice(0, -3) : name;
}

/**
 * Group entries by top-level folder, folders alphabetical with root files
 * last, entries alphabetical by path.
 * @param {MemoryEntry[]} entries
 * @returns {{ folder: string; entries: MemoryEntry[]; size: number }[]}
 */
export function groupByFolder(entries) {
	/** @type {Map<string, MemoryEntry[]>} */
	const map = new Map();
	for (const entry of entries) {
		const folder = folderOf(entry.path);
		if (!map.has(folder)) map.set(folder, []);
		map.get(folder)?.push(entry);
	}
	const groups = [...map.entries()].map(([folder, list]) => ({
		folder,
		entries: [...list].sort((a, b) => a.path.localeCompare(b.path)),
		size: list.reduce((sum, e) => sum + e.size, 0),
	}));
	groups.sort((a, b) => {
		if (a.folder === "") return 1;
		if (b.folder === "") return -1;
		return a.folder.localeCompare(b.folder);
	});
	return groups;
}

/**
 * Paths explicitly linked to `path` in the memory graph, sorted.
 * @param {MemoryGraph | null | undefined} graph
 * @param {string} path
 */
export function relatedPaths(graph, path) {
	if (!graph) return [];
	const related = new Set();
	for (const [a, b] of graph.edges) {
		if (a === path) related.add(b);
		else if (b === path) related.add(a);
	}
	return [...related].sort();
}

/** @param {number} bytes */
export function formatSize(bytes) {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * Case-insensitive local filter used while the server search is pending or
 * for short queries.
 * @param {MemoryEntry[]} entries
 * @param {string} query
 */
export function filterEntries(entries, query) {
	const q = query.trim().toLowerCase();
	if (!q) return entries;
	return entries.filter((e) => e.path.toLowerCase().includes(q) || e.summary.toLowerCase().includes(q));
}
