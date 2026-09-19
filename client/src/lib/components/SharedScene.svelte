<script lang="ts">
	import { onMount, onDestroy } from "svelte";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { getSkinStore } from "$lib/stores/skin.svelte.js";

	const store = getSceneStore();
	const skinStore = getSkinStore();

	let container: HTMLDivElement | undefined = $state();

	function easeOutCubic(x: number) { return 1 - Math.pow(1 - x, 3); }
	function easeInOutQuart(x: number) {
		return x < 0.5 ? 8 * x * x * x * x : 1 - Math.pow(-2 * x + 2, 4) / 2;
	}

	// ── Orb state ──
	interface OrbState {
		slug: string;
		// Position as % of container (50 = center)
		x: number;
		y: number;
		// Size in px
		size: number;
		opacity: number;
		visible: boolean;
	}

	let orbs = $state<OrbState[]>([]);
	let raf: number;
	let prevTime = 0;
	let elapsed = 0;
	let lastMode = '';

	// Convert 3D world X coord to CSS left% (camera at z=5, FOV 50)
	// At z=5 with FOV 50, visible width ≈ 2 * 5 * tan(25°) ≈ 4.66
	// So worldX=1.8 → 50% + 1.8/4.66*50% ≈ 69.3%
	const WORLD_TO_PCT = 50 / 4.66; // ~10.73% per world unit

	// Scale orbs to viewport — 450px at 900px+ height, shrinks for smaller windows
	function baseSize(): number {
		const h = container?.clientHeight ?? 900;
		return Math.min(450, h * 0.5);
	}

	const HOME_SCALE = 0.5;
	const FINAL_SCALE = 1.2;
	const FINAL_X = 1.8;
	const FINAL_Y = -0.1;

	function animate() {
		raf = requestAnimationFrame(animate);

		const now = performance.now();
		const delta = (now - prevTime) / 1000;
		prevTime = now;
		if (delta > 0.1) return; // skip large gaps

		store.tick();

		const m = store.mode;
		const sel = store.selectedSlug;
		const instances = store.instances;

		// Build slug list
		const slugs: string[] = instances.map(i => i.slug);
		if (sel && !slugs.includes(sel)) slugs.push(sel);

		// Ensure orbs array matches slugs
		const existing = new Map(orbs.map(o => [o.slug, o]));
		const newOrbs: OrbState[] = slugs.map(slug => {
			return existing.get(slug) ?? { slug, x: 50, y: 50, size: 0, opacity: 0, visible: false };
		});

		// Home positions
		const count = instances.length;
		const spacing = 1.4;
		const totalW = (count - 1) * spacing;
		const startX = -totalW / 2;
		const homePositions = new Map<string, number>();
		instances.forEach((inst, i) => {
			homePositions.set(inst.slug, startX + i * spacing);
		});

		const useLerp = m === "home" || m === "chat" || m === "onboarding";
		const lerpF = Math.min(delta * 6, 1);

		for (const orb of newOrbs) {
			const isSelected = orb.slug === sel;
			const isHovered = orb.slug === store.hoveredSlug;
			const homeX = homePositions.get(orb.slug) ?? 0;

			let tx = 50 + homeX * WORLD_TO_PCT;
			let ty = 50;
			let ts = HOME_SCALE * baseSize();
			let to = 1;

			if (m === "home") {
				ts = (isHovered ? 0.58 : HOME_SCALE) * baseSize();
			} else if (m === "onboarding") {
				if (isSelected) {
					tx = 50; ty = 50;
					// Full viewport background - use container size
					ts = baseSize();
					to = 0.6;
				} else {
					ts = 0; to = 0;
				}
			} else if (m === "selecting") {
				const e = easeInOutQuart(store.selectProgress);
				if (isSelected) {
					const hx = 50 + homeX * WORLD_TO_PCT;
					tx = hx + (50 - hx) * e;
					ty = 50;
					ts = (HOME_SCALE + e * (FINAL_SCALE - HOME_SCALE)) * baseSize();
				} else {
					ts = HOME_SCALE * (1 - e) * baseSize();
					to = 1 - e;
				}
			} else if (m === "intro") {
				if (isSelected) {
					const p = store.introProgress;
					if (p < 0.25) {
						const e = easeOutCubic(p / 0.25);
						tx = 50; ty = 50;
						ts = (FINAL_SCALE + e * 0.15) * baseSize();
					} else if (p < 0.58) {
						const e = easeInOutQuart((p - 0.25) / 0.33);
						tx = 50 + FINAL_X * WORLD_TO_PCT * e;
						ty = 50 + FINAL_Y * WORLD_TO_PCT * e;
						ts = (FINAL_SCALE * 1.15 + (FINAL_SCALE - FINAL_SCALE * 1.15) * e) * baseSize();
					} else {
						tx = 50 + FINAL_X * WORLD_TO_PCT;
						ty = 50 + FINAL_Y * WORLD_TO_PCT;
						ts = FINAL_SCALE * baseSize();
					}
				} else {
					ts = 0; to = 0;
				}
			} else if (m === "chat") {
				if (isSelected) {
					const isMobile = (container?.clientWidth ?? 800) < 640;
					if (store.presenting) {
						tx = 50;
						ty = 35;
						ts = FINAL_SCALE * baseSize() * 0.8;
					} else if (isMobile) {
						tx = 50;
						ty = 50;
						ts = FINAL_SCALE * baseSize();
					} else {
						tx = 50 + FINAL_X * WORLD_TO_PCT;
						ty = 50 + FINAL_Y * WORLD_TO_PCT;
						ts = FINAL_SCALE * baseSize();
					}
				} else {
					ts = 0; to = 0;
				}
			}

			if (useLerp) {
				orb.x += (tx - orb.x) * lerpF;
				orb.y += (ty - orb.y) * lerpF;
				orb.size += (ts - orb.size) * lerpF;
				orb.opacity += (to - orb.opacity) * lerpF;
			} else {
				orb.x = tx;
				orb.y = ty;
				orb.size = ts;
				orb.opacity = to;
			}

			orb.visible = orb.size > 1 && orb.opacity > 0.01;
		}

		orbs = newOrbs;
		// (all videos are now square — no format compensation needed)


		if (m !== lastMode) {
			lastMode = m;
		}
	}

	onMount(() => {
		// Mobile: animation still runs but orb is blurred/faded via CSS
		prevTime = performance.now();
		animate();
	});

	onDestroy(() => {
		if (raf) cancelAnimationFrame(raf);
	});

	function hitTestOrb(clientX: number, clientY: number): string | null {
		if (!container) return null;
		for (const orb of orbs) {
			if (!orb.visible) continue;
			const rect = container.getBoundingClientRect();
			const orbCx = rect.left + rect.width * orb.x / 100;
			const orbCy = rect.top + rect.height * orb.y / 100;
			const dx = clientX - orbCx;
			const dy = clientY - orbCy;
			const hitRadius = orb.size * 0.22;
			if (Math.sqrt(dx * dx + dy * dy) <= hitRadius) return orb.slug;
		}
		return null;
	}

	function handleSceneMove(e: MouseEvent) {
		if (store.mode !== "home" || !container) return;
		const hit = hitTestOrb(e.clientX, e.clientY);
		store.hoveredSlug = hit;
		(e.currentTarget as HTMLElement).style.cursor = hit ? 'pointer' : 'default';
	}

	function handleSceneClick(e: MouseEvent) {
		if (store.mode !== "home") return;
		const hit = hitTestOrb(e.clientX, e.clientY);
		if (hit) store.selectInstance(hit);
	}
