<script lang="ts">
	import { AppBridge, PostMessageTransport } from "@modelcontextprotocol/ext-apps/app-bridge";

	let {
		html,
		toolName,
		toolInput,
		toolOutput = "",
	}: {
		html: string;
		toolName: string;
		toolInput: string;
		toolOutput?: string;
	} = $props();

	let iframe: HTMLIFrameElement | undefined = $state();
	let ready = $state(false);
	let fullscreen = $state(false);
	let collapsed = $state(false);
	let bridgeRef: AppBridge | undefined = $state();
	let resultSent = false;
	let inputFinalized = false;
	let lastPartialInput = "";
	let backdropEl: HTMLDivElement | undefined;

	function enterFullscreen() {
		if (fullscreen) return;
		fullscreen = true;
		bridgeRef?.sendHostContextChange({ displayMode: "fullscreen" });
		// Add backdrop to body
		backdropEl = document.createElement("div");
		backdropEl.className = "mcp-fs-backdrop";
		backdropEl.addEventListener("click", exitFullscreen);
		document.body.appendChild(backdropEl);
		// Prevent body scroll
		document.body.style.overflow = "hidden";
	}

	function exitFullscreen() {
		if (!fullscreen) return;
		fullscreen = false;
		bridgeRef?.sendHostContextChange({ displayMode: "inline" });
		backdropEl?.remove();
		backdropEl = undefined;
		document.body.style.overflow = "";
	}

	function sendResult(bridge: AppBridge, output: string) {
		if (resultSent || !output) return;
		resultSent = true;
		try {
			const parsed = JSON.parse(output);
			if (parsed && Array.isArray(parsed.content)) {
				bridge.sendToolResult(parsed);
			} else {
				bridge.sendToolResult({
					content: [{ type: "text", text: typeof parsed === "string" ? parsed : JSON.stringify(parsed) }],
				});
			}
		} catch {
			bridge.sendToolResult({
				content: [{ type: "text", text: output }],
			});
		}
	}

	// Stream partial tool input as it arrives (for live drawing animation)
	$effect(() => {
		if (toolInput && ready && bridgeRef && !inputFinalized && toolInput !== lastPartialInput) {
			lastPartialInput = toolInput;
			try {
				const partial = JSON.parse(toolInput);
				bridgeRef.sendToolInputPartial({ arguments: partial });
			} catch {
				// Partial JSON not yet valid — skip
			}
		}
	});

	$effect(() => {
		if (toolOutput && ready && bridgeRef && !resultSent) {
			// Finalize input before sending result
			if (!inputFinalized) {
				inputFinalized = true;
				let args: Record<string, unknown> = {};
				try { args = JSON.parse(toolInput); } catch {}
				bridgeRef.sendToolInput({ arguments: args });
			}
			sendResult(bridgeRef, toolOutput);
		}
	});

	$effect(() => {
		if (!iframe) return;

		const doc = iframe.contentDocument;
		if (!doc) return;
		doc.open();
		doc.write(html);
		doc.close();

		try {
			const style = doc.createElement("style");
			style.textContent = `:root { color-scheme: dark; }`;
			doc.head?.appendChild(style);
		} catch {};

		const iframeWindow = iframe.contentWindow!;

		const bridge = new AppBridge(
			null,
			{ name: "nolune", version: "1.0.0" },
			{ openLinks: {} },
			{
				hostContext: {
					theme: "dark",
					platform: "web",
					containerDimensions: { maxHeight: 600 },
					displayMode: "inline",
					availableDisplayModes: ["inline", "fullscreen"],
				},
			},
		);
		bridgeRef = bridge;
		resultSent = false;
		inputFinalized = false;
		lastPartialInput = "";

		bridge.oninitialized = () => {
			ready = true;
			// Only send final input if we have the result (page reload) or the tool already finished
			if (toolOutput || inputFinalized) {
				let args: Record<string, unknown> = {};
				try { args = JSON.parse(toolInput); } catch {}
				inputFinalized = true;
				bridge.sendToolInput({ arguments: args });
				if (toolOutput) sendResult(bridge, toolOutput);
			}
		};

		bridge.onsizechange = ({ width, height }) => {
			if (!iframe || fullscreen) return;
			if (height !== undefined) iframe.style.height = `${Math.min(height, 600)}px`;
			if (width !== undefined) iframe.style.minWidth = `min(${width}px, 100%)`;
		};

		bridge.onrequestdisplaymode = async (params) => {
			if (params.mode === "fullscreen") enterFullscreen();
			else exitFullscreen();
			return { mode: params.mode === "fullscreen" ? "fullscreen" : "inline" };
		};

		bridge.onopenlink = async (params) => {
			window.open(params.url, "_blank", "noopener,noreferrer");
			return {};
		};

		const transport = new PostMessageTransport(iframeWindow, iframeWindow);
		bridge.connect(transport);

		return () => {
			exitFullscreen();
			bridgeRef = undefined;
			bridge.close();
		};
	});
