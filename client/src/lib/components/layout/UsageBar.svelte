<script lang="ts">
	import { fetchUsage } from "$lib/api/client.js";
	import type { Usage } from "$lib/api/types.js";

	let { tick = 0 }: { tick?: number } = $props();

	let usage = $state<Usage | null>(null);
	let now = $state(Date.now());

	async function load() {
		try {
			usage = await fetchUsage();
		} catch {
			usage = null;
		}
	}

	$effect(() => {
		void tick;
		load();
	});

	$effect(() => {
		const interval = setInterval(() => { load(); now = Date.now(); }, 60_000);
		return () => clearInterval(interval);
	});

	function pct(used: number, limit: number): number {
		if (limit <= 0) return 0;
		return Math.min(100, Math.round((used / limit) * 100));
	}

	function barColor(p: number): string {
		if (p >= 100) return "var(--destructive)";
		if (p >= 80) return "var(--foreground)";
		return "var(--primary)";
	}

	function formatTokens(n: number): string {
		if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
		if (n >= 1_000) return `${(n / 1_000).toFixed(0)}K`;
		return String(n);
	}

	let limit4h = $derived(usage?.tokens_4h_limit ?? 0);
	let used4h = $derived(usage?.tokens_last_4h ?? 0);

	let resetLabel = $derived.by(() => {
		if (!usage?.resets_at) return "";
		const resetMs = new Date(usage.resets_at).getTime();
		const diff = resetMs - now;
		if (diff <= 0) return "resets soon";
		const mins = Math.floor(diff / 60_000);
		if (mins < 60) return `${mins}m`;
		const h = Math.floor(mins / 60);
		const m = mins % 60;
		return m > 0 ? `${h}h${m}m` : `${h}h`;
	});
</script>

{#if limit4h > 0}
	{@const p = pct(used4h, limit4h)}
	<div class="usage-bar">
		<div class="usage-item" title="{formatTokens(used4h)} / {formatTokens(limit4h)} tokens (4h) — resets {resetLabel}">
			<span class="usage-label">Usage: {formatTokens(used4h)}/{formatTokens(limit4h)}</span>
			<div class="usage-track">
				<div class="usage-fill" style="width: {p}%; background: {barColor(p)}"></div>
			</div>
			{#if resetLabel}
				<span class="usage-reset">Resets {resetLabel}</span>
			{/if}
		</div>
	</div>
{/if}

<style>
.usage-bar{display:flex;justify-content:center;padding:8px 4px 0}.usage-item{display:flex;align-items:center;justify-content:center;flex-wrap:wrap;gap:8px;font:12px/1.5 var(--font-body);color:var(--text-muted)}.usage-track{width:48px;height:4px;border-radius:2px;background:var(--secondary);overflow:hidden}.usage-fill{height:100%;border-radius:2px;transition:width .3s}.usage-label,.usage-reset{white-space:nowrap}
</style>
