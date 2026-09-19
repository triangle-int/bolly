<script lang="ts">
	import { page } from "$app/state";
	import MemoryLibraryView from "$lib/components/memory/MemoryLibraryView.svelte";

	const slug = $derived(page.params.slug!);
</script>

{#key slug}
	<MemoryLibraryView {slug} />
{/key}
