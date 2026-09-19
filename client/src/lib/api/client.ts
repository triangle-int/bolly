import type {
	ChatMessage,
	ChatResponse,
	ChatSummary,
	ContextStats,
	Drop,
	InstanceSummary,
	RegistryEntry,
	ServerMeta,
	Skill,
	Soul,
	SoulTemplate,
	Thought,
	UpdateLlmRequest,
	MemoryEntry,
	Stats,
	UploadMeta,
	ChildAgent,
	AgentHistoryEntry,
} from "./types.js";
import { clearLegacyBrowserAuth } from "./legacy-auth-cleanup.js";

const BASE = "";

// ---------------------------------------------------------------------------
// Auth token management
// ---------------------------------------------------------------------------

const TOKEN_KEY = "nolune_auth_token";

function clearLegacyAuth() {
	clearLegacyBrowserAuth(
		typeof localStorage === "undefined" ? undefined : localStorage,
		typeof document === "undefined" ? undefined : document,
	);
}

export function isDesktopRelay(): boolean {
	return typeof window !== "undefined" && "__NOLUNE_DESKTOP_RELAY__" in window;
}

export function getAuthToken(): string | null {
	clearLegacyAuth();
	// Desktop authenticates in the native exact-origin relay, never in browser URLs.
	if (isDesktopRelay()) return null;
	if (typeof localStorage === "undefined") return null;
	return localStorage.getItem(TOKEN_KEY);
}

/** Build a public URL for media (video/image) that includes auth token in query string.
 *  Use this for <video src> and <img src> since they can't send Authorization headers. */
export function mediaUrl(slug: string, uploadId: string): string {
	const token = getAuthToken();
	const q = token ? `?token=${encodeURIComponent(token)}` : "";
	return `${BASE}/public/files/${encodeURIComponent(slug)}/${encodeURIComponent(uploadId)}${q}`;
}

export function setAuthToken(token: string) {
	if (isDesktopRelay()) throw new Error("Reconnect from the desktop dashboard.");
	if (typeof localStorage !== "undefined") {
		localStorage.setItem(TOKEN_KEY, token);
	}
	clearLegacyAuth();
}

export function clearAuthToken() {
	if (typeof localStorage !== "undefined") {
		localStorage.removeItem(TOKEN_KEY);
	}
	clearLegacyAuth();
}

function authHeaders(): Record<string, string> {
	const token = getAuthToken();
	return token ? { Authorization: `Bearer ${token}` } : {};
}

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

export class AuthError extends Error {
	constructor() {
		super("unauthorized");
		this.name = "AuthError";
	}
}

async function json<T>(url: string, init?: RequestInit): Promise<T> {
	const headers = {
		...authHeaders(),
		...(init?.headers as Record<string, string> | undefined),
	};
	const res = await fetch(`${BASE}${url}`, { ...init, headers });
	if (res.status === 401) {
		throw new AuthError();
	}
	if (!res.ok) {
		const text = await res.text().catch(() => res.statusText);
		throw new Error(text);
	}
	return res.json();
}

async function authedFetch(url: string, init?: RequestInit): Promise<Response> {
	const headers = {
		...authHeaders(),
		...(init?.headers as Record<string, string> | undefined),
	};
	return fetch(`${BASE}${url}`, { ...init, headers });
}

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

export function fetchMeta(): Promise<ServerMeta> {
	return json("/api/meta");
}

/**
 * The one companion this server owns (#103). Mirrors the server default and is
 * refreshed from `/api/meta` so the client never invents a second identity.
 */
export const DEFAULT_COMPANION_SLUG = "companion";
let companionSlug = DEFAULT_COMPANION_SLUG;

export function getCompanionSlug(): string {
	return companionSlug;
}

export async function refreshCompanionSlug(): Promise<string> {
	try {
		const meta = await fetchMeta();
		if (meta.companion_slug) companionSlug = meta.companion_slug;
	} catch {}
	return companionSlug;
}

export function fetchInstances(): Promise<InstanceSummary[]> {
	return json("/api/instances");
}

