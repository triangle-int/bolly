/**
 * Pairing codes are eight digits shown as `1234-5678`. Format whatever the
 * person typed or pasted into that shape, dropping everything else.
 *
 * @param {string} raw
 * @returns {string}
 */
export function formatPairingCodeInput(raw) {
	const digits = String(raw ?? "").replace(/\D/g, "").slice(0, 8);
	return digits.length > 4 ? `${digits.slice(0, 4)}-${digits.slice(4)}` : digits;
}

/**
 * @param {string} raw
 * @returns {boolean}
 */
export function isCompletePairingCode(raw) {
	return String(raw ?? "").replace(/\D/g, "").length === 8;
}

/**
 * Human sentence for a failed pairing attempt.
 *
 * @param {"invalid_code" | "rate_limited" | "cross_origin" | "auth_disabled" | "unknown"} reason
 * @returns {string}
 */
export function pairingErrorText(reason) {
	switch (reason) {
		case "invalid_code":
			return "That code didn't work. Codes expire after 5 minutes and only work once.";
		case "rate_limited":
			return "Too many attempts. Wait about 10 minutes, then get a fresh code.";
		case "cross_origin":
			return "Open Nolune directly at your server's address and try again.";
		case "auth_disabled":
			return "This server has no authentication, so no pairing is needed.";
		default:
			return "Could not reach your Nolune server. Check that it is running, then try again.";
	}
}
