<script lang="ts" module>
	import { cn, type WithElementRef } from "$lib/utils.js";
	import type { HTMLAttributes } from "svelte/elements";
	import type { Snippet } from "svelte";

	export interface ConversationContentProps extends WithElementRef<
		HTMLAttributes<HTMLDivElement>
	> {
		children?: Snippet;
		autoScroll?: boolean;
	}
</script>

<script lang="ts">
	import { getStickToBottomContext } from "./stick-to-bottom-context.svelte.js";
	import { watch } from "runed";

	let {
		class: className,
		children,
		autoScroll = true,
		ref = $bindable(null),
		...restProps
	}: ConversationContentProps = $props();

	const context = getStickToBottomContext();


	watch(
		() => ref,
		() => {
			if (ref && autoScroll) {
				context.setElement(ref);
				// Initial scroll to bottom
				context.scrollToBottom("smooth");
			}
		}
	);
</script>

<div
	bind:this={ref}
	class={cn("flex flex-col gap-8 p-4", className)}
	{...restProps}
>
	{@render children?.()}
</div>