</script>

<div class="scene-root" bind:this={container}>
{#if store.mode === 'home'}
	<div class="hit-layer"
		role="none"
		onmousemove={handleSceneMove}
		onclick={handleSceneClick}
		onmouseleave={() => { store.hoveredSlug = null; if (container) container.style.cursor = 'default'; }}
	></div>
{/if}
	{#each orbs as orb (orb.slug)}
		{#if orb.visible}
			<button
				class="orb-btn"
				aria-label={orb.slug}
				onclick={() => { if (store.mode === "home") store.selectInstance(orb.slug); }}
				style="left: {orb.x}%; top: {orb.y}%; width: {orb.size}px; height: {orb.size}px; opacity: {orb.opacity};"
				disabled={store.mode !== "home"}
			>
                <img class="moon-avatar" src={store.thinking ? skinStore.skin.avatar.thinking : skinStore.skin.avatar.idle} alt={store.thinking ? "Nolune is thinking" : "Nolune"} />
			</button>
		{/if}
	{/each}

	<!-- Memory orbit around the selected orb (desktop) / strip above chat (mobile) -->
	{#if store.recalledMemories.length > 0}
		{@const selOrb = orbs.find(o => o.slug === store.selectedSlug)}
		{#if selOrb}
			{@const count = store.recalledMemories.length}
			<!-- Desktop: orbit around orb -->
			<div class="memory-orbit" style="left: {selOrb.x}%; top: {selOrb.y}%; --orb-size: {selOrb.size}px;">
				<div class="memory-orbit-glow"></div>
				{#each store.recalledMemories as mem, i}
					{@const angle = (360 / count) * i - 90}
					<a
						class="memory-node"
						style="--angle: {angle}deg; --delay: {i * 200}ms; --i: {i};"
						href="/{store.selectedSlug}/memory?open={encodeURIComponent(mem.path)}"
					>
						<span class="memory-node-line"></span>
						<span class="memory-node-label">
							{mem.path.split('/').pop()?.replace('.md', '')}
						</span>
					</a>
				{/each}
			</div>
			<!-- Mobile: horizontal strip -->
			<div class="memory-strip">
				{#each store.recalledMemories as mem, i}
					<a
						class="memory-strip-chip"
						style="animation-delay: {i * 120}ms;"
						href="/{store.selectedSlug}/memory?open={encodeURIComponent(mem.path)}"
					>
						{mem.path.split('/').pop()?.replace('.md', '')}
					</a>
				{/each}
			</div>
		{/if}
	{/if}
</div>

<style>
	.scene-root {
		position: absolute;
		inset: 0;
		z-index: 0;
		overflow: hidden;
		pointer-events: none;
	}

	.hit-layer {
		position: absolute;
		inset: 0;
		pointer-events: auto;
		z-index: 10;
	}

	.orb-btn {
		display:flex;align-items:center;justify-content:center;
		position: absolute;
		background: none;
		border: none;
		padding: 0;
		cursor: pointer;
		pointer-events: auto;
		transform: translate(-50%, -50%);
		will-change: left, top, width, height, opacity;
		overflow: visible;
	}

	.orb-btn:disabled {
		cursor: default;
	}

	.moon-avatar { width: 70%; height: 70%; max-width:320px; max-height:320px; object-fit: contain; pointer-events: none; }


	/* ── Memory clouds (above orb) ── */
	/* ── Memory orbit ── */
	.memory-orbit {
		position: absolute;
		transform: translate(-50%, -50%);
		z-index: 20;
		pointer-events: none;
		width: calc(var(--orb-size) * 1.0);
		height: calc(var(--orb-size) * 1.0);
		animation: orbit-breathe 8s ease-in-out infinite;
	}

	.memory-orbit-glow {
		display:none;
		position: absolute;
		inset: 25%;
		border-radius: 50%;
		background: radial-gradient(circle, var(--primary) 0%, transparent 70%);
		animation: glow-pulse 2s ease-in-out infinite;
	}

	@keyframes glow-pulse {
		0%, 100% { opacity: 0.4; transform: scale(1); }
		50% { opacity: 1; transform: scale(1.1); }
	}

	.memory-node {
		position: absolute;
		left: 50%;
		top: 50%;
		width: 0;
		height: 0;
		text-decoration: none;
		pointer-events: auto;
		cursor: pointer;
		/* position on the ellipse */
		transform:
			rotate(var(--angle))
			translateX(calc(var(--orb-size) * 0.42))
			rotate(calc(-1 * var(--angle)));
		/* entry animation */
		opacity: 0;
		animation: node-connect 0.8s cubic-bezier(0.16, 1, 0.3, 1) forwards;
		animation-delay: var(--delay);
	}

	.memory-node-line {
		position: absolute;
		left: 50%;
		top: 50%;
		width: calc(var(--orb-size) * 0.42);
		height: 1px;
		transform-origin: 0 0;
		transform:
			rotate(calc(var(--angle) + 180deg));
		background: linear-gradient(
			90deg,
			var(--primary) 0%,
			var(--primary) 60%,
			transparent 100%
		);
		pointer-events: none;
	}

	.memory-node-label {
		position: absolute;
		transform: translate(-50%, -50%);
		padding: 12px;
		min-height:44px;
		border-radius: 0.75rem;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--primary);
		white-space: nowrap;
		font-family: var(--font-body);
		font-style: normal;
		font-size: 0.75rem;
		letter-spacing: 0.02em;
		color: var(--primary);
		transition: all 0.25s ease;
	}

	.memory-node:hover .memory-node-label {
		color: var(--primary);
		border-color: var(--primary);
		background: var(--primary);
		box-shadow: none;
	}

	.memory-node:hover .memory-node-line {
		background: linear-gradient(
			90deg,
			var(--primary) 0%,
			var(--primary) 60%,
			transparent 100%
		);
	}

	@keyframes node-connect {
		0% {
			opacity: 0;
			transform:
				rotate(var(--angle))
				translateX(0)
				rotate(calc(-1 * var(--angle)))
				scale(0.5);
		}
		60% {
			opacity: 1;
		}
		100% {
			opacity: 1;
			transform:
				rotate(var(--angle))
				translateX(calc(var(--orb-size) * 0.42))
				rotate(calc(-1 * var(--angle)))
				scale(1);
		}
	}

	@keyframes orbit-breathe {
		0%, 100% { transform: translate(-50%, -50%) scale(1); }
		50% { transform: translate(-50%, -50%) scale(1.04); }
	}


	/* ── Mobile memory strip ── */
	.memory-strip {
		display: none;
	}

	@media (max-width: 640px) {
		.moon-avatar{max-width:200px;max-height:200px;}
		.scene-root {
			pointer-events: none;
		}
		.orb-btn {
			pointer-events: none;
		}
		.memory-orbit {
			display: none;
		}
		.memory-strip {
			display: flex;
			position: fixed;
			bottom: calc(env(safe-area-inset-bottom, 0px) + 68px);
			left: 0;
			right: 0;
			z-index: 30;
			gap: 0.375rem;
			padding: 0.5rem 1rem;
			overflow-x: auto;
			scrollbar-width: none;
			-webkit-overflow-scrolling: touch;
			pointer-events: auto;
			mask-image: linear-gradient(90deg, transparent, black 0.5rem, black calc(100% - 0.5rem), transparent);
			-webkit-mask-image: linear-gradient(90deg, transparent, black 0.5rem, black calc(100% - 0.5rem), transparent);
		}
		.memory-strip::-webkit-scrollbar { display: none; }
		.memory-strip-chip {
			flex-shrink: 0;
			padding: 12px;
			min-height:44px;
			border-radius: 1rem;
			background: var(--card);
			backdrop-filter: none;
			-webkit-backdrop-filter: none;
			border: 1px solid var(--primary);
			font-family: var(--font-body);
			font-style: normal;
			font-size: 0.75rem;
			color: var(--primary);
			white-space: nowrap;
			text-decoration: none;
			opacity: 0;
			animation: strip-chip-in 0.4s cubic-bezier(0.16, 1, 0.3, 1) forwards;
			transition: all 0.2s ease;
		}
		.memory-strip-chip:active {
			color: var(--primary);
			border-color: var(--primary);
			background: var(--primary);
		}
		@keyframes strip-chip-in {
			from { opacity: 0; transform: translateY(8px); }
			to { opacity: 1; transform: translateY(0); }
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.memory-orbit { animation: none; }
	}
</style>
