<script lang="ts">
	import { Dialog } from "bits-ui";
	import { submitSecret, cancelSecret } from "$lib/api/client.js";

	interface Props {
		instanceSlug: string;
		requestId: string;
		prompt: string;
		target: string;
		onclose: () => void;
	}

	let { instanceSlug, requestId, prompt, target, onclose }: Props = $props();

	let value = $state("");
	let submitting = $state(false);
	let error = $state("");

	async function handleSubmit() {
		if (!value.trim() || submitting) return;
		submitting = true;
		error = "";
		try {
			await submitSecret(instanceSlug, requestId, value);
			onclose();
		} catch (e) {
			error = e instanceof Error ? e.message : "failed to submit";
		} finally {
			submitting = false;
		}
	}

	function handleCancel() {
		if (submitting) return;
		cancelSecret(instanceSlug, requestId).catch(() => {});
		onclose();
	}

</script>

<Dialog.Root open={true} onOpenChange={(open) => { if (!open) handleCancel(); }}>
<Dialog.Portal><Dialog.Overlay class="fixed inset-0 z-[200] bg-background/80" />
<Dialog.Content class="fixed left-1/2 top-1/2 z-[201] w-[calc(100%-32px)] max-w-sm -translate-x-1/2 -translate-y-1/2 rounded-2xl border border-border bg-card p-6" escapeKeydownBehavior={submitting ? "ignore" : "close"} interactOutsideBehavior={submitting ? "ignore" : "close"}>

		<div class="header">
			<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="icon">
				<rect x="3" y="11" width="18" height="11" rx="2" ry="2" />
				<path d="M7 11V7a5 5 0 0 1 10 0v4" />
			</svg>
			<Dialog.Title class="text-lg font-medium">Secret required</Dialog.Title>
		</div>

		<Dialog.Description class="text-sm text-secondary-foreground">{prompt}</Dialog.Description>
		<p class="target">{target}</p>

		<form onsubmit={(e) => { e.preventDefault(); handleSubmit(); }}>
			<label for="secret-value">Secret value</label>
			<input id="secret-value"
				type="password"
				bind:value
				placeholder="Enter value…"
				autocomplete="off"
				disabled={submitting}
			/>
			{#if error}
				<p class="error" role="alert">{error}</p>
			{/if}
			<div class="actions">
				<button type="button" class="btn-cancel" onclick={handleCancel} disabled={submitting}>
					Cancel
				</button>
				<button type="submit" class="btn-submit" disabled={submitting || !value.trim()}>
					{submitting ? "Saving…" : "Save"}
				</button>
			</div>
		</form>
</Dialog.Content></Dialog.Portal></Dialog.Root>

<style>
.header{display:flex;align-items:center;gap:12px;margin-bottom:16px}.icon{width:20px;height:20px;color:var(--primary)}.target{font:12px/1.5 var(--font-mono);color:var(--text-muted);margin:8px 0 24px;overflow-wrap:anywhere}label{display:block;font-size:14px;margin-bottom:8px}input{width:100%;min-height:44px;padding:10px 12px;border-radius:8px;border:1px solid var(--input);background:var(--background);color:var(--foreground);font-size:16px}input::placeholder{color:var(--text-placeholder)}.error{font-size:14px;color:var(--destructive);margin-top:8px}.actions{display:flex;justify-content:flex-end;gap:8px;margin-top:24px}.actions button{min-height:44px;padding:10px 16px;border-radius:8px;font-size:14px;cursor:pointer}.btn-cancel{background:var(--secondary);color:var(--foreground);border:1px solid var(--border)}.btn-submit{background:var(--primary);color:var(--primary-foreground)}button:disabled{opacity:.5;cursor:not-allowed}
</style>
