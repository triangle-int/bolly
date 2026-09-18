<script lang="ts">
	import favicon from '$lib/assets/favicon.png';

	let scrolled = $state(false);
	let mobileOpen = $state(false);

	$effect(() => {
		function onScroll() {
			scrolled = window.scrollY > 50;
		}
		window.addEventListener('scroll', onScroll, { passive: true });
		return () => window.removeEventListener('scroll', onScroll);
	});

	function closeMobile() {
		mobileOpen = false;
	}
</script>

<nav class="nav" class:nav-scrolled={scrolled}>
	<div class="nav-inner">
		<a href="/" class="nav-brand">
			<img src={favicon} alt="" class="nav-logo" />
			<span class="nav-name">bolly</span>
		</a>

		<!-- desktop -->
		<ul class="nav-links">
			<li><a href="/#features" class="nav-link">Features</a></li>
			<li><a href="/#how" class="nav-link">How it works</a></li>
			<li><a href="/#pricing" class="nav-link">Pricing</a></li>
			<li><a href="/skills" class="nav-link">Skills</a></li>
			<li><a href="/docs" class="nav-link">Docs</a></li>
			<li><a href="https://github.com/triangle-int/bolly" target="_blank" rel="noopener" class="nav-link">GitHub</a></li>
			<li><a href="/#pricing" class="nav-cta">Get started</a></li>
		</ul>

		<!-- mobile toggle -->
		<button
			class="mobile-toggle"
			onclick={() => mobileOpen = !mobileOpen}
			aria-label={mobileOpen ? 'Close menu' : 'Open menu'}
		>
			<span class="toggle-line" class:toggle-open={mobileOpen}></span>
			<span class="toggle-line" class:toggle-open={mobileOpen}></span>
		</button>
	</div>
</nav>

