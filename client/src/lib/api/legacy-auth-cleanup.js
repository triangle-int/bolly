const LEGACY_LOCAL_STORAGE_KEYS = ["nolune_auth_token", "bolly_auth_token", "bolly_token"];
const LEGACY_COOKIE_NAMES = ["nolune_token", "bolly_token", "bolly_auth_token"];

/**
 * Remove browser credentials written by historical Bolly/PWA clients.
 *
 * Browser JavaScript cannot clear arbitrary Domain/Path cookie variants or
 * HttpOnly cookies. The historical cookies were host-only, Path=/ and
 * non-HttpOnly, so these expirations cover the variants Nolune created. The
 * server only honours its own HttpOnly session cookie, which keeps unknown
 * variants fail-closed. Since issue #112 browsers pair for a session instead
 * of storing the API token, so nolune_auth_token is legacy too.
 *
 * @param {Pick<Storage, "removeItem"> | undefined} storage
 * @param {{ cookie: string } | undefined} cookieDocument
 */
export function clearLegacyBrowserAuth(storage, cookieDocument) {
	for (const key of LEGACY_LOCAL_STORAGE_KEYS) {
		try {
			storage?.removeItem(key);
		} catch {
			// Storage can be unavailable under browser privacy policies.
		}
	}

	for (const name of LEGACY_COOKIE_NAMES) {
		try {
			if (cookieDocument) {
				cookieDocument.cookie = `${name}=; Path=/; Max-Age=0; SameSite=Strict`;
			}
		} catch {
			// Cookie access can be unavailable under browser privacy policies.
		}
	}
}
