<script lang="ts">
	import Nav from '$lib/components/Nav.svelte';
	import Hero from '$lib/components/Hero.svelte';
	import ComputerUse from '$lib/components/ComputerUse.svelte';
	import Demo from '$lib/components/Demo.svelte';
	import Features from '$lib/components/Features.svelte';
	import HowItWorks from '$lib/components/HowItWorks.svelte';
	import Pricing from '$lib/components/Pricing.svelte';
	import Cta from '$lib/components/Cta.svelte';
	import Footer from '$lib/components/Footer.svelte';
</script>

<Nav />
<main>
	<Hero />
	<ComputerUse />
	<Demo />
	<Features />
	<HowItWorks />
	<Pricing />
	<Cta />
</main>
<Footer />
