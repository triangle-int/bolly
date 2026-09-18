/**
 * Skin store — per-instance skin selection, persisted on the server.
 *
 * Each skin defines a set of video clips used by SharedScene.
 */

import { getContext, setContext } from "svelte";
import { fetchSkin, updateSkin } from "$lib/api/client.js";

const SKIN_KEY = Symbol("skin");

/** A video clip with WebM (Chrome/Firefox) and MOV HEVC (Safari) sources */
export interface ClipSource {
	webm: string;
	mov: string;
}

export interface SkinDefinition {
	id: string;
	label: string;
	thumbnail: string;
	clips: {
		idle: ClipSource;
		onboarding: ClipSource;
		reborn: ClipSource;
		/** Thinking clips — picked randomly during thinking cycles */
		thinking: ClipSource[];
	};
}

function mintClip(name: string): ClipSource {
	return { webm: `/skins/mint/${name}.webm`, mov: `/skins/mint/${name}.mov` };
}

function orbClip(name: string): ClipSource {
	return { webm: `/skins/orb/${name}.webm`, mov: `/skins/orb/${name}.mov` };
}

export const SKINS: SkinDefinition[] = [
	{
		id: "orb",
		label: "Nolune",
		thumbnail: "/skins/orb/orb.webp",
		clips: {
			idle: orbClip("orb-idle-loop"),
			onboarding: orbClip("orb-onboarding"),
			reborn: orbClip("orb-reborn"),
			thinking: [
				orbClip("morph-cube"),
				orbClip("morph-tesseract"),
				orbClip("morph-prism"),
			],
		},
	},
	{
		id: "mint",
		label: "Minty",
		thumbnail: "/skins/mint/character.png",
		clips: {
			idle: mintClip("idle-loop"),
			onboarding: mintClip("onboarding"),
			reborn: mintClip("reborn"),
			thinking: [
				mintClip("reading"),
				mintClip("typing"),
			],
		},
	},
];

/** Pick the right video src for the current browser (HEVC alpha for Safari, VP9 for others) */
const _useHEVC = typeof document !== 'undefined' &&
	document.createElement('video').canPlayType('video/mp4; codecs="hvc1.2.4.L123.B0"') !== '';

export function clipSrc(clip: ClipSource): string {
	return _useHEVC ? clip.mov : clip.webm;
}

export interface SkinStore {
	readonly skinId: string;
	readonly skin: SkinDefinition;
	setSkin(skinId: string): void;
	setSlug(slug: string): void;
	loadForInstance(slug: string): Promise<void>;
}

export function createSkinStore(initialSlug = ""): SkinStore {
	let slug = $state(initialSlug);
	let skinId = $state("orb");

	const skin = $derived(SKINS.find((s) => s.id === skinId) ?? SKINS[0]);

	return {
		get skinId() { return skinId; },
		get skin() { return skin; },
		setSkin(id: string) {
			skinId = id;
			if (slug) updateSkin(slug, id).catch(() => {});
		},
		setSlug(s: string) {
			slug = s;
		},
		async loadForInstance(s: string) {
			slug = s;
			try {
				const res = await fetchSkin(s);
				skinId = res.skin || "orb";
			} catch {
				skinId = "orb";
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
