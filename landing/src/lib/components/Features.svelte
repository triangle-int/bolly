<script lang="ts">
	import Reveal from './Reveal.svelte';
	const traits = [
		{ mark: 'MEM', title: 'Inspectable memory', text: 'Memories are ordinary files you can read, edit, back up, or remove.' },
		{ mark: 'VOX', title: 'A voice and a mood', text: 'Voice, changing moods, and animated skins make the same identity feel present.' },
		{ mark: 'HB', title: 'Proactive, by choice', text: 'Heartbeats and schedules let Bolly check in and follow routines you define.' }
	];
</script>
<section id="companion" class="companion">
	<div class="section-shell">
		<div class="intro"><Reveal><p class="eyebrow">More than remote control</p><h2 class="section-title">The computer is capable.<br /><span>The companion is continuous.</span></h2></Reveal><Reveal delay={100}><p class="section-copy">Bolly is one recognizable presence across every conversation and connected computer — not a drawer full of disposable agent threads.</p></Reveal></div>
		<div class="identity">
			<Reveal><div class="portrait"><img src="/assets/plan-companion.png" alt="Bolly's warm amber companion skin" /><div class="portrait-meta"><span>IDENTITY / BOLLY</span><b>calm · curious · present</b></div></div></Reveal>
			<div class="trait-list">{#each traits as trait, i}<Reveal delay={i * 90}><article><span>{trait.mark}</span><div><h3>{trait.title}</h3><p>{trait.text}</p></div></article></Reveal>{/each}</div>
		</div>
		<p class="skin-note"><i></i> Skins change Bolly’s expression, not its memory or identity. Deeper customization is planned.</p>
	</div>
</section>
<style>
	.companion{border-bottom:1px solid var(--color-border);background:#0d0f14}.intro{display:grid;grid-template-columns:1.2fr .8fr;gap:5rem;align-items:end}.section-title span{font-family:var(--font-display);font-style:italic;font-weight:400;color:var(--color-warm)}.identity{display:grid;grid-template-columns:.85fr 1.15fr;gap:5rem;margin-top:4.5rem;align-items:center}.portrait{height:480px;position:relative;background:#08090d;border:1px solid var(--color-border-warm);overflow:hidden}.portrait::before{content:'';position:absolute;inset:0;background:radial-gradient(circle at 50% 45%,var(--color-warm-glow),transparent 55%)}.portrait img{width:100%;height:100%;object-fit:contain;position:relative}.portrait-meta{position:absolute;inset:auto 1rem 1rem;padding:1rem;background:#090b0edd;border-top:1px solid var(--color-border-warm)}.portrait-meta span,.portrait-meta b{display:block}.portrait-meta span{font:.52rem var(--font-mono);letter-spacing:.1em;color:var(--color-warm-dim)}.portrait-meta b{font-size:.72rem;margin-top:.45rem}.trait-list article{display:grid;grid-template-columns:50px 1fr;gap:1.2rem;padding:1.6rem 0;border-top:1px solid var(--color-border)}.trait-list article>span{font:600 .55rem var(--font-mono);color:var(--color-warm-dim)}article h3{font-size:1.1rem;margin:0 0 .5rem}article p{font-size:.85rem;color:var(--color-text-dim);line-height:1.65;margin:0;max-width:430px}.skin-note{font:500 .58rem var(--font-mono);letter-spacing:.05em;color:var(--color-text-ghost);margin:2rem 0 0}.skin-note i{display:inline-block;width:18px;height:1px;background:var(--color-warm);vertical-align:middle;margin-right:.6rem}@media(max-width:760px){.intro,.identity{grid-template-columns:1fr;gap:2rem}.identity{margin-top:3rem}.portrait{height:390px}}
</style>
