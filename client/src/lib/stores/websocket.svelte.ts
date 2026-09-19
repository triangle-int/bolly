import { createWebSocket, fetchSession } from "$lib/api/client.js";
import type { ServerEvent } from "$lib/api/types.js";

type EventHandler = (event: ServerEvent) => void;

let socket: WebSocket | null = null;
let connected = $state(false);
let reconnecting = $state(false);
let retryCount = $state(0);
let intentionalClose = false;
let retryTimer: ReturnType<typeof setTimeout> | null = null;
const handlers = new Set<EventHandler>();
const authLostHandlers = new Set<() => void>();

/** Server close code for a revoked browser session (see server ws.rs). */
const CLOSE_SESSION_REVOKED = 4401;

const MAX_RETRY_DELAY = 30_000;

function retryDelay(): number {
	// Exponential backoff: 1s, 2s, 4s, 8s, 16s, 30s cap
	return Math.min(1000 * 2 ** retryCount, MAX_RETRY_DELAY);
}

export function getWebSocket() {
	return {
		get connected() {
			return connected;
		},
		get reconnecting() {
			return reconnecting;
		},
		get retryCount() {
			return retryCount;
		},
		connect() {
			if (socket) return;
			intentionalClose = false;

			try {
				socket = createWebSocket();
			} catch {
				scheduleReconnect();
				return;
			}

			socket.addEventListener("open", () => {
				connected = true;
				reconnecting = false;
				retryCount = 0;
			});

			socket.addEventListener("close", (ev) => {
				connected = false;
				socket = null;
				if (intentionalClose) return;
				if (ev.code === CLOSE_SESSION_REVOKED) {
					notifyAuthLost();
					return;
				}
				// A failed upgrade does not expose its HTTP status; ask the
				// server whether this browser is still paired before retrying.
				void fetchSession()
					.then((session) => {
						if (session === null) notifyAuthLost();
						else scheduleReconnect();
					})
					.catch(() => scheduleReconnect());
			});

			socket.addEventListener("error", () => {
				// error always fires before close, so close handler will reconnect
			});

			socket.addEventListener("message", (ev) => {
				try {
					const event: ServerEvent = JSON.parse(ev.data);
					for (const handler of handlers) {
						handler(event);
					}
				} catch {
					// ignore malformed messages
				}
			});
		},
		subscribe(handler: EventHandler): () => void {
			handlers.add(handler);
			return () => handlers.delete(handler);
		},
		/** Called when the server no longer accepts this browser's session. */
		onAuthLost(handler: () => void): () => void {
			authLostHandlers.add(handler);
			return () => authLostHandlers.delete(handler);
		},
		disconnect() {
			intentionalClose = true;
			if (retryTimer) {
				clearTimeout(retryTimer);
				retryTimer = null;
			}
			socket?.close();
			socket = null;
			connected = false;
			reconnecting = false;
			retryCount = 0;
		},
	};
}

function notifyAuthLost() {
	reconnecting = false;
	retryCount = 0;
	for (const handler of authLostHandlers) handler();
}

function scheduleReconnect() {
	reconnecting = true;
	retryCount++;
	const delay = retryDelay();
	retryTimer = setTimeout(() => {
		retryTimer = null;
		getWebSocket().connect();
	}, delay);
}
