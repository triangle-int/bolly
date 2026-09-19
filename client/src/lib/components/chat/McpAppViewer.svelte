<script lang="ts">
	import Maximize2 from "@lucide/svelte/icons/maximize-2";
	import ChevronDown from "@lucide/svelte/icons/chevron-down";
	import ChevronUp from "@lucide/svelte/icons/chevron-up";
	import X from "@lucide/svelte/icons/x";
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

		// The frame is an opaque origin (sandbox="allow-scripts", no same-origin):
		// content goes in through srcdoc and the only channel is postMessage.
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
			// Untrusted content may only ask the host to open web links.
			if (/^https?:\/\//i.test(params.url)) {
				window.open(params.url, "_blank", "noopener,noreferrer");
			}
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
				<button class="mcp-app-btn" onclick={enterFullscreen} aria-label="Fullscreen" title="Fullscreen"><Maximize2 size={18} /></button>
			{/if}
			<button class="mcp-app-btn" onclick={() => collapsed = !collapsed} title={collapsed ? "Expand" : "Collapse"}>
				{#if collapsed}<ChevronDown size={18} />{:else}<ChevronUp size={18} />{/if}
			</button>
		</div>
	</div>
	{#if !collapsed}
		{#if fullscreen}
			<button class="mcp-fs-close" onclick={exitFullscreen}><X size={18} /> Close fullscreen</button>
		{/if}
		<iframe
			bind:this={iframe}
			sandbox="allow-scripts"
			referrerpolicy="no-referrer"
			srcdoc={`<style>:root{color-scheme:dark}</style>${html}`}
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
			background: color-mix(in srgb, var(--background) 75%, transparent);
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
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--text-secondary);
		letter-spacing: 0.06em;
		text-transform: none;
	}

	.mcp-app-controls {
		display: flex;
		gap: 0.25rem;
	}

	.mcp-app-btn {
		background: none;
		border: 1px solid var(--border);
		border-radius: 4px;
		color: var(--text-secondary);
		font-size: 0.75rem;
		cursor: pointer;
		padding: 0 0.35rem;
		line-height: 1.4;
		font-family: var(--font-body);
	}

	.mcp-app-btn:hover {
		color: var(--text-secondary);
		border-color: var(--border);
	}

	.mcp-app-frame {
		width: 100%;
		height: 480px;
		border: 1px solid var(--border);
		border-radius: 8px;
		background: var(--card);
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
		background: var(--popover);
		border: none;
		border-bottom: 1px solid var(--border);
		color: var(--foreground);
		font-size: 0.85rem;
		font-family: var(--font-body);
		cursor: pointer;
		display: flex;
		align-items: center;
		justify-content: center;
		gap: 0.5rem;
	}

	.mcp-fs-close:hover {
		background: var(--accent);
		color: var(--foreground);
	}

	.collapsed .mcp-app-header {
		margin-bottom: 0;
	}

 .mcp-app-btn {min-height:44px;min-width:44px;border-radius:8px;padding:8px 12px;background:var(--card);color:var(--foreground)}
 .mcp-app-btn:hover {background:var(--accent)}
 .mcp-app-label {font-size:13px;letter-spacing:0;color:var(--text-secondary)}
 .mcp-fs-close {height:44px;background:var(--popover);border-color:var(--border);color:var(--foreground);font-family:var(--font-body)}
 .mcp-fs-close:hover {background:var(--accent);color:var(--foreground)}
 .mcp-app-frame.fs {inset:44px 0 0;height:calc(100dvh - 44px)}

</style>
