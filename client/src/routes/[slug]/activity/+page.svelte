<script lang="ts">
	import { page } from "$app/state";
	import ActivityView from "$lib/components/activity/ActivityView.svelte";

	const slug = $derived(page.params.slug!);
</script>

{#key slug}
	<ActivityView {slug} />
{/key}
