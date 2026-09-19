<script lang="ts">
	import { Collapsible } from "bits-ui";
	import { Badge } from "$lib/components/ui/badge/index.js";
	import { cn } from "$lib/utils.js";

	import CheckCircleIcon from "@lucide/svelte/icons/check-circle";
	import ChevronDownIcon from "@lucide/svelte/icons/chevron-down";
	import CircleIcon from "@lucide/svelte/icons/circle";
	import ClockIcon from "@lucide/svelte/icons/clock";
	import WrenchIcon from "@lucide/svelte/icons/wrench";
	import XCircleIcon from "@lucide/svelte/icons/x-circle";

	type ToolUIPartType = string;
	type ToolUIPartState =
		| "input-streaming"
		| "input-available"
		| "output-available"
		| "output-error"
		| "recorded";

	interface ToolHeaderProps {
		type: ToolUIPartType;
		state: ToolUIPartState;
		class?: string;
		[key: string]: any;
	}

	let { type, state, class: className = "", ...restProps }: ToolHeaderProps = $props();

	let getStatusBadge = $derived.by(() => {
		let labels = {
			"recorded": "Recorded",
			"input-streaming": "Pending",
			"input-available": "Running",
			"output-available": "Completed",
			"output-error": "Error",
		} as const;

		let icons = {
			"recorded": CircleIcon,
			"input-streaming": CircleIcon,
			"input-available": ClockIcon,
			"output-available": CheckCircleIcon,
			"output-error": XCircleIcon,
		} as const;

		let IconComponent = icons[state];
		let label = labels[state];

		return { IconComponent, label };
	});
	let IconComponent = $derived(getStatusBadge.IconComponent);

	let id = $props.id();
</script>

<Collapsible.Trigger
	{id}
	class={cn("group flex min-h-11 w-full items-center justify-between gap-3 p-3 text-left", className)}
	{...restProps}
>
	<div class="flex min-w-0 flex-wrap items-center gap-2">
		<WrenchIcon class="text-muted-foreground size-4" />
		<span class="min-w-0 break-all text-sm font-medium">{type}</span>
		<Badge class="gap-1.5 rounded-full text-xs" variant="secondary">
			<IconComponent
				class={cn(
					"size-4",
					state === "input-available" && "animate-pulse",
					state === "output-available" && "text-primary",
					state === "output-error" && "text-destructive"
				)}
			/>

			{getStatusBadge.label}
		</Badge>
	</div>
	<ChevronDownIcon
		class="text-muted-foreground size-4 transition-transform group-data-[state=open]:rotate-180"
	/>
</Collapsible.Trigger>
