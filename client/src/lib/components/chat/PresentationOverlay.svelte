<script lang="ts">
	import { untrack } from "svelte";
	import type { ChatMessage } from "$lib/api/types.js";
	import PresentationBubble from "./PresentationBubble.svelte";

	type StreamItem =
		| { type: "message"; data: ChatMessage }
		| { type: "activity"; id: string; kind: string; label: string; timestamp: string }
		| { type: "mcp_app"; id: string; toolName: string; toolInput: string; toolOutput: string; html: string }
		| { type: "compaction"; id: string; count: number; timestamp: string };

	let {
		stream = [],
		mood = "calm",
		thinking = false,
		voiceAmplitude = 0,
		streamingMessageId = "",
		onSend,
		onStop,
	}: {
		stream?: StreamItem[];
		mood?: string;
		thinking?: boolean;
		voiceAmplitude?: number;
		streamingMessageId?: string;
		onSend?: (content: string) => void;
		onStop?: () => void;
	} = $props();

	let inputValue = $state("");
	let inputEl: HTMLTextAreaElement | undefined = $state();
	let inputVisible = $state(false);
	let messageColumn: HTMLDivElement | undefined = $state();

	// Track visible bubbles
	type VisibleBubble = {
		id: string;
		side: "left" | "right";
	};

	let visible = $state<VisibleBubble[]>([]);
	const initialIds = untrack(() => stream).filter(s => s.type === "message").map(s => (s as { type: "message"; data: ChatMessage }).data.id);
	let seenIds = new Set<string>(initialIds);

	// Watch stream for new messages
	$effect(() => {
		const msgs = stream.filter(s => s.type === "message") as { type: "message"; data: ChatMessage }[];

		for (const item of msgs) {
			if (seenIds.has(item.data.id)) continue;
			seenIds.add(item.data.id);

			visible = [...visible, {
				id: item.data.id,
				side: item.data.role === "user" ? "right" : "left",
			}];

			// Auto-scroll to bottom
			requestAnimationFrame(() => {
				if (messageColumn) {
					messageColumn.scrollTop = messageColumn.scrollHeight;
				}
			});
		}
	});

	function removeBubble(id: string) {
		visible = visible.filter(b => b.id !== id);
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter" && !e.shiftKey) {
			e.preventDefault();
			const trimmed = inputValue.trim();
			if (trimmed) {
				onSend?.(trimmed);
				inputValue = "";
				setTimeout(() => { inputVisible = false; }, 300);
			}
		}
		if (e.key === "Escape") {
			if (thinking) onStop?.();
			inputVisible = false;
			inputEl?.blur();
		}
	}

	function showInput() {
		inputVisible = true;
		requestAnimationFrame(() => inputEl?.focus({ preventScroll: true }));
	}

	function handleGlobalKey(e: KeyboardEvent) {
		if (e.metaKey || e.ctrlKey || e.altKey) return;
		if (e.key === "Escape") return;
		if (e.key.length === 1 && !inputVisible) {
			showInput();
		}
	}
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div class="present-root" onclick={showInput} onkeydown={handleGlobalKey} role="application">
	<div class="present-column" bind:this={messageColumn}>
		<div class="present-spacer"></div>
		{#each visible as bubble (bubble.id)}
			{@const msg = stream.find(s => s.type === "message" && s.data.id === bubble.id)}
			{#if msg && msg.type === "message"}
				<PresentationBubble
					message={msg.data}
					side={bubble.side}
					streaming={msg.data.id === streamingMessageId}
					onexpire={() => removeBubble(bubble.id)}
				/>
			{/if}
		{/each}

		{#if thinking && visible.length === 0}
			<div class="present-thinking">
				<div class="pt-dot" style="animation-delay: 0ms"></div>
				<div class="pt-dot" style="animation-delay: 200ms"></div>
				<div class="pt-dot" style="animation-delay: 400ms"></div>
			</div>
		{/if}
	</div>

	<div class="present-bar" class:present-bar-visible={inputVisible}>
		<div class="present-bar-glass">
			<div class="present-bar-glow"></div>
			<textarea
				bind:this={inputEl}
				bind:value={inputValue}
				onkeydown={handleKeydown}
				onfocus={(e) => { e.currentTarget.scrollTop = 0; }}
				onblur={() => { if (!inputValue) inputVisible = false; }}
				class="present-textarea"
				placeholder="..."
				rows={1}
				autocomplete="off"
				aria-label="Chat input"
			></textarea>
		</div>
	</div>
</div>

<style>
	.present-root {
		position: fixed;
		inset: 0;
		z-index: 200;
		overflow: hidden;
		overscroll-behavior: none;
		touch-action: none;
		display: flex;
		flex-direction: column;
	}

	/* Bottom-anchored message column — no overlap, ever */
	.present-column {
		position: relative;
		z-index: 10;
		flex: 1;
		display: flex;
		flex-direction: column;
		gap: 1.2vh;
		padding: 3vh 6vw 14vh;
		overflow-y: auto;
		overflow-x: hidden;
		pointer-events: none;
		/* Hide scrollbar */
		scrollbar-width: none;
	}
	.present-column::-webkit-scrollbar {
		display: none;
	}

	/* Push messages to the bottom */
	.present-spacer {
		flex: 1;
	}

	.present-thinking {
		display: flex;
		gap: 0.6rem;
		align-self: center;
		padding: 1rem 0;
	}

	.pt-dot {
		width: 8px;
		height: 8px;
		border-radius: 50%;
		background: var(--card);
		box-shadow: none;
		animation: pt-bounce 1.4s ease-in-out infinite;
	}

	@keyframes pt-bounce {
		0%, 60%, 100% { transform: translateY(0); opacity: 0.3; }
		30% { transform: translateY(-10px); opacity: 1; }
	}

	/* --- Input bar --- */

	.present-bar {
		position: absolute;
		bottom: 0;
		left: 0;
		right: 0;
		z-index: 20;
		padding: 2vh 8vw 4vh;
		transform: translateY(100%);
		opacity: 0;
		transition:
			transform 0.6s cubic-bezier(0.16, 1, 0.3, 1),
			opacity 0.4s ease;
		pointer-events: none;
	}

	.present-bar-visible {
		transform: translateY(0);
		opacity: 1;
		pointer-events: auto;
	}

	.present-bar-glass {
		position: relative;
		max-width: 800px;
		margin: 0 auto;
		overflow: hidden;
		border-radius: 20px;
		background: var(--card);


		border: 1px solid var(--border);
		box-shadow: none;
	}

	.present-bar-glow {
		position: absolute;
		bottom: -20px;
		left: 50%;
		transform: translateX(-50%);
		width: 60%;
		height: 40px;
		border-radius: 50%;
		background: var(--card);
		animation: bar-glow-pulse 3s ease-in-out infinite;
		pointer-events: none;
	}

	@keyframes bar-glow-pulse {
		0%, 100% { opacity: 0.5; width: 60%; }
		50% { opacity: 1; width: 75%; }
	}

	.present-textarea {
		display: block;
		width: 100%;
		padding: 1.2rem 1.6rem;
		font-family: var(--font-body);
		font-size: clamp(1.2rem, 2.2vw, 1.6rem);
		line-height: 1.5;
		color: var(--text-secondary);
		background: transparent;
		border: none;
		outline: none;
		resize: none;
		caret-color: var(--primary);
	}

	.present-textarea::placeholder {
		color: var(--text-secondary);
		font-style: normal;
		font-family: var(--font-display);
	}

 .pt-dot {background:var(--primary)}
 .present-bar-glow {display:none}
 .present-bar-glass {border-radius:12px}
 .present-bar-glass:focus-within {border-color:var(--ring);outline:2px solid var(--ring);outline-offset:2px}
 .present-textarea {color:var(--foreground);caret-color:var(--primary)}
 .present-textarea::placeholder {color:var(--text-muted);opacity:1;font-family:var(--font-body)}

</style>
