<script lang="ts">
	import Conversation from "$lib/components/ai-elements/conversation/conversation.svelte";
	import ConversationContent from "$lib/components/ai-elements/conversation/conversation-content.svelte";
	import { untrack } from "svelte";
	import { goto } from "$app/navigation";
	import { clearContext, fetchChats, fetchCompanionName, fetchMessages, fetchMood, sendMessage, stopAgent, uploadFile } from "$lib/api/client.js";
	import type { ChatMessage, ChatSummary, ServerEvent } from "$lib/api/types.js";
	import { getWebSocket } from "$lib/stores/websocket.svelte.js";
	import MessageBubble from "./MessageBubble.svelte";
	import ChatInput from "./ChatInput.svelte";
	import PresentationOverlay from "./PresentationOverlay.svelte";
	import { getPresentationState } from "$lib/stores/presentation.svelte.js";
	import CreatureBubble from "./CreatureBubble.svelte";
	import StreamActivity from "./StreamActivity.svelte";
	import ContextStats from "./ContextStats.svelte";
	import HeartbeatUpdateBanner from "./HeartbeatUpdateBanner.svelte";
import McpAppViewer from "./McpAppViewer.svelte";
	import { play, playImmediate, preload } from "$lib/sounds.js";
	import { hapticMedium, hapticDouble, hapticError } from "$lib/haptics.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { getVoiceState } from "$lib/stores/voice.svelte.js";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { playBase64Audio, stopTts, warmUpAudio, clearAudioQueue } from "$lib/tts.js";
	import * as AlertDialog from "$lib/components/ui/alert-dialog/index.js";
	import TerminalSquare from "@lucide/svelte/icons/terminal-square";
	import BarChart3 from "@lucide/svelte/icons/bar-chart-3";
	import Eraser from "@lucide/svelte/icons/eraser";
	import Minimize2 from "@lucide/svelte/icons/minimize-2";
	import Volume2 from "@lucide/svelte/icons/volume-2";
	import VolumeOff from "@lucide/svelte/icons/volume-off";

	const toast = getToasts();
	const voice = getVoiceState();
	const presentation = getPresentationState();
	const scene = getSceneStore();

	let { slug, chatId }: { slug: string; chatId: string } = $props();

	type StreamItem =
		| { type: "message"; data: ChatMessage }
		| { type: "activity"; id: string; kind: "tool" | "mood" | "state" | "output"; label: string; timestamp: string }
