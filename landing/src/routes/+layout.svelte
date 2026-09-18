<script lang="ts">
	import '../app.css';
	import favicon from '$lib/assets/favicon.png';
	import { dev } from '$app/environment';
	import { injectAnalytics } from '@vercel/analytics/sveltekit';

	injectAnalytics({ mode: dev ? 'development' : 'production' });
	let { children } = $props();
</script>

<svelte:head>
	<title>Bolly — Your AI, with a computer of its own</title>
	<meta name="description" content="Run one persistent AI companion on an always-on computer, connect your other devices, and ask it to work across them from one interface." />
	<meta name="theme-color" content="#090a0d" />
	<link rel="icon" href={favicon} />
</svelte:head>

{@render children()}