export async function deleteInstance(slug: string): Promise<void> {
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}`, {
		method: "DELETE",
	});
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}

export function fetchChats(slug: string): Promise<ChatSummary[]> {
	return json(`/api/chat/${encodeURIComponent(slug)}/chats`);
}

export function fetchMessages(slug: string, chatId = "default"): Promise<ChatResponse> {
	return json(`/api/chat/${encodeURIComponent(slug)}/${encodeURIComponent(chatId)}/messages`);
}

export function sendMessage(
	slug: string,
	content: string,
	chatId = "default",
	voiceMode = false,
): Promise<ChatResponse> {
	return json("/api/chat", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ instance_slug: slug, content, chat_id: chatId, voice_mode: voiceMode }),
	});
}

export function updateLlmConfig(req: {
	api_key?: string;
	openai?: string;
	google_ai?: string;
	elevenlabs?: string;
	openrouter?: string;
}): Promise<void> {
	return json("/api/config/llm", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(req),
	});
}

export interface EmbeddingStatus {
	version: number;
	enabled: boolean;
	provider: string;
	model: string;
	dimensions: number;
	base_url: string | null;
	authentication: "OpenAI bearer key" | "none";
	configured: boolean;
	status: "available" | "unavailable" | "unverified";
	reason: string | null;
	fallback: "bm25";
	changes_require_restart: boolean;
	update_semantics: "full_replacement";
	needs_restart: boolean;
	pending: Omit<EmbeddingStatus, "pending" | "needs_restart"> | null;
}

export function fetchConfigStatus(): Promise<{
	embedding?: EmbeddingStatus;
	llm_configured: boolean;
	provider?: string;
	setup_required?: string | null;
	model?: string | null;
	model_mode?: string;
	configured_keys?: string[];
}> {
	return json("/api/config/status");
}

export function updateProvider(provider: 'anthropic' | 'openai'): Promise<{ status: string; provider: string }> {
	return json("/api/config/provider", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ provider }),
	});
}

export function fetchServerConfig(): Promise<{ host: string; port: number; auth_token_set: boolean }> {
	return json("/api/config/server");
}

export function updateServerConfig(updates: { host?: string; port?: number; auth_token?: string }): Promise<{ status: string; needs_restart: boolean }> {
	return json("/api/config/server", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(updates),
	});
}

export function updateModelMode(mode: string): Promise<{ status: string; model_mode: string }> {
	return json("/api/config/model-mode", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ mode }),
	});
}

export interface McpServerInfo {
	name: string;
	url?: string;
	connected: boolean;
}

export function fetchMcpServers(): Promise<McpServerInfo[]> {
	return json("/api/config/mcp");
}

export function addMcpServer(name: string, url: string): Promise<{ status: string; name: string; tool_count: number }> {
	return json("/api/config/mcp", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ name, url }),
	});
}

export function fetchSuggestedMcp(): Promise<{
	name: string;
	description: string;
	url: string;
	requires_key: boolean;
	key_env: string;
	key_url: string;
	installed: boolean;
}[]> {
	return json("/api/config/mcp/suggested");
}

export function removeMcpServer(name: string): Promise<void> {
	return json(`/api/config/mcp/${encodeURIComponent(name)}`, {
		method: "DELETE",
	});
}

export function fetchTimezone(slug: string): Promise<{ timezone: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/timezone`);
}

export function updateTimezone(slug: string, timezone: string): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/timezone`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ timezone }),
	});
}

export function fetchGithubConfig(): Promise<{ configured: boolean }> {
	return json("/api/config/github");
}

export function updateGithubToken(token: string): Promise<{ status: string; configured: boolean }> {
	return json("/api/config/github", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ token }),
	});
}

// ---------------------------------------------------------------------------
// Email config (per-instance SMTP/IMAP)
// ---------------------------------------------------------------------------

export interface EmailConfig {
	smtp_host: string;
	smtp_port: number;
	smtp_user: string;
	smtp_password: string;
	smtp_from: string;
	imap_host: string;
	imap_port: number;
	imap_user: string;
	imap_password: string;
}

export function fetchEmailAccounts(slug: string): Promise<{ accounts: Partial<EmailConfig>[] }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/email`);
}

export function saveEmailAccounts(slug: string, accounts: EmailConfig[]): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/email`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ accounts }),
	});
}

export function deleteAllEmailAccounts(slug: string): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/email`, {
		method: "DELETE",
	});
}

export function fetchSoul(slug: string): Promise<Soul> {
	return json(`/api/instances/${encodeURIComponent(slug)}/soul`);
}

export function updateSoul(slug: string, content: string): Promise<Soul> {
	return json(`/api/instances/${encodeURIComponent(slug)}/soul`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ content }),
	});
}

