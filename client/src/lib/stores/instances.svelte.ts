import { fetchInstances, AuthError } from "$lib/api/client.js";
import type { InstanceSummary } from "$lib/api/types.js";

let instances = $state<InstanceSummary[]>([]);
let loading = $state(true);
let error = $state("");

export function getInstances() {
	return {
		get error() { return error; },
		get list() {
			return instances;
		},
		get loading() {
			return loading;
		},
		async refresh() {
			loading = true;
			error = "";
			try {
				instances = await fetchInstances();
			} catch (e) {
				error = "Cannot reach your Nolune server. Check that it is running, then try again.";
				if (e instanceof AuthError) throw e;
			} finally {
				loading = false;
			}
		},
		upsert(instance: InstanceSummary) {
			const idx = instances.findIndex((i) => i.slug === instance.slug);
			if (idx >= 0) {
				instances[idx] = instance;
			} else {
				instances = [...instances, instance];
			}
		},
		remove(slug: string) {
			instances = instances.filter((i) => i.slug !== slug);
		},
	};
}
