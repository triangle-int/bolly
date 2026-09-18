<script lang="ts">
 import { onMount } from 'svelte';
 const expressions = ['curious', 'happy', 'sleepy', 'wink'] as const;
 let expression = $state(0);
 let paused = $state(false);
 let visible = $state(true);
 let container: HTMLDivElement;
 onMount(() => {
  const observer = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; });
  observer.observe(container);
  return () => observer.disconnect();
 });
</script>

<div class="companion" bind:this={container} class:paused={paused || !visible}>
 <button class="moon" onclick={() => expression = (expression + 1) % expressions.length} aria-label={`Nolune is ${expressions[expression]}. Change expression`} title="Say hello to Nolune">
  <svg viewBox="0 0 320 320" fill="none" aria-hidden="true">
   <g class="body">
    <path d="M155 24C161 23 164 29 160 34C127 74 130 129 157 168C183 207 228 224 277 210C284 208 289 214 285 221C262 266 217 292 170 290C93 287 36 230 36 157C36 91 85 34 155 24Z" fill="#B7A9E7" />
    <g class="face">
     {#if expressions[expression] === 'happy'}
      <path d="M75 165Q81 151 87 165M104 165Q110 151 116 165" stroke="#201D29" stroke-width="5" stroke-linecap="round" />
     {:else if expressions[expression] === 'sleepy'}
      <path d="M75 165Q81 170 87 165M104 165Q110 170 116 165" stroke="#201D29" stroke-width="5" stroke-linecap="round" />
     {:else if expressions[expression] === 'wink'}
      <ellipse class="eye" cx="81" cy="164" rx="6" ry="9" fill="#201D29" />
      <path d="M104 165Q110 158 116 165" stroke="#201D29" stroke-width="5" stroke-linecap="round" />
     {:else}
      <g class="eyes"><ellipse cx="81" cy="164" rx="6" ry="9" fill="#201D29" /><ellipse cx="110" cy="161" rx="6" ry="10" fill="#201D29" /></g>
     {/if}
    </g>
   </g>
  </svg>
 </button>
 <div class="controls"><span>Tap the moon. Say hello.</span><button class="pause" onclick={() => paused = !paused} aria-label={paused ? 'Resume moon animation' : 'Pause moon animation'} aria-pressed={paused}>{paused ? 'Play' : 'Pause'} <span aria-hidden="true">{paused ? '▷' : 'Ⅱ'}</span></button></div>
</div>

<style>
.companion{width:100%;max-width:470px}.moon{display:block;padding:0;width:100%;border-radius:45%;cursor:pointer}.moon svg{width:100%;height:auto;display:block;overflow:visible}.body{transform-origin:160px 165px;animation:float 7s ease-in-out infinite}.face{animation:glance 11s ease-in-out infinite;transform-origin:95px 164px}.eyes,.eye{transform-box:fill-box;transform-origin:center;animation:blink 6.5s ease-in-out infinite}.moon:active svg{transform:scale(.97)}.controls{display:flex;justify-content:center;align-items:center;gap:1rem;margin-top:.5rem;font-size:.65rem;color:#b3aabe}.pause{padding:.5rem .6rem;border:1px solid #ffffff20;border-radius:4px;font-size:.65rem}.pause:hover{color:#f6f3ec;border-color:#b7a9e7}.paused .body,.paused .face,.paused .eyes,.paused .eye{animation-play-state:paused}
@keyframes float{0%,100%{transform:translateY(0) rotate(-2deg)}50%{transform:translateY(-9px) rotate(2deg)}}
@keyframes glance{0%,25%,45%,100%{transform:translate(0,0)}30%,40%{transform:translate(4px,-3px)}70%,80%{transform:translate(-3px,2px)}}
@keyframes blink{0%,42%,46%,73%,77%,100%{transform:scaleY(1)}44%,75%{transform:scaleY(.08)}}
@media(prefers-reduced-motion:reduce){.body,.face,.eyes,.eye{animation:none}.pause{display:none}.moon:active svg{transform:none}}
</style>
