import type {
	ChatMessage,
	ChatResponse,
	ChatSummary,
	ContextStats,
	Drop,
	CompanionContext,
	RegistryEntry,
	ServerMeta,
	Skill,
	Soul,
	SoulTemplate,
	ProactiveRun,
	ProactivePolicy,
	UpdateLlmRequest,
	MemoryEntry,
	UploadMeta,
} from "./types.js";
import { clearLegacyBrowserAuth } from "./legacy-auth-cleanup.js";

const BASE = "";

// ---------------------------------------------------------------------------
// Authentication
//
// Browsers authenticate with a paired-session cookie (HttpOnly, set by the
// server after `POST /api/session/pair`). The client never holds a credential:
// same-origin fetches and WebSockets carry the cookie automatically. The
// server API token stays with automation, the CLI and the desktop relay.
// ---------------------------------------------------------------------------

function clearLegacyAuth() {
	clearLegacyBrowserAuth(
		typeof localStorage === "undefined" ? undefined : localStorage,
		typeof document === "undefined" ? undefined : document,
	);
}

export function isDesktopRelay(): boolean {
	return typeof window !== "undefined" && "__NOLUNE_DESKTOP_RELAY__" in window;
}

/** Issue a resource-scoped browser URL through the session-authenticated API. */
export interface ResourceGrant { url: string; refresh_after_seconds: number }
export async function mediaUrl(slug: string, uploadId: string): Promise<ResourceGrant> {
    return json<ResourceGrant>(`/api/instances/${encodeURIComponent(slug)}/resource-capabilities/files`, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ id: uploadId }),
    });
}

export async function memoryMediaUrl(slug: string, path: string): Promise<ResourceGrant> {
    return json<ResourceGrant>(`/api/instances/${encodeURIComponent(slug)}/resource-capabilities/memory`, {
        method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ path }),
    });
}

export type AuthKind = "disabled" | "token" | "session";

export interface PairedDevice {
	id: string;
	label: string;
	paired_via: string;
	host: string;
	created_at: number;
	last_seen_at: number;
	expires_at: number;
	current: boolean;
}

export interface PairingCode {
	id: string;
	code: string;
	expires_at: number;
	expires_in_secs: number;
	bound_host?: string;
}

export class PairingError extends Error {
	constructor(
		public readonly reason: "invalid_code" | "rate_limited" | "cross_origin" | "auth_disabled" | "unknown",
		public readonly status: number,
	) {
		super(reason);
		this.name = "PairingError";
	}
}

/** Redeem a one-time pairing code; the server answers with the session cookie. */
export async function pairBrowser(code: string): Promise<PairedDevice> {
	// The desktop relay authenticates natively and strips cookies; pairing
	// inside it would never take effect.
	if (isDesktopRelay()) throw new Error("Reconnect from the desktop dashboard.");
	clearLegacyAuth();
	const res = await fetch(`${BASE}/api/session/pair`, {
		method: "POST",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ code }),
	});
	if (!res.ok) {
		let reason: PairingError["reason"] = "unknown";
		try {
			const body = await res.json();
			if (
				body?.error === "invalid_code" ||
				body?.error === "rate_limited" ||
				body?.error === "cross_origin" ||
				body?.error === "auth_disabled"
			) {
				reason = body.error;
			}
		} catch {
			// no JSON body
		}
		throw new PairingError(reason, res.status);
	}
	const body = await res.json();
	return { ...body.session, current: true };
}

/** Whether this browser is currently authenticated, and how. Never throws on 401. */
export async function fetchSession(): Promise<{ auth: AuthKind; session: PairedDevice | null } | null> {
	const res = await fetch(`${BASE}/api/session`);
	if (res.status === 401) return null;
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
	return res.json();
}

export function createPairingCode(): Promise<PairingCode> {
	return json("/api/session/pairing", { method: "POST" });
}

export function fetchPairedDevices(): Promise<{ auth: AuthKind; devices: PairedDevice[] }> {
	return json("/api/session/devices");
}

export async function revokePairedDevice(id: string): Promise<void> {
	const res = await authedFetch(`/api/session/devices/${encodeURIComponent(id)}`, { method: "DELETE" });
	if (res.status === 401) throw new AuthError();
	if (!res.ok && res.status !== 404) throw new Error(await res.text().catch(() => res.statusText));
}

