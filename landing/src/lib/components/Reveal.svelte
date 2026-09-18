<script lang="ts">
	import type { Snippet } from 'svelte';
	import { onMount } from 'svelte';

	let { children, delay = 0 }: { children: Snippet; delay?: number } = $props();

	let el: HTMLDivElement | undefined = $state();

	onMount(() => {
		if (!el) return;

		// Already in viewport — show immediately
		if (el.getBoundingClientRect().top < window.innerHeight) {
			el.style.transitionDelay = delay + 'ms';
			el.classList.add('visible');
			return;
		}

		const observer = new IntersectionObserver(
			(entries) => {
				if (entries[0]?.isIntersecting) {
					el!.style.transitionDelay = delay + 'ms';
					el!.classList.add('visible');
					observer.disconnect();
				}
			},
			{ threshold: 0.05 }
		);
		observer.observe(el);
		return () => observer.disconnect();
	});
</script>

<div bind:this={el} class="reveal">
	{@render children()}
</div>

<style>
	.reveal {
		opacity: 0;
		transform: translateY(24px);
		transition: opacity 0.8s cubic-bezier(0.16, 1, 0.3, 1),
					transform 0.8s cubic-bezier(0.16, 1, 0.3, 1);
	}

	:global(.reveal.visible) {
		opacity: 1;
		transform: translateY(0);
	}

	@media (prefers-reduced-motion: reduce) {
		.reveal { opacity: 1; transform: none; transition: none; }
	}
</style>
