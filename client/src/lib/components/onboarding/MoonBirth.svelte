<script lang="ts">
	import { onMount } from 'svelte';

	let { name = 'Nolune', oncomplete }: { name?: string; oncomplete: () => void } = $props();
	let finished = false;
	function finish() {
		if (finished) return;
		finished = true;
		oncomplete();
	}
	onMount(() => {
		const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
		const timer = window.setTimeout(finish, reducedMotion ? 1000 : 3400);
		return () => window.clearTimeout(timer);
	});
</script>

<div class="birth" aria-label="Your companion is here">
	<div class="arrival">
		<svg viewBox="0 0 320 320" fill="none" aria-hidden="true">
			<g class="moon">
				<path d="M155 24C161 23 164 29 160 34C127 74 130 129 157 168C183 207 228 224 277 210C284 208 289 214 285 221C262 266 217 292 170 290C93 287 36 230 36 157C36 91 85 34 155 24Z" fill="var(--primary)" />
				<g class="eyes" fill="var(--primary-foreground)"><ellipse cx="81" cy="164" rx="6" ry="9"/><ellipse cx="110" cy="164" rx="6" ry="9"/></g>
			</g>
		</svg>
		<div class="greeting" role="status"><p class="nl-eyebrow">A little presence. Entirely yours.</p><h2>Hello, {name}.</h2></div>
	</div>
	<button class="skip nl-button-secondary" onclick={finish}>Continue</button>
</div>

<style>
.birth{position:fixed;inset:0;z-index:100;display:flex;align-items:center;justify-content:center;background:var(--background);padding:24px;text-align:center}
.arrival{width:min(100%,440px)}svg{display:block;width:clamp(160px,42vw,280px);margin:0 auto 32px;overflow:visible}
.moon{transform-origin:160px 160px;animation:moon-born 2.4s cubic-bezier(.22,1,.36,1) both}
.eyes{transform-box:fill-box;transform-origin:center;animation:eyes-awake 2.8s ease both}
.greeting{animation:greeting-in .7s ease 1.8s both}h2{font:400 clamp(32px,6vw,48px)/1.15 var(--font-display);margin:16px 0;overflow-wrap:anywhere;color:var(--foreground)}
.skip{position:absolute;bottom:calc(24px + env(safe-area-inset-bottom,0px));left:50%;transform:translateX(-50%)}
@keyframes moon-born{0%{opacity:0;transform:translateY(28px) scale(.08) rotate(-32deg)}20%{opacity:1}65%{transform:translateY(-8px) scale(1.04) rotate(4deg)}100%{opacity:1;transform:translateY(0) scale(1) rotate(0)}}
@keyframes eyes-awake{0%,48%{transform:scaleY(.08)}62%,78%{transform:scaleY(1)}83%{transform:scaleY(.08)}89%,100%{transform:scaleY(1)}}
@keyframes greeting-in{from{opacity:0;transform:translateY(8px)}to{opacity:1;transform:translateY(0)}}
@media(prefers-reduced-motion:reduce){.moon,.eyes,.greeting{animation:none}}
</style>