export function applySoulTemplate(
	slug: string,
	templateId: string,
): Promise<Soul> {
	return json(`/api/instances/${encodeURIComponent(slug)}/soul/apply-template`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ template_id: templateId }),
	});
}

export function fetchSoulTemplates(): Promise<SoulTemplate[]> {
	return json("/api/soul/templates");
}

export async function stopAgent(slug: string, chatId = "default"): Promise<void> {
	await authedFetch(`/api/chat/${encodeURIComponent(slug)}/${encodeURIComponent(chatId)}/stop`, { method: "POST" });
}

export async function clearContext(slug: string, chatId = "default"): Promise<void> {
	await authedFetch(`/api/chat/${encodeURIComponent(slug)}/${encodeURIComponent(chatId)}/context`, { method: "DELETE" });
}

export function fetchVoiceId(slug: string): Promise<{ voice_id: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/voice`);
}

export function updateVoiceId(slug: string, voiceId: string): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/voice`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ voice_id: voiceId }),
	});
}

export interface ScheduledTask {
	id: string;
	task: string;
	deliver_at: number;
	created_at: number;
}

export function fetchScheduledTasks(slug: string): Promise<ScheduledTask[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/scheduled`);
}

export async function cancelScheduledTask(slug: string, taskId: string): Promise<void> {
	const res = await authedFetch(
		`/api/instances/${encodeURIComponent(slug)}/scheduled/${encodeURIComponent(taskId)}`,
		{ method: "DELETE" },
	);
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}

export function fetchSkin(slug: string): Promise<{ skin: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/skin`);
}

export function updateSkin(slug: string, skin: string): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/skin`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ skin }),
	});
}

export function fetchVoiceEnabled(slug: string): Promise<{ voice_enabled: boolean }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/voice-mode`);
}

export function updateVoiceEnabled(slug: string, enabled: boolean): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/voice-mode`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ voice_enabled: enabled }),
	});
}

export function fetchMood(slug: string): Promise<{ mood: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/mood`);
}

export function fetchCompanionName(slug: string): Promise<{ name: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/companion-name`);
}

export function setCompanionName(slug: string, name: string): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/companion-name`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ name }),
	});
}

export function fetchThoughts(slug: string): Promise<Thought[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/thoughts`);
}


export function machineHello(slug: string): Promise<void> {
	return authedFetch(`/api/instances/${encodeURIComponent(slug)}/machine-hello`, { method: "POST" }).then(() => {});
}

export function machineBye(slug: string): Promise<void> {
	return authedFetch(`/api/instances/${encodeURIComponent(slug)}/machine-bye`, { method: "POST" }).then(() => {});
}

export function fetchAgents(slug: string): Promise<ChildAgent[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agents`);
}

export function triggerAgent(slug: string, agentName: string): Promise<{ status: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agents/${encodeURIComponent(agentName)}/run`, {
		method: "POST",
	});
}

export function updateAgent(slug: string, agentName: string, updates: Record<string, unknown>): Promise<ChildAgent> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agents/${encodeURIComponent(agentName)}`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(updates),
	});
}

export function resetAgent(slug: string, agentName: string): Promise<ChildAgent> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agents/${encodeURIComponent(agentName)}/reset`, {
		method: "POST",
	});
}

export function fetchAgentHistory(slug: string, agentName: string): Promise<AgentHistoryEntry[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agents/${encodeURIComponent(agentName)}/history`);
}

export function fetchAgentRuns(slug: string, limit = 50, agentName?: string): Promise<import("./types.js").AgentRunSummary[]> {
	const params = new URLSearchParams({ limit: String(limit) });
	if (agentName) params.set("agent_name", agentName);
	return json(`/api/instances/${encodeURIComponent(slug)}/agent-runs?${params}`);
}

export function fetchAgentRun(slug: string, runId: string): Promise<import("./types.js").AgentRun> {
	return json(`/api/instances/${encodeURIComponent(slug)}/agent-runs/${encodeURIComponent(runId)}`);
}

export function fetchStats(slug: string): Promise<Stats> {
	return json(`/api/instances/${encodeURIComponent(slug)}/stats`);
}

export function fetchMemory(slug: string): Promise<MemoryEntry[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/memory`);
}

export interface MemorySearchResult {
	path: string;
	text: string;
	score: number;
	source_type?: string;
	media_url?: string;
}

export function searchMemory(slug: string, query: string, limit = 10): Promise<MemorySearchResult[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/memory/search?q=${encodeURIComponent(query)}&limit=${limit}`);
}

