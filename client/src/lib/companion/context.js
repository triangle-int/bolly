// @ts-check
/**
 * One companion per server (#103, #104). These helpers are the only place the
 * client reasons about the companion slug, so no route or store can select or
 * create a second identity.
 */

/** Mirrors the server constant; refreshed from `GET /api/companion`. */
export const DEFAULT_COMPANION_SLUG = "companion";

/**
 * @typedef {{ slug: string; exists: boolean; companion_name: string; soul_exists: boolean }} CompanionContext
 */

/**
 * Route that opens the companion (chat, or onboarding when it does not exist yet).
 * @param {string | null | undefined} slug
 */
export function companionHome(slug) {
	return `/${slug || DEFAULT_COMPANION_SLUG}`;
}

/**
 * Redirect target for a URL whose identity segment is not the canonical slug,
 * preserving everything after it (chat ids, memory paths, settings). Returns
 * `null` when the URL already addresses the companion.
 * @param {string} canonical
 * @param {string} slug the identity segment found in the URL
 * @param {string} pathname
 */
export function redirectForStaleSlug(canonical, slug, pathname) {
	if (slug === canonical) return null;
	const prefix = `/${slug}`;
	const rest = pathname.startsWith(prefix) ? pathname.slice(prefix.length) : "";
	return `/${canonical}${rest}`;
}

/**
 * Onboarding is shown only after the server confirmed the companion is absent.
 * @param {CompanionContext | null | undefined} context
 */
export function needsOnboarding(context) {
	return !!context && !context.exists;
}

/**
 * Opening line of onboarding. The user's name is optional now that the home
 * page no longer asks for one before opening the companion.
 * @param {string | null | undefined} preferredName
 */
export function introGreeting(preferredName) {
	const name = preferredName?.trim();
	return name ? `hey, ${name}.` : "hey.";
}

/**
 * Prefix of the first message sent on the user's behalf after onboarding.
 * @param {string | null | undefined} preferredName
 * @param {string} languageLabel
 */
export function onboardingHandshake(preferredName, languageLabel) {
	const name = preferredName?.trim();
	const language = `please speak to me in ${languageLabel}.`;
	return name ? `my name is ${name}. ${language}` : language;
}
