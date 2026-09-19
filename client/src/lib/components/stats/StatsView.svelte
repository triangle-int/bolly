<script lang="ts">
	import { fetchStats } from "$lib/api/client.js";
	import type { Stats } from "$lib/api/types.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";

	const toast = getToasts();
	let { slug }: { slug: string } = $props();

	let stats = $state<Stats | null>(null);
	let loading = $state(true);
	let loadError = $state("");

	async function load() {
		loading = true;
		loadError = "";
		try {
			stats = await fetchStats(slug);
		} catch {
			loadError = "Could not load statistics. Please try again.";
			toast.error("failed to load stats");
		} finally {
			loading = false;
		}
	}

	$effect(() => { load(); });

	// --- derived data ---

	const DAY_LABELS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
	const HOUR_LABELS = Array.from({ length: 24 }, (_, i) =>
		i === 0 ? "12a" : i < 12 ? `${i}a` : i === 12 ? "12p" : `${i - 12}p`
	);

	let peakHour = $derived.by(() => {
		if (!stats || !stats.hourly_activity.some(count => count > 0)) return -1;
		return stats.hourly_activity.indexOf(Math.max(...stats.hourly_activity));
	});

	let peakDay = $derived.by(() => {
		if (!stats || !stats.daily_activity.some(count => count > 0)) return -1;
		return stats.daily_activity.indexOf(Math.max(...stats.daily_activity));
	});

	let topMoods = $derived.by(() => {
		if (!stats) return [];
		return Object.entries(stats.mood_counts)
			.sort((a, b) => b[1] - a[1])
			.slice(0, 6);
	});

	let totalMoodCount = $derived(topMoods.reduce((s, [, c]) => s + c, 0));

	let daysSinceFirst = $derived.by(() => {
		if (!stats?.first_message_at) return 0;
		const first = parseInt(stats.first_message_at);
		return Math.floor((Date.now() - first) / 86400000) + 1;
	});

	let heatmapMax = $derived.by(() => {
		if (!stats) return 1;
		return Math.max(...stats.hourly_activity, 1);
	});

	const MONTH_LABELS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

	function localDateStr(d: Date): string {
		const y = d.getFullYear();
		const m = String(d.getMonth() + 1).padStart(2, "0");
		const day = String(d.getDate()).padStart(2, "0");
		return `${y}-${m}-${day}`;
	}

	let contributionWeeks = $derived.by(() => {
		if (!stats) return [];
		const map = new Map(stats.daily_history.map(([d, c]) => [d, c]));
		const maxCount = Math.max(...stats.daily_history.map(([, c]) => c), 1);
		const weeks: { date: string; count: number; level: number }[][] = [];
		const today = new Date();
		const start = new Date(today);
		const daysSinceMonday = (start.getDay() + 6) % 7;
		start.setDate(start.getDate() - 52 * 7 - daysSinceMonday);

		let currentWeek: { date: string; count: number; level: number }[] = [];
		for (let d = new Date(start); d <= today; d.setDate(d.getDate() + 1)) {
			const ds = localDateStr(d);
			const count = map.get(ds) ?? 0;
			const level = count === 0 ? 0 : Math.min(4, Math.ceil((count / maxCount) * 4));
			currentWeek.push({ date: ds, count, level });
			if (currentWeek.length === 7) {
				weeks.push(currentWeek);
				currentWeek = [];
			}
		}
		if (currentWeek.length > 0) weeks.push(currentWeek);
		return weeks;
	});

	let monthLabels = $derived.by(() => {
		if (contributionWeeks.length === 0) return [];
		const labels: { label: string; col: number }[] = [];
		let lastMonth = -1;
		for (let i = 0; i < contributionWeeks.length; i++) {
			const firstDay = contributionWeeks[i][0];
			if (!firstDay) continue;
			const month = new Date(firstDay.date + "T00:00:00").getMonth();
			if (month !== lastMonth) {
				labels.push({ label: MONTH_LABELS[month], col: i });
				lastMonth = month;
			}
		}
		return labels;
	});

	const moodColors: Record<string, string> = {
		calm: "var(--primary)", curious: "var(--primary)",
		excited: "var(--primary)", warm: "var(--primary)",
		happy: "var(--primary)", playful: "var(--primary)",
		thoughtful: "var(--primary)", focused: "var(--primary)",
		tender: "var(--primary)", loving: "var(--primary)",
		creative: "var(--primary)", energetic: "var(--primary)",
		melancholic: "var(--primary)", anxious: "var(--primary)",
		grateful: "var(--primary)", nostalgic: "var(--primary)",
	};

	function getMoodColor(mood: string): string {
		return moodColors[mood] ?? "var(--primary)";
	}

	function formatInterval(secs: number): string {
		if (secs < 60) return `${Math.round(secs)}s`;
		if (secs < 3600) return `${Math.round(secs / 60)}m`;
		return `${(secs / 3600).toFixed(1)}h`;
	}
