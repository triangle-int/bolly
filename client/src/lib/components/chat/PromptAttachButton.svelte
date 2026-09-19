<script lang="ts">
 import { getAttachmentsContext } from '$lib/components/ai-elements/prompt-input/context/attachments.svelte.js';
 import { Paperclip } from '@lucide/svelte';
 let { disabled = false }: { disabled?: boolean } = $props();
 const context = getAttachmentsContext();
</script>
<button type="button" {disabled} onclick={context.openFileDialog} aria-label="Attach files" title="Attach files" class="flex size-11 shrink-0 items-center justify-center rounded-lg text-muted-foreground hover:bg-accent hover:text-foreground disabled:opacity-50"><Paperclip size={18} /></button>
