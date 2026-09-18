<script lang="ts">
	import Reveal from './Reveal.svelte';
	const nodes = [
		{ n: '01', title: 'A home for one persistent mind', text: 'Your Mac mini or home server keeps Nolune’s identity, memory, tools, and routines running around the clock.' },
		{ n: '02', title: 'Eyes and hands where you work', text: 'Desktop apps connect the screen, apps, files, and terminal on your MacBook or desktop — only on machines you choose.' },
		{ n: '03', title: 'One interface across all of it', text: 'Talk to the same Nolune, select a connected computer, and follow each action from request to completion.' }
	];
</script>
<section id="computer-use" class="system-section">
	<div class="section-shell">
		<Reveal><p class="eyebrow">One mind · many surfaces</p><h2 class="section-title">Nolune lives at home.<br />Its reach travels with you.</h2><p class="section-copy">The always-on server is Nolune’s brain. Native desktop apps are secure bridges to the computers where your work actually happens.</p></Reveal>
		<div class="topology" aria-label="Diagram of Nolune connecting its home server to desktop computers">
			<div class="node primary"><div class="node-head"><span class="light"></span><span>MAC MINI · HOME</span></div><strong>Nolune brain</strong><p>identity / memory / schedules</p><small>ONLINE · 18D 04H</small></div>
			<div class="route"><span>encrypted connection</span><i></i></div>
			<div class="endpoints"><div class="node"><div class="node-head"><span class="light amber"></span><span>MACBOOK · WORK</span></div><strong>Desktop app</strong><p>screen / Xcode / files / terminal</p><small>CONNECTED</small></div><div class="node muted"><div class="node-head"><span class="light"></span><span>LINUX · STUDIO</span></div><strong>Desktop app</strong><p>browser / files / terminal</p><small>AVAILABLE</small></div></div>
		</div>
		<div class="principles">{#each nodes as item, i}<Reveal delay={i * 90}><article><span>{item.n}</span><h3>{item.title}</h3><p>{item.text}</p></article></Reveal>{/each}</div>
	</div>
</section>
<style>
	.system-section{border-bottom:1px solid var(--color-border);background:#0b0d11}.section-title{max-width:800px}.topology{margin:4.5rem 0 3rem;display:grid;grid-template-columns:1fr .7fr 1fr;align-items:center}.node{background:var(--surface-2);border:1px solid var(--color-border);padding:1.4rem;box-shadow:0 18px 50px #0005}.node.primary{border-color:var(--color-border-warm)}.node-head{display:flex;align-items:center;gap:.5rem;font:600 .58rem var(--font-mono);letter-spacing:.12em;color:var(--color-text-ghost);padding-bottom:1rem;border-bottom:1px solid var(--color-border)}.light{width:6px;height:6px;border-radius:50%;background:#61d298;box-shadow:0 0 10px #61d29888}.light.amber{background:var(--color-warm);box-shadow:0 0 10px var(--color-warm)}.node strong{display:block;font-size:1.2rem;margin:1.3rem 0 .35rem}.node p{margin:0 0 1.4rem;color:var(--color-text-dim);font-size:.83rem}.node small{font:500 .56rem var(--font-mono);letter-spacing:.1em;color:var(--color-warm-dim)}.endpoints{display:grid;gap:.75rem}.node.muted{opacity:.68}.route{height:1px;background:linear-gradient(90deg,var(--color-border-warm),var(--color-warm),var(--color-border-warm));position:relative}.route span{position:absolute;bottom:.8rem;left:50%;transform:translateX(-50%);white-space:nowrap;font:500 .52rem var(--font-mono);letter-spacing:.1em;color:var(--color-text-ghost)}.route i{position:absolute;width:8px;height:8px;background:var(--color-warm);border-radius:50%;top:-3px;animation:route 3s infinite}.principles{display:grid;grid-template-columns:repeat(3,1fr);border-top:1px solid var(--color-border)}article{padding:2.2rem 2rem 0 0}article>span{font:500 .6rem var(--font-mono);color:var(--color-warm-dim)}article h3{font-size:1.05rem;margin:1rem 0 .6rem}article p{font-size:.83rem;line-height:1.65;color:var(--color-text-dim);max-width:310px}@keyframes route{from{left:0}to{left:100%}}
	@media(max-width:760px){.topology{grid-template-columns:1fr;margin-top:3rem}.route{height:70px;width:1px;margin:auto;background:linear-gradient(var(--color-border-warm),var(--color-warm),var(--color-border-warm))}.route span{bottom:auto;left:1rem;top:50%;transform:rotate(90deg) translateY(-50%);transform-origin:left}.route i{left:-3px;animation:route-v 3s infinite}.principles{grid-template-columns:1fr}.principles article{padding-right:0;border-bottom:1px solid var(--color-border)}}@keyframes route-v{from{top:0}to{top:100%}}@media(prefers-reduced-motion:reduce){.route i{animation:none}}
</style>
