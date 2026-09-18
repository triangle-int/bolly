<script lang="ts">
	import Reveal from './Reveal.svelte';
	let copied = $state(false);
	let copyStatus = $state('');
	let resetTimer: ReturnType<typeof setTimeout>;
	const command = 'curl -fsSL https://nolune.dev/install.sh | bash';
	async function copyCommand() {
		clearTimeout(resetTimer);
		try {
			await navigator.clipboard.writeText(command);
			copied = true;
			copyStatus = 'Install command copied to clipboard.';
		} catch {
			copied = false;
			copyStatus = 'Could not copy the install command. Select and copy it manually.';
		}
		resetTimer = setTimeout(() => {
			copied = false;
			copyStatus = '';
		}, 1800);
	}
</script>
<section id="install" class="install"><div class="section-shell"><div class="install-grid"><Reveal><div><p class="eyebrow">Native installation</p><h2 class="section-title">A home for Nolune,<br />on hardware you own.</h2><p class="section-copy">Install the server on macOS or Linux, then download the desktop app for every computer you want to connect.</p><div class="facts"><span>OPEN SOURCE</span><span>MIT LICENSED</span><span>BYOK</span></div></div></Reveal><Reveal delay={100}><div class="terminal"><div class="terminal-head"><span>TERMINAL · INSTALL SERVER</span><i>macOS / Linux</i></div><div class="command"><span>$</span><code>{command}</code><button onclick={copyCommand} aria-label="Copy install command">{copied ? 'COPIED' : 'COPY'}</button></div><p class="copy-status" aria-live="polite" aria-atomic="true">{copyStatus}</p><div class="output"><p><b>01</b> Downloads the native server</p><p><b>02</b> Creates the local service</p><p><b>03</b> Opens onboarding at localhost:26559</p></div><a class="release" href="https://github.com/triangle-int/nolune/releases" target="_blank" rel="noopener"><span>DESKTOP APPS</span><strong>Download for macOS, Windows, or Linux</strong><i>↗</i></a></div></Reveal></div></div></section>
<style>
	.install{background:#0b0d11;border-bottom:1px solid var(--color-border)}.install-grid{display:grid;grid-template-columns:.9fr 1.1fr;gap:6rem;align-items:center}.facts{display:flex;flex-wrap:wrap;gap:.5rem;margin-top:2rem}.facts span{border:1px solid var(--color-border);padding:.5rem .65rem;font:500 .52rem var(--font-mono);letter-spacing:.1em;color:var(--color-text-dim)}.terminal{border:1px solid var(--color-border-warm);background:#08090c;box-shadow:0 25px 70px #0007}.terminal-head{padding:.8rem 1rem;border-bottom:1px solid var(--color-border);display:flex;justify-content:space-between;font:500 .53rem var(--font-mono);letter-spacing:.1em;color:var(--color-text-dim)}.terminal-head i{font-style:normal}.command{display:grid;grid-template-columns:auto 1fr auto;gap:.7rem;align-items:center;padding:1.6rem 1.2rem;border-bottom:1px solid var(--color-border);color:var(--color-warm)}.command code{font:500 clamp(.6rem,1.2vw,.72rem) var(--font-mono);white-space:nowrap;overflow-x:auto}.command button{font:600 .5rem var(--font-mono);letter-spacing:.08em;color:var(--color-text-dim);padding:.45rem;border:1px solid var(--color-border)}.copy-status{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}.output{padding:1.2rem}.output p{font:.59rem var(--font-mono);color:var(--color-text-dim);margin:.65rem}.output b{color:var(--color-warm);margin-right:1rem}.release{border-top:1px solid var(--color-border);padding:1.2rem;display:grid;grid-template-columns:1fr auto;align-items:center}.release span,.release strong{display:block}.release span{font:500 .5rem var(--font-mono);letter-spacing:.1em;color:var(--color-warm-dim);margin-bottom:.4rem}.release strong{font-size:.74rem}.release i{grid-area:1/2/3/3;color:var(--color-warm);font-style:normal}@media(max-width:800px){.install-grid{grid-template-columns:1fr;gap:3rem}.command code{font-size:.55rem}.command button{font-size:.625rem}}@media(max-width:460px){.command{grid-template-columns:auto 1fr}.command button{grid-column:2;justify-self:start}.terminal-head i{display:none}}
</style>