export async function revokeAllPairedDevices(): Promise<number> {
	const body = await json<{ revoked: number }>("/api/session/devices", { method: "DELETE" });
	return body.revoked;
}

export async function logoutSession(): Promise<void> {
	const res = await authedFetch("/api/session/logout", { method: "POST" });
	if (!res.ok && res.status !== 401) throw new Error(await res.text().catch(() => res.statusText));
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
	const res = await fetch(`${BASE}${url}`, init);
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
	return fetch(`${BASE}${url}`, init);
}

// ---------------------------------------------------------------------------
// API functions
// ---------------------------------------------------------------------------

export function fetchMeta(): Promise<ServerMeta> {
	return json("/api/meta");
}

/** The one companion this server owns. `exists` drives onboarding. */
export function fetchCompanion(): Promise<CompanionContext> {
	return json("/api/companion");
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

/** Activity receipts and initiative policy (#92, #94). */
export function fetchActivity(slug: string, limit = 50): Promise<ProactiveRun[]> {
	return json(`/api/instances/${encodeURIComponent(slug)}/activity?limit=${limit}`);
}

export async function cancelActivity(slug: string, runId: string): Promise<void> {
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}/activity/${encodeURIComponent(runId)}/cancel`, { method: "POST" });
	if (res.status === 401) throw new AuthError();
	if (!res.ok) throw new Error(await res.text().catch(() => res.statusText));
}

export function retryActivity(slug: string, runId: string): Promise<ProactiveRun> {
	return json(`/api/instances/${encodeURIComponent(slug)}/activity/${encodeURIComponent(runId)}/retry`, { method: "POST" });
}

export function fetchProactivePolicy(slug: string): Promise<ProactivePolicy> {
	return json(`/api/instances/${encodeURIComponent(slug)}/proactive`);
}

export function updateProactivePolicy(slug: string, policy: ProactivePolicy): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/proactive`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify(policy),
	});
}


export function machineHello(slug: string): Promise<void> {
	return authedFetch(`/api/instances/${encodeURIComponent(slug)}/machine-hello`, { method: "POST" }).then(() => {});
}

export function machineBye(slug: string): Promise<void> {
	return authedFetch(`/api/instances/${encodeURIComponent(slug)}/machine-bye`, { method: "POST" }).then(() => {});
}

/** Interaction-rhythm tracking (#95): the one retained behavioral aggregate. */
export function fetchRhythmTracking(slug: string): Promise<{ enabled: boolean }> {
	return json(`/api/instances/${encodeURIComponent(slug)}/rhythm`);
}

export function updateRhythmTracking(slug: string, enabled: boolean): Promise<void> {
	return json(`/api/instances/${encodeURIComponent(slug)}/rhythm`, {
		method: "PUT",
		headers: { "Content-Type": "application/json" },
		body: JSON.stringify({ enabled }),
	});
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

function encodedMemoryPath(path: string): string {
    return path.split('/').map(part => encodeURIComponent(part).replace(/[!'()*]/g, char => `%${char.charCodeAt(0).toString(16).toUpperCase()}`)).join('/');
}

export async function fetchMemoryContent(slug: string, path: string): Promise<string> {
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}/memory/${encodedMemoryPath(path)}`);
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
	const res = await authedFetch(`/api/instances/${encodeURIComponent(slug)}/memory/${encodedMemoryPath(path)}`, { method: 'DELETE' });
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

export async function uploadFileUrl(slug: string, uploadId: string): Promise<string> {
    return (await mediaUrl(slug, uploadId)).url;
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
	return `${BASE}/api/instances/${encodeURIComponent(slug)}/export`;
}

/** Stream-download the export archive, reporting bytes received via callback. */
export async function exportInstance(
	slug: string,
	onProgress?: (downloadedBytes: number) => void,
): Promise<Blob> {
	const url = exportInstanceUrl(slug);
	const res = await authedFetch(url);
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
		{ method: "POST", body: form },
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

// ISSUE-112: sole query-control-token exemption; WebSocket handshake only.
export function createWebSocket(): WebSocket {
	// Same-origin: the browser attaches the session cookie to the upgrade.
	const proto = location.protocol === "https:" ? "wss:" : "ws:";
	return new WebSocket(`${proto}//${location.host}/api/ws`);
}
