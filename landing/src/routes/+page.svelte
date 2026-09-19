<script lang="ts">
 import Nav from '$lib/components/Nav.svelte';
 import Hero from '$lib/components/Hero.svelte';
 import MoonStory from '$lib/components/MoonStory.svelte';
 import Install from '$lib/components/Install.svelte';
 import Cta from '$lib/components/Cta.svelte';
 import Footer from '$lib/components/Footer.svelte';
</script>
<a class="skip" href="#main">Skip to content</a>
<Nav />
<main id="main"><Hero /><MoonStory /><Install /><Cta /></main>
<Footer />
<style>.skip{position:fixed;top:1rem;left:1rem;z-index:200;background:#f6f3ec;color:#201d29;padding:1rem;transform:translateY(-200%)}.skip:focus{transform:translateY(0)}</style>
