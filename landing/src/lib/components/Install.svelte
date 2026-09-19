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
.install{background:#282432;border-bottom:1px solid var(--color-border)}.install-grid{display:grid;grid-template-columns:minmax(0,.9fr) minmax(0,1.1fr);gap:3rem;align-items:center}.facts{display:flex;flex-wrap:wrap;gap:.5rem;margin-top:2rem}.facts span{border:1px solid var(--color-border);border-radius:4px;padding:.5rem .65rem;font-size:.65rem;letter-spacing:.06em;color:var(--color-text-dim)}.terminal{border:1px solid var(--color-border-warm);background:#201d29;border-radius:10px;overflow:hidden}.terminal-head{padding:1rem 1.2rem;border-bottom:1px solid var(--color-border);display:flex;justify-content:space-between;gap:1rem;font-size:.65rem;letter-spacing:.05em;color:var(--color-text-dim)}.terminal-head i{font-style:normal}.command{display:grid;grid-template-columns:auto minmax(0,1fr);gap:.75rem;align-items:center;padding:1.4rem 1.2rem;border-bottom:1px solid var(--color-border);color:var(--color-warm)}.command code{font:.75rem var(--font-mono);white-space:nowrap;overflow-x:auto;padding:.35rem 0;scrollbar-width:thin}.command button{grid-column:2;justify-self:start;font-size:.68rem;color:var(--color-text-dim);padding:.5rem .8rem;border:1px solid var(--color-border);border-radius:4px}.command button:hover{background:#ffffff0a}.copy-status{position:absolute;width:1px;height:1px;padding:0;margin:-1px;overflow:hidden;clip:rect(0,0,0,0);white-space:nowrap;border:0}.output{padding:1rem 1.2rem}.output p{font-size:.75rem;color:var(--color-text-dim);margin:.8rem 0;line-height:1.5}.output b{font-size:.65rem;color:var(--color-warm);margin-right:.8rem}.release{border-top:1px solid var(--color-border);padding:1.2rem;display:grid;grid-template-columns:1fr auto;align-items:center;gap:.2rem .5rem}.release span,.release strong{display:block}.release span{font-size:.6rem;letter-spacing:.1em;color:var(--color-warm-dim);margin-bottom:.4rem}.release strong{font-size:.8rem;font-weight:500;line-height:1.6}.release i{grid-area:1/2/3/3;color:var(--color-warm);font-style:normal}@media(max-width:800px){.install-grid{grid-template-columns:1fr;gap:2rem}}@media(max-width:460px){.terminal-head i{display:none}}
</style>
