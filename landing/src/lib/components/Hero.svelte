<section class="hero" aria-labelledby="hero-title">
	<div class="grid">
		<div class="copy">
			<p class="eyebrow">Open source · self-hosted · always present</p>
			<h1 id="hero-title">Your AI, with a computer of its own.</h1>
			<p class="lede">Run Bolly on an always-on computer, connect your other devices, and ask it to work across them from one interface.</p>
			<div class="actions"><a class="btn-primary" href="#install">Install Bolly <span>↓</span></a><a class="btn-secondary" href="https://github.com/triangle-int/bolly" target="_blank" rel="noopener">View on GitHub ↗</a></div>
			<div class="proof"><span>MIT licensed</span><span>macOS + Linux server</span><span>Desktop apps</span></div>
		</div>

		<div class="system" aria-label="Bolly connected computer system">
			<div class="wire wire-a"></div><div class="wire wire-b"></div>
			<div class="machine home"><span class="status-dot"></span><div><strong>Mac mini · home</strong><small>ONLINE · BOLLY BRAIN</small></div></div>
			<div class="orb-wrap">
				<picture><source media="(prefers-reduced-motion: reduce)" srcset="/assets/hero-orb.webp" /><img class="poster" src="/assets/hero-orb.webp" alt="Bolly's amber liquid-glass orb" /></picture>
				<video class="orb-video" autoplay muted loop playsinline preload="auto" poster="/assets/hero-orb.webp" aria-hidden="true"><source src="/assets/hero-orb-connected.mp4" type="video/mp4" /></video>
				<div class="core-label"><span></span> ONE MIND · ACTIVE</div>
			</div>
			<div class="machine laptop"><span class="status-dot active"></span><div><strong>MacBook · connected</strong><small>OPENING XCODE…</small></div></div>
		</div>
	</div>
</section>

<style>
	.hero{min-height:100svh;display:grid;align-items:center;padding:8rem 1.5rem 4rem;position:relative;overflow:hidden;border-bottom:1px solid var(--color-border)}.hero::before{content:'';position:absolute;inset:0;background:linear-gradient(90deg,transparent 49.9%,oklch(.78 .12 75/.045) 50%,transparent 50.1%),linear-gradient(oklch(1 0 0/.018) 1px,transparent 1px);background-size:100% 100%,100% 80px;mask-image:linear-gradient(to bottom,black,transparent)}.grid{width:100%;max-width:1180px;margin:auto;display:grid;grid-template-columns:.95fr 1.05fr;align-items:center;gap:5vw;position:relative}.copy{z-index:2}.copy h1{font-size:clamp(3.25rem,6vw,5.8rem);line-height:.93;letter-spacing:-.065em;font-weight:500;max-width:700px;margin:1.35rem 0 1.5rem}.lede{font-size:clamp(1rem,1.5vw,1.2rem);line-height:1.65;color:var(--color-text-dim);max-width:590px}.actions{display:flex;gap:.75rem;margin:2rem 0 2.4rem}.proof{display:flex;flex-wrap:wrap;gap:.65rem 1.4rem;font-family:var(--font-mono);font-size:.63rem;letter-spacing:.08em;color:var(--color-text-ghost);text-transform:uppercase}.proof span::before{content:'◆';font-size:.45rem;color:var(--color-warm-dim);margin-right:.5rem}
	.system{height:620px;position:relative;display:grid;place-items:center}.system::before{content:'';position:absolute;width:70%;aspect-ratio:1;border:1px solid oklch(.78 .12 75/.1);border-radius:50%;box-shadow:0 0 100px oklch(.78 .12 75/.07),inset 0 0 80px oklch(.78 .12 75/.03)}.orb-wrap{width:min(500px,90%);aspect-ratio:1;position:relative;display:grid;place-items:center;z-index:2}.poster,.orb-video{position:absolute;width:100%;height:100%;object-fit:cover;mask-image:radial-gradient(circle,black 8%,transparent 64%)}.orb-video{z-index:1}.core-label{position:absolute;bottom:7%;z-index:3;font:600 .58rem var(--font-mono);letter-spacing:.14em;color:var(--color-warm-dim)}.core-label span{display:inline-block;width:5px;height:5px;border-radius:50%;background:var(--color-warm);box-shadow:0 0 10px var(--color-warm);margin-right:.5rem;animation:pulse 2s infinite}.machine{position:absolute;z-index:5;display:flex;align-items:center;gap:.8rem;padding:.85rem 1rem;background:#0b0d12;border:1px solid var(--color-border-warm);box-shadow:0 12px 40px #0008;min-width:200px}.machine strong{display:block;font-size:.74rem;font-weight:500}.machine small{display:block;margin-top:.3rem;font:500 .55rem var(--font-mono);letter-spacing:.08em;color:var(--color-text-ghost)}.home{top:10%;left:0}.laptop{right:-1rem;bottom:11%}.status-dot{width:7px;height:7px;background:#65d49a;border-radius:50%;box-shadow:0 0 12px #65d49a99}.status-dot.active{background:var(--color-warm);box-shadow:0 0 12px var(--color-warm)}.wire{position:absolute;height:1px;background:linear-gradient(90deg,var(--color-warm),transparent);transform-origin:left;z-index:1}.wire::after{content:'';position:absolute;width:5px;height:5px;border-radius:50%;background:var(--color-warm);top:-2px;animation:travel 2.5s linear infinite}.wire-a{width:180px;top:27%;left:25%;transform:rotate(23deg)}.wire-b{width:190px;right:22%;bottom:29%;transform:rotate(31deg)}
	@keyframes pulse{50%{opacity:.35}}@keyframes travel{from{left:0}to{left:100%;opacity:0}}
	@media(max-width:900px){.hero{padding-top:7rem}.grid{grid-template-columns:1fr;text-align:center}.copy h1,.lede{margin-left:auto;margin-right:auto}.actions,.proof{justify-content:center}.system{height:500px}.home{left:5%}.laptop{right:3%}}@media(max-width:560px){.copy h1{font-size:clamp(2.8rem,14vw,4rem)}.actions{flex-direction:column}.system{height:390px}.machine{min-width:auto;padding:.65rem}.machine strong{font-size:.62rem}.machine small{font-size:.48rem}.home{top:7%;left:0}.laptop{right:0;bottom:4%}.wire{display:none}}@media(prefers-reduced-motion:reduce){.orb-video{display:none}.core-label span,.wire::after{animation:none}}
</style>