</script>

<div class="mcp-app" class:collapsed>
	<div class="mcp-app-header">
		<span class="mcp-app-label">{toolName}</span>
		<div class="mcp-app-controls">
			{#if !collapsed}
				<button class="mcp-app-btn" onclick={enterFullscreen} title="Fullscreen">⊞</button>
			{/if}
			<button class="mcp-app-btn" onclick={() => collapsed = !collapsed} title={collapsed ? "Expand" : "Collapse"}>
				{collapsed ? "+" : "−"}
			</button>
		</div>
	</div>
	{#if !collapsed}
		{#if fullscreen}
			<button class="mcp-fs-close" onclick={exitFullscreen}>✕ close</button>
		{/if}
		<iframe
			bind:this={iframe}
			sandbox="allow-scripts allow-same-origin"
			title={toolName}
			class="mcp-app-frame"
			class:loaded={ready}
			class:fs={fullscreen}
		></iframe>
	{/if}
</div>

<svelte:head>
	{@html `<style>
		.mcp-fs-backdrop {
			position: fixed;
			inset: 0;
			z-index: 99998;
			background: rgba(0,0,0,0.7);
		}
	</style>`}
</svelte:head>

<style>
	.mcp-app {
		max-width: 100%;
		margin: 0.5rem 0;
		animation: app-enter 0.45s cubic-bezier(0.16, 1, 0.3, 1) both;
		position: relative;
	}

	@keyframes app-enter {
		from { opacity: 0; transform: translateY(6px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.mcp-app-header {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 0.35rem;
	}

	.mcp-app-label {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: oklch(0.78 0.12 75 / 40%);
		letter-spacing: 0.06em;
		text-transform: lowercase;
	}

	.mcp-app-controls {
		display: flex;
		gap: 0.25rem;
	}

	.mcp-app-btn {
		background: none;
		border: 1px solid oklch(0.78 0.12 75 / 15%);
		border-radius: 4px;
		color: oklch(0.6 0 0);
		font-size: 0.75rem;
		cursor: pointer;
		padding: 0 0.35rem;
		line-height: 1.4;
		font-family: var(--font-mono);
	}

	.mcp-app-btn:hover {
		color: oklch(0.85 0 0);
		border-color: oklch(0.78 0.12 75 / 30%);
	}

	.mcp-app-frame {
		width: 100%;
		height: 480px;
		border: 1px solid oklch(0.78 0.12 75 / 10%);
		border-radius: 8px;
		background: oklch(0.10 0.01 280);
		opacity: 0;
		transition: opacity 0.3s ease;
		display: block;
	}

	.mcp-app-frame.loaded {
		opacity: 1;
	}

	/* Fullscreen: position:fixed on the iframe itself escapes stacking context */
	.mcp-app-frame.fs {
		position: fixed;
		inset: 2.5rem 0 0 0;
		z-index: 99999;
		width: 100%;
		height: calc(100vh - 2.5rem);
		border: none;
		border-radius: 0;
	}

	.mcp-fs-close {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		z-index: 100000;
		height: 2.5rem;
		background: #1a1a1a;
		border: none;
		border-bottom: 1px solid #333;
		color: #eee;
		font-size: 0.85rem;
		font-family: var(--font-mono);
		cursor: pointer;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.5rem;
	}

	.mcp-fs-close:hover {
		background: #c44;
		color: #fff;
	}

	.collapsed .mcp-app-header {
		margin-bottom: 0;
	}
</style>
