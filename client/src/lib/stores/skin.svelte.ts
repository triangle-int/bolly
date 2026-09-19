/**
 * Skin store — per-instance skin selection, persisted on the server.
 *
 * Little Moon uses lightweight SVG expressions.
 */

import { getContext, setContext } from "svelte";
import { fetchSkin, updateSkin } from "$lib/api/client.js";

const SKIN_KEY = Symbol("skin");

export interface SkinDefinition {
 id: string;
 label: string;
 thumbnail: string;
 avatar: { idle: string; thinking: string };
}

export const SKINS: SkinDefinition[] = [{
 id: "moon",
 label: "Nolune · Little Moon",
 thumbnail: "/skins/moon/character.svg",
 avatar: { idle: "/skins/moon/character.svg", thinking: "/skins/moon/thinking.svg" },
}];

export interface SkinStore {
	readonly skinId: string;
	readonly skin: SkinDefinition;
	setSkin(skinId: string): void;
	setSlug(slug: string): void;
	loadForInstance(slug: string): Promise<void>;
}

export function createSkinStore(initialSlug = ""): SkinStore {
	let slug = $state(initialSlug);
	let skinId = $state("moon");

	const skin = $derived(SKINS.find((s) => s.id === skinId) ?? SKINS[0]);

	return {
		get skinId() { return skinId; },
		get skin() { return skin; },
		setSkin(id: string) {
			skinId = SKINS.some(s => s.id === id) ? id : SKINS[0].id;
			if (slug) updateSkin(slug, skinId).catch(() => {});
		},
		setSlug(s: string) {
			slug = s;
		},
		async loadForInstance(s: string) {
			slug = s;
			try {
				const res = await fetchSkin(s);
				skinId = SKINS.some(s => s.id === res.skin) ? res.skin : SKINS[0].id;
			} catch {
				skinId = "moon";
			}
		},
	};
}

export function setSkinStore(store: SkinStore) {
	setContext(SKIN_KEY, store);
}

export function getSkinStore(): SkinStore {
	return getContext<SkinStore>(SKIN_KEY);
}
