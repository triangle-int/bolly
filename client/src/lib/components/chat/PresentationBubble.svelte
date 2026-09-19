<script lang="ts">
	import { onMount } from "svelte";
	import type { ChatMessage } from "$lib/api/types.js";
	import { Marked } from "marked";

	let {
		message,
		side = "right",
		streaming = false,
		onexpire,
	}: {
		message: ChatMessage;
		side?: "left" | "right";
		streaming?: boolean;
		onexpire?: () => void;
	} = $props();

	const marked = new Marked({ breaks: true, gfm: true });

	let fading = $state(false);
	let expired = false;
	let entered = $state(false);

	const isUser = $derived(message.role === "user");
	const html = $derived(
		isUser ? message.content : (marked.parse(message.content) as string)
	);

	onMount(() => {
		// Trigger enter animation on next frame
		requestAnimationFrame(() => { entered = true; });

		let expiryTimer: ReturnType<typeof setTimeout> | undefined;
		let streamCheckInterval: ReturnType<typeof setInterval> | undefined;

		function startExpiry() {
			if (expiryTimer) return;
			expiryTimer = setTimeout(() => {
				fading = true;
				setTimeout(() => {
					if (!expired) { expired = true; onexpire?.(); }
				}, 1500);
			}, 12000);
		}

		if (!streaming) startExpiry();

		streamCheckInterval = setInterval(() => {
			if (!streaming && !expiryTimer) startExpiry();
		}, 500);

		return () => {
			clearTimeout(expiryTimer);
			clearInterval(streamCheckInterval);
		};
	});
</script>

<div
	class="pbubble"
	class:pbubble-left={side === "left"}
	class:pbubble-right={side === "right"}
	class:pbubble-entered={entered}
	class:pbubble-fading={fading}
	class:pbubble-streaming={streaming}
>
	<div class="pbubble-glass">
		{#if isUser}
			<span class="pbubble-text">{message.content}</span>
		{:else}
			<div class="pbubble-text pbubble-prose">{@html html}</div>
		{/if}
	</div>
</div>

<style>
	.pbubble {
		width: fit-content;
		max-width: 55vw;
		opacity: 0;
		transform: translateX(80px) scale(0.92);
		transition:
			opacity 0.7s cubic-bezier(0.16, 1, 0.3, 1),
			transform 0.7s cubic-bezier(0.16, 1, 0.3, 1),
			filter 0.7s ease;
		pointer-events: none;
	}

	.pbubble-left {
		align-self: flex-start;
		transform: translateX(-80px) scale(0.92);
	}

	.pbubble-right {
		align-self: flex-end;
	}

	.pbubble-entered {
		opacity: 1;
		transform: translateX(0) scale(1);
	}

	.pbubble-fading {
		opacity: 0 !important;
		transform: scale(0.9) translateY(-10px) !important;
		filter: blur(4px);
	}

	.pbubble-glass {
		position: relative;
		padding: 1rem 1.5rem;
		background: var(--card);


		border: 1px solid var(--border);
		border-top-color: var(--border);
		border-bottom-color: var(--border);
		border-radius: 20px;
		box-shadow: none;
		overflow: hidden;
	}

	/* Specular highlight line */
	.pbubble-glass::before {
		content: "";
		position: absolute;
		top: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: var(--card);
		pointer-events: none;
	}

	/* Inner refraction gradient */
	.pbubble-glass::after {
		content: "";
		position: absolute;
		top: 0;
		left: 0;
		right: 0;
		height: 45%;
		background: var(--card);
		pointer-events: none;
		border-radius: 20px 20px 0 0;
	}

	.pbubble-left .pbubble-glass {
		border-radius: 20px 20px 20px 4px;
		background: var(--card);
	}

	.pbubble-right .pbubble-glass {
		border-radius: 20px 20px 4px 20px;
	}

	.pbubble-streaming .pbubble-glass {
		box-shadow: none;
	}

	.pbubble-text {
		font-family: var(--font-body);
		font-size: clamp(1.3rem, 2.5vw, 2rem);
		line-height: 1.55;
		letter-spacing: 0.01em;
		color: var(--text-secondary);
		word-break: break-word;
	}

	.pbubble-left .pbubble-text {
		color: var(--text-secondary);
		font-size: clamp(1rem, 2vw, 1.4rem);
	}

	/* Prose */
	.pbubble-prose :global(p) { margin: 0.3em 0; }
	.pbubble-prose :global(p:first-child) { margin-top: 0; }
	.pbubble-prose :global(p:last-child) { margin-bottom: 0; }
	.pbubble-prose :global(strong) { color: var(--text-secondary); }
	.pbubble-prose :global(em) { font-style: normal; }
	.pbubble-prose :global(code) {
		font-family: var(--font-body);
		font-size: 0.85em;
		background: var(--card);
		padding: 0.1em 0.3em;
		border-radius: 6px;
	}
	.pbubble-prose :global(a) { color: var(--text-secondary); }

 .pbubble-glass {background:var(--card);border:1px solid var(--border);border-radius:16px}
 .pbubble-glass::before,.pbubble-glass::after {display:none}
 .pbubble-left .pbubble-glass {background:var(--accent);border-radius:16px}
 .pbubble-right .pbubble-glass {border-radius:16px}
 .pbubble-text,.pbubble-prose :global(strong) {color:var(--foreground)}
 .pbubble-prose :global(code) {font-family:var(--font-mono);background:var(--background)}
 .pbubble-prose :global(em) {font-style:italic}
 .pbubble-prose :global(a) {color:var(--primary);text-decoration:underline}

</style>