</script>

<div class="stats-container">
	{#if loading}
		<span class="sr-only" role="status">Loading statistics…</span>
		<div class="stats-loading"><div class="stats-loading-dot"></div></div>
	{:else if loadError}
		<div class="load-error" role="alert"><p>{loadError}</p><button class="nl-button-secondary" onclick={load}>Try again</button></div>
	{:else if stats}
		<div class="stats-scroll">
			<!-- Hero numbers -->
			<div class="stats-hero">
				<div class="hero-card" style="animation-delay: 0ms">
					<span class="hero-value">{stats.total_messages.toLocaleString()}</span>
					<span class="hero-label">Messages</span>
				</div>
				<div class="hero-card" style="animation-delay: 60ms">
					<span class="hero-value">{daysSinceFirst}</span>
					<span class="hero-label">Days together</span>
				</div>
				<div class="hero-card" style="animation-delay: 120ms">
					<span class="hero-value">{stats.streak_days}</span>
					<span class="hero-label">Day streak</span>
					{#if stats.streak_days > 0}
						<div class="streak-glow"></div>
					{/if}
				</div>
			</div>

			<!-- Contribution heatmap -->
			<section class="glass-card" style="animation-delay: 150ms">
				<h3 class="card-title">Activity</h3>
				<div class="heatmap-container">
					<div class="heatmap-months">
						{#each monthLabels as { label, col }, i}
							{@const nextCol = i + 1 < monthLabels.length ? monthLabels[i + 1].col : contributionWeeks.length}
							<span
								class="heatmap-month-label"
								style="width: {(nextCol - col) * 11}px"
							>{label}</span>
						{/each}
					</div>
					<div class="heatmap-with-days">
						<div class="heatmap-day-labels">
							<span class="heatmap-day-label"></span>
							<span class="heatmap-day-label">Mon</span>
							<span class="heatmap-day-label"></span>
							<span class="heatmap-day-label">Wed</span>
							<span class="heatmap-day-label"></span>
							<span class="heatmap-day-label">Fri</span>
							<span class="heatmap-day-label"></span>
						</div>
						<div class="heatmap-grid">
							{#each contributionWeeks as week}
								<div class="heatmap-col">
									{#each week as day}
										<div
											class="heatmap-cell"
											class:heatmap-0={day.level === 0}
											class:heatmap-1={day.level === 1}
											class:heatmap-2={day.level === 2}
											class:heatmap-3={day.level === 3}
											class:heatmap-4={day.level === 4}
											title="{day.date}: {day.count} messages"
										></div>
									{/each}
								</div>
							{/each}
						</div>
					</div>
					<div class="heatmap-legend">
						<span class="heatmap-legend-label">Less</span>
						<div class="heatmap-cell heatmap-0"></div>
						<div class="heatmap-cell heatmap-1"></div>
						<div class="heatmap-cell heatmap-2"></div>
						<div class="heatmap-cell heatmap-3"></div>
						<div class="heatmap-cell heatmap-4"></div>
						<span class="heatmap-legend-label">More</span>
					</div>
				</div>
			</section>

			<!-- Hourly activity -->
			<section class="glass-card" style="animation-delay: 200ms">
				<h3 class="card-title">
					Peak hours
					<span class="card-hint">{peakHour >= 0 ? `Most active at ${HOUR_LABELS[peakHour]}` : "No hourly activity yet"}</span>
				</h3>
				<div class="bar-chart">
					{#each stats.hourly_activity as count, i}
						{@const pct = count / heatmapMax}
						<div class="bar-col" title="{HOUR_LABELS[i]}: {count} messages">
							<div
								class="bar-fill"
								class:bar-fill-peak={i === peakHour}
								style="height: {count > 0 ? Math.max(pct * 100, 2) : 0}%; min-height: {count > 0 ? 2 : 0}px"
							></div>
							{#if i % 3 === 0}
								<span class="bar-label">{HOUR_LABELS[i]}</span>
							{/if}
						</div>
					{/each}
				</div>
			</section>

			<!-- Day of week -->
			<section class="glass-card" style="animation-delay: 250ms">
				<h3 class="card-title">
					Day of week
					<span class="card-hint">{peakDay >= 0 ? `${DAY_LABELS[peakDay]} is your most active day` : "No daily activity yet"}</span>
				</h3>
				<div class="day-bars">
					{#each stats.daily_activity as count, i}
						{@const max = Math.max(...stats.daily_activity, 1)}
						{@const pct = count / max}
						<div class="day-row">
							<span class="day-label" class:day-label-peak={i === peakDay}>{DAY_LABELS[i]}</span>
							<div class="day-track">
								<div class="day-fill" class:day-fill-peak={i === peakDay} style="width: {pct * 100}%"></div>
							</div>
							<span class="day-count">{count}</span>
						</div>
					{/each}
				</div>
			</section>

			<!-- Mood distribution -->
			{#if topMoods.length > 0}
				<section class="glass-card" style="animation-delay: 300ms">
					<h3 class="card-title">Mood palette</h3>
					<div class="mood-chart">
						{#each topMoods as [mood, count]}
							{@const pct = (count / totalMoodCount) * 100}
							<div class="mood-row">
								<div class="mood-dot" style="background: {getMoodColor(mood)}"></div>
								<span class="mood-name">{mood}</span>
								<div class="mood-track">
									<div class="mood-fill" style="width: {pct}%; background: {getMoodColor(mood)}"></div>
								</div>
								<span class="mood-pct">{pct.toFixed(0)}%</span>
							</div>
						{/each}
					</div>
				</section>
			{/if}

			<!-- Quick stats -->
			<section class="glass-card" style="animation-delay: 350ms">
				<h3 class="card-title">Details</h3>
				<div class="detail-grid">
					<div class="detail-item">
						<span class="detail-value">{Math.round(stats.avg_message_length)}</span>
						<span class="detail-label">Average message length</span>
					</div>
					<div class="detail-item">
						<span class="detail-value">{formatInterval(stats.avg_response_interval_secs)}</span>
						<span class="detail-label">Average reply interval</span>
					</div>
					<div class="detail-item">
						<span class="detail-value">{stats.daily_history.length}</span>
						<span class="detail-label">Active days</span>
					</div>
					<div class="detail-item">
						<span class="detail-value">{stats.total_messages > 0 && stats.daily_history.length > 0 ? (stats.total_messages / stats.daily_history.length).toFixed(1) : "0"}</span>
						<span class="detail-label">Messages per day</span>
					</div>
				</div>
			</section>
		</div>
	{:else}
		<div class="stats-empty">No activity yet</div>
	{/if}
</div>

<style>
	.load-error { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 16px; min-height: 220px; padding: 24px; color: var(--text-secondary); text-align: center; }
	.stats-container {
		height: 100%;
		overflow: hidden;
		position: relative;
	}

	.stats-loading {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
	}

	.stats-loading-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--card);
		animation:none;
	}

	.stats-empty {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	.stats-scroll {
		height: 100%;
		overflow-y: auto;
		padding: 1.5rem;
		max-width: 560px;
		margin: 0 auto;
		display: flex;
		flex-direction: column;
		gap: 1rem;
		scrollbar-width: none;
	}
	.stats-scroll::-webkit-scrollbar { display: none; }

	/* ═══ Glass card ═══ */

	.glass-card {
		position: relative;
		padding: 1.125rem 1.25rem;
		border-radius: 1rem;
		border: 1px solid var(--border);
		border-top-color: var(--border);
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		box-shadow: none;
		display: flex;
		flex-direction: column;
		gap: 0.875rem;
		animation: none;
	}

	/* Specular top highlight */
	.glass-card::before {
		content: "";
		position: absolute;
		top: 0;
		left: 15%;
		right: 15%;
		height: 1px;
		background: var(--card);
		pointer-events: none;
	}

	@keyframes card-enter {
		from { opacity: 1; transform: translateY(12px); filter: blur(4px); }
		to { opacity: 1; transform: translateY(0); filter: blur(0px); }
	}

	.card-title {
		font-family: var(--font-body);
		font-size: 0.75rem;
		font-weight: 400;
		color: var(--text-secondary);
		letter-spacing: 0.06em;
		display: flex;
		align-items: baseline;
		gap: 0.5rem;
	}

	.card-hint {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		font-style: normal;
	}

	/* ═══ Hero ═══ */

	.stats-hero {
		display: flex;
		justify-content: center;
		gap: 0.75rem;
		padding: 0.5rem 0;
	}

	.hero-card {
		position: relative;
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 0.375rem;
		padding: 1.25rem 0.75rem;
		border-radius: 1rem;
		border: 1px solid var(--border);
		border-top-color: var(--border);
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		box-shadow: none;
		animation: none;
		overflow: hidden;
	}

	.hero-card::before {
		content: "";
		position: absolute;
		top: 0;
		left: 20%;
		right: 20%;
		height: 1px;
		background: var(--card);
		pointer-events: none;
	}

	.hero-value {
		font-family: var(--font-display);
		font-size: 1.75rem;
		font-weight: 300;
		color: var(--text-secondary);
		line-height: 1;
	}

	.hero-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.06em;
	}

	.streak-glow {
		position: absolute;
		inset: 0;
		border-radius: inherit;
		background: var(--card);
		pointer-events: none;
		animation: none;
	}

	@keyframes streak-pulse {
		0%, 100% { opacity: 0.5; }
		50% { opacity: 1; }
	}

	/* ═══ Heatmap ═══ */

	.heatmap-container {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
	}

	.heatmap-grid {
		display: flex;
		gap: 2px;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.heatmap-grid::-webkit-scrollbar { display: none; }

	.heatmap-months {
		display: flex;
		padding-left: 28px;
		overflow: hidden;
	}

	.heatmap-month-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		flex-shrink: 0;
	}

	.heatmap-with-days {
		display: flex;
		gap: 3px;
	}

	.heatmap-day-labels {
		display: flex;
		flex-direction: column;
		gap: 2px;
		flex-shrink: 0;
		width: 24px;
	}

	.heatmap-day-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		height: 9px;
		line-height: 9px;
		text-align: right;
	}

	.heatmap-col {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.heatmap-cell {
		width: 9px;
		height: 9px;
		border-radius: 2.5px;
		transition: background 0.2s ease;
	}

	.heatmap-0 { background: var(--card); }
	.heatmap-1 { background: var(--card); }
	.heatmap-2 { background: var(--card); }
	.heatmap-3 { background: var(--card); }
	.heatmap-4 { background: var(--card); box-shadow: none; }

	.heatmap-legend {
		display: flex;
		align-items: center;
		gap: 3px;
		justify-content: flex-end;
	}

	.heatmap-legend-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		margin: 0 2px;
	}

	.heatmap-legend .heatmap-cell {
		width: 8px;
		height: 8px;
	}

	/* ═══ Bar chart (hourly) ═══ */

	.bar-chart {
		display: flex;
		align-items: flex-end;
		gap: 2px;
		height: 88px;
		padding-bottom: 16px;
		position: relative;
	}

	.bar-col {
		flex: 1;
		display: flex;
		flex-direction: column;
		align-items: center;
		height: 100%;
		position: relative;
		justify-content: flex-end;
	}

	.bar-fill {
		width: 100%;
		border-radius: 2px 2px 0 0;
		background: var(--card);
		min-height: 2px;
		transition: height 0.4s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.bar-fill-peak {
		background: var(--card);
		box-shadow: none;
	}

	.bar-label {
		position: absolute;
		bottom: -14px;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
	}

	/* ═══ Day bars ═══ */

	.day-bars {
		display: flex;
		flex-direction: column;
		gap: 0.375rem;
	}

	.day-row {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.day-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		width: 28px;
		text-align: right;
		transition: color 0.2s ease;
	}

	.day-label-peak {
		color: var(--text-secondary);
	}

	.day-track {
		flex: 1;
		height: 8px;
		border-radius: 4px;
		background: var(--card);
		overflow: hidden;
	}

	.day-fill {
		height: 100%;
		border-radius: 4px;
		background: var(--card);
		transition: width 0.5s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.day-fill-peak {
		background: var(--card);
		box-shadow: none;
	}

	.day-count {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		width: 28px;
	}

	/* ═══ Mood chart ═══ */

	.mood-chart {
		display: flex;
		flex-direction: column;
		gap: 0.4rem;
	}

	.mood-row {
		display: flex;
		align-items: center;
		gap: 0.5rem;
	}

	.mood-dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		flex-shrink: 0;
	}

	.mood-name {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		width: 72px;
	}

	.mood-track {
		flex: 1;
		height: 6px;
		border-radius: 3px;
		background: var(--card);
		overflow: hidden;
	}

	.mood-fill {
		height: 100%;
		border-radius: 3px;
		opacity: 0.55;
		transition: width 0.5s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.mood-pct {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		width: 28px;
		text-align: right;
	}

	/* ═══ Details grid ═══ */

	.detail-grid {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 0.625rem;
	}

	.detail-item {
		display: flex;
		flex-direction: column;
		gap: 0.2rem;
		padding: 0.75rem;
		border-radius: 0.625rem;
		background: var(--card);
		border: 1px solid var(--border);
	}

	.detail-value {
		font-family: var(--font-display);
		font-size: 1.1rem;
		font-weight: 300;
		color: var(--text-secondary);
	}

	.detail-label {
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		letter-spacing: 0.04em;
	}

	/* ═══ Responsive ═══ */

	@media (max-width: 640px) {
		.stats-scroll { padding: 1rem; }
		.stats-hero { gap: 0.5rem; }
		.hero-value { font-size: 1.4rem; }
		.hero-card { padding: 1rem 0.5rem; }
		.glass-card { padding: 1rem; }
	}

/* Little Moon surfaces, controls, and readable content. */

.stats-scroll { padding: 32px; max-width: 1040px; width: 100%; margin-inline: auto; }
.glass-card, .hero-card { background: var(--card); border: 1px solid var(--border); border-radius: 16px; padding: 24px; }
.glass-card::before, .hero-card::before, .streak-glow { display: none; }
.card-title { font: 500 18px var(--font-body); color: var(--foreground); flex-wrap: wrap; }
.hero-value { color: var(--primary); font-size: 32px; font-weight: 400; }
.hero-label, .card-hint, .detail-label { font-size: 13px; }
.detail-value { color: var(--foreground); font-size: 24px; }
.detail-item { background: var(--background); border-color: var(--border); padding: 16px; }
.heatmap-0, .day-track, .mood-track { background: var(--accent); }
.heatmap-1 { background: color-mix(in srgb, var(--primary) 25%, var(--card)); }
.heatmap-2 { background: color-mix(in srgb, var(--primary) 45%, var(--card)); }
.heatmap-3 { background: color-mix(in srgb, var(--primary) 70%, var(--card)); }
.heatmap-4, .bar-fill-peak, .day-fill-peak { background: var(--primary); }
.bar-fill, .day-fill { background: color-mix(in srgb, var(--primary) 65%, var(--card)); }
.mood-fill { opacity: 1; }
.heatmap-cell { height: 12px; width: 9px; }
.heatmap-day-label { height: 12px; line-height: 12px; }
.heatmap-day-labels { width: 32px; }
.heatmap-months { padding-left: 35px; }
.day-label, .day-count, .mood-pct { width: 36px; }
@media (max-width: 640px) { .stats-scroll { padding: 20px; } .hero-card { padding: 16px 8px; } .hero-value { font-size: 26px; } .hero-label { text-align: center; } }

button { min-height: 44px; font-family: var(--font-body); }

button:focus-visible { outline: 2px solid var(--ring); outline-offset: 3px; }

@media (prefers-reduced-motion: reduce) { *, *::before, *::after { animation: none !important; transition: none !important; } }

.stats-loading-dot { background: var(--primary); }
</style>
