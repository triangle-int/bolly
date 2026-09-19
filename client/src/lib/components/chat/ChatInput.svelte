<script lang="ts">
 import { play } from '$lib/sounds.js';
 import { hapticLight, hapticMedium } from '$lib/haptics.js';
 import { fetchConfigStatus, updateModelMode } from '$lib/api/client.js';
 import PromptComposer from './PromptComposer.svelte';
 let { onSend, onStop, disabled = false, agentRunning = false, mood = 'calm', uploadProgress = null }:
 { onSend: (content: string, files?: File[]) => void | boolean | Promise<void | boolean>; onStop: () => void; disabled?: boolean; agentRunning?: boolean; mood?: string; uploadProgress?: { fileIndex: number; fileCount: number; loaded: number; total: number } | null } = $props();
 let modelMode = $state('auto');
 let modelError = $state('');
 let changingMode = false;
 $effect(() => { fetchConfigStatus().then(s => { if (s.model_mode) modelMode = s.model_mode; }).catch(() => {}); });
 async function changeMode(mode: string) {
  if (changingMode) return;
  changingMode = true;
  modelError = '';
  try { await updateModelMode(mode); modelMode = mode; hapticLight(); }
  catch { modelError = 'Could not change model mode. Please try again.'; }
  finally { changingMode = false; }
 }
 async function send(content: string, files?: File[]) { play('message_send'); hapticLight(); return await onSend(content, files); }
</script>
<div class="chat-composer" data-mood={mood}>
 <PromptComposer onSend={send} {onStop} {disabled} {agentRunning} {modelMode} onModelChange={changeMode} onFileAdd={() => { play('attachment_added'); hapticMedium(); }}>
  {#snippet footer()}
   {#if uploadProgress}
    {@const pct = uploadProgress.total > 0 ? Math.min(100, uploadProgress.loaded / uploadProgress.total * 100) : 0}
    <div class="upload"><progress value={pct} max="100" aria-label="File upload progress"></progress><span role="status">Uploading file {uploadProgress.fileIndex + 1} of {uploadProgress.fileCount} · {pct.toFixed(0)}%</span></div>
   {/if}
   {#if modelError}<p role="alert">{modelError}</p>{/if}
  {/snippet}
 </PromptComposer>
</div>
<style>
 .chat-composer{padding:12px 24px max(16px,env(safe-area-inset-bottom));width:100%;max-width:688px;margin:0 auto;min-width:0;flex-shrink:0}.upload{display:flex;flex-direction:column;gap:6px;padding:8px 0;color:var(--text-secondary);font-size:12px}progress{width:100%;height:4px;accent-color:var(--primary)}p{color:var(--destructive);font-size:13px}@media(max-width:720px){.chat-composer{padding-left:12px;padding-right:12px}}
</style>
