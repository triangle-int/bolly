<script lang="ts">
 import MoonCompanion from './MoonCompanion.svelte';
</script>
<section class="hero" aria-labelledby="hero-title">
 <div class="hero-inner">
  <div class="copy">
   <p class="eyebrow">A personal AI, entirely yours</p>
   <h1 id="hero-title">A little more<br />space for you.</h1>
   <p class="lede">An AI companion that remembers you<br class="desktop-break" /> and works across your computers.</p>
   <div class="actions"><a class="btn-primary" href="#install">Meet Nolune <span aria-hidden="true">↗</span></a><a class="btn-secondary" href="https://github.com/triangle-int/nolune" target="_blank" rel="noopener">View on GitHub <span aria-hidden="true">↗</span></a></div>
   <p class="proof"><span aria-hidden="true">◇</span> Open source. Self-hosted. Yours.</p>
   <div class="message" aria-label="Example message from Nolune"><img src="/assets/nolune-moon.svg" alt="" width="40" height="40" /><div><div class="message-head"><strong>nolune</strong><span>9:00 AM</span></div><p>Your morning brief is ready.</p></div></div>
  </div>
  <div class="character"><MoonCompanion /></div>
 </div>
 <div class="hero-foot"><span>A familiar presence. A little more possibility.</span><a href="#companion">Get to know Nolune <span aria-hidden="true">↓</span></a></div>
</section>
<style>
.hero{padding:9.5rem 2.5rem 0;background:#201d29}.hero-inner{max-width:1160px;margin:auto;display:grid;grid-template-columns:1.2fr 1fr;align-items:center;gap:2rem;min-height:560px}.copy{padding:1rem 0 3rem}.eyebrow{margin:0 0 1.8rem}.copy h1{font:400 clamp(3.7rem,6.8vw,6.1rem)/1.02 var(--font-display);letter-spacing:-.035em;margin:0 0 1.65rem}.lede{font-size:1.15rem;line-height:1.6;color:#d4cddb;margin:0}.actions{display:flex;gap:.85rem;margin-top:1.8rem}.proof{display:flex;gap:.5rem;align-items:center;color:#b3aabe;font-size:.74rem;margin:1rem 0 2.7rem}.proof span{font-size:1.35rem;color:#b7a9e7}.character{display:flex;align-items:center;justify-content:center;padding-bottom:2rem}.message{display:inline-flex;align-items:center;gap:.7rem;border:1px solid #ffffff19;border-radius:12px;padding:.85rem 1.1rem .85rem .7rem;background:#ffffff02}.message-head{display:flex;align-items:center;gap:1.2rem}.message strong{font-size:.8rem;font-weight:500}.message-head span{font-size:.62rem;color:#aaa2b8}.message p{font-size:.82rem;color:#d4cddb;margin:.35rem 0 0}.hero-foot{max-width:1160px;margin:1rem auto 0;border-top:1px solid #ffffff15;display:flex;justify-content:space-between;padding:1.35rem 0;font-size:.72rem;color:#b7aec3}.hero-foot a{color:#e7e0ee}.hero-foot a span{margin-left:1rem}
@media(min-width:1500px){.hero{padding-top:11rem}.hero-inner{min-height:640px}}@media(max-width:760px){.hero{padding:7rem 1.4rem 0}.hero-inner{grid-template-columns:1fr;gap:0}.copy{padding:0}.copy h1{font-size:clamp(3.5rem,11vw,5rem)}.character{grid-row:2;padding:1rem 0;width:65%;max-width:300px;justify-self:center}.proof{margin-bottom:1.8rem}.hero-foot>span{display:none}.hero-foot{justify-content:center}.message{font-size:.75rem}.desktop-break{display:none}}@media(max-width:380px){.actions{flex-direction:column}.copy h1{font-size:3.1rem}}
</style>
