<script lang="ts">
	import Reveal from './Reveal.svelte';
	const steps = [
		{ num: '01', tag: 'HOME', title: 'Give Nolune a home', text: 'Install the native server on an always-on Mac or Linux machine. It holds one identity, memory, and set of routines.' },
		{ num: '02', tag: 'CONNECT', title: 'Connect your computers', text: 'Add the desktop app wherever you want Nolune to see and act. You choose each machine and its operating-system permissions.' },
		{ num: '03', tag: 'WORK', title: 'Ask from one place', text: 'Select a computer, describe the outcome, and follow Nolune’s observable → acting → done timeline.' }
	];
</script>
<section id="how" class="how"><div class="section-shell"><Reveal><p class="eyebrow">How it works</p><h2 class="section-title">Set up the brain.<br />Connect the hands.</h2></Reveal><div class="steps">{#each steps as step, i}<Reveal delay={i * 100}><article><div class="num">{step.num}</div><span>{step.tag}</span><h3>{step.title}</h3><p>{step.text}</p>{#if i < steps.length - 1}<i class="connector"></i>{/if}</article></Reveal>{/each}</div><Reveal delay={200}><p class="ownership">Self-hosted and BYOK. No required Nolune cloud account. Model-provider pricing and limits still apply.</p></Reveal></div></section>
<style>
	.how{border-bottom:1px solid var(--color-border)}.steps{display:grid;grid-template-columns:repeat(3,1fr);margin-top:4rem;border-top:1px solid var(--color-border-warm)}article{position:relative;padding:2rem 2.5rem 1rem 0;min-height:260px}.num{font:400 4rem var(--font-display);font-style:italic;color:oklch(.78 .12 75/.12);line-height:1;margin-bottom:2.5rem}article>span{font:600 .55rem var(--font-mono);letter-spacing:.12em;color:var(--color-warm-dim)}h3{font-size:1.2rem;margin:.8rem 0}article p{font-size:.85rem;line-height:1.7;color:var(--color-text-dim);max-width:300px}.connector{position:absolute;right:1.5rem;top:3.7rem;width:45%;height:1px;background:linear-gradient(90deg,var(--color-border-warm),transparent)}.ownership{border-left:1px solid var(--color-warm);margin:3rem 0 0;padding:.8rem 1rem;font:500 .65rem var(--font-mono);letter-spacing:.04em;color:var(--color-text-ghost)}@media(max-width:700px){.steps{grid-template-columns:1fr}.steps article{min-height:auto;padding:2rem 0;border-bottom:1px solid var(--color-border)}.num{margin-bottom:1.5rem}.connector{display:none}}
</style>