export function reindexMemory(slug: string): Promise<{ status: string }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/memory/reindex`, { method: 'POST' });
}

export async function fetchMemoryContent(slug: string, path: string): Promise<string> {
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}/memory/${path}`);
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
	return res.text();
}

export interface VectorEntry {
	path: string;
	source_type: string;
	content_preview: string;
	upload_id?: string;
}

export function fetchVectors(slug: string): Promise<VectorEntry[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/memory/vectors`);
}

export function fetchMemoryGraph(slug: string): Promise<import("./types.js").MemoryGraph> {
	return json(`/api/instances/${encodeURIComponent(slug)}/memory/graph`);
}

export async function deleteMemoryFile(slug: string, path: string): Promise<void> {
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}/memory/${path}`, { method: 'DELETE' });
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}

export function fetchDrops(slug: string): Promise<Drop[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/drops`);
}

export function fetchDrop(slug: string, dropId: string): Promise<Drop> {
	return json(
		`/api/instances/${encodeURIComponent(slug)}/drops/${encodeURIComponent(dropId)}`,
	);
}

export async function deleteDrop(slug: string, dropId: string): Promise<void> {
	await authedFetch(
		`/api/instances/${encodeURIComponent(slug)}/drops/${encodeURIComponent(dropId)}`,
		{ method: "DELETE" },
	);
}

export async function uploadFile(
	slug: string,
	file: File,
	onProgress?: (loaded: number, total: number) => void,
): Promise<UploadMeta> {
	return new Promise((resolve, reject) => {
		const xhr = new XMLHttpRequest();
		xhr.open("POST", `${BASE}/api/instances/${encodeURIComponent(slug)}/uploads`);

		const token = getAuthToken();
		if (token) xhr.setRequestHeader("Authorization", `Bearer ${token}`);

		if (onProgress) {
			xhr.upload.onprogress = (e) => {
				if (e.lengthComputable) onProgress(e.loaded, e.total);
			};
		}

		xhr.onload = () => {
			if (xhr.status === 401) return reject(new AuthError());
			if (xhr.status < 200 || xhr.status >= 300) return reject(new Error(xhr.responseText || xhr.statusText));
			try {
				resolve(JSON.parse(xhr.responseText));
			} catch {
				reject(new Error("invalid response"));
			}
		};

		xhr.onerror = () => reject(new Error("upload failed"));

		const form = new FormData();
		form.append("file", file);
		xhr.send(form);
	});
}

export function fetchUploads(slug: string): Promise<UploadMeta[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/uploads`);
}

export async function deleteUpload(slug: string, uploadId: string): Promise<void> {
	await authedFetch(
		`/api/instances/${encodeURIComponent(slug)}/uploads/${encodeURIComponent(uploadId)}`,
		{ method: "DELETE" },
	);
}

export function uploadFileUrl(slug: string, uploadId: string): string {
	const base = `/api/instances/${encodeURIComponent(slug)}/uploads/${encodeURIComponent(uploadId)}/file`;
	const token = getAuthToken();
	return token ? `${base}?token=${encodeURIComponent(token)}` : base;
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

export function fetchSkills(): Promise<Skill[]> {
	return json("/api/skills");
}

export function fetchSkill(skillId: string): Promise<Skill> {
	return json(`/api/skills/${encodeURIComponent(skillId)}`);
}

export function createSkill(skill: Skill): Promise<Skill> {
	return json("/api/skills", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(skill),
	});
}

export async function deleteSkill(skillId: string): Promise<void> {
	await authedFetch(`/api/skills/${encodeURIComponent(skillId)}`, {
		method: "DELETE",
	});
}

export function fetchRegistry(): Promise<RegistryEntry[]> {
	return json("/api/skills/registry");
}

export function installRegistrySkill(id: string): Promise<Skill> {
	return json("/api/skills/registry/install", {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ id }),
	});
}

export function fetchContextStats(slug: string, chatId = "default"): Promise<ContextStats> {
	return json(`/api/instances/${encodeURIComponent(slug)}/${encodeURIComponent(chatId)}/context-stats`);
}

export async function submitSecret(slug: string, id: string, value: string): Promise<void> {
	await authedFetch(`/api/instances/${encodeURIComponent(slug)}/secret`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ id, value }),
	});
}

