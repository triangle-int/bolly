<script lang="ts">
	import { fetchAgents, triggerAgent, updateAgent, resetAgent, fetchAgentHistory, fetchAgentRuns, fetchAgentRun } from "$lib/api/client.js";
	import type { ChildAgent, AgentHistoryEntry, AgentRunSummary, AgentRun } from "$lib/api/types.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { goto } from "$app/navigation";

	const toast = getToasts();
	let { slug }: { slug: string } = $props();

	let agents = $state<ChildAgent[]>([]);
	let loading = $state(true);
	let loadError = $state("");
	let triggering = $state<string | null>(null);

	// Config editing
	let editingAgent = $state<string | null>(null);
	let saving = $state(false);

	// History panel
	let selectedAgent = $state<string | null>(null);
	let history = $state<AgentHistoryEntry[]>([]);
	let historyLoading = $state(false);
	let historyError = $state("");

	// Activity panel
	let runs = $state<AgentRunSummary[]>([]);
	let runsLoading = $state(true);
	let runsError = $state("");
	let selectedRun = $state<AgentRun | null>(null);
	let runLoading = $state(false);
	let expandedRunId = $state<string | null>(null);

	async function load() {
		loading = true;
		loadError = "";
		try {
			agents = await fetchAgents(slug);
		} catch {
			loadError = "Could not load agents. Please try again.";
			toast.error("failed to load agents");
		} finally {
			loading = false;
		}
	}

	async function loadRuns() {
		runsLoading = true;
		runsError = "";
		try {
			runs = await fetchAgentRuns(slug, 30);
		} catch {
			runsError = "Could not load recent activity.";
		} finally {
			runsLoading = false;
		}
	}

	$effect(() => {
		load();
		loadRuns();
	});

	async function handleTrigger(name: string) {
		triggering = name;
		try {
			await triggerAgent(slug, name);
			toast.success(`${name} triggered`);
			// Refresh after a delay to pick up new last_run
			setTimeout(() => load(), 3000);
		} catch {
			toast.error(`failed to trigger ${name}`);
		} finally {
			triggering = null;
		}
	}

	async function showHistory(name: string) {
		if (selectedAgent === name) {
			selectedAgent = null;
			return;
		}
		selectedAgent = name;
		historyLoading = true;
		historyError = "";
		try {
			history = await fetchAgentHistory(slug, name);
		} catch {
			historyError = "Could not load this agent’s history. Close and reopen to try again.";
		} finally {
			historyLoading = false;
		}
	}

	const ALL_TOOL_GROUPS = ["memory", "creative", "communication", "files", "commands", "email", "computer", "media"];
	const BUILTIN_NAMES = ["companion", "reflection", "night-maintenance", "explore-code", "deep-research"];
	const MODEL_OPTIONS = [
		{ value: "default", label: "default" },
		{ value: "heavy", label: "opus" },
		{ value: "fast", label: "sonnet" },
		{ value: "cheap", label: "haiku" },
	];

	function isBuiltin(name: string): boolean {
		return BUILTIN_NAMES.includes(name);
	}

	async function toggleEnabled(agent: ChildAgent) {
		try {
			await updateAgent(slug, agent.name, { enabled: !agent.enabled });
			agent.enabled = !agent.enabled;
			agents = [...agents]; // trigger reactivity
			toast.success(`${agent.name} ${agent.enabled ? "enabled" : "disabled"}`);
		} catch {
			toast.error("failed to update agent");
		}
	}

	async function saveAgentField(agentName: string, field: string, value: unknown) {
		saving = true;
		try {
			const updated = await updateAgent(slug, agentName, { [field]: value });
			agents = agents.map(a => a.name === agentName ? { ...a, ...updated } : a);
			toast.success("saved");
		} catch {
			toast.error("failed to save");
		} finally {
			saving = false;
		}
	}

	async function handleReset(agentName: string) {
		try {
			const updated = await resetAgent(slug, agentName);
			agents = agents.map(a => a.name === agentName ? { ...a, ...updated } : a);
			toast.success(`${agentName} reset to defaults`);
		} catch (e) {
			toast.error("failed to reset");
		}
	}

	function toggleToolGroup(agent: ChildAgent, group: string) {
		const groups = agent.tool_groups ?? [];
		const next = groups.includes(group)
			? groups.filter(g => g !== group)
			: [...groups, group];
		saveAgentField(agent.name, "tool_groups", next);
	}

	function addAgent() {
		// Navigate to chat with a pre-filled message asking the main agent to create a child agent
		const message = encodeURIComponent(
			"I want to create a new child agent. Help me define what it should do, how often it should run, and write the TOML config for it."
		);
		goto(`/${slug}/chat?draft=${message}`);
	}

	function formatInterval(hours: number): string {
		if (hours === 0) return "on-demand";
		if (hours < 1) return `${Math.round(hours * 60)}m`;
		if (hours < 24) return `${hours}h`;
		if (hours === 24) return "daily";
		if (hours === 72) return "3 days";
		return `${Math.round(hours / 24)}d`;
	}

	function formatLastRun(ts: number): string {
		if (ts === 0) return "never";
		const diff = Date.now() / 1000 - ts;
		const mins = Math.floor(diff / 60);
		const hours = Math.floor(diff / 3600);
		const days = Math.floor(diff / 86400);
		if (mins < 1) return "just now";
		if (mins < 60) return `${mins}m ago`;
		if (hours < 24) return `${hours}h ago`;
		return `${days}d ago`;
	}

	function modelLabel(model: string): string {
		switch (model) {
			case "heavy": return "opus";
			case "fast": return "sonnet";
			case "cheap": return "haiku";
			default: return "default";
		}
	}

	const modelColors: Record<string, string> = {
		heavy: "var(--primary)",
		fast: "var(--primary)",
		cheap: "var(--primary)",
		default: "var(--primary)",
	};

	const kindColors: Record<string, string> = {
		scheduled: "var(--primary)",
		on_demand: "var(--primary)",
	};

	function formatDuration(ms: number): string {
		if (ms < 1000) return `${ms}ms`;
		const secs = ms / 1000;
		if (secs < 60) return `${secs.toFixed(1)}s`;
		const mins = Math.floor(secs / 60);
		const remainSecs = Math.round(secs % 60);
		return `${mins}m${remainSecs}s`;
	}

	function formatTokens(n: number): string {
		if (n < 1000) return String(n);
		if (n < 10000) return `${(n / 1000).toFixed(1)}k`;
		return `${Math.round(n / 1000)}k`;
	}

	function formatTimestamp(epochMs: number): string {
		if (epochMs === 0) return "never";
		const diff = (Date.now() - epochMs) / 1000;
		const mins = Math.floor(diff / 60);
		const hours = Math.floor(diff / 3600);
		const days = Math.floor(diff / 86400);
		if (mins < 1) return "just now";
		if (mins < 60) return `${mins}m ago`;
		if (hours < 24) return `${hours}h ago`;
		return `${days}d ago`;
	}

	function runStatusLabel(status: AgentRunSummary['status']): string {
		if (status === 'completed') return 'completed';
		return 'failed';
	}

	function runStatusFailed(status: AgentRunSummary['status']): boolean {
		return status !== 'completed';
	}

	function runErrorMessage(status: AgentRunSummary['status']): string {
		if (status === 'completed') return '';
		if (typeof status === 'object' && 'failed' in status) return status.failed.error;
		return 'unknown error';
	}

	async function toggleRunTrace(run: AgentRunSummary) {
		if (expandedRunId === run.id) {
			expandedRunId = null;
			selectedRun = null;
			return;
		}
		expandedRunId = run.id;
		selectedRun = null;
		runLoading = true;
		try {
			selectedRun = await fetchAgentRun(slug, run.id);
		} catch {
			toast.error("failed to load run trace");
			expandedRunId = null;
		} finally {
			runLoading = false;
		}
	}

	function traceRole(msg: any): string {
		return msg?.role ?? 'unknown';
	}

	function traceContent(msg: any): string {
		if (typeof msg?.content === 'string') return msg.content;
		if (Array.isArray(msg?.content)) {
			return msg.content
				.filter((b: any) => b.type === 'text')
				.map((b: any) => b.text)
				.join('\n');
		}
		return '';
	}

	function traceToolUses(msg: any): { name: string; input: string }[] {
		if (!Array.isArray(msg?.content)) return [];
		return msg.content
			.filter((b: any) => b.type === 'tool_use')
			.map((b: any) => ({
				name: b.name ?? 'tool',
				input: typeof b.input === 'string' ? b.input : JSON.stringify(b.input ?? {}).slice(0, 300),
			}));
	}

	function traceToolResults(msg: any): { content: string }[] {
		if (!Array.isArray(msg?.content)) return [];
		return msg.content
			.filter((b: any) => b.type === 'tool_result')
			.map((b: any) => {
				let text = '';
				if (typeof b.content === 'string') text = b.content;
				else if (Array.isArray(b.content)) {
					text = b.content
						.filter((c: any) => c.type === 'text')
						.map((c: any) => c.text)
						.join('\n');
				}
				return { content: text.slice(0, 500) + (text.length > 500 ? '...' : '') };
			});
	}