{#if mobileOpen}
	<button class="mobile-backdrop" onclick={closeMobile} aria-label="Close menu"></button>
	<div class="mobile-menu">
		<a href="/#features" onclick={closeMobile} class="mobile-link">Features</a>
		<a href="/#how" onclick={closeMobile} class="mobile-link">How it works</a>
		<a href="/#pricing" onclick={closeMobile} class="mobile-link">Pricing</a>
		<a href="/skills" class="mobile-link">Skills</a>
		<a href="/docs" class="mobile-link">Docs</a>
		<a href="https://github.com/triangle-int/bolly" target="_blank" rel="noopener" class="mobile-link">GitHub</a>
		<div class="mobile-divider"></div>
		<a href="/#pricing" class="mobile-cta">Get started</a>
	</div>
{/if}

<style>
	.nav {
		position: fixed;
		top: 0;
		left: 0;
		right: 0;
		z-index: 100;
		padding: 1rem 0;
		backdrop-filter: blur(24px) saturate(160%) brightness(1.04);
		background: oklch(0.04 0.015 260 / 50%);
		border-bottom: 1px solid var(--glass-border);
		transition: all 0.4s cubic-bezier(0.16, 1, 0.3, 1);
	}

	.nav::after {
		content: '';
		position: absolute;
		bottom: 0;
		left: 10%;
		right: 10%;
		height: 1px;
		background: linear-gradient(90deg, transparent, oklch(1 0 0 / 8%), transparent);
		pointer-events: none;
	}

	.nav-scrolled {
		background: oklch(0.04 0.015 260 / 70%);
		border-bottom-color: oklch(1 0 0 / 10%);
		box-shadow: 0 8px 32px oklch(0 0 0 / 20%);
	}

	.nav-inner {
		max-width: 1100px;
		margin: 0 auto;
		padding: 0 1.5rem;
		display: flex;
		align-items: center;
		justify-content: space-between;
	}

	.nav-brand {
		display: flex;
		align-items: center;
		gap: 0.625rem;
		text-decoration: none;
	}

	.nav-logo {
		width: 2rem;
		height: 2rem;
		object-fit: contain;
	}

	.nav-name {
		font-family: var(--font-display);
		font-style: italic;
		font-size: 1.25rem;
		color: var(--color-text);
		letter-spacing: -0.02em;
	}

	.nav-links {
		display: flex;
		align-items: center;
		gap: 2rem;
		list-style: none;
	}

	.nav-link {
		font-size: 0.8125rem;
		color: oklch(0.90 0.02 75 / 45%);
		letter-spacing: 0.02em;
		transition: color 0.3s ease;
		text-decoration: none;
	}

	.nav-link:hover {
		color: oklch(0.90 0.02 75 / 85%);
	}

	.nav-cta {
		font-size: 0.8125rem;
		padding: 0.5rem 1.25rem;
		border-radius: 2rem;
		color: var(--color-warm);
		background: var(--glass-bg);
		backdrop-filter: var(--glass-blur);
		border: 1px solid oklch(0.78 0.12 75 / 12%);
		border-top-color: oklch(0.78 0.12 75 / 20%);
		transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
		text-decoration: none;
	}

	.nav-cta:hover {
		border-color: oklch(0.78 0.12 75 / 30%);
		box-shadow: 0 0 30px oklch(0.78 0.12 75 / 8%);
	}

	.mobile-toggle {
		display: none;
		flex-direction: column;
		gap: 5px;
		padding: 0.5rem;
		cursor: pointer;
		background: none;
		border: none;
	}

	.toggle-line {
		width: 18px;
		height: 1.5px;
		background: oklch(0.78 0.12 75 / 50%);
		border-radius: 1px;
		transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
		transform-origin: center;
	}

	.toggle-line.toggle-open:first-child {
		transform: translateY(3.25px) rotate(45deg);
	}
	.toggle-line.toggle-open:last-child {
		transform: translateY(-3.25px) rotate(-45deg);
	}

	.mobile-backdrop {
		position: fixed;
		inset: 0;
		z-index: 90;
		background: oklch(0 0 0 / 50%);
		backdrop-filter: blur(8px);
		animation: fade-in 0.2s ease both;
		border: none;
		cursor: default;
	}

	.mobile-menu {
		position: fixed;
		top: 0;
		right: 0;
		bottom: 0;
		width: min(280px, 80vw);
		z-index: 95;
		background: oklch(0.05 0.015 260 / 80%);
		backdrop-filter: blur(32px) saturate(160%) brightness(1.04);
		border-left: 1px solid var(--glass-border);
		padding: 5rem 1.5rem 2rem;
		display: flex;
		flex-direction: column;
		gap: 0.25rem;
		animation: slide-in 0.3s cubic-bezier(0.16, 1, 0.3, 1) both;
	}

	.mobile-link {
		display: block;
		padding: 0.75rem;
		font-size: 0.875rem;
		color: oklch(0.90 0.02 75 / 50%);
		border-radius: 0.75rem;
		transition: all 0.2s ease;
		text-decoration: none;
	}
	.mobile-link:hover {
		color: oklch(0.90 0.02 75 / 85%);
		background: oklch(1 0 0 / 4%);
	}

	.mobile-divider {
		height: 1px;
		background: linear-gradient(90deg, oklch(1 0 0 / 8%), transparent);
		margin: 0.5rem 0.75rem;
	}

	.mobile-cta {
		display: block;
		text-align: center;
		margin: 0.5rem 0.75rem 0;
		padding: 0.75rem 1.5rem;
		border-radius: 2rem;
		font-size: 0.875rem;
		color: var(--color-warm);
		background: var(--glass-bg);
		border: 1px solid oklch(0.78 0.12 75 / 12%);
		transition: all 0.2s ease;
		text-decoration: none;
	}
	.mobile-cta:hover {
		background: oklch(1 0 0 / 8%);
		border-color: oklch(0.78 0.12 75 / 30%);
	}

	@media (max-width: 768px) {
		.nav-links { display: none; }
		.mobile-toggle { display: flex; }
	}

	@keyframes fade-in {
		from { opacity: 0; }
		to { opacity: 1; }
	}

	@keyframes slide-in {
		from { transform: translateX(100%); }
		to { transform: translateX(0); }
	}
</style>
