<script lang="ts">
	import { Collapsible } from "bits-ui";
	import { cn } from "$lib/utils.js";
	import type { Snippet } from "svelte";

	// indexing

	interface ToolProps {
		class?: string;
		children?: Snippet;
		[key: string]: any;
	}

	let { class: className = "", children, ...restProps }: ToolProps = $props();

	let id = $props.id();
</script>

<Collapsible.Root {id} class={cn("not-prose mb-4 w-full rounded-md border", className)} {...restProps}>
	{@render children?.()}
</Collapsible.Root>
