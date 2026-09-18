<script lang="ts">
	import { page } from "$app/stores";
	import { onMount } from "svelte";
	import { fetchSkin } from "$lib/api/client.js";

	const slug = $derived($page.params.slug!);

	/** Skin clip definitions (duplicated from skin store to avoid context dependency) */
	interface ClipSource { webm: string; mov: string; }
	interface SkinClips {
		idle: ClipSource;
		thinking: ClipSource[];
	}

	const skinClips: Record<string, SkinClips> = {
		orb: {
			idle: { webm: "/skins/orb/orb-idle-loop.webm", mov: "/skins/orb/orb-idle-loop.mov" },
			thinking: [
				{ webm: "/skins/orb/morph-cube.webm", mov: "/skins/orb/morph-cube.mov" },
				{ webm: "/skins/orb/morph-tesseract.webm", mov: "/skins/orb/morph-tesseract.mov" },
				{ webm: "/skins/orb/morph-prism.webm", mov: "/skins/orb/morph-prism.mov" },
			],
		},
		mint: {
			idle: { webm: "/skins/mint/idle-loop.webm", mov: "/skins/mint/idle-loop.mov" },
			thinking: [
				{ webm: "/skins/mint/reading.webm", mov: "/skins/mint/reading.mov" },
				{ webm: "/skins/mint/typing.webm", mov: "/skins/mint/typing.mov" },
			],
		},
	};

	const useHEVC = typeof document !== "undefined" &&
		document.createElement("video").canPlayType('video/mp4; codecs="hvc1.2.4.L123.B0"') !== "";

	function src(clip: ClipSource): string { return useHEVC ? clip.mov : clip.webm; }

	let skinId = $state("orb");
	let thinking = $state(false);
	let recording = $state(false);
	let videoEl: HTMLVideoElement | undefined = $state();

	const clips = $derived(skinClips[skinId] ?? skinClips.orb);
	let thinkingIdx = $state(0);
	const currentClip = $derived(thinking ? (clips.thinking[thinkingIdx] ?? clips.idle) : clips.idle);
	const currentSrc = $derived(src(currentClip));
	const isLooping = $derived(!thinking);

	// Apply video source
	$effect(() => {
		if (videoEl && currentSrc) {
			videoEl.src = currentSrc;
			videoEl.loop = isLooping;
			videoEl.play().catch(() => {});
		}
	});

	function handleEnded() {
		if (thinking) {
			thinkingIdx = Math.floor(Math.random() * clips.thinking.length);
		}
	}

	onMount(() => {
		let mounted = true;
		// Fetch skin from server
		void fetchSkin(slug).then((res) => {
			if (mounted && res.skin && skinClips[res.skin]) skinId = res.skin;
		}).catch(() => {});

		// Listen for SSE/WebSocket events for thinking state
		// For now, poll the agent_running status
		const poll = setInterval(async () => {
			try {
				const res = await fetch(`/api/instances/${slug}/chat/default`);
				if (res.ok) {
					const data = await res.json();
					if (mounted) thinking = data.agent_running ?? false;
				}
			} catch {}
		}, 2000);

		return () => {
			mounted = false;
			clearInterval(poll);
		};
	});
</script>

<div class="overlay">
	<div class="pip">
		<!-- svelte-ignore a11y_media_has_caption -->
		<video
			bind:this={videoEl}
			class="pip-video"
			muted
			playsinline
			autoplay
			onended={handleEnded}
		></video>
		{#if recording}
			<div class="pip-rec"></div>
		{/if}
	</div>
</div>

<style>
	:global(html), :global(body) {
		background: transparent !important;
		margin: 0;
		padding: 0;
		overflow: hidden;
	}

	.overlay {
		position: fixed;
		inset: 0;
		pointer-events: none;
	}

	.pip {
		position: absolute;
		bottom: 16px;
		right: 16px;
		width: 56px;
		height: 56px;
		border-radius: 50%;
		overflow: hidden;
		background: oklch(0.06 0.02 260 / 90%);
		border: 2px solid oklch(0.78 0.12 75 / 30%);
		box-shadow: 0 2px 16px oklch(0 0 0 / 60%),
		            0 0 24px oklch(0.78 0.12 75 / 8%);
		animation: breathe 4s ease-in-out infinite;
	}

	@keyframes breathe {
		0%, 100% { transform: scale(1); }
		50% { transform: scale(1.03); }
	}

	.pip-video {
		width: 100%;
		height: 100%;
		object-fit: cover;
	}

	.pip-rec {
		position: absolute;
		top: -1px;
		right: -1px;
		width: 12px;
		height: 12px;
		border-radius: 50%;
		background: oklch(0.62 0.25 25);
		box-shadow: 0 0 8px oklch(0.62 0.25 25 / 80%);
		animation: rec-pulse 1.5s ease-in-out infinite;
		border: 2px solid oklch(0.06 0.02 260);
	}

	@keyframes rec-pulse {
		0%, 100% { opacity: 1; }
		50% { opacity: 0.4; }
	}
</style>
