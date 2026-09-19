<script lang="ts">
	import { CircleAlert, CircleCheck, Info } from "@lucide/svelte";
	import { getToasts } from "$lib/stores/toast.svelte.js";

	const toasts = getToasts();
</script>

{#if toasts.list.length > 0}
	<div class="toast-container">
		{#each toasts.list as toast (toast.id)}
			<div class="toast toast-{toast.kind}" role="alert">
				<span class="toast-icon">
					{#if toast.kind === "error"}<CircleAlert size={18} />{:else if toast.kind === "success"}<CircleCheck size={18} />{:else}<Info size={18} />{/if}
				</span>
				<span class="toast-msg">{toast.message}</span>
				<button class="toast-close" onclick={() => toasts.dismiss(toast.id)} aria-label="Dismiss">
					<svg viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.5" class="w-2.5 h-2.5">
						<path d="M2 2l8 8M10 2l-8 8" stroke-linecap="round"/>
					</svg>
				</button>
			</div>
		{/each}
	</div>
{/if}

<style>
.toast-container{position:fixed;bottom:calc(24px + env(safe-area-inset-bottom,0px));left:50%;transform:translateX(-50%);display:flex;flex-direction:column;gap:8px;z-index:200;pointer-events:none;width:max-content;max-width:calc(100vw - 32px)}
.toast{pointer-events:auto;display:flex;align-items:center;gap:12px;padding:8px 8px 8px 16px;border-radius:12px;background:var(--popover);border:1px solid var(--border);font:14px/1.5 var(--font-body);color:var(--foreground)}
.toast-error .toast-icon{color:var(--destructive)}.toast-icon{color:var(--primary);flex-shrink:0}.toast-msg{overflow-wrap:anywhere}.toast-close{display:grid;place-items:center;width:44px;height:44px;border-radius:8px;flex-shrink:0;color:var(--text-muted);cursor:pointer}.toast-close:hover{background:var(--accent);color:var(--foreground)}
</style>