</script>

<div class="agents-page">
	{#if loading}
		<span class="sr-only" role="status">Loading agents…</span>
		<div class="agents-center">
			<div class="pulse-dot"></div>
		</div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else}
		<div class="agents-header">
			<div class="agents-title">
				<span class="agents-count">{agents.length}</span>
				Agents
			</div>
			<button class="agents-add" onclick={addAgent}>
				+ New agent
			</button>
		</div>

		<p class="agents-hint">
			built-in agents run on their own schedule — manual trigger is for custom agents that don't need a timer
		</p>

		{#if agents.length === 0}
			<div class="agents-center">
				<p class="empty-text">No agents yet</p>
				<p class="empty-sub">Agents are autonomous helpers that wake up on their own schedule.</p>
			</div>
		{:else}
			<div class="agents-list">
				{#each agents as agent (agent.name)}
					{@const color = modelColors[agent.model] ?? "var(--primary)"}
					{@const isRunning = triggering === agent.name}
					{@const isExpanded = selectedAgent === agent.name}

					<div class="agent-card" class:agent-disabled={!agent.enabled}>
						<!-- Status indicator -->
						<div class="agent-status" style="background: {color};" class:agent-status-due={agent.is_due}></div>

						<!-- Main info -->
						<div class="agent-main">
							<div class="agent-top">
								<span class="agent-name">{agent.name}</span>
								<span class="agent-model" style="color: {color};">{modelLabel(agent.model)}</span>
								<span class="agent-interval">{agent.interval_hours === 0 ? 'on-demand' : `every ${formatInterval(agent.interval_hours)}`}</span>
							</div>

							<p class="agent-desc">{agent.description}</p>

							<div class="agent-meta">
								<span class="agent-last-run">
									{#if agent.last_run === 0}
										never ran
									{:else}
										last ran {formatLastRun(agent.last_run)}
									{/if}
								</span>
								{#if agent.is_due}
									<span class="agent-due-badge">Due</span>
								{/if}
								{#if !agent.enabled}
									<span class="agent-paused-badge">Paused</span>
								{/if}
								{#if agent.modified_fields?.length > 0}
									<span class="agent-modified-badge" title="Modified: {agent.modified_fields.join(', ')}">Modified</span>
								{/if}
							</div>
						</div>

						<!-- Actions -->
						<div class="agent-actions">
							<button
								class="agent-btn"
								onclick={() => toggleEnabled(agent)}
								aria-label={agent.enabled ? `Disable ${agent.name}` : `Enable ${agent.name}`} title={agent.enabled ? "Disable" : "Enable"}
							>
								{#if agent.enabled}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z"/><circle cx="12" cy="12" r="3"/></svg>
								{:else}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M17.94 17.94A10.07 10.07 0 0112 20c-7 0-11-8-11-8a18.45 18.45 0 015.06-5.94"/><line x1="1" y1="1" x2="23" y2="23"/></svg>
								{/if}
							</button>
							<button
								class="agent-btn agent-btn-run"
								onclick={() => handleTrigger(agent.name)}
								disabled={isRunning || !agent.enabled}
								aria-label={`Run now: ${agent.name}`} title="Run now"
							>
								{#if isRunning}
									<span class="agent-spinner"></span>
								{:else}
									<svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
								{/if}
							</button>
							<button
								class="agent-btn"
								onclick={() => editingAgent = editingAgent === agent.name ? null : agent.name}
								class:agent-btn-active={editingAgent === agent.name}
								aria-label={`Configure: ${agent.name}`} title="Configure"
							>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/></svg>
							</button>
							<button
								class="agent-btn agent-btn-history"
								onclick={() => showHistory(agent.name)}
								class:agent-btn-active={isExpanded}
								aria-label={`View history: ${agent.name}`} title="View history"
							>
								<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
							</button>
						</div>
					</div>

					<!-- Config panel -->
					{#if editingAgent === agent.name}
						<div class="agent-config">
							<div class="config-row">
								<label class="config-label" for={`interval-${agent.name}`}>Interval</label>
								<div class="config-field">
									<input id={`interval-${agent.name}`} class="config-input config-input-sm" type="number" min="0" step="0.25"
										value={agent.interval_hours}
										onchange={(e) => saveAgentField(agent.name, "interval_hours", parseFloat((e.target as HTMLInputElement).value))}
									/>
									<span class="config-unit">hours</span>
									<span class="config-hint">{agent.interval_hours === 0 ? "on-demand" : agent.interval_hours < 1 ? `= ${Math.round(agent.interval_hours * 60)}m` : `= ${agent.interval_hours}h`}</span>
								</div>
							</div>
							<div class="config-row">
								<span class="config-label">Model</span>
								<div class="config-field">
									{#each MODEL_OPTIONS as opt (opt.value)}
										<button class="config-chip" class:config-chip-active={agent.model === opt.value} aria-pressed={agent.model === opt.value}
											onclick={() => saveAgentField(agent.name, "model", opt.value)}
										>{opt.label}</button>
									{/each}
								</div>
							</div>
							<div class="config-row">
								<span class="config-label">Tools</span>
								<div class="config-field config-field-wrap">
									{#each ALL_TOOL_GROUPS as group (group)}
										<button class="config-chip" class:config-chip-active={(agent.tool_groups ?? []).includes(group)} aria-pressed={(agent.tool_groups ?? []).includes(group)}
											onclick={() => toggleToolGroup(agent, group)}
										>{group}</button>
									{/each}
								</div>
							</div>
							<div class="config-row config-row-full">
								<label class="config-label" for={`prompt-${agent.name}`}>Prompt</label>
								<textarea id={`prompt-${agent.name}`} class="config-textarea" value={agent.prompt} rows="4"
									onchange={(e) => saveAgentField(agent.name, "prompt", (e.target as HTMLTextAreaElement).value)}
								></textarea>
							</div>
							{#if isBuiltin(agent.name)}
								<div class="config-row config-row-actions">
									<button class="config-reset" onclick={() => handleReset(agent.name)}>Reset to defaults</button>
								</div>
							{/if}
						</div>
					{/if}

					<!-- History panel -->
					{#if isExpanded}
						<div class="agent-history">
							{#if historyLoading}
								<div class="history-loading"><div class="pulse-dot"></div></div>
							{:else if historyError}
								<p class="history-empty" role="alert">{historyError}</p>
							{:else if history.length === 0}
								<p class="history-empty">No history yet</p>
							{:else}
								{#each history as entry (entry.id)}
									<div class="history-entry">
										<span class="history-time">{formatLastRun(parseInt(entry.timestamp) / 1000)}</span>
										<p class="history-content">{entry.content.slice(0, 300)}{entry.content.length > 300 ? "..." : ""}</p>
									</div>
								{/each}
							{/if}
						</div>
					{/if}
				{/each}
			</div>
		{/if}

		<!-- Recent activity section -->
		<div class="activity-section">
			<div class="activity-header">
				<div class="agents-title">
					<span class="agents-count">{runs.length}</span>
					Recent activity
				</div>
			</div>

			{#if runsLoading}
				<div class="activity-loading"><div class="pulse-dot"></div></div>
			{:else if runsError}
				<div class="load-error" role="alert"><p>{runsError}</p><button class="nl-button-secondary" onclick={loadRuns}>Try again</button></div>
			{:else if runs.length === 0}
				<div class="activity-empty">
					<p class="empty-text">No recent runs</p>
				</div>
			{:else}
				<div class="runs-list">
					{#each runs as run (run.id)}
						{@const color = kindColors[run.agent_kind] ?? kindColors.scheduled}
						{@const isFailed = runStatusFailed(run.status)}
						{@const isExpanded = expandedRunId === run.id}

						<button
							class="run-card"
							class:run-card-expanded={isExpanded}
							class:run-card-failed={isFailed}
							onclick={() => toggleRunTrace(run)}
						>
							<div class="run-dot" style="background: {isFailed ? 'var(--destructive)' : color};"></div>

							<div class="run-main">
								<div class="run-top">
									<span class="run-agent" style="color: {color};">{run.agent_name}</span>
									<span class="run-trigger">{run.trigger}</span>
								</div>
								<div class="run-meta">
									<span class="run-time">{formatTimestamp(run.started_at)}</span>
									<span class="run-sep">&middot;</span>
									<span class="run-duration">{formatDuration(run.duration_ms)}</span>
									<span class="run-sep">&middot;</span>
									<span class="run-tokens">{formatTokens(run.tokens_used)} tok</span>
									<span class="run-sep">&middot;</span>
									<span class="run-status" class:run-status-fail={isFailed}>
										{runStatusLabel(run.status)}
									</span>
								</div>
								{#if run.summary}
									<p class="run-summary">{run.summary}</p>
								{/if}
							</div>

							<div class="run-chevron" class:run-chevron-open={isExpanded}>
								<svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9"/></svg>
							</div>
						</button>

						<!-- Expanded trace -->
						{#if isExpanded}
							<div class="run-trace">
								{#if runLoading}
									<div class="activity-loading"><div class="pulse-dot"></div></div>
								{:else if selectedRun && selectedRun.trace.length > 0}
									<div class="trace-scroll">
										{#each selectedRun.trace as msg, i (i)}
											{@const role = traceRole(msg)}
											{@const text = traceContent(msg)}
											{@const toolUses = traceToolUses(msg)}
											{@const toolResults = traceToolResults(msg)}

											<div class="trace-msg" class:trace-msg-user={role === 'user'} class:trace-msg-assistant={role === 'assistant'}>
												<span class="trace-role">{role}</span>

												{#if text}
													<p class="trace-text">{text.slice(0, 800)}{text.length > 800 ? '...' : ''}</p>
												{/if}

												{#each toolUses as tu, ti (ti)}
													<div class="trace-tool-use">
														<span class="trace-tool-name">{tu.name}</span>
														<pre class="trace-tool-input">{tu.input}</pre>
													</div>
												{/each}

												{#each toolResults as tr, tri (tri)}
													<div class="trace-tool-result">
														<pre class="trace-tool-output">{tr.content}</pre>
													</div>
												{/each}
											</div>
										{/each}
									</div>
								{:else if selectedRun}
									<p class="history-empty">No messages in this trace</p>
								{/if}

								{#if selectedRun && runStatusFailed(selectedRun.status)}
									<div class="trace-error">
										{runErrorMessage(selectedRun.status)}
									</div>
								{/if}
							</div>
						{/if}
					{/each}
				</div>
			{/if}
		</div>
	{/if}
</div>

<style>
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.agents-page {
		height: 100%;
		overflow-y: auto;
		padding: 2rem 1.5rem;
	}

	.agents-center {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		height: 100%;
		gap: 0.75rem;
	}

	.pulse-dot {
		width: 6px; height: 6px; border-radius: 50%;
		background: var(--card);
		animation:none;
	}
	@keyframes pulse { 0%, 100% { opacity: 1; transform: scale(1); } 50% { opacity: 0.3; transform: scale(0.7); } }

	.empty-text {
		font-family: var(--font-display);
		font-style: normal;
		font-size: 0.9rem;
		color: var(--text-secondary);
	}
	.empty-sub {
		font-size: 0.75rem;
		color: var(--text-secondary);
		max-width: 30ch;
		text-align: center;
		line-height: 1.5;
	}

	/* Header */
	.agents-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 1.25rem;
		max-width: 600px;
		margin-left: auto;
		margin-right: auto;
	}

	.agents-title {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.agents-count {
		color: var(--text-secondary);
		margin-right: 0.25rem;
	}

	.agents-hint {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		max-width: 600px;
		margin: -0.5rem auto 1rem;
		letter-spacing: 0.02em;
		line-height: 1.5;
	}

	.agents-add {
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
	.agents-add:hover {
		color: var(--text-secondary);
		background: var(--card);
		border-color: var(--border);
	}

	/* Agent list */
	.agents-list {
		max-width: 600px;
		margin: 0 auto;
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	/* Agent card */
	.agent-card {
		display: flex;
		align-items: flex-start;
		gap: 0.75rem;
		padding: 0.875rem 1rem;
		border-radius: 0.625rem;
		background: var(--card);
		border: 1px solid var(--border);
		transition: all 0.2s ease;
	}
	.agent-card:hover {
		background: var(--card);
		border-color: var(--border);
	}

	.agent-disabled {
		opacity: 0.45;
	}

	.agent-status {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		margin-top: 0.45rem;
		flex-shrink: 0;
		opacity: 0.4;
		transition: opacity 0.3s ease;
	}
	.agent-status-due {
		opacity: 1;
		box-shadow: none;
	}

	.agent-main {
		flex: 1;
		min-width: 0;
	}

	.agent-top {
		display: flex;
		align-items: baseline;
		gap: 0.5rem;
		flex-wrap: wrap;
		margin-bottom: 0.25rem;
	}

	.agent-name {
		font-family: var(--font-body);
		font-size: 0.82rem;
		color: var(--text-secondary);
		letter-spacing: 0.02em;
	}

	.agent-model {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.05em;
		opacity: 0.6;
		text-transform: none;
	}

	.agent-interval {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.agent-desc {
		font-size: 0.75rem;
		color: var(--text-secondary);
		line-height: 1.5;
		margin: 0;
	}

	.agent-meta {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		margin-top: 0.375rem;
	}

	.agent-last-run {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.agent-due-badge {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.06em;
		padding: 0.1rem 0.35rem;
		border-radius: 0.5rem;
		background: var(--card);
		color: var(--text-secondary);
	}

	.agent-paused-badge {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.06em;
		padding: 0.1rem 0.35rem;
		border-radius: 0.5rem;
		background: var(--card);
		color: var(--text-secondary);
	}

	.agent-modified-badge {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.06em;
		padding: 0.1rem 0.35rem;
		border-radius: 0.5rem;
		background: var(--card);
		color: var(--text-secondary);
		cursor: help;
	}

	/* Actions */
	.agent-actions {
		display: flex;
		gap: 0.25rem;
		flex-shrink: 0;
		margin-top: 0.125rem;
	}

	.agent-btn {
		width: 28px;
		height: 28px;
		display: flex;
		align-items: center;
		justify-content: center;
		border: 1px solid var(--border);
		border-radius: 0.375rem;
		background: none;
		color: var(--text-secondary);
		cursor: pointer;
		transition: all 0.2s ease;
	}
	.agent-btn:hover:not(:disabled) {
		color: var(--text-secondary);
		border-color: var(--border);
		background: var(--card);
	}
	.agent-btn:disabled {
		opacity: 0.3;
		cursor: not-allowed;
	}
	.agent-btn-active {
		color: var(--text-secondary);
		border-color: var(--border);
		background: var(--card);
	}

	.agent-btn-run:hover:not(:disabled) {
		color: var(--text-secondary);
		border-color: var(--border);
	}

	.agent-spinner {
		width: 10px; height: 10px; border-radius: 50%;
		border: 1.5px solid var(--border);
		border-top-color: var(--border);
		animation: none;
	}
	@keyframes spin { to { transform: rotate(360deg); } }

	/* History panel */
	.agent-history {
		margin-left: 1.5rem;
		padding: 0.75rem 1rem;
		border-left: 2px solid var(--border);
		display: flex;
		flex-direction: column;
		gap: 0.625rem;
	}

	.history-loading {
		display: flex;
		justify-content: center;
		padding: 0.5rem;
	}

	.history-empty {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		margin: 0;
	}

	.history-entry {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
	}

	.history-time {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.history-content {
		font-size: 0.75rem;
		color: var(--text-secondary);
		line-height: 1.5;
		margin: 0;
		white-space: pre-line;
	}

	/* Activity section */
	.activity-section {
		max-width: 600px;
		margin: 2rem auto 0;
	}

	.activity-header {
		margin-bottom: 1rem;
	}

	.activity-loading {
		display: flex;
		justify-content: center;
		padding: 1rem 0;
	}

	.activity-empty {
		text-align: center;
		padding: 1rem 0;
	}

	/* Run cards */
	.runs-list {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
	}

	.run-card {
		display: flex;
		align-items: flex-start;
		gap: 0.625rem;
		padding: 0.625rem 0.875rem;
		border-radius: 0.5rem;
		background: var(--card);
		border: 1px solid var(--border);
		transition: all 0.2s ease;
		cursor: pointer;
		text-align: left;
		width: 100%;
		font: inherit;
		color: inherit;
	}
	.run-card:hover {
		background: var(--card);
		border-color: var(--border);
	}
	.run-card-expanded {
		background: var(--card);
		border-color: var(--border);
		border-bottom-left-radius: 0;
		border-bottom-right-radius: 0;
	}
	.run-card-failed {
		border-color: var(--border);
	}

	.run-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		margin-top: 0.4rem;
		flex-shrink: 0;
		opacity: 0.5;
	}

	.run-main {
		flex: 1;
		min-width: 0;
	}

	.run-top {
		display: flex;
		align-items: baseline;
		gap: 0.4rem;
		margin-bottom: 0.15rem;
	}

	.run-agent {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.02em;
	}

	.run-trigger {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.run-meta {
		display: flex;
		align-items: center;
		gap: 0.3rem;
		flex-wrap: wrap;
	}

	.run-time,
	.run-duration,
	.run-tokens {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.run-sep {
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.run-status {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}
	.run-status-fail {
		color: var(--destructive);
	}

	.run-summary {
		font-size: 0.75rem;
		color: var(--text-secondary);
		line-height: 1.4;
		margin: 0.2rem 0 0;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.run-chevron {
		flex-shrink: 0;
		margin-top: 0.25rem;
		color: var(--text-secondary);
		transition: transform 0.2s ease;
	}
	.run-chevron-open {
		transform: rotate(180deg);
	}

	/* Trace view */
	.run-trace {
		border: 1px solid var(--border);
		border-top: none;
		border-bottom-left-radius: 0.5rem;
		border-bottom-right-radius: 0.5rem;
		background: var(--card);
		padding: 0.5rem;
		margin-bottom: 0.375rem;
	}

	.trace-scroll {
		max-height: 400px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
		scrollbar-width: thin;
		scrollbar-color: var(--border) transparent;
	}

	.trace-msg {
		padding: 0.5rem 0.625rem;
		border-radius: 0.375rem;
		border: 1px solid var(--border);
	}
	.trace-msg-user {
		background: var(--card);
	}
	.trace-msg-assistant {
		background: var(--card);
	}

	.trace-role {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.06em;
		text-transform: none;
		color: var(--text-secondary);
		display: block;
		margin-bottom: 0.25rem;
	}

	.trace-text {
		font-size: 0.75rem;
		color: var(--text-secondary);
		line-height: 1.5;
		margin: 0;
		white-space: pre-wrap;
		word-break: break-word;
	}
	.trace-msg-assistant .trace-text {
		color: var(--text-secondary);
	}

	.trace-tool-use {
		margin-top: 0.25rem;
		padding: 0.35rem 0.5rem;
		border-radius: 0.25rem;
		background: var(--card);
		border: 1px solid var(--border);
	}

	.trace-tool-name {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	.trace-tool-input {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		margin: 0.15rem 0 0;
		white-space: pre-wrap;
		word-break: break-all;
		line-height: 1.4;
	}

	.trace-tool-result {
		margin-top: 0.25rem;
		padding: 0.35rem 0.5rem;
		border-radius: 0.25rem;
		background: var(--card);
		border: 1px solid var(--border);
	}

	.trace-tool-output {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		margin: 0;
		white-space: pre-wrap;
		word-break: break-all;
		line-height: 1.4;
		max-height: 200px;
		overflow-y: auto;
	}

	.trace-error {
		margin-top: 0.375rem;
		padding: 0.4rem 0.625rem;
		border-radius: 0.375rem;
		background: var(--card);
		border: 1px solid var(--border);
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--destructive);
		line-height: 1.4;
	}

	/* Config panel */
	.agent-config {
		padding: 0.75rem 1rem;
		margin: -0.25rem 0 0;
		border-top: 1px solid var(--border);
		display: flex;
		flex-direction: column;
		gap: 0.625rem;
	}

	.config-row {
		display: flex;
		align-items: center;
		gap: 0.75rem;
	}

	.config-row-full {
		flex-direction: column;
		align-items: stretch;
	}

	.config-row-actions {
		justify-content: flex-end;
		padding-top: 0.25rem;
	}

	.config-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.05em;
		min-width: 50px;
		flex-shrink: 0;
	}

	.config-field {
		display: flex;
		align-items: center;
		gap: 0.375rem;
	}

	.config-field-wrap {
		flex-wrap: wrap;
	}

	.config-input {
		font-family: var(--font-body);
		font-size: 0.75rem;
		padding: 0.3rem 0.5rem;
		border-radius: 0.375rem;
		border: 1px solid var(--border);
		background: var(--card);
		color: var(--foreground);
		outline: none;
	}
	.config-input:focus { border-color: var(--border); }
	.config-input-sm { width: 70px; }

	.config-unit {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.config-hint {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.config-chip {
		font-family: var(--font-body);
		font-size: 0.75rem;
		padding: 0.2rem 0.5rem;
		border-radius: 0.375rem;
		border: 1px solid var(--border);
		background: var(--card);
		color: var(--text-secondary);
		cursor: pointer;
		transition: all 0.15s ease;
		letter-spacing: 0.03em;
	}
	.config-chip:hover { border-color: var(--border); color: var(--text-secondary); }
	.config-chip-active {
		background: var(--card);
		border-color: var(--border);
		color: var(--text-secondary);
	}

	.config-textarea {
		font-family: var(--font-body);
		font-size: 0.75rem;
		line-height: 1.5;
		padding: 0.5rem;
		border-radius: 0.375rem;
		border: 1px solid var(--border);
		background: var(--card);
		color: var(--foreground);
		outline: none;
		resize: vertical;
		min-height: 80px;
	}
	.config-textarea:focus { border-color: var(--border); }

	.config-reset {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: none;
		border: 1px solid var(--border);
		padding: 0.25rem 0.6rem;
		border-radius: 0.375rem;
		cursor: pointer;
		letter-spacing: 0.04em;
		transition: all 0.2s ease;
	}
	.config-reset:hover {
		color: var(--text-secondary);
		border-color: var(--border);
		background: var(--card);
	}

	@media (max-width: 640px) {
		.agents-page { padding: 1.5rem 1rem; }
		.config-row { flex-direction: column; align-items: stretch; }
		.config-label { min-width: unset; }
	}

/* Little Moon surfaces, controls, and readable content. */

.agents-page { padding: 32px; color: var(--foreground); }
.agents-header, .agents-hint, .agents-list, .activity-section { max-width: 1040px; margin-left: auto; margin-right: auto; }
.agents-title { font-size: 24px; color: var(--foreground); font-family: var(--font-display); }
.agents-count { color: var(--primary); }
.agents-hint { font-size: 14px; line-height: 1.6; }
.agent-card, .run-card, .agent-config, .agent-history, .run-trace { background: var(--card); border: 1px solid var(--border); border-radius: 16px; padding: 20px; }
.agent-disabled { opacity: 1; }
.agent-name { font: 500 18px var(--font-body); }
.agent-desc, .history-content, .run-summary { font-size: 14px; line-height: 1.6; }
.agent-btn { width: 44px; height: 44px; border-radius: 8px; border-color: var(--border); color: var(--text-secondary); }
.agent-btn:hover, .agent-btn-active, .config-chip-active { background: var(--accent); color: var(--primary); border-color: var(--primary); }
.agents-add { background: var(--primary); color: var(--primary-foreground); border: 1px solid var(--primary); padding: 8px 16px; }
.agents-add:hover { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); }
.config-field { flex-wrap: wrap; }
.config-chip, .config-reset { min-height: 44px; padding: 8px 12px; }
.config-input, .config-textarea { font-size: 16px; min-height: 44px; border-color: var(--input); background: var(--background); }
.config-label { font-size: 14px; color: var(--text-secondary); }
.empty-text { font: 400 28px var(--font-display); color: var(--foreground); }
.empty-sub { font-size: 14px; max-width: 42ch; }
.run-status-fail, .trace-error { color: var(--destructive); }
.trace-text, .trace-tool-input, .trace-tool-output { font-family: var(--font-mono); font-size: 13px; }
@media (max-width: 640px) { .agents-page { padding: 20px; } .agent-card { flex-wrap: wrap; gap: 12px; } .agent-actions { width: 100%; justify-content: flex-end; } .agent-main { min-width: 0; } .agent-top { flex-wrap: wrap; } }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible, input:focus-visible, textarea:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.pulse-dot { background: var(--primary); }

.agent-model { opacity: 1; }
</style>
