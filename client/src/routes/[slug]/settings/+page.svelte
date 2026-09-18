<script lang="ts">
	import { page } from "$app/state";
	import { embeddingStatusText } from "$lib/embedding-status.js";
	import type { EmbeddingStatus } from "$lib/api/client.js";
	import {
		fetchGoogleAccounts,
		getGoogleConnectUrl,
		disconnectGoogleAccount,
		fetchMcpServers,
		addMcpServer,
		removeMcpServer,
		fetchGithubConfig,
		updateGithubToken,
		fetchTimezone,
		updateTimezone,
		fetchVoiceId,
		updateVoiceId,
		fetchEmailAccounts,
		saveEmailAccounts,
		deleteAllEmailAccounts,
		fetchUsage,
		fetchConfigStatus,
		updateModelMode,
		exportInstance,
		importInstance,
		reindexMemory,
		importKnowledge,
		fetchScheduledTasks,
		cancelScheduledTask,
		fetchSuggestedMcp,
		updateLlmConfig,
		updateProvider,
		type ScheduledTask,
	} from "$lib/api/client.js";
	import type { McpServerInfo, EmailConfig } from "$lib/api/client.js";
	import type { Usage, ServerEvent } from "$lib/api/types.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import { getSkinStore, SKINS } from "$lib/stores/skin.svelte.js";
	import { onDestroy } from "svelte";

	const slug = $derived(page.params.slug!);
	const skinStore = getSkinStore();

	// --- suggested extensions (loaded from server) ---
	interface SuggestedMcp {
		name: string;
		description: string;
		url: string;
		requires_key: boolean;
		key_env: string;
		key_url: string;
		installed: boolean;
	}
	let suggestedMcp = $state<SuggestedMcp[]>([]);

	// Export / Import
	let exporting = $state(false);
	let exportBytes = $state(0);
	let exportError = $state("");
	let importing = $state(false);
	let importError = $state("");
	let importDone = $state(false);
	let importFileInput: HTMLInputElement | undefined = $state();

	function formatBytes(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
		return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
	}

	async function handleExport() {
		exporting = true;
		exportBytes = 0;
		exportError = "";
		try {
			const blob = await exportInstance(slug, (bytes) => { exportBytes = bytes; });
			const url = URL.createObjectURL(blob);
			const a = document.createElement("a");
			a.href = url;
			a.download = `${slug}.tar.gz`;
			a.click();
			URL.revokeObjectURL(url);
		} catch (e) {
			exportError = e instanceof Error ? e.message : "export failed";
		} finally {
			exporting = false;
		}
	}

	async function handleImport() {
		const file = importFileInput?.files?.[0];
		if (!file) return;
		importing = true;
		importError = "";
		importDone = false;
		try {
			await importInstance(slug, file);
			importDone = true;
			setTimeout(() => { importDone = false; }, 4000);
		} catch (e) {
			importError = e instanceof Error ? e.message : "import failed";
		} finally {
			importing = false;
			if (importFileInput) importFileInput.value = "";
		}
	}

	// Import knowledge
	let importingKnowledge = $state(false);
	let knowledgeError = $state("");
	let knowledgeStarted = $state(false);
	let knowledgeFileInput: HTMLInputElement | undefined = $state();

	// Import progress (via WebSocket)
	let importStage = $state<string | null>(null);
	let importDetail = $state("");
	const ws = getWebSocket();
	const unsub = ws.subscribe((event: ServerEvent) => {
		if (event.type === "import_progress" && event.instance_slug === slug) {
			importStage = event.stage;
			importDetail = event.detail;
			if (event.stage === "done" || event.stage === "error") {
				setTimeout(() => { importStage = null; importDetail = ""; }, 10000);
			}
		}
	});
	onDestroy(unsub);

	async function handleImportKnowledge() {
		const files = knowledgeFileInput?.files;
		if (!files || files.length === 0) return;
		importingKnowledge = true;
		knowledgeError = "";
		knowledgeStarted = false;
		try {
			await importKnowledge(slug, files);
			knowledgeStarted = true;
			setTimeout(() => { knowledgeStarted = false; }, 8000);
		} catch (e) {
			knowledgeError = e instanceof Error ? e.message : "import failed";
		} finally {
			importingKnowledge = false;
			if (knowledgeFileInput) knowledgeFileInput.value = "";
		}
	}

	// Reindex memory
	let reindexing = $state(false);

	async function handleReindex() {
		reindexing = true;
		try {
			await reindexMemory(slug);
		} catch (e) {
			console.error('reindex failed', e);
		} finally {
			reindexing = false;
		}
	}

	// Google state
	let accounts = $state<{ email: string }[]>([]);
	let loading = $state(true);
	let disconnecting = $state<string | null>(null);
	let connecting = $state(false);
	let error = $state("");

	// MCP state
	let mcpServers = $state<McpServerInfo[]>([]);
	let mcpLoading = $state(true);
	let mcpBusy = $state<string | null>(null);
	let mcpError = $state("");
	let mcpNewName = $state("");
	let mcpNewUrl = $state("");
	let showCustomForm = $state(false);

	// GitHub state
	let ghConfigured = $state(false);
	let ghLoading = $state(true);
	let ghToken = $state("");
	let ghSaving = $state(false);
	let ghError = $state("");
	let ghEditing = $state(false);

	// Cloud/managed detection
	let isManaged = $state(false);

	// Server state
	let serverHost = $state("0.0.0.0");
	let serverPort = $state(26559);
	let serverAuthSet = $state(false);
	let serverLoading = $state(true);
	let serverSaving = $state(false);
	let serverNeedsRestart = $state(false);
	let serverAuthInput = $state("");
	let serverPortInput = $state("26559");

	async function loadServer() {
		serverLoading = true;
		try {
			const { fetchServerConfig, fetchConfigStatus } = await import("$lib/api/client.js");
			const res = await fetchServerConfig();
			serverHost = res.host;
			serverPort = res.port;
			serverPortInput = String(res.port);
			serverAuthSet = res.auth_token_set;
			// Also check if managed (cloud) by fetching status
			try {
				const status = await fetchConfigStatus();
				isManaged = !!status.is_managed;
			} catch { /* ignore */ }
		} catch {
			// not critical
		} finally {
			serverLoading = false;
		}
	}

	async function saveServerPort() {
		const port = parseInt(serverPortInput);
		if (isNaN(port) || port < 1 || port > 65535) return;
		serverSaving = true;
		try {
			const { updateServerConfig } = await import("$lib/api/client.js");
			const res = await updateServerConfig({ port });
			serverPort = port;
			serverNeedsRestart = res.needs_restart;
		} catch {
			// ignore
		} finally {
			serverSaving = false;
		}
	}

	async function saveServerAuth() {
		serverSaving = true;
		try {
			const { updateServerConfig } = await import("$lib/api/client.js");
			await updateServerConfig({ auth_token: serverAuthInput.trim() });
			serverAuthSet = serverAuthInput.trim().length > 0;
			serverAuthInput = "";
		} catch {
			// ignore
		} finally {
			serverSaving = false;
		}
	}

	async function clearServerAuth() {
		serverSaving = true;
		try {
			const { updateServerConfig } = await import("$lib/api/client.js");
			await updateServerConfig({ auth_token: "" });
			serverAuthSet = false;
		} catch {
			// ignore
		} finally {
			serverSaving = false;
		}
	}

	// Voice state
	let voiceId = $state("");
	let voiceLoading = $state(true);
	let voiceSaving = $state(false);
	let voiceInput = $state("");

	async function loadVoice() {
		voiceLoading = true;
		try {
			const res = await fetchVoiceId(slug);
			voiceId = res.voice_id || "";
			voiceInput = voiceId;
		} catch {
			// not critical
		} finally {
			voiceLoading = false;
		}
	}

	async function saveVoice() {
		voiceSaving = true;
		try {
			await updateVoiceId(slug, voiceInput.trim());
			voiceId = voiceInput.trim();
		} catch {
			// ignore
		} finally {
			voiceSaving = false;
		}
	}

	async function clearVoice() {
		voiceSaving = true;
		try {
			await updateVoiceId(slug, "");
			voiceId = "";
			voiceInput = "";
		} catch {
			// ignore
		} finally {
			voiceSaving = false;
		}
	}

	async function loadGithub() {
		ghLoading = true;
		try {
			const res = await fetchGithubConfig();
			ghConfigured = res.configured;
		} catch {
			// not critical
		} finally {
			ghLoading = false;
		}
	}

	async function saveGithubToken() {
		const token = ghToken.trim();
		ghSaving = true;
		ghError = "";
		try {
			const res = await updateGithubToken(token);
			ghConfigured = res.configured;
			ghToken = "";
			ghEditing = false;
		} catch (e: any) {
			ghError = e?.message || "failed to save token";
		} finally {
			ghSaving = false;
		}
	}

	async function disconnectGithub() {
		ghSaving = true;
		ghError = "";
		try {
			const res = await updateGithubToken("");
			ghConfigured = res.configured;
		} catch (e: any) {
			ghError = e?.message || "failed to disconnect";
		} finally {
			ghSaving = false;
		}
	}

	// Update state

	// Usage state
	let usage = $state<Usage | null>(null);
	$effect(() => { fetchUsage().then(u => usage = u).catch(() => {}); });

	const apiKeyDefs = [
		{ id: "api_key", name: "Anthropic", hint: "sk-ant-...", required: false, configKey: "anthropic" },
		{ id: "openai", name: "OpenAI", hint: "Chat + semantic memory (independent of chat provider)", required: false, configKey: "openai" },
		{ id: "google_ai", name: "Google AI", hint: "Video analysis", required: false, configKey: "google_ai" },
		{ id: "elevenlabs", name: "ElevenLabs", hint: "Text-to-speech voice", required: false, configKey: "elevenlabs" },
	];

	// Provider + Model mode + API keys state
	let embeddingStatus = $state<EmbeddingStatus | undefined>(undefined);
	let provider = $state("anthropic");
	let setupRequired = $state<string | null>(null);
	let providerSaving = $state(false);
	let modelMode = $state("auto");
	let modelModeSaving = $state(false);
	let configuredKeys = $state<string[]>([]);
	let keySaving = $state("");
	let keyError = $state("");
	let keyEditing = $state("");
	let keyEditValue = $state("");

	$effect(() => {
		fetchConfigStatus().then(s => {
			if (s.model_mode) modelMode = s.model_mode;
			if (s.configured_keys) configuredKeys = s.configured_keys;
			if (s.provider) provider = s.provider === "api" ? "anthropic" : s.provider;
			setupRequired = s.setup_required ?? null;
			embeddingStatus = s.embedding;
		}).catch(() => {});
	});

	async function setProvider(p: 'anthropic' | 'openai') {
		providerSaving = true;
		try {
			await updateProvider(p);
			provider = p;
			setupRequired = (await fetchConfigStatus()).setup_required ?? null;
		} catch {
		} finally {
			providerSaving = false;
		}
	}

	async function saveKey(field: string, value: string) {
		keySaving = field;
		keyError = "";
		try {
			await updateLlmConfig({ [field]: value.trim() });
			// Refresh status
			const s = await fetchConfigStatus();
			setupRequired = s.setup_required ?? null;
			embeddingStatus = s.embedding;
			if (s.configured_keys) configuredKeys = s.configured_keys;
		} catch (e) {
			keyError = e instanceof Error ? e.message : "failed";
		} finally {
			keySaving = "";
		}
	}

	async function setModelMode(mode: string) {
		modelModeSaving = true;
		try {
			await updateModelMode(mode);
			modelMode = mode;
		} catch {
			// revert on failure
		} finally {
			modelModeSaving = false;
		}
	}

	function formatTokens(n: number): string {
		if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
		if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
		return String(n);
	}

	function usagePct(used: number, limit: number): number {
		if (limit <= 0) return 0;
		return Math.min(100, Math.round((used / limit) * 100));
	}

	// Timezone state
	let tzValue = $state("");
	let tzLoading = $state(true);
	let tzSaving = $state(false);

	const COMMON_TIMEZONES = [
		"Asia/Bishkek", "Asia/Almaty", "Asia/Tashkent",
		"Europe/Moscow", "Europe/London", "Europe/Berlin", "Europe/Paris",
		"America/New_York", "America/Chicago", "America/Denver", "America/Los_Angeles",
		"Asia/Tokyo", "Asia/Shanghai", "Asia/Kolkata", "Asia/Dubai",
		"Australia/Sydney", "Pacific/Auckland",
	];

	async function loadTimezone() {
		tzLoading = true;
		try {
			const res = await fetchTimezone(slug);
			tzValue = res.timezone || "";
		} catch {
			// not critical
		} finally {
			tzLoading = false;
		}
	}

	async function saveTimezone(tz: string) {
		tzSaving = true;
		try {
			await updateTimezone(slug, tz);
			tzValue = tz;
		} catch {
			// ignore
		} finally {
			tzSaving = false;
		}
	}

	// Email state
	let emailAccounts = $state<Partial<EmailConfig>[]>([]);
	let emailLoading = $state(true);
	let emailSaving = $state(false);
	let emailError = $state("");
	let emailAdding = $state(false);

	function emptyEmailForm(): EmailConfig {
		return {
			smtp_host: "", smtp_port: 587, smtp_user: "", smtp_password: "", smtp_from: "",
			imap_host: "", imap_port: 993, imap_user: "", imap_password: "",
		};
	}
	let emailForm = $state<EmailConfig>(emptyEmailForm());

	async function loadEmail() {
		emailLoading = true;
		try {
			const res = await fetchEmailAccounts(slug);
			emailAccounts = res.accounts || [];
		} catch {
			// not critical
		} finally {
			emailLoading = false;
		}
	}

	async function saveNewEmail() {
		emailSaving = true;
		emailError = "";
		try {
			// Merge existing accounts (fill in missing passwords) + new one
			const existing: EmailConfig[] = emailAccounts.map(a => ({
				...emptyEmailForm(),
				...a,
			}));
			existing.push(emailForm);
			await saveEmailAccounts(slug, existing);
			emailAdding = false;
			emailForm = emptyEmailForm();
			await loadEmail();
		} catch (e: any) {
			emailError = e?.message || "failed to save email account";
		} finally {
			emailSaving = false;
		}
	}

	async function removeEmailAccount(index: number) {
		emailSaving = true;
		emailError = "";
		try {
			const remaining = emailAccounts.filter((_, i) => i !== index).map(a => ({
				...emptyEmailForm(),
				...a,
			}));
			if (remaining.length > 0) {
				await saveEmailAccounts(slug, remaining);
			} else {
				await deleteAllEmailAccounts(slug);
			}
			await loadEmail();
		} catch (e: any) {
			emailError = e?.message || "failed to remove email account";
		} finally {
			emailSaving = false;
		}
	}

	function isInstalled(name: string): boolean {
		return mcpServers.some((s) => s.name === name);
	}

	function isConnected(name: string): boolean {
		return mcpServers.some((s) => s.name === name && s.connected);
	}

	// Custom servers = installed servers that aren't in the catalog
	let customServers = $derived(
		mcpServers.filter((s) => !suggestedMcp.some((c) => c.name === s.name)),
	);

	async function loadAccounts() {
		loading = true;
		error = "";
		try {
			accounts = await fetchGoogleAccounts(slug);
		} catch (e) {
			console.error("Failed to load Google accounts:", e);
		} finally {
			loading = false;
		}
	}

	async function connectGoogle() {
		connecting = true;
		error = "";
		try {
			const url = await getGoogleConnectUrl(slug);
			window.location.href = url;
		} catch (e: any) {
			error = e?.message || "Failed to start Google connection";
			connecting = false;
		}
	}

	async function disconnect(email: string) {
		disconnecting = email;
		error = "";
		try {
			await disconnectGoogleAccount(slug, email);
			accounts = accounts.filter((a) => a.email !== email);
		} catch (e) {
			error = `Failed to disconnect ${email}`;
		} finally {
			disconnecting = null;
		}
	}

	async function loadMcpServers() {
		mcpLoading = true;
		mcpError = "";
		try {
			[mcpServers, suggestedMcp] = await Promise.all([
				fetchMcpServers(),
				fetchSuggestedMcp(),
			]);
		} catch (e) {
			console.error("Failed to load MCP servers:", e);
		} finally {
			mcpLoading = false;
		}
	}

	async function handleAddCustom() {
		const name = mcpNewName.trim();
		const url = mcpNewUrl.trim();
		if (!name || !url) {
			mcpError = "name and url are required";
			return;
		}
		mcpBusy = name;
		mcpError = "";
		try {
			await addMcpServer(name, url);
			mcpNewName = "";
			mcpNewUrl = "";
			showCustomForm = false;
			await loadMcpServers();
		} catch (e: any) {
			mcpError = e?.message || "failed to add server";
		} finally {
			mcpBusy = null;
		}
	}

	async function handleRemoveCustom(name: string) {
		mcpBusy = name;
		mcpError = "";
		try {
			await removeMcpServer(name);
			mcpServers = mcpServers.filter((s) => s.name !== name);
		} catch (e) {
			mcpError = `failed to remove ${name}`;
		} finally {
			mcpBusy = null;
		}
	}

	$effect(() => {
		slug;
		loadAccounts();
		loadMcpServers();
		loadGithub();
		loadTimezone();
		loadEmail();
		loadVoice();
		loadServer();
		loadScheduled();
	});

	// Scheduled tasks
	let scheduledTasks = $state<ScheduledTask[]>([]);
	let scheduledLoading = $state(true);
	let cancellingId = $state<string | null>(null);

	async function loadScheduled() {
		scheduledLoading = true;
		try {
			scheduledTasks = await fetchScheduledTasks(slug);
		} catch {
			// not critical
		} finally {
			scheduledLoading = false;
		}
	}

	async function cancelTask(id: string) {
		cancellingId = id;
		try {
			await cancelScheduledTask(slug, id);
			scheduledTasks = scheduledTasks.filter((t) => t.id !== id);
		} catch {
			// ignore
		} finally {
			cancellingId = null;
		}
	}

	function formatDeliverAt(ts: number): string {
		const d = new Date(ts * 1000);
		const now = Date.now();
		const diff = ts * 1000 - now;
		if (diff <= 0) return "delivering...";
		if (diff < 60_000) return `in ${Math.ceil(diff / 1000)}s`;
		if (diff < 3600_000) return `in ${Math.ceil(diff / 60_000)}m`;
		if (diff < 86400_000) {
			const h = Math.floor(diff / 3600_000);
			const m = Math.ceil((diff % 3600_000) / 60_000);
			return m > 0 ? `in ${h}h ${m}m` : `in ${h}h`;
		}
		return d.toLocaleString([], { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
	}
</script>

<div class="settings-page">
	<h2 class="settings-title">settings</h2>

	<div class="settings-grid">

	<!-- Server (self-hosted only) -->
	{#if !isManaged}
	<section class="settings-section">
		<div class="section-header">
			<div>
				<h3 class="section-label">server</h3>
				<p class="section-desc">Network and authentication settings.</p>
			</div>
		</div>

		{#if serverLoading}
			<p class="dim-text">loading...</p>
		{:else}
			<div class="setting-row">
				<label class="setting-label">Port</label>
				<div class="setting-input-row">
					<input
						class="setting-input"
						type="number"
						min="1"
						max="65535"
						bind:value={serverPortInput}
					/>
					{#if String(serverPort) !== serverPortInput}
						<button class="setting-btn" onclick={saveServerPort} disabled={serverSaving}>
							{serverSaving ? "..." : "save"}
						</button>
					{/if}
				</div>
				{#if serverNeedsRestart}
					<p class="setting-hint setting-warning">restart nolune to apply port change</p>
				{/if}
			</div>

			<div class="setting-row">
				<label class="setting-label">Auth token</label>
				{#if serverAuthSet}
					<div class="setting-input-row">
						<span class="dim-text">configured</span>
						<button class="setting-btn setting-btn-danger" onclick={clearServerAuth} disabled={serverSaving}>
							remove
						</button>
					</div>
				{:else}
					<div class="setting-input-row">
						<input
							class="setting-input"
							type="password"
							placeholder="optional — protects your API"
							bind:value={serverAuthInput}
						/>
						{#if serverAuthInput.trim()}
							<button class="setting-btn" onclick={saveServerAuth} disabled={serverSaving}>
								{serverSaving ? "..." : "set"}
							</button>
						{/if}
					</div>
				{/if}
				<p class="setting-hint">leave empty for no authentication</p>
			</div>

			<div class="setting-row">
				<label class="setting-label">Host</label>
				<span class="dim-text">{serverHost}</span>
			</div>
		{/if}
	</section>
	{/if}

	<!-- Usage -->
	{#if usage && (usage.tokens_4h_limit > 0 || usage.tokens_week_limit > 0 || usage.tokens_month_limit > 0)}
		<section class="settings-section">
			<div class="section-header">
				<img src="/icons/icon-usage.png" alt="" class="section-icon-img" />
				<div>
					<h3 class="section-label">usage</h3>
					<p class="section-desc">Token usage across time windows.</p>
				</div>
			</div>
			<div class="usage-windows">
				{#if usage.tokens_4h_limit > 0}
					{@const p = usagePct(usage.tokens_last_4h, usage.tokens_4h_limit)}
					<div class="usage-window">
						<div class="usage-window-header">
							<span class="usage-window-label">4 hours</span>
							<span class="usage-window-value">{formatTokens(usage.tokens_last_4h)} / {formatTokens(usage.tokens_4h_limit)}</span>
						</div>
						<div class="usage-window-track">
							<div class="usage-window-fill" style="width: {p}%; background: {p >= 90 ? 'oklch(0.65 0.2 25)' : p >= 70 ? 'oklch(0.75 0.15 85)' : 'oklch(0.55 0.08 var(--accent-hue) / 50%)'}"></div>
						</div>
					</div>
				{/if}
				{#if usage.tokens_week_limit > 0}
					{@const p = usagePct(usage.tokens_this_week, usage.tokens_week_limit)}
					<div class="usage-window">
						<div class="usage-window-header">
							<span class="usage-window-label">this week</span>
							<span class="usage-window-value">{formatTokens(usage.tokens_this_week)} / {formatTokens(usage.tokens_week_limit)}</span>
						</div>
						<div class="usage-window-track">
							<div class="usage-window-fill" style="width: {p}%; background: {p >= 90 ? 'oklch(0.65 0.2 25)' : p >= 70 ? 'oklch(0.75 0.15 85)' : 'oklch(0.55 0.08 var(--accent-hue) / 50%)'}"></div>
						</div>
					</div>
				{/if}
				{#if usage.tokens_month_limit > 0}
					{@const p = usagePct(usage.tokens_this_month, usage.tokens_month_limit)}
					<div class="usage-window">
						<div class="usage-window-header">
							<span class="usage-window-label">this month</span>
							<span class="usage-window-value">{formatTokens(usage.tokens_this_month)} / {formatTokens(usage.tokens_month_limit)}</span>
						</div>
						<div class="usage-window-track">
							<div class="usage-window-fill" style="width: {p}%; background: {p >= 90 ? 'oklch(0.65 0.2 25)' : p >= 70 ? 'oklch(0.75 0.15 85)' : 'oklch(0.55 0.08 var(--accent-hue) / 50%)'}"></div>
						</div>
					</div>
				{/if}
			</div>
		</section>
	{/if}

	<!-- Skin -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon">✦</div>
			<div>
				<h3 class="section-label">skin</h3>
				<p class="section-desc">Change how your companion looks and animates.</p>
			</div>
		</div>
		<div class="model-mode-options">
			{#each SKINS as skin (skin.id)}
				<button
					class="mode-option skin-option"
					class:mode-active={skinStore.skinId === skin.id}
					onclick={() => skinStore.setSkin(skin.id)}
				>
					<img src={skin.thumbnail} alt={skin.label} class="skin-thumb" />
					<div>
						<span class="mode-name">{skin.label}</span>
						<span class="mode-desc">{skin.avatar ? "Little Moon companion" : `${skin.clips.thinking.length} animations`}</span>
					</div>
				</button>
			{/each}
		</div>
	</section>

	<!-- Model Mode -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-model.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">model mode</h3>
				<p class="section-desc">Choose how your companion picks the AI model for each message.</p>
			</div>
		</div>
		<div class="model-mode-options" class:disabled={modelModeSaving}>
			<button
				class="mode-option"
				class:mode-active={modelMode === "auto"}
				onclick={() => setModelMode("auto")}
				disabled={modelModeSaving}
			>
				<span class="mode-name">auto</span>
				<span class="mode-desc">smart routing — cheap for casual, powerful when needed</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={modelMode === "fast"}
				onclick={() => setModelMode("fast")}
				disabled={modelModeSaving}
			>
				<span class="mode-name">fast</span>
				<span class="mode-desc">always use the lightweight model — saves budget</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={modelMode === "heavy"}
				onclick={() => setModelMode("heavy")}
				disabled={modelModeSaving}
			>
				<span class="mode-name">heavy</span>
				<span class="mode-desc">always use the powerful model — uses 10x more budget</span>
			</button>
		</div>
	</section>

	<!-- Provider -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon">⚡</div>
			<div>
				<h3 class="section-label">provider</h3>
				<p class="section-desc">Choose which AI powers your companion.</p>
			</div>
		</div>
		{#if setupRequired}<p class="section-desc">{setupRequired}</p>{/if}
		<div class="model-mode-options" class:disabled={providerSaving}>
			<button
				class="mode-option"
				class:mode-active={provider === "anthropic"}
				onclick={() => setProvider("anthropic")}
				disabled={providerSaving}
			>
				<span class="mode-name">Anthropic</span>
				<span class="mode-desc">pay-per-use with your own Anthropic API key</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={provider === "openai"}
				onclick={() => setProvider("openai")}
				disabled={providerSaving}
			>
				<span class="mode-name">OpenAI</span>
				<span class="mode-desc">pay-per-use with your own OpenAI API key</span>
			</button>
		</div>
	</section>

	<!-- API Keys -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon">🔑</div>
			<div>
				<h3 class="section-label">api keys</h3>
				<p class="section-desc">Connect your API keys. Required for API key providers.</p>
			</div>
		</div>
		<p class="section-desc">{embeddingStatusText(embeddingStatus)}</p>
		<p class="section-desc">Embedding settings and OpenAI key changes require a server restart. Updates replace the full embedding configuration. Local OpenAI-compatible endpoints are unauthenticated and never receive your OpenAI key. Raw media is saved without semantic indexing.</p>
		<div class="keys-list">
			{#each apiKeyDefs as key (key.id)}
				{@const configured = configuredKeys.includes(key.configKey)}
				<div class="key-row">
					<div class="key-info">
						<span class="key-name">{key.name}{key.required ? " *" : ""}</span>
						<span class="key-hint">{key.hint}</span>
					</div>
					<div class="key-action">
						{#if keyEditing === key.id}
							<input
								class="key-input"
								type="password"
								placeholder="{key.name} key..."
								bind:value={keyEditValue}
								onkeydown={(e) => {
									if (e.key === "Enter" && keyEditValue.trim()) {
										saveKey(key.id, keyEditValue);
										keyEditing = "";
										keyEditValue = "";
									}
									if (e.key === "Escape") { keyEditing = ""; keyEditValue = ""; }
								}}
							/>
							<button
								class="key-change"
								onclick={() => {
									if (keyEditValue.trim()) saveKey(key.id, keyEditValue);
									keyEditing = "";
									keyEditValue = "";
								}}
								disabled={keySaving === key.id}
							>{keySaving === key.id ? "saving..." : "save"}</button>
							<button class="key-change" onclick={() => { keyEditing = ""; keyEditValue = ""; }}>cancel</button>
						{:else if configured}
							<span class="key-badge key-badge-ok">connected</span>
							<button
								class="key-change"
								onclick={() => { keyEditing = key.id; keyEditValue = ""; }}
								disabled={keySaving === key.id}
							>change</button>
							<button
								class="key-change key-change-remove"
								onclick={() => saveKey(key.id, "")}
								disabled={keySaving === key.id}
							>remove</button>
						{:else}
							<button
								class="key-change key-change-add"
								onclick={() => { keyEditing = key.id; keyEditValue = ""; }}
								disabled={keySaving === key.id}
							>{keySaving === key.id ? "saving..." : "add key"}</button>
						{/if}
					</div>
				</div>
			{/each}
		</div>
		{#if keyError}
			<p class="key-error">{keyError}</p>
		{/if}
	</section>

	<!-- Timezone -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-timezone.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">timezone</h3>
				<p class="section-desc">
					Set your local timezone so your companion knows the right time of day.
				</p>
			</div>
		</div>

		{#if tzLoading}
			<div class="ext-loading">
				<div class="loading-dot"></div>
			</div>
		{:else}
			<div class="tz-picker">
				<select
					class="tz-select"
					value={tzValue}
					disabled={tzSaving}
					onchange={(e) => saveTimezone((e.target as HTMLSelectElement).value)}
				>
					<option value="">UTC (default)</option>
					{#each COMMON_TIMEZONES as tz}
						<option value={tz} selected={tzValue === tz}>{tz.replace(/_/g, " ")}</option>
					{/each}
				</select>
				{#if tzValue}
					<span class="tz-current">{tzValue.replace(/_/g, " ")}</span>
				{/if}
			</div>
		{/if}
	</section>

	<!-- Extensions (MCP Servers) -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-extensions.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">extensions</h3>
				<p class="section-desc">Give your companion new abilities via MCP servers.</p>
			</div>
		</div>

		{#if mcpLoading}
			<div class="ext-loading"><div class="loading-dot"></div></div>
		{:else}
			<!-- Suggested servers -->
			{#if suggestedMcp.length > 0}
				<div class="keys-list">
					{#each suggestedMcp as entry (entry.name)}
						{@const installed = isInstalled(entry.name)}
						{@const connected = isConnected(entry.name)}
						{@const busy = mcpBusy === entry.name}
						<div class="key-row">
							<div class="key-info">
								<span class="key-name">{entry.name}</span>
								<span class="key-hint">{entry.description}</span>
							</div>
							<div class="key-action">
								{#if installed && connected}
									<span class="key-badge key-badge-ok">connected</span>
									<button class="key-change" disabled={busy} onclick={() => handleRemoveCustom(entry.name)}>
										{busy ? "..." : "remove"}
									</button>
								{:else if installed}
									<span class="key-badge" style="background: oklch(0.78 0.12 75 / 12%); color: oklch(0.78 0.12 75);">reconnecting</span>
								{:else}
									<button
										class="key-change key-change-add"
										disabled={busy}
										onclick={async () => {
											if (entry.requires_key) {
												const key = prompt(`${entry.name} API key:\n\nGet one at ${entry.key_url}`);
												if (!key) return;
												// Set env var via the MCP server URL with key in headers
												mcpBusy = entry.name;
												try {
													await addMcpServer(entry.name, entry.url);
													await loadMcpServers();
												} catch (e) {
													mcpError = e instanceof Error ? e.message : "failed";
												} finally {
													mcpBusy = null;
												}
											} else {
												mcpBusy = entry.name;
												try {
													await addMcpServer(entry.name, entry.url);
													await loadMcpServers();
												} catch (e) {
													mcpError = e instanceof Error ? e.message : "failed";
												} finally {
													mcpBusy = null;
												}
											}
										}}
									>
										{busy ? "connecting..." : "add"}
									</button>
								{/if}
							</div>
						</div>
					{/each}
				</div>
			{/if}

			<!-- Custom/user-added servers -->
			{#if customServers.length > 0}
				<div class="keys-list" style="margin-top: 12px;">
					{#each customServers as server}
						<div class="key-row">
							<div class="key-info">
								<span class="key-name">{server.name}</span>
								<span class="key-hint">{server.url ?? "local process"}</span>
							</div>
							<div class="key-action">
								<span class="key-badge" class:key-badge-ok={server.connected}
									style={server.connected ? "" : "background: oklch(0.65 0.15 25 / 12%); color: oklch(0.65 0.15 25);"}
								>
									{server.connected ? "connected" : "disconnected"}
								</span>
								<button class="key-change" disabled={mcpBusy === server.name} onclick={() => handleRemoveCustom(server.name)}>
									{mcpBusy === server.name ? "..." : "remove"}
								</button>
							</div>
						</div>
					{/each}
				</div>
			{/if}

			<!-- Add custom -->
			<div style="margin-top: 12px;">
				{#if showCustomForm}
					<div class="ext-custom-form">
						<div class="ext-custom-form-title">add custom server</div>
						<input class="ext-input" type="text" placeholder="name" bind:value={mcpNewName} />
						<input class="ext-input" type="url" placeholder="server url" bind:value={mcpNewUrl} />
						<div class="ext-form-actions">
							<button class="ext-form-btn ext-form-add" disabled={mcpBusy !== null} onclick={handleAddCustom}>
								{mcpBusy ? "connecting..." : "add"}
							</button>
							<button class="ext-form-btn ext-form-cancel" onclick={() => { showCustomForm = false; mcpError = ""; }}>
								cancel
							</button>
						</div>
					</div>
				{:else}
					<button class="ext-advanced-toggle" onclick={() => showCustomForm = true}>
						+ add custom server
					</button>
				{/if}
			</div>
		{/if}

		{#if mcpError}
			<p class="key-error">{mcpError}</p>
		{/if}
	</section>

	<!-- Google Accounts -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-google.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">google accounts</h3>
				<p class="section-desc">
					Connect Google to enable Gmail, Calendar, and Drive tools.
				</p>
			</div>
		</div>

		{#if loading}
			<div class="ext-loading">
				<div class="loading-dot"></div>
			</div>
		{:else}
			{#if accounts.length > 0}
				<div class="accounts-list">
					{#each accounts as account}
						<div class="account-row">
							<span class="account-email">{account.email}</span>
							<button
								class="ext-remove-btn"
								disabled={disconnecting === account.email}
								onclick={() => disconnect(account.email)}
							>
								{disconnecting === account.email
									? "..."
									: "disconnect"}
							</button>
						</div>
					{/each}
				</div>
			{:else}
				<p class="no-accounts">no google accounts connected</p>
			{/if}

			<button
				class="ext-form-btn ext-form-add"
				disabled={connecting}
				onclick={connectGoogle}
			>
				{connecting ? "connecting..." : "+ connect google account"}
			</button>
		{/if}

		{#if error}
			<p class="error-msg">{error}</p>
		{/if}
	</section>

	<!-- Email (SMTP/IMAP) -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-email.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">email</h3>
				<p class="section-desc">
					Connect any email via SMTP/IMAP (iCloud, Outlook, Yahoo, etc.)
					to enable send and read email tools.
				</p>
			</div>
		</div>

		{#if emailLoading}
			<div class="ext-loading">
				<div class="loading-dot"></div>
			</div>
		{:else}
			<!-- Existing accounts -->
			{#if emailAccounts.length > 0}
				<div class="accounts-list">
					{#each emailAccounts as acct, i}
						<div class="account-row">
							<span class="account-email">{acct.smtp_from || acct.smtp_user || acct.imap_user || "account"}</span>
							<button
								class="ext-remove-btn"
								disabled={emailSaving}
								onclick={() => removeEmailAccount(i)}
							>
								{emailSaving ? "..." : "remove"}
							</button>
						</div>
					{/each}
				</div>
			{/if}

			<!-- Add new account form -->
			{#if emailAdding}
				<div class="email-form">
					<div class="email-form-group">
						<span class="email-form-label">outgoing (smtp)</span>
						<input class="ext-input" type="text" placeholder="smtp host (e.g. smtp.mail.me.com)" bind:value={emailForm.smtp_host} />
						<div class="email-form-row">
							<input class="ext-input" type="number" placeholder="port" bind:value={emailForm.smtp_port} style="width: 5rem;" />
							<input class="ext-input" style="flex:1" type="text" placeholder="username / email" bind:value={emailForm.smtp_user} />
						</div>
						<input class="ext-input" type="password" placeholder="password / app-specific password" bind:value={emailForm.smtp_password} />
						<input class="ext-input" type="email" placeholder="from address (e.g. user@icloud.com)" bind:value={emailForm.smtp_from} />
					</div>

					<div class="email-form-group">
						<span class="email-form-label">incoming (imap)</span>
						<input class="ext-input" type="text" placeholder="imap host (e.g. imap.mail.me.com)" bind:value={emailForm.imap_host} />
						<div class="email-form-row">
							<input class="ext-input" type="number" placeholder="port" bind:value={emailForm.imap_port} style="width: 5rem;" />
							<input class="ext-input" style="flex:1" type="text" placeholder="username / email" bind:value={emailForm.imap_user} />
						</div>
						<input class="ext-input" type="password" placeholder="password / app-specific password" bind:value={emailForm.imap_password} />
					</div>

					<div class="ext-form-actions">
						<button
							class="ext-form-btn ext-form-add"
							disabled={emailSaving || (!emailForm.smtp_host && !emailForm.imap_host)}
							onclick={saveNewEmail}
						>
							{emailSaving ? "saving..." : "add account"}
						</button>
						<button class="ext-form-btn ext-form-cancel" onclick={() => { emailAdding = false; emailError = ""; emailForm = emptyEmailForm(); }}>
							cancel
						</button>
					</div>
				</div>
			{:else}
				<button
					class="ext-form-btn ext-form-add"
					onclick={() => emailAdding = true}
				>
					+ add email account
				</button>
			{/if}
		{/if}

		{#if emailError}
			<p class="error-msg">{emailError}</p>
		{/if}
	</section>

	<!-- GitHub -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-github.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">github</h3>
				<p class="section-desc">
					Connect GitHub to enable cloning repos, creating branches, PRs, and managing issues.
				</p>
			</div>
		</div>

		{#if ghLoading}
			<div class="ext-loading">
				<div class="loading-dot"></div>
			</div>
		{:else if ghConfigured && !ghEditing}
			<div class="gh-status">
				<div class="gh-status-info">
					<span class="gh-status-dot"></span>
					<span class="gh-status-text">token configured</span>
				</div>
				<div class="gh-status-actions">
					<button class="ext-form-btn ext-form-cancel" onclick={() => ghEditing = true}>
						change
					</button>
					<button
						class="ext-remove-btn"
						disabled={ghSaving}
						onclick={disconnectGithub}
					>
						{ghSaving ? "..." : "remove"}
					</button>
				</div>
			</div>
		{:else}
			<div class="gh-token-form">
				<input
					class="ext-input"
					type="password"
					placeholder="ghp_... or github_pat_..."
					bind:value={ghToken}
					onkeydown={(e) => e.key === "Enter" && saveGithubToken()}
				/>
				<div class="ext-form-actions">
					<button
						class="ext-form-btn ext-form-add"
						disabled={ghSaving || !ghToken.trim()}
						onclick={saveGithubToken}
					>
						{ghSaving ? "saving..." : "save token"}
					</button>
					{#if ghEditing}
						<button class="ext-form-btn ext-form-cancel" onclick={() => { ghEditing = false; ghToken = ""; ghError = ""; }}>
							cancel
						</button>
					{/if}
				</div>
			</div>
		{/if}

		{#if ghError}
			<p class="error-msg">{ghError}</p>
		{/if}
	</section>

	<!-- Voice -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-voice.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">voice</h3>
				<p class="section-desc">
					ElevenLabs voice ID for text-to-speech. Leave empty to use the default voice.
				</p>
			</div>
		</div>

		{#if voiceLoading}
			<div class="ext-loading">
				<div class="loading-dot"></div>
			</div>
		{:else}
			<div class="gh-token-form">
				<input
					class="ext-input"
					type="text"
					placeholder="e.g. TWutjvRaJqAX89preB4e"
					bind:value={voiceInput}
					onkeydown={(e) => e.key === "Enter" && saveVoice()}
				/>
				<div class="ext-form-actions">
					<button
						class="ext-form-btn ext-form-add"
						disabled={voiceSaving || voiceInput.trim() === voiceId}
						onclick={saveVoice}
					>
						{voiceSaving ? "saving..." : "save"}
					</button>
					{#if voiceId}
						<button
							class="ext-form-btn ext-form-cancel"
							disabled={voiceSaving}
							onclick={clearVoice}
						>
							reset to default
						</button>
					{/if}
				</div>
			</div>
		{/if}
	</section>

	<!-- Scheduled tasks -->
	{#if !scheduledLoading && scheduledTasks.length > 0}
		<section class="settings-section">
			<div class="section-header">
				<img src="/icons/icon-scheduled.png" alt="" class="section-icon-img" />
				<div>
					<h3 class="section-label">scheduled</h3>
					<p class="section-desc">{scheduledTasks.length} pending task{scheduledTasks.length === 1 ? "" : "s"}</p>
				</div>
			</div>
			<div class="sched-list">
				{#each scheduledTasks as task (task.id)}
					<div class="sched-item">
						<div class="sched-content">
							<span class="sched-text">{task.task.length > 80 ? task.task.slice(0, 80) + "…" : task.task}</span>
							<span class="sched-time">{formatDeliverAt(task.deliver_at)}</span>
						</div>
						<button
							class="sched-cancel"
							disabled={cancellingId === task.id}
							onclick={() => cancelTask(task.id)}
						>
							{cancellingId === task.id ? "…" : "cancel"}
						</button>
					</div>
				{/each}
			</div>
		</section>
	{/if}

	<!-- Export / Import -->
	<section class="settings-section">
		<div class="section-header">
			<img src="/icons/icon-data.png" alt="" class="section-icon-img" />
			<div>
				<h3 class="section-label">data</h3>
			</div>
		</div>
		<div class="data-actions">
			<button class="data-btn" onclick={handleExport} disabled={exporting}>
				{#if exporting}
					exporting… {formatBytes(exportBytes)}
				{:else}
					export
				{/if}
			</button>
			<label class="data-btn data-btn-import">
				{#if importing}
					importing...
				{:else if importDone}
					imported!
				{:else}
					import
				{/if}
				<input
					type="file"
					accept=".tar.gz,.tgz"
					bind:this={importFileInput}
					onchange={handleImport}
					hidden
					disabled={importing}
				/>
			</label>
		</div>
		<p class="data-hint">export downloads a .tar.gz of all instance data (soul, memory, drops, chat history). import merges into the current instance.</p>

		<div class="data-actions" style="margin-top: 1.25rem;">
			<label class="data-btn data-btn-knowledge">
				{#if importingKnowledge}
					uploading...
				{:else if knowledgeStarted}
					started!
				{:else}
					import knowledge
				{/if}
				<input
					type="file"
					accept=".json,.txt,.md,.csv"
					multiple
					bind:this={knowledgeFileInput}
					onchange={handleImportKnowledge}
					hidden
					disabled={importingKnowledge}
				/>
			</label>
			<span class="data-hint">drop your Claude export, notes, or any personal data — AI will extract facts and add them to memory</span>
		</div>
		{#if knowledgeError}
			<p class="error-msg">{knowledgeError}</p>
		{/if}
		{#if knowledgeStarted && !importStage}
			<p class="data-hint" style="color: oklch(0.72 0.15 155); margin-top: 0.5rem;">
				processing in background — check memory after a few minutes
			</p>
		{/if}

		{#if importStage}
			<div class="import-progress" class:import-done={importStage === 'done'} class:import-error={importStage === 'error'}>
				<div class="import-progress-header">
					{#if importStage === 'done'}
						<span class="import-progress-icon">&#10003;</span>
					{:else if importStage === 'error'}
						<span class="import-progress-icon">&#10007;</span>
					{:else}
						<span class="import-progress-spinner"></span>
					{/if}
					<span class="import-progress-stage">{importStage}</span>
				</div>
				<p class="import-progress-detail">{importDetail}</p>
			</div>
		{/if}

		<div class="data-actions" style="margin-top: 0.75rem;">
			<button
				class="ext-form-btn"
				disabled={reindexing}
				onclick={handleReindex}
			>
				{reindexing ? 'reindexing...' : 'reindex memory'}
			</button>
			<span class="data-hint">rebuild vector search index for all memories</span>
		</div>
		{#if importError}
			<p class="error-msg">{importError}</p>
		{/if}
		{#if exportError}
			<p class="error-msg">{exportError}</p>
		{/if}
	</section>

	</div><!-- settings-grid -->
</div>

<style>
	/* Usage */
	.usage-windows {
		display: flex;
		flex-direction: column;
		gap: 0.625rem;
	}
	.usage-window-header {
		display: flex;
		justify-content: space-between;
		align-items: baseline;
		margin-bottom: 0.25rem;
	}
	.usage-window-label {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--foreground);
	}
	.usage-window-value {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: oklch(0.55 0.08 var(--accent-hue) / 30%);
	}
	.usage-window-track {
		height: 4px;
		border-radius: 2px;
		background: oklch(var(--ink) / 5%);
		overflow: hidden;
	}
	.usage-window-fill {
		height: 100%;
		border-radius: 2px;
		transition: width 0.5s ease;
	}

	.settings-page {
		padding: 2rem 2.5rem;
		padding-bottom: calc(2rem + env(safe-area-inset-bottom, 0px));
		max-width: 1200px;
		margin: 0 auto;
		height: 100%;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 1.5rem;
	}

	.settings-title {
		font-family: var(--font-display);
		font-style: italic;
		font-size: 1.25rem;
		font-weight: 400;
		color: oklch(0.88 0.02 240 / 80%);
		margin-bottom: 0.5rem;
	}

	.settings-grid {
		display: grid;
		grid-template-columns: repeat(2, 1fr);
		gap: 1rem;
		align-items: start;
	}

	@media (max-width: 900px) {
		.settings-grid {
			grid-template-columns: 1fr;
		}
	}

	@media (max-width: 768px) {
		.settings-page {
			padding: 1.5rem 1rem;
		}
	}

	.settings-section {
		position: relative;
		padding: 1.25rem;
		border-radius: 1rem;
		border: 1px solid oklch(var(--ink) / 10%);
		border-top-color: oklch(var(--ink) / 18%);
		background: linear-gradient(
			150deg,
			oklch(var(--ink) / 5%) 0%,
			oklch(0.5 0.02 250 / 8%) 40%,
			oklch(var(--ink) / 3%) 100%
		);
		backdrop-filter: blur(20px) saturate(150%) brightness(1.05);
		-webkit-backdrop-filter: blur(20px) saturate(150%) brightness(1.05);
		box-shadow:
			0 2px 12px oklch(var(--shade) / 12%),
			inset 0 1px 0 oklch(var(--ink) / 8%),
			inset 0 -1px 0 oklch(var(--shade) / 4%);
		overflow: hidden;
	}
	.settings-section::before {
		content: "";
		position: absolute;
		top: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: linear-gradient(90deg, transparent, oklch(var(--ink) / 20%), transparent);
		pointer-events: none;
	}

	.section-header {
		display: flex;
		align-items: flex-start;
		gap: 0.75rem;
		margin-bottom: 1rem;
	}

	.section-icon-img {
		width: 2.5rem;
		height: 2.5rem;
		object-fit: cover;
		flex-shrink: 0;
		border-radius: 0.625rem;
		border: 1px solid oklch(var(--ink) / 10%);
		border-top-color: oklch(var(--ink) / 18%);
		box-shadow:
			0 2px 8px oklch(var(--shade) / 20%),
			inset 0 1px 0 oklch(var(--ink) / 6%);
	}

	.tz-picker {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.tz-select {
		flex: 1;
		font-family: var(--font-mono);
		font-size: 0.72rem;
		color: var(--foreground);
		background: oklch(var(--ink) / 3%);
		border: 1px solid oklch(var(--ink) / 8%);
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		outline: none;
		transition: border-color 0.2s ease;
		appearance: none;
		cursor: pointer;
	}
	.tz-select:focus {
		border-color: oklch(0.55 0.08 var(--accent-hue) / 30%);
	}
	.tz-select option {
		background: oklch(0.10 0.015 280);
		color: var(--foreground);
	}

	.tz-current {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: oklch(0.75 0.10 200 / 50%);
		white-space: nowrap;
	}

	.section-label {
		font-family: var(--font-mono);
		font-size: 0.8rem;
		color: var(--foreground);
		letter-spacing: 0.02em;
		margin-bottom: 0.2rem;
	}

	.section-desc {
		font-family: var(--font-body);
		font-size: 0.7rem;
		color: var(--foreground);
	}

	/* --- extension catalog --- */

	.ext-loading {
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 1rem;
	}

	.ext-remove-btn {
		font-family: var(--font-body);
		font-size: 0.72rem;
		color: oklch(0.65 0.12 25 / 50%);
		background: none;
		border: none;
		cursor: pointer;
		padding: 0.25rem 0.5rem;
		border-radius: 0.25rem;
		transition: all 0.2s ease;
	}
	.ext-remove-btn:hover:not(:disabled) {
		color: oklch(0.65 0.15 25 / 90%);
		background: oklch(0.65 0.15 25 / 8%);
	}
	.ext-remove-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.ext-input {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--foreground);
		background: oklch(var(--ink) / 3%);
		border: 1px solid oklch(var(--ink) / 8%);
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		outline: none;
		transition: border-color 0.2s ease;
	}
	.ext-input:focus {
		border-color: oklch(0.55 0.08 var(--accent-hue) / 30%);
	}
	.ext-input::placeholder {
		color: var(--foreground);
	}

	.ext-form-actions {
		display: flex;
		gap: 0.5rem;
	}

	.ext-form-btn {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		padding: 0.4rem 0.85rem;
		border-radius: 0.375rem;
		cursor: pointer;
		transition: all 0.2s ease;
		letter-spacing: 0.02em;
	}

	.ext-form-add {
		color: oklch(0.55 0.08 var(--accent-hue) / 70%);
		background: oklch(0.55 0.08 var(--accent-hue) / 8%);
		border: 1px solid oklch(0.55 0.08 var(--accent-hue) / 15%);
	}
	.ext-form-add:hover:not(:disabled) {
		background: oklch(0.55 0.08 var(--accent-hue) / 14%);
		border-color: oklch(0.55 0.08 var(--accent-hue) / 35%);
	}
	.ext-form-add:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.ext-form-cancel {
		color: oklch(0.70 0.02 280 / 45%);
		background: none;
		border: 1px solid oklch(var(--ink) / 6%);
	}
	.ext-form-cancel:hover {
		color: oklch(0.80 0.02 280 / 60%);
		background: oklch(var(--ink) / 3%);
	}

	/* --- google / shared --- */

	.accounts-list {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
		margin-bottom: 0.75rem;
	}

	.account-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		background: oklch(var(--ink) / 3%);
		border: 1px solid oklch(var(--ink) / 5%);
	}

	.account-email {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: var(--foreground);
	}

	.no-accounts {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--foreground);
		font-style: italic;
		margin-bottom: 0.75rem;
	}

	.loading-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: oklch(0.55 0.08 var(--accent-hue) / 30%);
		animation: pulse 1.5s ease-in-out infinite;
	}
	@keyframes pulse {
		0%, 100% { opacity: 1; transform: scale(1); }
		50% { opacity: 0.3; transform: scale(0.7); }
	}

	.error-msg {
		font-family: var(--font-body);
		font-size: 0.7rem;
		color: oklch(0.65 0.15 25 / 70%);
		font-style: italic;
		margin-top: 0.5rem;
	}

	/* --- github --- */

	.gh-status {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		background: oklch(var(--ink) / 3%);
		border: 1px solid oklch(var(--ink) / 5%);
	}

	.gh-status-info {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.gh-status-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: oklch(0.70 0.12 145 / 70%);
	}

	.gh-status-text {
		font-family: var(--font-mono);
		font-size: 0.72rem;
		color: var(--foreground);
	}

	.gh-status-actions {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}

	.gh-token-form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	/* --- email --- */

	.email-form {
		display: flex;
		flex-direction: column;
		gap: 0.75rem;
	}

	.email-form-group {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}

	.email-form-label {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: oklch(0.55 0.02 280 / 45%);
		letter-spacing: 0.04em;
		text-transform: uppercase;
	}

	.email-form-row {
		display: flex;
		gap: 0.4rem;
	}

	/* Data export/import */
	.data-actions {
		display: flex;
		gap: 0.5rem;
	}
	.data-btn {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		padding: 0.4rem 0.9rem;
		border-radius: 0.375rem;
		cursor: pointer;
		transition: all 0.2s ease;
		color: oklch(0.55 0.08 var(--accent-hue) / 70%);
		background: oklch(0.55 0.08 var(--accent-hue) / 8%);
		border: 1px solid oklch(0.55 0.08 var(--accent-hue) / 15%);
	}
	.data-btn:hover {
		background: oklch(0.55 0.08 var(--accent-hue) / 14%);
		border-color: oklch(0.55 0.08 var(--accent-hue) / 25%);
	}
	.data-btn-knowledge {
		color: oklch(0.78 0.12 75 / 80%);
		background: oklch(0.78 0.12 75 / 8%);
		border-color: oklch(0.78 0.12 75 / 18%);
	}
	.data-btn-knowledge:hover {
		background: oklch(0.78 0.12 75 / 14%);
		border-color: oklch(0.78 0.12 75 / 28%);
	}
	.import-progress {
		margin-top: 0.75rem;
		padding: 0.625rem 0.875rem;
		border-radius: 0.5rem;
		background: oklch(0.55 0.08 var(--accent-hue) / 6%);
		border: 1px solid oklch(0.55 0.08 var(--accent-hue) / 12%);
	}
	.import-progress.import-done {
		background: oklch(0.72 0.15 155 / 6%);
		border-color: oklch(0.72 0.15 155 / 15%);
	}
	.import-progress.import-error {
		background: oklch(0.65 0.15 25 / 6%);
		border-color: oklch(0.65 0.15 25 / 15%);
	}
	.import-progress-header {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}
	.import-progress-stage {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		color: oklch(0.55 0.08 var(--accent-hue) / 60%);
	}
	.import-done .import-progress-stage {
		color: oklch(0.72 0.15 155 / 80%);
	}
	.import-error .import-progress-stage {
		color: oklch(0.65 0.15 25 / 80%);
	}
	.import-progress-icon {
		font-size: 0.75rem;
	}
	.import-done .import-progress-icon {
		color: oklch(0.72 0.15 155);
	}
	.import-error .import-progress-icon {
		color: oklch(0.65 0.15 25);
	}
	.import-progress-detail {
		font-size: 0.7rem;
		color: oklch(0.55 0.08 var(--accent-hue) / 40%);
		margin-top: 0.25rem;
	}
	.import-progress-spinner {
		width: 12px;
		height: 12px;
		border: 1.5px solid oklch(0.55 0.08 var(--accent-hue) / 20%);
		border-top-color: oklch(0.78 0.12 75 / 60%);
		border-radius: 50%;
		animation: spin 0.8s linear infinite;
	}
	@keyframes spin {
		to { transform: rotate(360deg); }
	}
	/* Scheduled messages */
	.sched-list {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
	}
	.sched-item {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.5rem 0.625rem;
		border-radius: 0.5rem;
		background: oklch(0.4 0.04 220 / 6%);
		border: 1px solid oklch(0.5 0.06 220 / 6%);
	}
	.sched-content {
		flex: 1;
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
	}
	.sched-text {
		font-family: var(--font-body);
		font-size: 0.72rem;
		color: oklch(0.85 0.02 220 / 65%);
		line-height: 1.35;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sched-time {
		font-family: var(--font-mono);
		font-size: 0.65rem;
		color: oklch(0.65 0.08 220 / 40%);
		letter-spacing: 0.04em;
	}
	.sched-cancel {
		flex-shrink: 0;
		font-family: var(--font-mono);
		font-size: 0.68rem;
		padding: 0.2rem 0.5rem;
		border-radius: 0.3rem;
		background: none;
		border: 1px solid oklch(0.65 0.10 25 / 20%);
		color: oklch(0.65 0.10 25 / 55%);
		cursor: pointer;
		transition: all 0.2s ease;
	}
	.sched-cancel:hover:not(:disabled) {
		border-color: oklch(0.65 0.14 25 / 40%);
		color: oklch(0.65 0.14 25 / 80%);
		background: oklch(0.65 0.10 25 / 8%);
	}
	.sched-cancel:disabled {
		opacity: 0.4;
		cursor: default;
	}

	.data-hint {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: var(--foreground);
		margin-top: 0.5rem;
		line-height: 1.5;
	}

	/* --- model mode --- */
	.model-mode-options {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
	}
	.model-mode-options.disabled {
		opacity: 0.5;
		pointer-events: none;
	}
	.skin-option {
		flex-direction: row;
		align-items: center;
		gap: 0.75rem;
	}
	.skin-thumb {
		width: 48px;
		height: 48px;
		border-radius: 0.375rem;
		object-fit: cover;
		flex-shrink: 0;
	}

	.mode-option {
		display: flex;
		flex-direction: column;
		gap: 0.15rem;
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		background: oklch(var(--ink) / 3%);
		border: 1px solid oklch(var(--ink) / 6%);
		cursor: pointer;
		text-align: left;
		transition: all 0.15s;
	}
	.mode-option:hover:not(:disabled) {
		background: oklch(var(--ink) / 5%);
		border-color: oklch(var(--ink) / 10%);
	}
	.mode-active {
		background: oklch(0.55 0.08 var(--accent-hue) / 8%);
		border-color: oklch(0.55 0.08 var(--accent-hue) / 25%);
	}
	.mode-active:hover:not(:disabled) {
		background: oklch(0.55 0.08 var(--accent-hue) / 12%);
	}
	.mode-name {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--foreground);
	}
	.mode-active .mode-name {
		color: oklch(0.55 0.08 var(--accent-hue));
	}
	.mode-desc {
		font-family: var(--font-mono);
		font-size: 0.68rem;
		color: var(--foreground);
	}

	/* ── API Keys ── */
	.keys-list { display: flex; flex-direction: column; gap: 1px; background: oklch(var(--ink) / 4%); border-radius: 12px; overflow: hidden; }
	.key-row { display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; background: var(--surface-elevated); }
	.key-info { display: flex; flex-direction: column; gap: 2px; }
	.key-name { font-size: 0.82rem; font-weight: 500; color: var(--foreground); }
	.key-hint { font-size: 0.68rem; color: var(--text-muted); }
	.key-action { display: flex; align-items: center; gap: 8px; }
	.key-badge { font-size: 0.65rem; padding: 2px 8px; border-radius: 12px; }
	.key-badge-ok { background: oklch(0.72 0.17 142 / 12%); color: oklch(0.72 0.17 142); }
	.key-change { font-size: 0.68rem; color: var(--text-muted); background: none; border: none; cursor: pointer; font-family: var(--font-body); transition: color 0.2s; }
	.key-change:hover { color: var(--foreground); }
	.key-change-add { color: var(--color-warm); }
	.key-change-add:hover { color: oklch(0.88 0.14 75); }
	.key-change-remove { color: oklch(0.55 0.08 25 / 60%); }
	.key-change-remove:hover { color: oklch(0.65 0.15 25); }
	.key-input {
		font-size: 0.72rem;
		font-family: var(--font-mono, monospace);
		padding: 4px 8px;
		border-radius: 6px;
		border: 1px solid var(--border, oklch(1 0 0 / 10%));
		background: var(--surface-input, oklch(1 0 0 / 5%));
		color: var(--foreground);
		outline: none;
		min-width: 160px;
	}
	.key-input:focus { border-color: var(--color-warm, oklch(0.78 0.12 75)); }
	.key-error { margin-top: 8px; font-size: 0.72rem; color: oklch(0.65 0.15 25 / 70%); font-style: italic; }

	/* --- Claude CLI OAuth --- */
	.cli-oauth-section {
		margin-top: 1rem;
		padding: 1rem;
		border-radius: 12px;
		background: var(--surface-elevated);
		border: 1px solid oklch(var(--ink) / 8%);
	}
	.cli-instruction {
		font-size: 0.82rem;
		color: var(--text-muted);
		margin: 0 0 0.75rem;
	}
	.cli-oauth-row {
		display: flex;
		gap: 0.5rem;
		align-items: center;
	}
	.cli-code-input {
		flex: 1;
		background: var(--surface-input);
		border: 1px solid var(--surface-input-border);
		border-radius: 8px;
		padding: 0.5rem 0.75rem;
		font-size: 0.82rem;
		color: var(--foreground);
		font-family: inherit;
	}
	.cli-code-input::placeholder { color: var(--text-muted); }
	.cli-code-input:focus { outline: none; border-color: var(--color-warm); }

	/* ── Server settings ── */
	.setting-row { display: flex; flex-direction: column; gap: 4px; padding: 10px 0; }
	.setting-row + .setting-row { border-top: 1px solid oklch(var(--ink) / 4%); }
	.setting-label { font-size: 0.75rem; font-weight: 500; color: var(--foreground); letter-spacing: 0.03em; }
	.setting-input-row { display: flex; align-items: center; gap: 8px; }
	.setting-input {
		flex: 1; max-width: 200px;
		padding: 6px 10px; border-radius: 6px; border: 1px solid oklch(var(--ink) / 8%);
		background: oklch(var(--ink) / 3%); color: var(--foreground); font-size: 0.8rem;
		font-family: var(--font-mono); outline: none; transition: border-color 0.2s;
	}
	.setting-input:focus { border-color: oklch(0.78 0.12 75 / 30%); }
	.setting-btn {
		padding: 4px 12px; border-radius: 6px; border: 1px solid oklch(0.78 0.12 75 / 15%);
		background: oklch(0.78 0.12 75 / 6%); color: oklch(0.78 0.12 75 / 70%);
		font-size: 0.72rem; font-family: var(--font-mono); cursor: pointer; transition: all 0.2s;
	}
	.setting-btn:hover:not(:disabled) { background: oklch(0.78 0.12 75 / 12%); border-color: oklch(0.78 0.12 75 / 25%); }
	.setting-btn:disabled { opacity: 0.4; cursor: not-allowed; }
	.setting-btn-danger { border-color: oklch(0.65 0.15 25 / 15%); background: oklch(0.65 0.15 25 / 6%); color: oklch(0.65 0.15 25 / 70%); }
	.setting-btn-danger:hover:not(:disabled) { background: oklch(0.65 0.15 25 / 12%); }
	.setting-hint { font-size: 0.65rem; color: oklch(var(--ink) / 20%); margin: 0; }
	.setting-warning { color: oklch(0.78 0.12 75 / 60%); }
	.dim-text { font-size: 0.75rem; color: oklch(var(--ink) / 25%); font-family: var(--font-mono); }
</style>