export async function cancelSecret(slug: string, id: string): Promise<void> {
	await authedFetch(`/api/instances/${encodeURIComponent(slug)}/secret/${encodeURIComponent(id)}`, {
		method: "DELETE",
	});
}

// ---------------------------------------------------------------------------
// Heartbeat updates
// ---------------------------------------------------------------------------

export function fetchHeartbeatUpdates(slug: string): Promise<import("./types.js").HeartbeatUpdate[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/heartbeat/updates`);
}

export async function applyHeartbeatUpdate(slug: string, updateId: string): Promise<void> {
	const res = await authedFetch(
		`/api/instances/${encodeURIComponent(slug)}/heartbeat/updates/${encodeURIComponent(updateId)}/apply`,
		{ method: "POST" },
	);
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}

export async function dismissHeartbeatUpdate(slug: string, updateId: string): Promise<void> {
	const res = await authedFetch(
		`/api/instances/${encodeURIComponent(slug)}/heartbeat/updates/${encodeURIComponent(updateId)}/dismiss`,
		{ method: "POST" },
	);
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}


// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Updates
// ---------------------------------------------------------------------------

export interface UpdateCheck {
	current: string;
	latest: string;
	update_available: boolean;
	commit: string;
}

export interface ChangelogEntry {
	version: string;
	body: string;
}

export function fetchChangelog(): Promise<ChangelogEntry[]> {
	return json("/api/update/changelog");
}

export function checkUpdate(): Promise<UpdateCheck> {
	return json("/api/update/check");
}

export async function applyUpdate(): Promise<{ ok: boolean; message?: string }> {
	return json("/api/update/apply", { method: "POST" });
}

export function getUpdateChannel(): Promise<{ channel: string }> {
	return json("/api/update/channel");
}

export function setUpdateChannel(channel: string): Promise<{ ok: boolean; channel: string }> {
	return json("/api/update/channel", {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ channel }),
	});
}

// ---------------------------------------------------------------------------
// Export / Import
// ---------------------------------------------------------------------------

export function exportInstanceUrl(slug: string): string {
	const base = `${BASE}/api/instances/${encodeURIComponent(slug)}/export`;
	const token = getAuthToken();
	return token ? `${base}?token=${encodeURIComponent(token)}` : base;
}

/** Stream-download the export archive, reporting bytes received via callback. */
export async function exportInstance(
	slug: string,
	onProgress?: (downloadedBytes: number) => void,
): Promise<Blob> {
	const url = exportInstanceUrl(slug);
	const res = await fetch(url);
	if (!res.ok) throw new Error(await res.text() || "export failed");

	const reader = res.body!.getReader();
	const chunks: BlobPart[] = [];
	let downloaded = 0;

	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		chunks.push(value as BlobPart);
		downloaded += value.length;
		onProgress?.(downloaded);
	}

	return new Blob(chunks, { type: "application/gzip" });
}

export async function importInstance(slug: string, file: File): Promise<{ ok: boolean }> {
	const form = new FormData();
	form.append("file", file);
	const res = await fetch(
		`${BASE}/api/instances/${encodeURIComponent(slug)}/import`,
		{ method: "POST", body: form, headers: authHeaders() },
	);
	if (!res.ok) {
		const text = await res.text();
		throw new Error(text || "import failed");
	}
	return res.json();
}

// ---------------------------------------------------------------------------
// Computer Use
// ---------------------------------------------------------------------------

export async function submitComputerUseResult(
	slug: string,
	requestId: string,
	result: { type: "screenshot"; image: string; width: number; height: number; scale: number }
		| { type: "action"; success: boolean; error?: string },
): Promise<void> {
	await authedFetch(
		`${BASE}/api/instances/${encodeURIComponent(slug)}/computer-use/${encodeURIComponent(requestId)}`,
		{
			method: "POST",
			headers: { "Content-Type": "application/json" },
			body: JSON.stringify(result),
		},
	);
}

// ---------------------------------------------------------------------------
// WebSocket
// ---------------------------------------------------------------------------

export function createWebSocket(): WebSocket {
	const proto = location.protocol === "https:" ? "wss:" : "ws:";
	const token = getAuthToken();
	const query = token ? `?token=${encodeURIComponent(token)}` : "";
	return new WebSocket(`${proto}//${location.host}/api/ws${query}`);
}
