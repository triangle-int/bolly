<script lang="ts">
 import Tool from '$lib/components/ai-elements/tool/tool.svelte';
 import ToolHeader from '$lib/components/ai-elements/tool/tool-header.svelte';
 import ToolContent from '$lib/components/ai-elements/tool/tool-content.svelte';
 let { kind = 'tool', label, timestamp }: { kind?: 'tool' | 'mood' | 'state' | 'output'; label: string; timestamp?: string } = $props();
 const displayLabel = $derived(label.replace(/\\n/g, '\n').replace(/\\t/g, '\t'));
 const firstLine = $derived(displayLabel.split('\n')[0]);
</script>
{#if kind === 'tool' || kind === 'output'}
 <Tool class="my-2 min-w-0 rounded-xl border-border bg-card">
  <ToolHeader type={kind === 'output' ? 'Tool output' : 'Tool activity'} state="recorded" />
  <ToolContent>
   <div class="details"><pre>{displayLabel}</pre>{#if timestamp}<span class="timestamp">{timestamp}</span>{/if}</div>
  </ToolContent>
 </Tool>
{:else}
 <div class="activity"><span>{firstLine}</span>{#if timestamp}<span class="timestamp">{timestamp}</span>{/if}</div>
{/if}
<style>
 .details{padding:0 16px 16px;min-width:0}pre{font:400 12px/1.65 var(--font-mono);white-space:pre-wrap;overflow-wrap:anywhere;max-height:300px;overflow:auto;margin:0 0 8px;color:var(--text-secondary)}.activity{display:flex;justify-content:space-between;gap:12px;padding:8px 0;color:var(--text-muted);font-size:12px}.activity>span:first-child{overflow-wrap:anywhere;min-width:0}.timestamp{color:var(--text-timestamp);font-size:11px;white-space:nowrap}
</style>
