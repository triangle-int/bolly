import { fetchCompanion, AuthError } from "$lib/api/client.js";
import type { CompanionContext } from "$lib/api/types.js";
import { DEFAULT_COMPANION_SLUG } from "$lib/companion/context.js";

let context = $state<CompanionContext | null>(null);
let loading = $state(true);
let error = $state("");

/** The one companion this server owns. There is nothing to select between. */
export function getCompanion() {
	return {
		get context() {
			return context;
		},
		/** Canonical slug; falls back to the server default until loaded. */
		get slug() {
			return context?.slug || DEFAULT_COMPANION_SLUG;
		},
		get loading() {
			return loading;
		},
		get error() {
			return error;
		},
		async refresh() {
			loading = true;
			error = "";
			try {
				context = await fetchCompanion();
			} catch (e) {
				error = "Cannot reach your Nolune server. Check that it is running, then try again.";
				if (e instanceof AuthError) throw e;
			} finally {
				loading = false;
			}
		},
	};
}
