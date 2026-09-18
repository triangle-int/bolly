<script lang="ts">
	import { tick } from 'svelte';
	import favicon from '$lib/assets/favicon.png';
	let scrolled = $state(false);
	let mobileOpen = $state(false);
	let toggleButton: HTMLButtonElement;
	let mobileMenu = $state<HTMLDivElement>();

	$effect(() => {
		const onScroll = () => (scrolled = window.scrollY > 32);
		onScroll();
		window.addEventListener('scroll', onScroll, { passive: true });
		return () => window.removeEventListener('scroll', onScroll);
	});

	$effect(() => {
		if (!mobileOpen) return;

		const previousOverflow = document.body.style.overflow;
		document.body.style.overflow = 'hidden';
		void tick().then(() => {
			if (!mobileOpen || !mobileMenu) return;
			const firstFocusable = mobileMenu.querySelector<HTMLElement>('a[href], button:not([disabled])');
			if (firstFocusable) firstFocusable.focus();
			else mobileMenu.focus();
		});

		return () => {
			document.body.style.overflow = previousOverflow;
		};
	});

	function close() {
		mobileOpen = false;
		void tick().then(() => {
			if (!mobileOpen) toggleButton.focus();
		});
	}

	function toggleMenu() {
		if (mobileOpen) close();
		else mobileOpen = true;
	}

	function handleMenuKeydown(event: KeyboardEvent) {
		if (!mobileOpen || !mobileMenu) return;

		if (event.key === 'Escape') {
			event.preventDefault();
			close();
			return;
		}

		if (event.key !== 'Tab') return;
		const focusable = Array.from(mobileMenu.querySelectorAll<HTMLElement>('a[href], button:not([disabled]), [tabindex]:not([tabindex="-1"])'));
		const first = focusable[0];
		const last = focusable.at(-1);
		if (!first || !last) {
			event.preventDefault();
			mobileMenu.focus();
			return;
		}

		const active = document.activeElement;
		if (event.shiftKey && (active === first || !mobileMenu.contains(active))) {
			event.preventDefault();
			last.focus();
		} else if (!event.shiftKey && (active === last || !mobileMenu.contains(active))) {
			event.preventDefault();
			first.focus();
		}
	}
</script>

<svelte:window onkeydown={handleMenuKeydown} />

<nav class:scrolled aria-label="Primary navigation">
	<div class="inner">
		<a href="/" class="brand" aria-label="Bolly home"><img src={favicon} alt="" /><span>bolly</span></a>
		<div class="links">
			<a href="#how">How it works</a><a href="#computer-use">Computer use</a><a href="#companion">Companion</a><a href="#install">Install</a>
			<a href="https://github.com/triangle-int/bolly" target="_blank" rel="noopener">GitHub ↗</a>
			<a class="install" href="#install">Install Bolly</a>
		</div>
		<button bind:this={toggleButton} class="toggle" onclick={toggleMenu} aria-expanded={mobileOpen} aria-controls="mobile-menu" aria-label={mobileOpen ? 'Close menu' : 'Open menu'}>
			<span></span><span></span>
		</button>
	</div>
</nav>
{#if mobileOpen}
	<button class="backdrop" onclick={close} aria-label="Close menu"></button>
	<div bind:this={mobileMenu} class="mobile" id="mobile-menu" role="dialog" aria-modal="true" aria-label="Navigation menu" tabindex="-1">
		<a href="#how" onclick={close}>How it works</a><a href="#computer-use" onclick={close}>Computer use</a><a href="#companion" onclick={close}>Companion</a><a href="#install" onclick={close}>Install</a>
		<a href="https://github.com/triangle-int/bolly" target="_blank" rel="noopener">GitHub ↗</a>
		<a class="install" href="#install" onclick={close}>Install Bolly</a>
	</div>
{/if}

<style>
	nav{position:fixed;inset:0 0 auto;z-index:100;padding:1rem 0;border-bottom:1px solid transparent;transition:.25s ease}nav.scrolled{padding:.7rem 0;background:oklch(.04 .015 260/.9);backdrop-filter:blur(18px);border-color:var(--color-border)}
	.inner{max-width:1180px;margin:auto;padding:0 1.5rem;display:flex;align-items:center;justify-content:space-between}.brand{display:flex;align-items:center;gap:.65rem}.brand img{width:1.9rem;height:1.9rem}.brand span{font-family:var(--font-display);font-style:italic;font-size:1.25rem}.links{display:flex;align-items:center;gap:1.65rem}.links a,.mobile a{font-size:.78rem;color:var(--color-text-dim);transition:color .2s}.links a:hover,.mobile a:hover{color:var(--color-text)}
	.install{padding:.62rem 1rem!important;border:1px solid var(--color-border-warm);background:var(--color-warm-ghost);color:var(--color-warm)!important}.toggle{display:none;padding:.5rem}.toggle span{display:block;width:20px;height:1px;background:var(--color-warm);margin:6px}.backdrop{position:fixed;inset:0;z-index:90;background:#0009}.mobile{position:fixed;z-index:95;inset:0 0 0 auto;width:min(310px,84vw);background:#0b0d12;padding:6rem 1.5rem;display:flex;flex-direction:column;gap:.25rem;border-left:1px solid var(--color-border)}.mobile a{padding:1rem;border-bottom:1px solid var(--color-border)}
	@media(max-width:820px){.links{display:none}.toggle{display:block}}
	@media(prefers-reduced-motion:reduce){nav{transition:none}}
</style>
