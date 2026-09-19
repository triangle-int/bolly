<script lang="ts">
	import type { Snippet } from "svelte";
	let { ondismiss, children }: { ondismiss?: () => void; children: Snippet } = $props();
</script>

<div class="creature-bubble">
	<div class="creature-bubble-content">
		{@render children()}
	</div>
	{#if ondismiss}
		<button class="creature-bubble-dismiss" onclick={ondismiss}>OK</button>
	{/if}
</div>

<style>
	.creature-bubble {
		margin: 0 0 0.5rem;
		padding: 0.5rem 0.75rem;
		border-radius: 0.625rem;
		background: var(--card);
		border: 1px solid var(--border);
		font-family: var(--font-body);
		font-size: 0.8125rem;
		color: var(--foreground);
		line-height: 1.5;
		display: flex;
		align-items: flex-start;
		gap: 0.5rem;
		animation: bubble-in 0.4s cubic-bezier(0.16, 1, 0.3, 1) both;
	}

	@keyframes bubble-in {
		from { opacity: 0; transform: translateY(6px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.creature-bubble-content {
		flex: 1;
		min-width: 0;
	}

	.creature-bubble :global(a) {
		color: var(--text-secondary);
		text-decoration: underline;
	}

	.creature-bubble-dismiss {
		flex-shrink: 0;
		font-family: var(--font-body);
		font-size: 0.75rem;
		color: var(--text-secondary);
		background: none;
		border: 1px solid var(--border);
		border-radius: 0.25rem;
		padding: 0.1rem 0.4rem;
		cursor: pointer;
		margin-top: 0.1rem;
	}

	.creature-bubble-dismiss:hover {
		color: var(--text-secondary);
		border-color: var(--border);
	}

 .creature-bubble {padding:16px;border-radius:12px;font-size:14px}
 .creature-bubble :global(a) {color:var(--primary)}
 .creature-bubble-dismiss {min-width:44px;min-height:44px;border-radius:8px;background:var(--secondary);color:var(--foreground)}

</style>
