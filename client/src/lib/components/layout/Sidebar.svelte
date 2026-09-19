<script lang="ts">
	import { page } from "$app/state";
	import { getInstances } from "$lib/stores/instances.svelte.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import Badge from "$lib/components/ui/badge/badge.svelte";
	import Separator from "$lib/components/ui/separator/separator.svelte";
	import type { ServerEvent } from "$lib/api/types.js";
	import MessageSquare from "@lucide/svelte/icons/message-square";
	import Sparkles from "@lucide/svelte/icons/sparkles";
	import Brain from "@lucide/svelte/icons/brain";
	import Home from "@lucide/svelte/icons/home";

	const instances = getInstances();
	const ws = getWebSocket();

	$effect(() => {
		instances.refresh();
		ws.connect();

		const unsub = ws.subscribe((event: ServerEvent) => {
			if (event.type === "instance_discovered") {
				instances.upsert(event.instance);
			}
		});

		return () => {
			unsub();
			ws.disconnect();
		};
	});

	function isActive(slug: string) {
		return page.url.pathname === `/${slug}`;
	}
</script>

<aside class="flex h-full w-60 flex-col border-r border-sidebar-border bg-sidebar">
	<div class="flex items-center gap-2.5 px-5 py-5">
		<div class="flex h-8 w-8 items-center justify-center rounded-lg bg-warm/15">
			<img src="/skins/moon/character.svg" alt="" class="h-7 w-7" />
		</div>
		<span class="font-display text-base font-bold tracking-tight text-sidebar-foreground">
			nolune
		</span>
		<div class="ml-auto">
			<div
				class="h-1.5 w-1.5 rounded-full {ws.connected ? 'bg-emerald-400 pulse-alive' : 'bg-muted-foreground/30'}"
				title={ws.connected ? "Connected" : "Disconnected"}
			></div>
		</div>
	</div>

	<Separator />

	<nav class="flex-1 space-y-0.5 overflow-y-auto px-2.5 py-3">
		<a
			href="/"
			class="flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm transition-colors
				{page.url.pathname === '/' ? 'bg-sidebar-accent text-sidebar-foreground' : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
		>
			<Home class="h-4 w-4 shrink-0" />
			<span>Home</span>
		</a>

		{#if instances.list.length > 0}
			<div class="px-2.5 pb-1 pt-4">
				<span class="text-[11px] font-medium uppercase tracking-widest text-muted-foreground/60">
					Instances
				</span>
			</div>
		{/if}

		{#each instances.list as instance (instance.slug)}
			<a
				href="/{instance.slug}"
				class="group flex items-center gap-2.5 rounded-md px-2.5 py-2 text-sm transition-colors
					{isActive(instance.slug) ? 'bg-sidebar-accent text-sidebar-foreground' : 'text-muted-foreground hover:bg-sidebar-accent/50 hover:text-sidebar-foreground'}"
			>
				<div class="flex h-6 w-6 shrink-0 items-center justify-center rounded-md bg-warm/10 text-warm text-xs font-semibold">
					{instance.slug[0]?.toUpperCase() ?? "?"}
				</div>
				<span class="truncate">{instance.slug}</span>
				<div class="ml-auto flex items-center gap-1.5">
					{#if instance.soul_exists}
						<Sparkles class="h-3 w-3 text-warm/60" />
					{/if}
					{#if instance.has_memory}
						<Brain class="h-3 w-3 text-muted-foreground/40" />
					{/if}
					{#if instance.drops_count > 0}
						<Badge variant="secondary" class="h-4 px-1 text-[10px]">
							{instance.drops_count}
						</Badge>
					{/if}
				</div>
			</a>
		{/each}

		{#if instances.loading}
			<div class="space-y-2 px-2.5 py-2">
				{#each [1, 2] as _}
					<div class="h-8 animate-pulse rounded-md bg-muted/50"></div>
				{/each}
			</div>
		{/if}
	</nav>

	<Separator />

	<div class="px-5 py-3.5">
		<div class="flex items-center gap-2 text-xs text-muted-foreground/50">
			<MessageSquare class="h-3 w-3" />
			<span>{instances.list.length} instance{instances.list.length !== 1 ? "s" : ""}</span>
		</div>
	</div>
</aside>
