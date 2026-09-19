<script lang="ts">
 import PromptInput from '$lib/components/ai-elements/prompt-input/core/root.svelte';
 import PromptTextarea from '$lib/components/ai-elements/prompt-input/controls/textarea.svelte';
 import PromptSubmit from '$lib/components/ai-elements/prompt-input/controls/submit.svelte';
 import PromptToolbar from '$lib/components/ai-elements/prompt-input/layout/toolbar.svelte';
 import type { Message, PromptInputAttachment } from '$lib/components/ai-elements/prompt-input/context/types.js';
 import PromptAttachments from './PromptAttachments.svelte';
 import PromptAttachButton from './PromptAttachButton.svelte';
 import type { Snippet } from 'svelte';
 import * as Select from '$lib/components/ui/select/index.js';
 const modelModes = [
  { value: 'auto', label: 'Auto' },
  { value: 'fast', label: 'Fast' },
  { value: 'heavy', label: 'Heavy' },
 ];
 let { onSend, onStop, disabled = false, agentRunning = false, modelMode, onModelChange, footer, onFileAdd }:
 { onSend: (text: string, files?: File[]) => void | boolean | Promise<void | boolean>; onStop: () => void; disabled?: boolean; agentRunning?: boolean; modelMode?: string; onModelChange?: (mode: string) => void; footer?: Snippet; onFileAdd?: () => void } = $props();
 const modelId = $props.id();
 let value = $state('');
 let attachments = $state<PromptInputAttachment[]>([]);
 let submitting = $state(false);
 let error = $state('');
 let textarea: HTMLTextAreaElement | null = $state(null);
 const busy = $derived(disabled || submitting);
 async function submit(message: Message) {
  if (busy || agentRunning) return false;
  submitting = true;
  error = '';
  try {
   const result = await onSend(message.text.trim(), message.attachments.length ? message.attachments.map(a => a.file) : undefined);
   if (result === false) { error = 'Message not sent. Your draft and files are still here.'; return false; }
  } catch {
   error = 'Message not sent. Your draft and files are still here.';
   return false;
  } finally { submitting = false; }
 }
 $effect(() => {
  if (!busy && textarea && document.activeElement === document.body) textarea.focus();
 });
</script>
<div class="composer">
 <PromptInput class="border-input bg-card shadow-none focus-within:border-ring" bind:attachments multiple disabled={busy || agentRunning} onSubmit={submit} {onFileAdd} onError={e => error = e.message}>
  <PromptAttachments disabled={busy} />
  <PromptTextarea bind:value bind:ref={textarea} aria-label="Message Nolune" placeholder="What’s on your mind?" disabled={submitting || (disabled && !agentRunning)} class="min-h-20 p-4 text-base md:text-base" />
  <PromptToolbar class="gap-2 px-2 pb-2">
   <div class="flex min-w-0 items-center gap-1">
    <PromptAttachButton disabled={busy || agentRunning} />
    {#if modelMode && onModelChange}
     <label class="sr-only" for={modelId}>Model mode</label>
     <Select.Root type="single" items={modelModes} disabled={busy}
      bind:value={() => modelMode ?? 'auto', next => onModelChange?.(next)}>
      <Select.Trigger id={modelId} aria-label="Model mode" class="w-28 gap-3 border-border bg-card px-3 text-[13px] text-secondary-foreground shadow-none data-[size=default]:h-11 dark:bg-card dark:hover:bg-accent">
       <span data-slot="select-value">{modelModes.find(mode => mode.value === modelMode)?.label ?? "Model mode"}</span>
      </Select.Trigger>
      <Select.Content side="top" align="start" sideOffset={8} class="min-w-40 border border-border p-1 shadow-lg">
       {#each modelModes as mode (mode.value)}
        <Select.Item value={mode.value} label={mode.label} class="min-h-11 pl-3 pr-9">{mode.label}</Select.Item>
       {/each}
      </Select.Content>
     </Select.Root>
    {:else}<span class="hint">Enter to send</span>{/if}
   </div>
   <PromptSubmit class="size-11 shrink-0 bg-primary text-primary-foreground hover:bg-primary/90" status={agentRunning ? 'streaming' : submitting || disabled ? 'submitted' : 'ready'} {onStop} disabled={!agentRunning && (busy || (!value.trim() && !attachments.length))} />
  </PromptToolbar>
 </PromptInput>
 {#if error}<p role="alert" class="error">{error}</p>{/if}
 {@render footer?.()}
</div>
<style>
 /* The form provides the focus border; an inner outline would divide the composer. */
 .composer :global(textarea:focus-visible){outline:none}

 .composer{width:100%;min-width:0}.hint{font-size:12px;color:var(--text-muted)}.error{font-size:13px;color:var(--destructive);padding:8px 4px;margin:0}
</style>
