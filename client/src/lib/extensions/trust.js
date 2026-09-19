// @ts-check
/**
 * Pure helpers for the extension trust model (#97). Curated servers come from
 * the server's reviewed catalog; anything else is custom and needs an
 * explicit acknowledgement plus per-tool grants before a chat can use it.
 */

/** @typedef {{ name: string; url: string }} CuratedEntry */
/** @typedef {{ name: string; enabled: boolean; description?: string }} ToolGrant */
/** @typedef {{ name: string; url?: string; trust: "curated" | "custom"; connected: boolean; tools: ToolGrant[] }} ExtensionServer */

/**
 * @param {CuratedEntry[]} curated
 * @param {string} name
 * @param {string} url
 */
export function isCurated(curated, name, url) {
	return curated.some((entry) => entry.name === name && entry.url === url);
}

/**
 * Whether adding this server must carry `acknowledge_untrusted: true`.
 * @param {CuratedEntry[]} curated
 * @param {string} name
 * @param {string} url
 */
export function requiresAcknowledgement(curated, name, url) {
	return !isCurated(curated, name, url);
}

/** @param {ExtensionServer} server */
export function grantSummary(server) {
	const total = server.tools.length;
	const enabled = server.tools.filter((t) => t.enabled).length;
	if (total === 0) return "no tools discovered";
	if (enabled === 0) return `no tools allowed (${total} available)`;
	if (enabled === total) return total === 1 ? "1 tool allowed" : `all ${total} tools allowed`;
	return `${enabled} of ${total} tools allowed`;
}

/** @param {ExtensionServer} server */
export function trustLabel(server) {
	return server.trust === "curated" ? "Reviewed" : "Custom, not reviewed";
}

/**
 * Names of tools that would be active in ordinary chats.
 * @param {ExtensionServer[]} servers
 */
export function activeToolNames(servers) {
	return servers
		.filter((s) => s.connected)
		.flatMap((s) => s.tools.filter((t) => t.enabled).map((t) => `${s.name}/${t.name}`))
		.sort();
}
