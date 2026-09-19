<script lang="ts">
 import { getAttachmentsContext } from '$lib/components/ai-elements/prompt-input/context/attachments.svelte.js';
 import { FileText, X } from '@lucide/svelte';
 let { disabled = false }: { disabled?: boolean } = $props();
 const context = getAttachmentsContext();
</script>
{#if context.attachments.length}
 <ul class="attachments" aria-label="Attached files">
  {#each context.attachments as attachment (attachment.id)}
   <li>
    {#if attachment.mediaType.startsWith('image/') && attachment.previewUrl}<img src={attachment.previewUrl} alt="" />{:else}<FileText size={18} />{/if}
    <span>{attachment.filename}</span>
    <button type="button" {disabled} aria-label={`Remove ${attachment.filename}`} onclick={() => context.remove(attachment.id)}><X size={16} /></button>
   </li>
  {/each}
 </ul>
{/if}
<style>
 .attachments{display:flex;flex-wrap:wrap;gap:8px;padding:12px 12px 0;margin:0;list-style:none}li{display:flex;align-items:center;gap:8px;max-width:100%;padding-left:10px;background:var(--accent);border:1px solid var(--border);border-radius:8px;color:var(--foreground);font-size:13px}li>span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap;max-width:180px}img{width:28px;height:28px;object-fit:cover;border-radius:4px}button{display:grid;place-items:center;width:44px;height:44px;flex-shrink:0}button:disabled{opacity:.5}
</style>