| { type: "mcp_app"; id: string; toolName: string; toolInput: string; toolOutput: string; html: string }
		| { type: "compaction"; id: string; count: number; timestamp: string };

	let activeChatId = $derived(chatId);
	let chats = $state<ChatSummary[]>([]);
	let companionName = $state("");
	let messages = $state<ChatMessage[]>([]);
	let stream = $state<StreamItem[]>([]);
	let loading = $state(true);
	let historyError = $state("");
	let historyRequest = 0;
	let sending = $state(false);
	let agentRunning = $state(false);

	const savedMood = typeof localStorage !== "undefined" ? localStorage.getItem("mood:" + untrack(() => slug)) : null;
	let mood = $state(savedMood || "calm");
	let scrollContainer: HTMLDivElement | null = $state(null);
	let isConnected = $state(false);
	let showChatList = $state(false);
	let clearDialogOpen = $state(false);
	let showContextStats = $state(false);
	let showToolActivity = $state(
		typeof localStorage !== "undefined"
			? (localStorage.getItem("nolune:showToolActivity") ?? "false") === "true"
			: false,
	);
	let streamingMessageId = $state("");
	/** Accumulates streamed text for TTS when voice is enabled. */
	let voiceText = $state("");
	/** Message IDs from the current agent turn (for voice reveal). */
	let turnMessageIds = $state<string[]>([]);

	preload("message_receive", "message_send", "error");

	function handleStreamDelta(messageId: string, delta: string) {
		if (voice.enabled) {
			voiceText += delta;
			if (!turnMessageIds.includes(messageId)) {
				turnMessageIds = [...turnMessageIds, messageId];
			}
			scrollToBottomIfNear();
			return;
		}

		streamingMessageId = messageId;

		// Find or create the message bubble with this stable ID
		const existingIdx = stream.findIndex(
			s => s.type === "message" && s.data.id === messageId
		);

		if (existingIdx >= 0) {
			const item = stream[existingIdx] as { type: "message"; data: ChatMessage };
			item.data.content += delta;
			stream = stream; // trigger reactivity
		} else {
			const msg: ChatMessage = {
				id: messageId,
				role: "assistant",
				content: delta,
				created_at: String(Date.now()),
			};
			stream = [...stream, { type: "message", data: msg }];
		}

		scrollToBottomIfNear();
	}

	function clearStreaming() {
		streamingMessageId = "";
	}

	// ── Sync state to shared 3D scene ──
	// Keep mood/thinking/voice synced to scene store
	$effect(() => { scene.setMood(mood); });
	$effect(() => { scene.setThinking(sending || agentRunning); });
	$effect(() => { scene.setVoiceAmplitude(voice.amplitude); });
	// Sync presentation mode to scene (camera targets blob)
	$effect(() => { scene.presenting = presentation.active; });

	const ws = getWebSocket();
	let hadConnection = false;

	// Reload full chat after WebSocket reconnect to pick up missed messages
	$effect(() => {
		const isConnected = ws.connected;
		untrack(() => {
			if (!isConnected) return;
			if (!hadConnection) {
				// First connection — skip, the main load effect handles this
				hadConnection = true;
				return;
			}
			// Reconnected — re-fetch to pick up messages we missed
			fetchMessages(slug, chatId)
				.then((res) => {
					historyError = "";
					messages = res.messages.filter((m) => !isToolActivity(m));
					stream = messagesToStream(res.messages);
					agentRunning = res.agent_running;
					if (agentRunning) pushActivity("state", "thinking...");
					scrollToBottomIfNear();
				})
				.catch(() => {});
		});
	});

	/** Tracks whether the user has intentionally scrolled away from the bottom. */
	let userScrolledUp = false;
	let programmaticScroll = false;

	function handleScroll() {
		if (programmaticScroll) return;
		if (!scrollContainer) return;
		const { scrollTop, scrollHeight, clientHeight } = scrollContainer;
		const nearBottom = scrollHeight - scrollTop - clientHeight < 150;
		if (nearBottom) {
			userScrolledUp = false;
		} else {
			userScrolledUp = true;
		}
	}

	/** Always scroll to bottom (used after sending a message / initial load). */
	function scrollToBottom() {
		userScrolledUp = false;
		requestAnimationFrame(() => {
			if (scrollContainer) {
				programmaticScroll = true;
				scrollContainer.scrollTop = scrollContainer.scrollHeight;
				programmaticScroll = false;
			}
		});
	}

	/** Scroll to bottom only if the user hasn't scrolled away. */
	function scrollToBottomIfNear() {
		if (!userScrolledUp) scrollToBottom();
	}

	function now() {
		return new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
	}

	function pushActivity(kind: "tool" | "mood" | "state" | "output", label: string, idPrefix?: string) {
		// Dedup: skip if the last activity of the same kind has the same label
		const last = stream.findLast((s) => s.type === "activity" && s.kind === kind);
		if (last && last.type === "activity" && last.label === label) return;
		stream = [...stream, {
			type: "activity",
			id: `${idPrefix ?? ""}${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
			kind,
			label,
			timestamp: now(),
		}];
		scrollToBottomIfNear();
	}

	function addMessage(msg: ChatMessage) {
		// Check if this message already exists (was created during streaming)
		const existingIdx = stream.findIndex(
			s => s.type === "message" && s.data.id === msg.id
		);

		if (existingIdx >= 0) {
			// Message already exists — update metadata from server (model, timestamp, content)
			const item = stream[existingIdx] as { type: "message"; data: ChatMessage };
			item.data.created_at = msg.created_at;
			item.data.model = msg.model;
			item.data.kind = msg.kind;
			item.data.content = msg.content; // server version is authoritative
			stream = stream;
		} else {
			// New message — add to stream
			stream = [...stream, { type: "message", data: msg }];
			scrollToBottomIfNear();
		}

		if (!messages.some(m => m.id === msg.id)) {
			messages = [...messages, msg];
		}
	}

	/** Reconcile local state with a server snapshot — game-style sync. */
	function reconcileSnapshot(serverMessages: ChatMessage[], serverAgentRunning: boolean) {
		historyError = "";
		const serverStream = messagesToStream(serverMessages);

		// Detect new assistant messages not in local stream (by content, not ID —
		// rig_history may assign different IDs than streaming)
		const localContents = new Set(
			stream.filter(s => s.type === "message")
				.map(s => (s as { type: "message"; data: ChatMessage }).data.content)
		);
		let hasNew = false;
		for (const item of serverStream) {
			if (item.type !== "message") continue;
			const msg = (item as { type: "message"; data: ChatMessage }).data;
			if (msg.role === "assistant" && !localContents.has(msg.content)) {
				hasNew = true;
				break;
			}
		}

		if (hasNew && !voice.enabled) {
			play("message_receive");
			hapticMedium();
		}

		// Snapshot is ground truth — replace stream entirely.
		// Don't try to preserve streaming items (they may duplicate
		// server messages that have different IDs but same content).
		stream = serverStream;
		streamingMessageId = "";
		messages = serverMessages.filter((m) => !isToolActivity(m));
		agentRunning = serverAgentRunning;

		scrollToBottomIfNear();
	}

	function isToolActivity(msg: ChatMessage): boolean {
		if (msg.kind === "tool_call" || msg.kind === "tool_output" || msg.kind === "mcp_app" || msg.kind === "compaction") return true;
		if (msg.content.startsWith("[restart]")) return true;
		return msg.role === "assistant" && (
			msg.content.startsWith("[tool activity]") ||
			msg.content.startsWith("[tool:") ||
			msg.content.startsWith("[system]")
		);
	}

	function toolActivityToStreamItem(msg: ChatMessage): StreamItem | null {
		const ts = new Date(Number(msg.created_at)).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

		if (msg.kind === "compaction") {
			return {
				type: "compaction" as const,
				id: msg.id,
				count: 0,
				timestamp: ts,
			};
		}

		if (msg.kind === "tool_call" || msg.kind === "tool_output") {
			if (msg.tool_name === "set_mood") return null;
			return {
				type: "activity" as const,
				id: msg.id,
				kind: msg.kind === "tool_output" ? "output" as const : "tool" as const,
				label: msg.content,
				timestamp: ts,
			};
		}
		if (msg.content.startsWith("[tool:")) {
			if (msg.content.startsWith("[tool: set_mood]")) return null;
			const isOutput = msg.content.includes(" output]");
			return {
				type: "activity" as const,
				id: msg.id,
				kind: isOutput ? "output" as const : "tool" as const,
				label: msg.content.replace(/^\[tool:[^\]]*\]\s*/, ""),
				timestamp: ts,
			};
		}
		if (msg.content.startsWith("[system]") || msg.content.startsWith("[restart]")) {
			return {
				type: "activity" as const,
				id: msg.id,
				kind: "state" as const,
				label: msg.content.replace(/^\[(system|restart)\]\s*/, ""),
				timestamp: ts,
			};
		}
		// Legacy [tool activity] format — render as single activity item
		return {
			type: "activity" as const,
			id: msg.id,
			kind: "tool" as const,
			label: msg.content.replace(/^\[tool activity\]\s*/, ""),
			timestamp: ts,
		};
	}

	function messagesToStream(msgs: ChatMessage[]): StreamItem[] {
		return msgs.flatMap((m) => {
			if (m.kind === "mcp_app" && m.mcp_app_html && m.tool_name) {
				return [{
					type: "mcp_app" as const,
					id: m.id,
					toolName: m.tool_name,
					toolInput: m.mcp_app_input ?? "{}",
					toolOutput: m.content,
					html: m.mcp_app_html,
				}];
			}
			if (isToolActivity(m)) {
				const item = toolActivityToStreamItem(m);
				return item ? [item] : [];
			}
			return [{ type: "message" as const, data: m }];
		});
	}

	function loadChat(id: string) {
		showChatList = false;
		const path = id === "default" ? `/${slug}/chat` : `/${slug}/chat/${id}`;
		goto(path);
	}

	function refreshChatList() {
		fetchChats(slug)
			.then((res) => { chats = res; })
			.catch(() => {});
	}

	function newChat() {
		const id = `chat_${Date.now()}`;
		loadChat(id);
		// Will be created server-side on first message
		refreshChatList();
	}

	async function loadHistory(currentSlug = slug, currentChat = chatId) {
		const request = ++historyRequest;
		loading = true;
		historyError = "";
		try {
			const res = await fetchMessages(currentSlug, currentChat);
			if (request !== historyRequest || currentSlug !== slug || currentChat !== chatId) return;
			messages = res.messages.filter((m) => !isToolActivity(m));
			stream = messagesToStream(res.messages);
			agentRunning = res.agent_running;
			if (agentRunning) pushActivity("state", "thinking...");
			scrollToBottom();
		} catch (error) {
			if (request !== historyRequest || currentSlug !== slug || currentChat !== chatId) return;
			historyError = error instanceof Error && error.message === "unauthorized"
				? "Sign in again to load this conversation."
				: "We couldn’t load this conversation. Check your connection and model provider settings, then try again.";
		} finally {
			if (request === historyRequest && currentSlug === slug && currentChat === chatId) loading = false;
		}
	}

	$effect(() => {
		const currentSlug = slug;
		const currentChat = chatId;
		untrack(() => {
			messages = [];
			stream = [];
			loading = true;
			isConnected = false;
			streamingMessageId = "";

			refreshChatList();

			void loadHistory(currentSlug, currentChat);

			fetchMood(currentSlug)
				.then((res) => { if (res.mood) { mood = res.mood; localStorage.setItem("mood:" + currentSlug, res.mood); } })
				.catch(() => {}); // mood is non-critical

			fetchCompanionName(currentSlug)
				.then((res) => { if (res.name) companionName = res.name; })
				.catch(() => {}); // name is non-critical


		});

		const unsub = ws.subscribe((event: ServerEvent) => {
			isConnected = true;

			// Server says we missed events — re-fetch everything
			if ((event as any).type === "resync") {
				fetchMessages(currentSlug, currentChat)
					.then((res) => {
						reconcileSnapshot(res.messages, res.agent_running);
					})
					.catch(() => {});
				return;
			}

			if (event.instance_slug !== currentSlug) return;

			// Filter chat-specific events by chat_id
			const eventChatId = "chat_id" in event ? event.chat_id : undefined;
			if (eventChatId && eventChatId !== currentChat) return;

			if (event.type === "chat_message_created") {
				const msg = event.message;
				if (msg.kind === "mcp_app" && msg.mcp_app_html && msg.tool_name) {
					stream = [...stream, {
						type: "mcp_app" as const,
						id: msg.id,
						toolName: msg.tool_name,
						toolInput: msg.mcp_app_input ?? "{}",
						toolOutput: msg.content,
						html: msg.mcp_app_html,
					}];
					scrollToBottomIfNear();
				} else if (isToolActivity(msg)) {
					const item = toolActivityToStreamItem(msg);
					if (item) {
						// If this is a tool_output and we have a live-streamed output,
						// promote the live output (keep full content) and skip the truncated summary
						if (item.type === "activity" && item.kind === "output") {
							const liveIdx = stream.findLastIndex(
								(s) => s.type === "activity" && s.kind === "output" && s.id.startsWith("__live_")
							);
							if (liveIdx >= 0) {
								const live = stream[liveIdx] as typeof item;
								live.id = item.id;
								stream = stream;
							} else {
								stream = [...stream, item];
							}
						} else {
							stream = [...stream, item];
						}
					}
					scrollToBottomIfNear();
				} else {
					if (msg.role === "assistant") {
						if (voice.enabled) {
							turnMessageIds = [...turnMessageIds, msg.id];
						} else {
							play("message_receive"); hapticMedium();
						}
					}
					addMessage(msg);
					refreshChatList();
				}
			} else if (event.type === "mood_updated") {
				play("mood_shift");
				hapticDouble();
				mood = event.mood;
				localStorage.setItem("mood:" + slug, event.mood);
				pushActivity("mood", `mood → ${event.mood}`);
			} else if (event.type === "agent_running") {
				agentRunning = true;
				pushActivity("state", "thinking...");
			} else if (event.type === "chat_audio_ready") {
				// Server-generated TTS audio — queue for sequential playback
				const ids = event.message_ids;
				turnMessageIds = turnMessageIds.filter(id => !ids.includes(id));
				playBase64Audio(event.audio_base64, voice, ids);
			} else if (event.type === "agent_stopped") {
				agentRunning = false;
				sending = false;
				clearStreaming();
				// Fade out recalled memories after a delay
				setTimeout(() => { scene.recalledMemories = []; }, 6000);
				// Don't clear turnMessageIds immediately — TTS audio may still be
				// synthesizing. Give it time to arrive so word reveal works.
				// Fallback: if audio hasn't arrived after 15s, reveal all.
				if (voice.enabled && turnMessageIds.length > 0) {
					const pending = [...turnMessageIds];
					setTimeout(() => {
						// Only clear IDs that still haven't received audio
						const stillPending = turnMessageIds.filter(id => pending.includes(id));
						if (stillPending.length > 0) {
							turnMessageIds = turnMessageIds.filter(id => !pending.includes(id));
						}
					}, 15000);
				}
				voiceText = "";
			} else if (event.type === "tool_activity") {
				if (event.summary.startsWith("mood →")) return;
				const isOutput = event.tool_name.endsWith("_output");
				pushActivity(isOutput ? "output" : "tool", event.summary);
			} else if (event.type === "drop_created") {
				pushActivity("tool", `dropped: ${event.drop.title}`);
				play("drop_received");
				hapticDouble();
			} else if (event.type === "memory_recall") {
				scene.recalledMemories = event.memories.map((m: any) => ({
					path: m.path,
					preview: m.preview,
					score: m.score,
				}));
				// Auto-hide after agent finishes (handled by agent_stopped event);
			} else if (event.type === "tool_output_chunk") {
				// Append chunk to live output activity, or create one
				const liveIdx = stream.findLastIndex(
					(s) => s.type === "activity" && s.kind === "output" && s.id.startsWith("__live_")
				);
				if (liveIdx >= 0) {
					const item = stream[liveIdx] as StreamItem & { type: "activity" };
					item.label += event.chunk;
					stream = stream;
				} else {
					pushActivity("output", event.chunk, "__live_");
				}
				scrollToBottomIfNear();
			} else if (event.type === "chat_stream_delta") {
				handleStreamDelta(event.message_id, event.delta);
			} else if (event.type === "mcp_app_start") {
				// MCP App tool call starting — show iframe immediately
				stream = [...stream, {
					type: "mcp_app",
					id: `mcp_live_${Date.now()}`,
					toolName: event.tool_name,
					toolInput: "",
					toolOutput: "",
					html: event.html,
				}];
				scrollToBottomIfNear();
			} else if (event.type === "mcp_app_input_delta") {
				// Append JSON delta to the live MCP App stream item
				const liveIdx = stream.findLastIndex((s) => s.type === "mcp_app" && s.id.startsWith("mcp_live_"));
				if (liveIdx >= 0) {
					const item = stream[liveIdx] as StreamItem & { type: "mcp_app" };
					item.toolInput += event.delta;
					stream = stream; // trigger reactivity
				}
			} else if (event.type === "mcp_app_result") {
				// Tool result arrived — update the matching mcp_app stream item
				// First try live item, then by message_id
				let idx = stream.findLastIndex((s) => s.type === "mcp_app" && s.id.startsWith("mcp_live_"));
				if (idx < 0) idx = stream.findIndex((s) => s.type === "mcp_app" && s.id === event.message_id);
				if (idx >= 0) {
					const item = stream[idx] as StreamItem & { type: "mcp_app" };
					item.toolOutput = event.tool_output;
					item.id = event.message_id; // promote to persisted id
					stream = stream; // trigger reactivity
				}
			} else if (event.type === "context_compacting") {
				stream = [...stream, {
					type: "compaction",
					id: `compact_${Date.now()}`,
					count: event.messages_compacted,
					timestamp: new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }),
				}];
				scrollToBottomIfNear();
			} else if ((event as any).type === "chat_snapshot") {
				const snap = event as any;
				reconcileSnapshot(snap.messages, snap.agent_running);
			}
		});
		return unsub;
	});

	let uploadProgress = $state<{ fileIndex: number; fileCount: number; loaded: number; total: number } | null>(null);

	async function handleSend(content: string, files?: File[]) {
		// Warm up AudioContext on user gesture (send is always click/Enter)
		if (voice.enabled) warmUpAudio();
		sending = true;
		try {
			// Upload files first, then reference them in the message
			let finalContent = content;
			if (files && files.length > 0) {
				const uploadResults = [];
				for (let i = 0; i < files.length; i++) {
					uploadProgress = { fileIndex: i, fileCount: files.length, loaded: 0, total: files[i].size };
					uploadResults.push(await uploadFile(slug, files[i], (loaded, total) => {
						uploadProgress = { fileIndex: i, fileCount: files.length, loaded, total };
					}));
				}
				uploadProgress = null;
				const refs = uploadResults
					.map((u) => `[attached: ${u.original_name} (${u.id})]`)
					.join("\n");
				finalContent = finalContent ? `${finalContent}\n\n${refs}` : refs;
			}
			// Stop any playing TTS when sending a new message
			if (voice.speaking) stopTts(voice);
			clearAudioQueue();
			voiceText = "";
			turnMessageIds = [];
			const res = await sendMessage(slug, finalContent, activeChatId, voice.enabled);
			for (const msg of res.messages) addMessage(msg);
			return true;
		} catch (e) {
			play("error");
			hapticError();
			sending = false;
			uploadProgress = null;
			const msg = e instanceof Error ? e.message : "failed to send";
			if (msg.includes("rate limit")) {
				try {
					const parsed = JSON.parse(msg);
					toast.error(parsed.detail ?? "rate limited — try again later");
				} catch {
					toast.error("rate limited — try again later");
				}
			} else {
				toast.error(msg);
			}
		}
		return false;
	}

	async function handleStop() {
		await stopAgent(slug, activeChatId);
	}

	async function handleClear() {
		clearDialogOpen = false;
		await clearContext(slug, activeChatId);
		messages = [];
		stream = [];
	}

	/** Is this message waiting for or currently playing TTS? */
	function isVoiceMessage(msgId: string): boolean {
		return voice.enabled && (turnMessageIds.includes(msgId) || voice.speakingIds.has(msgId));
	}

	/** Compute per-message reveal progress.
	 *  - In turnMessageIds (waiting for audio): 0 (all words hidden)
	 *  - In speakingIds (audio playing): 0→1 based on playback
	 *  - Neither (done or no voice): 1 (fully visible) */
	function getMessageRevealProgress(msgId: string): number {
		// Waiting for audio — show nothing yet
		if (voice.enabled && turnMessageIds.includes(msgId) && !voice.speakingIds.has(msgId)) {
			return 0;
		}

		// Currently playing — distribute progress across speaking messages
		if (voice.speaking && voice.speakingIds.has(msgId)) {
			const speakingMsgs = stream
				.filter(s => s.type === "message" && voice.speakingIds.has(s.data.id))
				.map(s => (s as { type: "message"; data: ChatMessage }).data);

			const wordCounts = speakingMsgs.map(m => m.content.split(/\s+/).filter(w => w).length);
			const totalWords = wordCounts.reduce((a, b) => a + b, 0);
			if (totalWords === 0) return 1;

			const msgIndex = speakingMsgs.findIndex(m => m.id === msgId);
			if (msgIndex < 0) return 1;

			const wordsBefore = wordCounts.slice(0, msgIndex).reduce((a, b) => a + b, 0);
			const wordsInMsg = wordCounts[msgIndex];
			const revealedWords = voice.revealProgress * totalWords;
			const localRevealed = Math.max(0, Math.min(wordsInMsg, revealedWords - wordsBefore));
			return localRevealed / wordsInMsg;
		}

		// Done or not a voice message
		return 1;
	}

	function streamKey(item: StreamItem): string {
		return item.type === "message" ? item.data.id : item.id;
	}

	function getPrev(item: StreamItem, index: number): ChatMessage | undefined {
		if (item.type !== "message") return undefined;
		for (let i = index - 1; i >= 0; i--) {
			if (stream[i].type === "message") return (stream[i] as { type: "message"; data: ChatMessage }).data;
		}
		return undefined;
	}

	function getNext(item: StreamItem, index: number): ChatMessage | undefined {
		if (item.type !== "message") return undefined;
		for (let i = index + 1; i < stream.length; i++) {
			if (stream[i].type === "message") return (stream[i] as { type: "message"; data: ChatMessage }).data;
		}
		return undefined;
	}

	function chatLabel(chat: ChatSummary): string {
		if (chat.title && chat.title !== "untitled") return chat.title;
		if (chat.id === "default") return "default";
		return chat.id.replace("chat_", "#");
	}

	function handlePresentationSend(content: string) {
		handleSend(content);
	}
</script>

<svelte:window onkeydown={(e) => {
	if ((e.metaKey || e.ctrlKey) && e.key === "p") {
		e.preventDefault();
		presentation.toggle();
	}
}} />

{#if presentation.active}
	<PresentationOverlay
		{stream}
		{mood}
		thinking={sending || agentRunning}
		voiceAmplitude={voice.amplitude}
		{streamingMessageId}
		onSend={handlePresentationSend}
		onStop={handleStop}
	/>
{:else}

<div class="chat-space" class:chat-active={sending || agentRunning} class:chat-intro-playing={scene.mode !== "chat"}>

	<header class="chat-bar">
		<div class="bar-left">
			<div class="bar-led" class:bar-led-on={isConnected}></div>
			<span class="bar-name">{companionName || slug}</span>
			<span class="bar-mood" data-mood={mood}>{mood}</span>
			{#if sending || agentRunning}
				<span class="bar-activity">
					<span class="bar-activity-dot"></span>
					working
				</span>
			{/if}
		</div>
		<div class="bar-right">
			<button onclick={() => { voice.toggle(); if (voice.enabled) warmUpAudio(); if (!voice.enabled && voice.speaking) stopTts(voice); }} onmousedown={(e) => e.preventDefault()} class="bar-btn" class:bar-btn-active={voice.enabled} title={voice.enabled ? "Mute voice" : "Enable voice"}>
				{#if voice.enabled}
					<Volume2 size={18} />
				{:else}
					<VolumeOff size={18} />
				{/if}
			</button>
			<button onclick={() => { showToolActivity = !showToolActivity; localStorage.setItem("nolune:showToolActivity", String(showToolActivity)); }} onmousedown={(e) => e.preventDefault()} class="bar-btn" class:bar-btn-active={showToolActivity} title="Toggle tool activity">
				<TerminalSquare size={18} />
			</button>
			<button onclick={() => showContextStats = true} onmousedown={(e) => e.preventDefault()} class="bar-btn" title="Context stats">
				<BarChart3 size={18} />
			</button>
			<AlertDialog.Root bind:open={clearDialogOpen}>
				<AlertDialog.Trigger class="bar-btn" title="Clear context">
					<Eraser size={18} />
				</AlertDialog.Trigger>
					<AlertDialog.Content class="clear-dialog">
						<AlertDialog.Header>
							<AlertDialog.Title class="clear-dialog-title">Clear context</AlertDialog.Title>
							<AlertDialog.Description class="clear-dialog-desc">
								This will erase all messages in this conversation. This cannot be undone.
							</AlertDialog.Description>
						</AlertDialog.Header>
						<AlertDialog.Footer class="clear-dialog-footer">
							<AlertDialog.Cancel class="clear-dialog-btn clear-dialog-cancel">Cancel</AlertDialog.Cancel>
							<AlertDialog.Action class="clear-dialog-btn clear-dialog-confirm" onclick={handleClear}>Clear messages</AlertDialog.Action>
						</AlertDialog.Footer>
					</AlertDialog.Content>
			</AlertDialog.Root>
		</div>
	</header>


	<!-- TODO: re-enable multi-chat when ready -->
	<!-- {#if showChatList}
		<div class="chat-list-overlay" onclick={() => showChatList = false} role="presentation"></div>
		<div class="chat-list">
			{#each chats as chat (chat.id)}
				<button
					class="chat-list-item"
					class:chat-list-active={chat.id === activeChatId}
					onclick={() => loadChat(chat.id)}
				>
					<span class="chat-list-label">{chatLabel(chat)}</span>
					<span class="chat-list-count">{chat.message_count}</span>
				</button>
			{:else}
				<div class="chat-list-empty">no chats yet</div>
			{/each}
		</div>
	{/if} -->

	<div class="chat-columns">
		<div class="chat-main">
			<Conversation class="chat-stream flex-1 overflow-y-auto" bind:ref={scrollContainer} onscroll={handleScroll} aria-label="Conversation">
				<ConversationContent autoScroll={false} class="mx-auto w-full max-w-[688px] gap-1 px-4 py-5 md:px-6">
					{#if historyError}
						<div class="history-error" role="alert">
							<p>{historyError}</p>
							<div class="history-error-actions">
								<button class="nl-button-secondary" disabled={loading || sending || agentRunning} onclick={() => loadHistory()}>Retry conversation</button>
								<a class="nl-button-secondary" href={`/${slug}/settings`}>Open settings</a>
							</div>
						</div>
					{/if}
					{#if loading}
						<div class="chat-loading" role="status"><div class="loading-dot" aria-hidden="true"></div><span>Loading conversation…</span></div>
					{:else if stream.length === 0 && !historyError}
						<div class="chat-empty"><p>What’s on your mind?</p></div>
					{:else}
						{#each stream as item, i (streamKey(item))}
							{#if item.type === "message"}
								<MessageBubble message={item.data} {slug} index={i} prevMessage={getPrev(item, i)} nextMessage={getNext(item, i)} speaking={isVoiceMessage(item.data.id)} revealProgress={getMessageRevealProgress(item.data.id)} streaming={item.data.id === streamingMessageId} />
							{:else if item.type === "mcp_app"}
								<McpAppViewer
									html={item.html}
									toolName={item.toolName}
									toolInput={item.toolInput}
									toolOutput={item.toolOutput}
								/>
							{:else if item.type === "compaction"}
								<div class="compaction-notice">
									<Minimize2 size={18} class="compaction-icon" />
									<span class="compaction-text">context compacted</span>
									<span class="compaction-time">{item.timestamp}</span>
								</div>
							{:else if item.kind === "mood" || showToolActivity}
								<StreamActivity kind={item.kind} label={item.label} timestamp={item.timestamp} />
							{/if}
						{/each}
					{/if}

					{#if sending || agentRunning}
						<div class="chat-thinking" role="status" aria-label="Nolune is thinking">
							<div class="think-dot" style="animation-delay: 0ms"></div>
							<div class="think-dot" style="animation-delay: 200ms"></div>
							<div class="think-dot" style="animation-delay: 400ms"></div>
						</div>
					{/if}
				</ConversationContent>
			</Conversation>

			<ChatInput onSend={handleSend} onStop={handleStop} disabled={sending || agentRunning} {agentRunning} {uploadProgress} />
		</div>

		<aside class="chat-sidebar">
			<div class="sidebar-banners">
				<HeartbeatUpdateBanner {slug} />

			</div>
		</aside>
	</div>

	<!-- Intro overlay UI (skip + name) -->
	{#if scene.mode === "intro" || scene.mode === "selecting"}
		<div class="intro-overlay-ui">
			{#if scene.introPhase === "settling"}
				<div class="intro-name">{slug}</div>
			{/if}
			{#if scene.mode === "intro"}
				<button class="intro-skip" onclick={() => scene.skipIntro()}>skip</button>
			{/if}
		</div>
	{/if}
</div>

{#if showContextStats}
	<ContextStats {slug} chatId={activeChatId} onclose={() => showContextStats = false} />
{/if}

{/if}<!-- end presentation else -->

<style>
	.chat-space {
		position: relative;
		display: flex;
		flex-direction: column;
		height: 100%;
		width: 100%;
		max-width: 100%;
		overflow: hidden;
	}

	/* --- Intro: hide all chat UI until scene is in chat mode --- */
	.chat-intro-playing > * {
		opacity: 0;
		pointer-events: none;
		transition: opacity 0.35s ease;
	}
	.chat-space:not(.chat-intro-playing) > * {
		opacity: 1;
		transition: opacity 0.35s ease;
	}

	/* --- Intro overlay UI --- */
	.intro-overlay-ui {
		position: absolute;
		inset: 0;
		z-index: 20;
		pointer-events: none;
	}
	.intro-name {
		position: absolute;
		bottom: 38%;
		left: 50%;
		transform: translateX(-50%);
		font-family: var(--font-display);
		font-size: clamp(1.5rem, 4vw, 2.5rem);
		font-weight: 300;
		font-style: normal;
		letter-spacing: 0.06em;
		color: var(--text-secondary);
		white-space: nowrap;
		animation: intro-name-in 1s cubic-bezier(0.16, 1, 0.3, 1) both;
	}
	@keyframes intro-name-in {
		from { opacity: 0; transform: translateX(-50%) translateY(10px); }
		to { opacity: 1; transform: translateX(-50%) translateY(0); }
	}
	.intro-skip {
		position: absolute;
		bottom: calc(2rem + env(safe-area-inset-bottom, 0px));
		right: 2rem;
		pointer-events: auto;
		padding: 0.4rem 1rem;
		border-radius: 2rem;
		background: var(--card);

		border: 1px solid var(--border);
		color: var(--text-secondary);
		font-family: var(--font-body);
		font-size: 0.8125rem;
		letter-spacing: 0.08em;
		cursor: pointer;
		transition: all 0.3s ease;
	}
	.intro-skip:hover { background: var(--card); color: var(--text-secondary); }

	/* --- Perimeter ambient glow (Siri-style) --- */
	.chat-space::before {
		content: "";
		position: absolute;
		inset: 0;
		pointer-events: none;
		z-index: 50;
		opacity: 0;
		box-shadow: none;
		transition: opacity 0.8s ease;
	}

	.chat-space.chat-active::before {
		opacity: 1;
		animation: perimeter-breathe-active 3s ease-in-out infinite;
	}

	/* --- bar --- */

	header.chat-bar {
		position: relative;
		z-index: 4;
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 0.5rem 1.25rem;
		flex-shrink: 0;
		background: var(--surface-tab);


		border-bottom: 1px solid var(--glass-border);
	}

	.bar-left {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		font-family: var(--font-body);
		font-size: 0.8125rem;
		letter-spacing: 0.03em;
	}

	.bar-right {
		display: flex;
		align-items: center;
		gap: 0.25rem;
	}

	.bar-led {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--card);
		transition: all 0.4s ease;
	}

	.bar-led-on {
		background: var(--card);
		box-shadow: none;
	}

	.bar-name {
		color: var(--text-primary);
	}

	.bar-mood {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		letter-spacing: 0.06em;
		color: var(--text-muted);
		transition: color 0.5s ease;
	}
	.bar-mood[data-mood="focused"] { color: var(--text-secondary); }
	.bar-mood[data-mood="playful"] { color: var(--text-secondary); }
	.bar-mood[data-mood="loving"] { color: var(--text-secondary); }
	.bar-mood[data-mood="warm"] { color: var(--text-secondary); }
	.bar-mood[data-mood="reflective"] { color: var(--text-secondary); }
	.bar-mood[data-mood="excited"] { color: var(--text-secondary); }
	.bar-mood[data-mood="curious"] { color: var(--text-secondary); }
	.bar-mood[data-mood="melancholy"] { color: var(--text-secondary); }
	.bar-mood[data-mood="sad"] { color: var(--text-secondary); }
	.bar-mood[data-mood="anxious"] { color: var(--text-secondary); }
	.bar-mood[data-mood="creative"] { color: var(--text-secondary); }
	.bar-mood[data-mood="energetic"] { color: var(--text-secondary); }
	.bar-mood[data-mood="tired"] { color: var(--text-secondary); }
	.bar-mood[data-mood="peaceful"] { color: var(--text-secondary); }

	.bar-activity {
		display: flex;
		align-items: center;
		gap: 0.35rem;
		font-family: var(--font-body);
		font-size: 0.8125rem;
		letter-spacing: 0.06em;
		color: var(--text-secondary);
		animation: fade-up 0.3s ease both;
	}

	.bar-activity-dot {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		background: var(--card);
		animation: pulse-alive 2.5s ease-in-out infinite;
	}

	.bar-btn {
		display: flex;
		align-items: center;
		justify-content: center;
		width: 1.75rem;
		height: 1.75rem;
		color: var(--text-muted);
		border-radius: 7px;
		transition: all 0.2s ease;
	}

	.bar-btn-active {
		color: var(--text-secondary);
	}

	.bar-btn:hover {
		color: var(--text-primary);
		background: var(--card);
	}

	/* --- columns --- */

	.chat-columns {
		position: relative;
		z-index: 2;
		flex: 1;
		min-height: 0;
		min-width: 0;
		display: grid;
		grid-template-columns: 1fr 1fr;
	}

	.chat-main {
		display: flex;
		flex-direction: column;
		min-height: 0;
		min-width: 0;
		border-right: 1px solid var(--border);
	}

	.chat-sidebar {
		display: flex;
		flex-direction: column;
		align-items: center;
		justify-content: center;
		padding: 1rem;
		gap: 1rem;
		overflow: hidden;
		position: relative;

	}

	.sidebar-banners {
		width: 100%;
		max-width: 220px;
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		z-index: 2;
	}


	/* --- stream --- */

	:global(.chat-stream) {
		flex: 1;
		min-height: 0;
		min-width: 0;
		overflow-y: auto;
		overflow-x: hidden;
	}


	.chat-loading {
		gap: 12px;
		color: var(--text-muted);
		font-size: 14px;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 3rem 0;
	}

	.loading-dot {
		width: 5px;
		height: 5px;
		border-radius: 50%;
		background: var(--card);
		box-shadow: none;
		animation: pulse 2s ease-in-out infinite;
	}

	@keyframes pulse {
		0%, 100% { opacity: 1; transform: scale(1); }
		50% { opacity: 0.3; transform: scale(0.7); }
	}

	.chat-empty {
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 5rem 0;
		animation: fade-up 0.8s cubic-bezier(0.16, 1, 0.3, 1) 0.4s both;
	}

	.chat-empty p {
		font-family: var(--font-display);
		font-size: 0.9rem;
		font-style: normal;
		color: var(--text-secondary);
		margin: 0;
	}

	@keyframes fade-up {
		from { opacity: 0; transform: translateY(6px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.chat-thinking {
		display: flex;
		gap: 0.4rem;
		padding: 0.8rem 0;
		justify-content: flex-end;
		align-self: flex-end;
		animation: fade-up 0.3s ease both;
	}

	.think-dot {
		width: 4px;
		height: 4px;
		border-radius: 50%;
		background: var(--card);
		box-shadow: none;
		animation: bounce 1.4s ease-in-out infinite;
	}

	@keyframes bounce {
		0%, 60%, 100% { transform: translateY(0); opacity: 0.25; }
		30% { transform: translateY(-5px); opacity: 1; }
	}

	/* --- compaction notice --- */

	.compaction-notice {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		padding: 0.5rem 0.75rem;
		margin: 0.5rem 0;
		margin-left: auto;
		border-radius: 10px;
		background: var(--card);


		border: 1px dashed var(--border);
		animation: act-in 0.35s cubic-bezier(0.16, 1, 0.3, 1) both;
	}

	.compaction-text {
		font-family: var(--font-body);
		font-size: 0.75rem;
		letter-spacing: 0.03em;
		color: var(--text-secondary);
		flex: 1;
	}

	.compaction-time {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--text-secondary);
		white-space: nowrap;
	}

	/* --- responsive --- */

	@media (max-width: 900px) {
		.chat-columns {
			grid-template-columns: 1fr;
		}
		.chat-sidebar {
			display: none;
		}
		.chat-main {
			border-right: none;
		}
	}

	@media (max-width: 720px) {
		header.chat-bar {
			padding: 0.5rem 0.75rem;
		}
		.bar-right {
			max-width: 50%;
		}
	}

	/* --- clear context dialog (glass) --- */

	:global(.clear-dialog) {
		background: var(--card) !important;


		border: 1px solid var(--border) !important;
		border-radius: 16px !important;
		padding: 1.5rem !important;
		box-shadow: none;
	}

	:global(.clear-dialog-title) {
		font-family: var(--font-body);
		font-size: 0.8rem;
		letter-spacing: 0.04em;
		color: var(--text-secondary);
		margin: 0;
	}

	:global(.clear-dialog-desc) {
		font-family: var(--font-body);
		font-size: 0.75rem;
		line-height: 1.5;
		color: var(--text-secondary);
		margin-top: 0.5rem;
	}

	:global(.clear-dialog-footer) {
		display: flex;
		justify-content: flex-end;
		gap: 0.5rem;
		margin-top: 1.25rem;
	}

	:global(.clear-dialog-btn) {
		font-family: var(--font-body);
		font-size: 0.8125rem;
		letter-spacing: 0.04em;
		padding: 0.4rem 1rem;
		border-radius: 8px;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	:global(.clear-dialog-cancel) {
		color: var(--text-secondary);
		background: var(--card);
		border: 1px solid var(--border);
	}

	:global(.clear-dialog-cancel:hover) {
		background: var(--card);
		color: var(--text-secondary);
	}

	:global(.clear-dialog-confirm) {
		color: var(--text-secondary);
		background: var(--card);
		border: 1px solid var(--border);
	}

	:global(.clear-dialog-confirm:hover) {
		background: var(--card);
	}

:global(.dark) .chat-space::before{display:none}:global(.dark) header.chat-bar{padding:12px 20px;background:var(--surface-tab);border-bottom:1px solid var(--border)}:global(.dark) .bar-left{font:400 13px var(--font-body);color:var(--text-secondary)}


 .intro-name {font-style:normal;letter-spacing:-.025em;color:var(--foreground)}
 .intro-skip {min-height:44px;background:var(--card);color:var(--foreground);border-radius:8px;font-size:14px}
 .bar-led {background:var(--text-muted);width:6px;height:6px}
 .bar-led-on,.bar-activity-dot,.loading-dot,.think-dot {background:var(--primary)}
 .bar-mood[data-mood],.bar-activity {color:var(--text-muted);font-size:12px;letter-spacing:0}
 .bar-btn {width:44px;height:44px;border-radius:8px;flex-shrink:0}
 .bar-btn-active {color:var(--primary);background:var(--accent)}
 .bar-btn:hover {background:var(--accent)}
 .chat-empty p {font:400 18px var(--font-body);line-height:1.6}
 .chat-empty {padding:48px 24px;text-align:center}
 .sidebar-banners {max-width:320px}
 .compaction-notice {background:var(--card);border:1px solid var(--border);border-radius:12px}
 :global(.clear-dialog-title) {font-size:18px;color:var(--foreground);letter-spacing:0}
 :global(.clear-dialog-desc) {font-size:14px;color:var(--text-secondary)}
 :global(.clear-dialog-btn) {min-height:44px;font-size:14px;letter-spacing:0}
 :global(.clear-dialog-confirm) {background:var(--destructive);color:var(--primary-foreground);border-color:var(--destructive)}
 :global(.clear-dialog-confirm:hover) {background:var(--destructive);filter:brightness(.95)}
 @media(max-width:480px){header.chat-bar{flex-wrap:wrap;gap:8px}.bar-right{max-width:none}.bar-left{flex-wrap:wrap}}


 .history-error {padding:20px;margin:8px 0;border:1px solid var(--border);border-radius:12px;background:var(--card);color:var(--text-secondary);font-size:14px;line-height:1.6}
 .history-error p {margin:0 0 16px}
 .history-error-actions {display:flex;flex-wrap:wrap;gap:8px}
</style>
