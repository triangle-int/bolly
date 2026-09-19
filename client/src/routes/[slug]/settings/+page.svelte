<script lang="ts">
	import Database from "@lucide/svelte/icons/database";
	import CalendarClock from "@lucide/svelte/icons/calendar-clock";
	import AudioLines from "@lucide/svelte/icons/audio-lines";
	import Github from "@lucide/svelte/icons/github";
	import Mail from "@lucide/svelte/icons/mail";

	import Puzzle from "@lucide/svelte/icons/puzzle";
	import Clock from "@lucide/svelte/icons/clock";
	import Brain from "@lucide/svelte/icons/brain";
	import Moon from "@lucide/svelte/icons/moon";
	import Zap from "@lucide/svelte/icons/zap";
	import KeyRound from "@lucide/svelte/icons/key-round";
	import * as Select from "$lib/components/ui/select/index.js";
	import { page } from "$app/state";
	import { embeddingStatusText } from "$lib/embedding-status.js";
	import type { EmbeddingStatus, PairedDevice, PairingCode, AuthKind } from "$lib/api/client.js";
	import {
		fetchPairedDevices,
		createPairingCode,
		revokePairedDevice,
		logoutSession,

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
		fetchConfigStatus,
		updateModelMode,
		exportInstance,
		importInstance,
		reindexMemory,
		fetchScheduledTasks,
		cancelScheduledTask,
		fetchSuggestedMcp,
		updateLlmConfig,
		updateProvider,
		type ScheduledTask,
	} from "$lib/api/client.js";
	import type { McpServerInfo, EmailConfig } from "$lib/api/client.js";
	import { SKINS } from "$lib/stores/skin.svelte.js";

	const slug = $derived(page.params.slug!);

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
			const { fetchServerConfig } = await import("$lib/api/client.js");
			const res = await fetchServerConfig();
			serverHost = res.host;
			serverPort = res.port;
			serverPortInput = String(res.port);
			serverAuthSet = res.auth_token_set;
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

	// Paired browsers (#112)
	let sessionAuth = $state<AuthKind>("disabled");
	let devices = $state<PairedDevice[]>([]);
	let devicesLoading = $state(true);
	let devicesError = $state("");
	let pairingCode = $state<PairingCode | null>(null);
	let pairingBusy = $state(false);
	let pairingSecondsLeft = $state(0);
	let pairingTimer: ReturnType<typeof setInterval> | null = null;
	let revokingId = $state("");

	async function loadDevices() {
		devicesLoading = true;
		devicesError = "";
		try {
			const res = await fetchPairedDevices();
			sessionAuth = res.auth;
			devices = res.devices;
		} catch {
			devicesError = "Could not load paired browsers.";
		} finally {
			devicesLoading = false;
		}
	}

	async function startPairing() {
		pairingBusy = true;
		devicesError = "";
		try {
			pairingCode = await createPairingCode();
			pairingSecondsLeft = pairingCode.expires_in_secs;
			if (pairingTimer) clearInterval(pairingTimer);
			pairingTimer = setInterval(() => {
				pairingSecondsLeft = Math.max(0, pairingSecondsLeft - 1);
				if (pairingSecondsLeft === 0) dismissPairingCode();
			}, 1000);
		} catch {
			devicesError = "Could not create a pairing code.";
		} finally {
			pairingBusy = false;
		}
	}

	function dismissPairingCode() {
		pairingCode = null;
		pairingSecondsLeft = 0;
		if (pairingTimer) {
			clearInterval(pairingTimer);
			pairingTimer = null;
		}
		// A code that was used shows up as a new device.
		loadDevices();
	}

	async function revokeDevice(device: PairedDevice) {
		revokingId = device.id;
		devicesError = "";
		try {
			await revokePairedDevice(device.id);
			if (device.current) {
				location.href = "/";
				return;
			}
			devices = devices.filter((d) => d.id !== device.id);
		} catch {
			devicesError = "Could not revoke that browser.";
		} finally {
			revokingId = "";
		}
	}

	async function signOut() {
		try {
			await logoutSession();
		} finally {
			location.href = "/";
		}
	}

	function timeAgo(unixSeconds: number): string {
		const delta = Math.max(0, Math.floor(Date.now() / 1000) - unixSeconds);
		if (delta < 90) return "just now";
		if (delta < 3600) return `${Math.round(delta / 60)} min ago`;
		if (delta < 86400 * 2) return `${Math.round(delta / 3600)} h ago`;
		return `${Math.round(delta / 86400)} days ago`;
	}

	function pairingCountdown(seconds: number): string {
		const m = Math.floor(seconds / 60);
		const s = seconds % 60;
		return `${m}:${String(s).padStart(2, "0")}`;
	}

	function pairedViaText(device: PairedDevice): string {
		if (device.paired_via === "cli") return "paired from the command line";
		if (device.paired_via.startsWith("browser:")) return "paired from another browser";
		if (device.paired_via === "desktop") return "paired from the desktop app";
		return "paired with the API token";
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

		loadMcpServers();
		loadGithub();
		loadTimezone();
		loadEmail();
		loadVoice();
		loadServer();
		loadDevices();
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
	<h2 class="settings-title">Settings</h2>

	<div class="settings-grid">

	<!-- Server -->
	<section class="settings-section">
		<div class="section-header">
			<div>
				<h3 class="section-label">Server</h3>
				<p class="section-desc">Network and authentication settings.</p>
			</div>
		</div>

		{#if serverLoading}
			<p class="dim-text">Loading...</p>
		{:else}
			<div class="setting-row">
				<label class="setting-label" for="server-port">Port</label>
				<div class="setting-input-row">
					<input
						class="setting-input"
						id="server-port"
						type="number"
						min="1"
						max="65535"
						bind:value={serverPortInput}
					/>
					{#if String(serverPort) !== serverPortInput}
						<button class="setting-btn" onclick={saveServerPort} disabled={serverSaving}>
							{serverSaving ? "..." : "Save"}
						</button>
					{/if}
				</div>
				{#if serverNeedsRestart}
					<p class="setting-hint setting-warning">Restart nolune to apply port change</p>
				{/if}
			</div>

			<div class="setting-row">
				<span class="setting-label">Auth token</span>
				{#if serverAuthSet}
					<div class="setting-input-row">
						<span class="dim-text">Configured</span>
						<button class="setting-btn setting-btn-danger" onclick={clearServerAuth} disabled={serverSaving}>
							Remove
						</button>
					</div>
				{:else}
					<div class="setting-input-row">
						<input
							class="setting-input"
							type="password"
							placeholder="optional — protects your API"
							aria-label="Auth token"
							bind:value={serverAuthInput}
						/>
						{#if serverAuthInput.trim()}
							<button class="setting-btn" onclick={saveServerAuth} disabled={serverSaving}>
								{serverSaving ? "..." : "Set"}
							</button>
						{/if}
					</div>
				{/if}
				<p class="setting-hint">API token for automation, the CLI and the desktop app. Browsers pair for a session instead and keep working when it changes. Leave empty for no authentication.</p>
			</div>

			<div class="setting-row">
				<span class="setting-label">Paired browsers</span>
				{#if devicesLoading}
					<p class="dim-text">Loading...</p>
				{:else if sessionAuth === "disabled"}
					<p class="setting-hint">Set an API token to require browsers to pair.</p>
				{:else}
					{#if devices.length === 0}
						<p class="setting-hint">No browsers are paired yet.</p>
					{:else}
						<ul class="device-list">
							{#each devices as device (device.id)}
								<li class="device-row">
									<div class="device-info">
										<span class="device-label">
											{device.label}
											{#if device.current}<span class="device-current">This browser</span>{/if}
										</span>
										<span class="device-meta">{device.host} · {pairedViaText(device)} · last seen {timeAgo(device.last_seen_at)}</span>
									</div>
									<button
										class="setting-btn setting-btn-danger"
										onclick={() => revokeDevice(device)}
										disabled={revokingId === device.id}
									>
										{device.current ? "Sign out" : "Revoke"}
									</button>
								</li>
							{/each}
						</ul>
					{/if}
					{#if pairingCode}
						<div class="pairing-panel" aria-live="polite">
							<span class="pairing-code">{pairingCode.code}</span>
							<p class="setting-hint">
								Enter this code on the new device within {pairingCountdown(pairingSecondsLeft)}. It works once.
								{#if pairingCode.bound_host}Open Nolune there at the same address, <strong>{pairingCode.bound_host}</strong>.{/if}
							</p>
							<button class="setting-btn" onclick={dismissPairingCode}>Done</button>
						</div>
					{:else}
						<div class="setting-input-row">
							<button class="setting-btn" onclick={startPairing} disabled={pairingBusy}>
								{pairingBusy ? "..." : "Pair another browser"}
							</button>
							{#if sessionAuth === "session" && devices.length === 0}
								<button class="setting-btn setting-btn-danger" onclick={signOut}>Sign out</button>
							{/if}
						</div>
					{/if}
					{#if devicesError}
						<p class="setting-hint setting-warning">{devicesError}</p>
					{/if}
				{/if}
			</div>

			<div class="setting-row">
				<span class="setting-label">Host</span>
				<span class="dim-text">{serverHost}</span>
			</div>
		{/if}
	</section>

	<!-- Skin -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Moon size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Companion</h3>
				<p class="section-desc">Little Moon, your familiar presence across devices.</p>
			</div>
		</div>
		<div class="model-mode-options">
			{#each SKINS as skin (skin.id)}
				<div class="mode-option skin-option mode-active">
					<img src={skin.thumbnail} alt={skin.label} class="skin-thumb" />
					<div>
						<span class="mode-name">{skin.label}</span>
						<span class="mode-desc">Little Moon companion</span>
					</div>
				</div>
			{/each}
		</div>
	</section>

	<!-- Model Mode -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Brain size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Model mode</h3>
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
				<span class="mode-name">Auto</span>
				<span class="mode-desc">Smart routing — cheap for casual, powerful when needed</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={modelMode === "fast"}
				onclick={() => setModelMode("fast")}
				disabled={modelModeSaving}
			>
				<span class="mode-name">Fast</span>
				<span class="mode-desc">Always use the lightweight model — saves budget</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={modelMode === "heavy"}
				onclick={() => setModelMode("heavy")}
				disabled={modelModeSaving}
			>
				<span class="mode-name">Heavy</span>
				<span class="mode-desc">Always use the powerful model — uses 10x more budget</span>
			</button>
		</div>
	</section>

	<!-- Provider -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Zap size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Provider</h3>
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
				<span class="mode-desc">Pay-per-use with your own Anthropic API key</span>
			</button>
			<button
				class="mode-option"
				class:mode-active={provider === "openai"}
				onclick={() => setProvider("openai")}
				disabled={providerSaving}
			>
				<span class="mode-name">OpenAI</span>
				<span class="mode-desc">Pay-per-use with your own OpenAI API key</span>
			</button>
		</div>
	</section>

	<!-- API Keys -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><KeyRound size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">API keys</h3>
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
                                aria-label={`${key.name} key`}
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
							>{keySaving === key.id ? "Saving..." : "Save"}</button>
							<button class="key-change" onclick={() => { keyEditing = ""; keyEditValue = ""; }}>Cancel</button>
						{:else if configured}
							<span class="key-badge key-badge-ok">Connected</span>
							<button
								class="key-change"
								onclick={() => { keyEditing = key.id; keyEditValue = ""; }}
								disabled={keySaving === key.id}
							>Change</button>
							<button
								class="key-change key-change-remove"
								onclick={() => saveKey(key.id, "")}
								disabled={keySaving === key.id}
							>Remove</button>
						{:else}
							<button
								class="key-change key-change-add"
								onclick={() => { keyEditing = key.id; keyEditValue = ""; }}
								disabled={keySaving === key.id}
							>{keySaving === key.id ? "Saving..." : "Add key"}</button>
						{/if}
					</div>
				</div>
			{/each}
		</div>
		{#if keyError}
			<p class="key-error" role="alert">{keyError}</p>
		{/if}
	</section>

	<!-- Timezone -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Clock size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Timezone</h3>
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
                <Select.Root type="single" value={tzValue || "default"} disabled={tzSaving}
                    onValueChange={(value) => saveTimezone(value === "default" ? "" : value)}>
                    <Select.Trigger aria-label="Timezone" class="h-11 w-full min-w-0 border-input bg-card text-foreground dark:bg-card">
                        <span data-slot="select-value">{tzValue ? tzValue.replace(/_/g, " ") : "UTC (default)"}</span>
                    </Select.Trigger>
                    <Select.Content class="max-h-80">
                        <Select.Item value="default" label="UTC (default)" class="min-h-11">UTC (default)</Select.Item>
                        {#each COMMON_TIMEZONES as tz}
                            <Select.Item value={tz} label={tz.replace(/_/g, " ")} class="min-h-11">{tz.replace(/_/g, " ")}</Select.Item>
                        {/each}
                    </Select.Content>
                </Select.Root>
				{#if tzValue}
					<span class="tz-current">{tzValue.replace(/_/g, " ")}</span>
				{/if}
			</div>
		{/if}
	</section>

	<!-- Extensions (MCP Servers) -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Puzzle size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Extensions</h3>
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
									<span class="key-badge key-badge-ok">Connected</span>
									<button class="key-change" disabled={busy} onclick={() => handleRemoveCustom(entry.name)}>
										{busy ? "..." : "Remove"}
									</button>
								{:else if installed}
									<span class="key-badge" style="background: color-mix(in srgb, var(--primary) 12%, transparent); color: var(--primary);">Reconnecting</span>
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
										{busy ? "Connecting..." : "Add"}
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
									style={server.connected ? "" : "background: var(--accent); color: var(--destructive);"}
								>
									{server.connected ? "Connected" : "Disconnected"}
								</span>
								<button class="key-change" disabled={mcpBusy === server.name} onclick={() => handleRemoveCustom(server.name)}>
									{mcpBusy === server.name ? "..." : "Remove"}
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
						<div class="ext-custom-form-title">Add custom server</div>
						<label class="integration-field">Server name<input class="ext-input" type="text" placeholder="name" bind:value={mcpNewName} /></label>
						<label class="integration-field">Server URL<input class="ext-input" type="url" placeholder="server url" bind:value={mcpNewUrl} /></label>
						<div class="ext-form-actions">
							<button class="ext-form-btn ext-form-add" disabled={mcpBusy !== null} onclick={handleAddCustom}>
								{mcpBusy ? "Connecting..." : "Add"}
							</button>
							<button class="ext-form-btn ext-form-cancel" onclick={() => { showCustomForm = false; mcpError = ""; }}>
								Cancel
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
			<p class="key-error" role="alert">{mcpError}</p>
		{/if}
	</section>


	<!-- Email (SMTP/IMAP) -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Mail size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Email</h3>
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
								{emailSaving ? "..." : "Remove"}
							</button>
						</div>
					{/each}
				</div>
			{/if}

			<!-- Add new account form -->
			{#if emailAdding}
				<div class="email-form">
					<div class="email-form-group">
						<span class="email-form-label">Outgoing (SMTP)</span>
						<label class="integration-field">SMTP host<input class="ext-input" type="text" placeholder="smtp host (e.g. smtp.mail.me.com)" bind:value={emailForm.smtp_host} /></label>
						<div class="email-form-row">
							<label class="integration-field">Port<input class="ext-input" type="number" placeholder="port" bind:value={emailForm.smtp_port} style="width: 5rem;" /></label>
							<label class="integration-field">Username or email<input class="ext-input" style="flex:1" type="text" placeholder="username / email" bind:value={emailForm.smtp_user} /></label>
						</div>
						<label class="integration-field">Password or app password<input class="ext-input" type="password" placeholder="password / app-specific password" bind:value={emailForm.smtp_password} /></label>
						<label class="integration-field">From address<input class="ext-input" type="email" placeholder="from address (e.g. user@icloud.com)" bind:value={emailForm.smtp_from} /></label>
					</div>

					<div class="email-form-group">
						<span class="email-form-label">Incoming (IMAP)</span>
						<label class="integration-field">IMAP host<input class="ext-input" type="text" placeholder="imap host (e.g. imap.mail.me.com)" bind:value={emailForm.imap_host} /></label>
						<div class="email-form-row">
							<label class="integration-field">Port<input class="ext-input" type="number" placeholder="port" bind:value={emailForm.imap_port} style="width: 5rem;" /></label>
							<label class="integration-field">Username or email<input class="ext-input" style="flex:1" type="text" placeholder="username / email" bind:value={emailForm.imap_user} /></label>
						</div>
						<label class="integration-field">Password or app password<input class="ext-input" type="password" placeholder="password / app-specific password" bind:value={emailForm.imap_password} /></label>
					</div>

					<div class="ext-form-actions">
						<button
							class="ext-form-btn ext-form-add"
							disabled={emailSaving || (!emailForm.smtp_host && !emailForm.imap_host)}
							onclick={saveNewEmail}
						>
							{emailSaving ? "Saving..." : "Add account"}
						</button>
						<button class="ext-form-btn ext-form-cancel" onclick={() => { emailAdding = false; emailError = ""; emailForm = emptyEmailForm(); }}>
							Cancel
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
			<p class="error-msg" role="alert">{emailError}</p>
		{/if}
	</section>

	<!-- GitHub -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Github size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">GitHub</h3>
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
					<span class="gh-status-text">Token configured</span>
				</div>
				<div class="gh-status-actions">
					<button class="ext-form-btn ext-form-cancel" onclick={() => ghEditing = true}>
						Change
					</button>
					<button
						class="ext-remove-btn"
						disabled={ghSaving}
						onclick={disconnectGithub}
					>
						{ghSaving ? "..." : "Remove"}
					</button>
				</div>
			</div>
		{:else}
			<div class="gh-token-form">
                <label for="github-token" class="setting-label">GitHub access token</label>
				<input
					class="ext-input"
					type="password"
					id="github-token"
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
						{ghSaving ? "Saving..." : "Save token"}
					</button>
					{#if ghEditing}
						<button class="ext-form-btn ext-form-cancel" onclick={() => { ghEditing = false; ghToken = ""; ghError = ""; }}>
							Cancel
						</button>
					{/if}
				</div>
			</div>
		{/if}

		{#if ghError}
			<p class="error-msg" role="alert">{ghError}</p>
		{/if}
	</section>

	<!-- Voice -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><AudioLines size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Voice</h3>
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
                <label for="voice-id" class="setting-label">ElevenLabs voice ID</label>
				<input
					class="ext-input"
					type="text"
					id="voice-id"
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
						{voiceSaving ? "Saving..." : "Save"}
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
				<div class="section-icon" aria-hidden="true"><CalendarClock size={20} strokeWidth={1.75} /></div>
				<div>
					<h3 class="section-label">Scheduled</h3>
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
							{cancellingId === task.id ? "…" : "Cancel"}
						</button>
					</div>
				{/each}
			</div>
		</section>
	{/if}

	<!-- Export / Import -->
	<section class="settings-section">
		<div class="section-header">
			<div class="section-icon" aria-hidden="true"><Database size={20} strokeWidth={1.75} /></div>
			<div>
				<h3 class="section-label">Data</h3>
			</div>
		</div>
		<div class="data-actions">
			<button class="data-btn" onclick={handleExport} disabled={exporting}>
				{#if exporting}
					Exporting… {formatBytes(exportBytes)}
				{:else}
					Export
				{/if}
			</button>
			<button type="button" class="data-btn data-btn-import" onclick={() => importFileInput?.click()} disabled={importing}>
				{#if importing}
					Importing...
				{:else if importDone}
					Imported!
				{:else}
					Import
				{/if}
				</button>
<input
					type="file"
					accept=".tar.gz,.tgz"
					bind:this={importFileInput}
					onchange={handleImport}
					hidden
					disabled={importing}
				/>
		</div>
		<p class="data-hint">Export downloads a .tar.gz of all instance data (soul, memory, drops, chat history). import merges into the current instance.</p>

		<div class="data-actions" style="margin-top: 0.75rem;">
			<button
				class="ext-form-btn"
				disabled={reindexing}
				onclick={handleReindex}
			>
				{reindexing ? 'Reindexing...' : 'Reindex memory'}
			</button>
			<span class="data-hint">Rebuild vector search index for all memories</span>
		</div>
		{#if importError}
			<p class="error-msg" role="alert">{importError}</p>
		{/if}
		{#if exportError}
			<p class="error-msg" role="alert">{exportError}</p>
		{/if}
	</section>

	</div><!-- settings-grid -->
</div>

<style>
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
		font-style: normal;
		font-size: 1.25rem;
		font-weight: 400;
		color: var(--text-secondary);
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
		border: 1px solid var(--border);
		border-top-color: var(--border);
		background: var(--card);
		backdrop-filter: none;
		backdrop-filter: none;
		box-shadow: none;
		overflow: hidden;
	}
	.settings-section::before {
		content: "";
		position: absolute;
		top: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: var(--card);
		pointer-events: none;
	}

	.section-header {
		display: flex;
		align-items: flex-start;
		gap: 0.75rem;
		margin-bottom: 1rem;
	}


	.tz-picker {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}


	.tz-current {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--text-secondary);
		white-space: nowrap;
	}

	.section-label {
		font-family: var(--font-body);
		font-size: 0.8rem;
		color: var(--foreground);
		letter-spacing: 0.02em;
		margin-bottom: 0.2rem;
	}

	.section-desc {
		font-family: var(--font-body);
		font-size: 0.8125rem;
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
		font-size: 0.8125rem;
		color: var(--destructive);
		background: none;
		border: none;
		cursor: pointer;
		padding: 0.25rem 0.5rem;
		border-radius: 0.25rem;
		transition: all 0.2s ease;
	}
	.ext-remove-btn:hover:not(:disabled) {
		color: var(--destructive);
		background: var(--accent);
	}
	.ext-remove-btn:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.ext-input {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--foreground);
		background: var(--popover);
		border: 1px solid var(--border);
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		outline: none;
		transition: border-color 0.2s ease;
	}
	.ext-input:focus {
		border-color: var(--border);
	}
	.ext-input::placeholder {
		color: var(--foreground);
	}

	.ext-form-actions {
		display: flex;
		gap: 0.5rem;
	}

	.ext-form-btn {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		padding: 0.4rem 0.85rem;
		border-radius: 0.375rem;
		cursor: pointer;
		transition: all 0.2s ease;
		letter-spacing: 0.02em;
	}

	.ext-form-add {
		color: var(--primary);
		background: var(--accent);
		border: 1px solid var(--border);
	}
	.ext-form-add:hover:not(:disabled) {
		background: var(--accent);
		border-color: var(--border);
	}
	.ext-form-add:disabled {
		opacity: 0.4;
		cursor: not-allowed;
	}

	.ext-form-cancel {
		color: var(--text-secondary);
		background: none;
		border: 1px solid var(--border);
	}
	.ext-form-cancel:hover {
		color: var(--text-secondary);
		background: var(--popover);
	}


	.loading-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--accent);
		animation: pulse 1.5s ease-in-out infinite;
	}
	@keyframes pulse {
		0%, 100% { opacity: 1; transform: scale(1); }
		50% { opacity: 0.3; transform: scale(0.7); }
	}

	.error-msg {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--destructive);
		font-style: normal;
		margin-top: 0.5rem;
	}

	/* --- github --- */

	.gh-status {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		background: var(--popover);
		border: 1px solid var(--border);
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
		background: var(--accent);
	}

	.gh-status-text {
		font-family: var(--font-body);
		font-size: 0.8125rem;
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
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
		text-transform: none;
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
		font-family: var(--font-body);
		font-size: 0.8125rem;
		padding: 0.4rem 0.9rem;
		border-radius: 0.375rem;
		cursor: pointer;
		transition: all 0.2s ease;
		color: var(--primary);
		background: var(--accent);
		border: 1px solid var(--border);
	}
	.data-btn:hover {
		background: var(--accent);
		border-color: var(--border);
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
		background: var(--popover);
		border: 1px solid var(--border);
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
		font-size: 0.8125rem;
		color: var(--text-secondary);
		line-height: 1.35;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.sched-time {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}
	.sched-cancel {
		flex-shrink: 0;
		font-family: var(--font-body);
		font-size: 0.8125rem;
		padding: 0.2rem 0.5rem;
		border-radius: 0.3rem;
		background: none;
		border: 1px solid var(--destructive);
		color: var(--destructive);
		cursor: pointer;
		transition: all 0.2s ease;
	}
	.sched-cancel:hover:not(:disabled) {
		border-color: var(--destructive);
		color: var(--destructive);
		background: var(--accent);
	}
	.sched-cancel:disabled {
		opacity: 0.4;
		cursor: default;
	}

	.data-hint {
		font-family: var(--font-body);
		font-size: 0.8125rem;
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
		background: var(--popover);
		border: 1px solid var(--border);
		cursor: pointer;
		text-align: left;
		transition: all 0.15s;
	}
	.mode-option:hover:not(:disabled) {
		background: var(--popover);
		border-color: var(--border);
	}
	.mode-active {
		background: var(--accent);
		border-color: var(--border);
	}
	.mode-active:hover:not(:disabled) {
		background: var(--accent);
	}
	.mode-name {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--foreground);
	}
	.mode-active .mode-name {
		color: var(--primary);
	}
	.mode-desc {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--foreground);
	}

	/* ── API Keys ── */
	.keys-list { display: flex; flex-direction: column; gap: 1px; background: var(--popover); border-radius: 12px; overflow: hidden; }
	.key-row { display: flex; align-items: center; justify-content: space-between; padding: 12px 16px; background: var(--surface-elevated); }
	.key-info { display: flex; flex-direction: column; gap: 2px; }
	.key-name { font-size: 0.82rem; font-weight: 500; color: var(--foreground); }
	.key-hint { font-size: 0.8125rem; color: var(--text-muted); }
	.key-action { display: flex; align-items: center; gap: 8px; }
	.key-badge { font-size: 0.8125rem; padding: 2px 8px; border-radius: 12px; }
	.key-badge-ok { background: var(--accent); color: var(--primary); }
	.key-change { font-size: 0.8125rem; color: var(--text-muted); background: none; border: none; cursor: pointer; font-family: var(--font-body); transition: color 0.2s; }
	.key-change:hover { color: var(--foreground); }
	.key-change-add { color: var(--color-warm); }
	.key-change-add:hover { color: var(--primary); }
	.key-change-remove { color: var(--destructive); }
	.key-change-remove:hover { color: var(--destructive); }
	.key-input {
		font-size: 0.8125rem;
		font-family: var(--font-mono, monospace);
		padding: 4px 8px;
		border-radius: 6px;
		border: 1px solid var(--border, var(--border));
		background: var(--surface-input, var(--popover));
		color: var(--foreground);
		outline: none;
		min-width: 160px;
	}
	.key-input:focus { border-color: var(--color-warm, var(--primary)); }
	.key-error { margin-top: 8px; font-size: 0.8125rem; color: var(--destructive); font-style: normal; }

	/* ── Server settings ── */
	.setting-row { display: flex; flex-direction: column; gap: 4px; padding: 10px 0; }
	.setting-row + .setting-row { border-top: 1px solid var(--border); }
	.setting-label { font-size: 0.8125rem; font-weight: 500; color: var(--foreground); letter-spacing: 0.03em; }
	.setting-input-row { display: flex; align-items: center; gap: 8px; }
	.setting-input {
		flex: 1; max-width: 200px;
		padding: 6px 10px; border-radius: 6px; border: 1px solid var(--border);
		background: var(--popover); color: var(--foreground); font-size: 0.8rem;
		font-family: var(--font-body); outline: none; transition: border-color 0.2s;
	}
	.setting-input:focus { border-color: var(--ring); }
	.setting-btn {
		min-height: 44px;
		padding: 4px 12px; border-radius: 6px; border: 1px solid var(--border);
		background: var(--accent); color: var(--primary);
		font-size: 0.8125rem; font-family: var(--font-body); cursor: pointer; transition: all 0.2s;
	}
	.setting-btn:hover:not(:disabled) { background: var(--accent); border-color: var(--border); }
	.setting-btn:disabled { opacity: 0.4; cursor: not-allowed; }
	.setting-btn-danger { border-color: var(--destructive); background: var(--accent); color: var(--destructive); }
	.setting-btn-danger:hover:not(:disabled) { background: var(--accent); }
	.setting-hint { font-size: 0.8125rem; color: var(--text-muted); margin: 0; }
	.setting-warning { color: var(--primary); }
	.device-list { display: flex; flex-direction: column; gap: 8px; margin: 4px 0 8px; padding: 0; list-style: none; }
	.device-row { display: flex; align-items: center; justify-content: space-between; gap: 12px; padding: 10px 12px; border-radius: 8px; border: 1px solid var(--border); background: var(--background); flex-wrap: wrap; }
	.device-info { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
	.device-label { display: flex; align-items: center; gap: 8px; font-size: 0.875rem; color: var(--foreground); }
	.device-current { font-size: 0.6875rem; letter-spacing: 0.04em; text-transform: uppercase; padding: 2px 6px; border-radius: 999px; background: var(--accent); color: var(--primary); }
	.device-meta { font-size: 0.75rem; color: var(--text-muted); overflow-wrap: anywhere; }
	.pairing-panel { display: flex; flex-direction: column; align-items: flex-start; gap: 8px; padding: 12px; border-radius: 8px; border: 1px solid var(--primary); background: var(--accent); }
	.pairing-code { font-family: var(--font-mono, monospace); font-size: 1.75rem; letter-spacing: 0.14em; color: var(--foreground); }
	.pairing-panel strong { color: var(--foreground); font-weight: 500; }
	.dim-text { font-size: 0.8125rem; color: var(--text-secondary); font-family: var(--font-body); }

    .settings-title { font-size: 2rem; color: var(--foreground); }
    .settings-section { padding: 24px; border-color: var(--border); min-width: 0; }
    .settings-section::before { display: none; }
    .section-label { font-size: 1.125rem; font-weight: 500; }
    .section-desc, .mode-desc, .data-hint { color: var(--text-secondary); line-height: 1.6; }
    .section-header > div, .key-info { min-width: 0; overflow-wrap: anywhere; }
    .ext-input, .key-input, .setting-input { min-height: 44px; min-width: 0; font-size: 1rem; background: var(--background); border-color: var(--input); }
    .ext-input:focus, .key-input:focus, .setting-input:focus { border-color: var(--ring); }
    .ext-input::placeholder, .key-input::placeholder { color: var(--text-muted); opacity: 1; }
    .ext-form-btn, .data-btn, .setting-btn, .ext-remove-btn, .key-change, .sched-cancel { min-height: 44px; border-radius: 8px; font-size: 0.875rem; }
    .ext-form-add, .setting-btn { background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); }
    .ext-form-add:hover:not(:disabled), .setting-btn:hover:not(:disabled) { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); border-color: var(--primary); }
    .setting-btn-danger, .setting-btn-danger:hover:not(:disabled) { background: var(--card); color: var(--destructive); border-color: var(--destructive); }
    .ext-form-cancel, .data-btn { background: var(--popover); border-color: var(--border); color: var(--foreground); }
    .data-btn:hover, .ext-form-cancel:hover { background: var(--accent); color: var(--foreground); }
    .mode-option { min-height: 64px; padding: 12px 16px; background: var(--background); }
    .mode-active, .mode-active:hover:not(:disabled) { background: var(--accent); border-color: var(--primary); }
    .mode-active .mode-name { color: var(--primary); }
    .mode-name { font-size: 0.875rem; }
    .setting-warning, .loading-dot { color: var(--primary); }
    .loading-dot { background: var(--primary); }
    .setting-input-row, .key-row, .key-action, .gh-status, .email-form-row, .data-actions { flex-wrap: wrap; }
    .key-action { max-width: 100%; }
    .tz-picker { flex-direction: column; align-items: stretch; }
    .setting-input { max-width: none; }
    @media (max-width: 600px) {
        .settings-page { padding: 24px 20px; }
        .settings-section { padding: 16px; }
        .key-row { padding: 12px; gap: 8px; }
    }

    .integration-field { display: flex; flex: 1 1 120px; min-width: 0; flex-direction: column; gap: 8px; color: var(--text-secondary); font-size: 14px; }
    .integration-field input { width: 100%; }
    .data-btn { display: inline-flex; align-items: center; justify-content: center; }
    .section-icon { display: flex; width: 40px; height: 40px; flex: 0 0 40px; align-items: center; justify-content: center; color: var(--primary); background: var(--accent); border: 1px solid var(--border); border-radius: 8px; }
</style>
