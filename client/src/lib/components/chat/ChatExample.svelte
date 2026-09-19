<script lang="ts">
 import { onDestroy } from 'svelte';
 import MessageBubble from './MessageBubble.svelte';
 import PromptComposer from './PromptComposer.svelte';
 import StreamActivity from './StreamActivity.svelte';
 import Conversation from '$lib/components/ai-elements/conversation/conversation.svelte';
 import ConversationContent from '$lib/components/ai-elements/conversation/conversation-content.svelte';
 import type { ChatMessage } from '$lib/api/types.js';
 let messages = $state<ChatMessage[]>([
  { id: 'example-user', role: 'user', content: 'Find my notes and help me plan a quieter afternoon.', created_at: '1767258000000' },
  { id: 'example-assistant', role: 'assistant', content: 'Here’s a little room to breathe.\n\n1. Finish the project brief.\n2. Leave space for a walk.\n3. Move the rest to tomorrow.\n\n**One thing at a time is enough.**', created_at: '1767258001000' }
 ]);
 let generating = $state(false);
 let failNext = $state(false);
 let modelMode = $state('auto');
 let status = $state('Sample conversation. Messages and files stay in this page and disappear on reload.');
 let timer: ReturnType<typeof setInterval> | undefined;
 function stop() { clearInterval(timer); generating = false; status = 'Example response stopped.'; }
 onDestroy(() => clearInterval(timer));
 function send(text: string, files?: File[]) {
  if (failNext) { failNext = false; status = 'Simulated failure. Try sending the same draft again.'; return false; }
  const now = Date.now();
  const id = `example-${now}`;
  messages.push({ id, role: 'user', content: text + (files?.length ? `\n\nSelected files (not uploaded): ${files.map(f => f.name).join(', ')}` : ''), created_at: String(now) });
  const reply: ChatMessage = { id: id + '-reply', role: 'assistant', content: '', created_at: String(now + 1) };
  messages.push(reply);
  generating = true;
  status = 'Simulating a streaming reply. No model or server is being contacted.';
  const response = 'A little more space, one step at a time. This is a sample streaming response using Nolune’s real message components.';
  let length = 0;
  timer = setInterval(() => {
   length += 4;
   messages[messages.length - 1].content = response.slice(0, length);
   if (length >= response.length) { clearInterval(timer); generating = false; status = 'Example complete. No data was saved.'; }
  }, 70);
  return true;
 }
</script>
<div class="example">
 <header><img src="/skins/moon/character.svg" alt="" width="32" height="32" /><div><strong>Nolune</strong><span>Interactive example</span></div><label><input type="checkbox" bind:checked={failNext} disabled={generating} /> Fail next send</label></header>
 <Conversation class="h-[440px] min-h-0" aria-label="Sample conversation">
  <ConversationContent class="min-h-0 flex-1 gap-1 overflow-y-auto px-4 py-4 md:px-6">
   {#each messages as message, index (message.id)}
    <MessageBubble {message} {index} streaming={generating && index === messages.length - 1} />
    {#if index === 0}<StreamActivity kind="output" label={'find_notes\nFound 3 notes in the project folder.\nExample output — no files were accessed.'} timestamp="09:00" />{/if}
   {/each}
  </ConversationContent>
 </Conversation>
 <div class="input"><PromptComposer onSend={send} onStop={stop} agentRunning={generating} disabled={generating} {modelMode} onModelChange={mode => modelMode = mode} /></div>
 <p role="status">{status}</p>
</div>
<style>
 .example{border:1px solid var(--border);border-radius:16px;background:var(--background);overflow:hidden;max-width:800px}header{display:flex;gap:12px;align-items:center;padding:16px 20px;border-bottom:1px solid var(--border);background:var(--card)}header>div{display:grid;gap:2px}strong{font-size:15px;font-weight:500}header span{font-size:12px;color:var(--text-muted)}label{margin-left:auto;display:flex;align-items:center;gap:8px;font-size:12px;color:var(--text-muted);min-height:44px}input{accent-color:var(--primary)}.input{padding:12px 16px 0}p{font-size:12px;line-height:1.6;color:var(--text-muted);padding:12px 20px;margin:0}@media(max-width:500px){header{padding:12px;gap:8px}label{max-width:105px}.input{padding:8px}}
</style>
