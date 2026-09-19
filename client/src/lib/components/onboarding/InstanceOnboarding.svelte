<script lang="ts">
	import MoonBirth from "./MoonBirth.svelte";
	import { saveOnboardingProvider } from "./provider.js";
	import ArrowRight from "@lucide/svelte/icons/arrow-right";
	import {
		sendMessage,
		fetchSoulTemplates,
		applySoulTemplate,
		fetchSoul,
		setCompanionName,
		fetchConfigStatus,
		updateLlmConfig,
		updateProvider,
	} from "$lib/api/client.js";
	import type { SoulTemplate } from "$lib/api/types.js";
	import { getCompanion } from "$lib/stores/companion.svelte.js";
	import { introGreeting, onboardingHandshake } from "$lib/companion/context.js";
	import { getSceneStore } from "$lib/stores/scene.svelte.js";
	import { getSkinStore, SKINS } from "$lib/stores/skin.svelte.js";
	import { getToasts } from "$lib/stores/toast.svelte.js";
	import { play, playImmediate, preload } from "$lib/sounds.js";
	import { hapticReveal } from "$lib/haptics.js";

	const toast = getToasts();
	const scene = getSceneStore();
	const skinStore = getSkinStore();

	let { slug, oncomplete }: { slug: string; oncomplete: () => void } = $props();

	// Optional: set by an earlier visit; never falls back to the companion slug.
	function readPreferredName(): string | null {
		try {
			return typeof localStorage !== "undefined" ? localStorage.getItem("nolune:preferredName") : null;
		} catch {
			return null;
		}
	}

	const companion = getCompanion();

	type Stage =
		| "reveal"
		| "intro"
		| "waiting-key"
		| "testing"
		| "picking-language"
		| "naming-companion"
		| "picking-skin"
		| "being-born"
		| "picking-soul"
		| "picking-provider"
		| "waiting-first"
		| "sending"
		| "departing";

	let stage = $state<Stage>("reveal");
	let revealed = $state(false);
	let firstMessage = $state("");
	let companionNameInput = $state("");
	let selectedProvider = $state<"anthropic" | "openai">("anthropic");
	const providerLabel = $derived(selectedProvider === "openai" ? "OpenAI" : "Anthropic");
	const providerKeyUrl = $derived(selectedProvider === "openai"
		? "https://platform.openai.com/api-keys"
		: "https://console.anthropic.com/settings/keys");
	let apiKeyInput = $state("");
	let apiKeyError = $state("");
	let messageInput: HTMLTextAreaElement | undefined = $state();
	let nameInputEl: HTMLInputElement | undefined = $state();
	let apiKeyInputEl: HTMLInputElement | undefined = $state();
	let chosenLanguage = $state(
		typeof localStorage !== "undefined" ? (localStorage.getItem("nolune:language") ?? "english") : "english",
	);
	let lines = $state<{ text: string; revealed: string; done: boolean }[]>([]);
	let soulTemplates = $state<SoulTemplate[]>([]);

	const LANGUAGES = [
		{ id: "english", label: "English" },
		{ id: "russian", label: "Русский" },
		{ id: "spanish", label: "Español" },
		{ id: "french", label: "Français" },
		{ id: "german", label: "Deutsch" },
		{ id: "japanese", label: "日本語" },
		{ id: "chinese", label: "中文" },
		{ id: "korean", label: "한국어" },
		{ id: "portuguese", label: "Português" },
		{ id: "italian", label: "Italiano" },
		{ id: "turkish", label: "Türkçe" },
		{ id: "arabic", label: "العربية" },
	];

	function typewrite(text: string, speed = 38): Promise<void> {
		return new Promise((resolve) => {
			const entry = { text, revealed: "", done: false };
			lines = [...lines, entry];
			const idx = lines.length - 1;
			let i = 0;
			function tick() {
				if (i >= text.length) { lines[idx].done = true; resolve(); return; }
				const char = text[i];
				lines[idx].revealed += char;
				i++;
				if (i % 3 === 0) playImmediate("typewriter", { pitchRange: [0.88, 1.15] });
				let delay = speed;
				if (char === "." || char === "?" || char === "!") delay = speed * 8;
				else if (char === ",") delay = speed * 3;
				else if (char === "\u2014" || char === "\u2013") delay = speed * 4;
				setTimeout(tick, delay);
			}
			setTimeout(tick, speed);
		});
	}

	function pause(ms: number): Promise<void> {
		return new Promise((r) => setTimeout(r, ms));
	}

	async function runSequence() {
		skinStore.setSlug(slug);

		preload("intro_reveal", "typewriter");
		hapticReveal();
		revealed = true;
		stage = "intro";
		await pause(400);
		await typewrite(introGreeting(readPreferredName()));
		await pause(400);
		await typewrite("a new space, just for us.");
		await pause(600);

		await typewrite("what language should we speak?");
		stage = "picking-language";
	}

	async function pickLanguage(langId: string) {
		chosenLanguage = langId;
		localStorage.setItem("nolune:language", langId);
		stage = "intro";
		await pause(300);
		const lang = LANGUAGES.find((l) => l.id === langId);
		await typewrite(`${lang?.label ?? langId}.`);
		await pause(400);
		await typewrite("what should i call myself?");
		stage = "naming-companion";
		await pause(100);
		nameInputEl?.focus();
	}

	async function submitCompanionName() {
		const name = companionNameInput.trim();
		if (!name) return;
		stage = "intro";
		await pause(200);
		await typewrite(`${name}. i like that.`);
		try { await setCompanionName(slug, name); } catch {}
		await pause(400);

		await typewrite("how should i look?");
		stage = "picking-skin";
	}

	async function pickSkin(skinId: string) {
		if (stage !== "picking-skin") return;
		skinStore.setSkin(skinId);
		stage = "intro";
		await pause(200);
		const skin = SKINS.find((s) => s.id === skinId);
		await typewrite(`${skin?.label ?? skinId}. let me show you.`);
		await pause(400);

		play("intro_reveal");
		stage = "being-born";
	}

	async function finishBirth() {
		if (stage !== "being-born") return;
		stage = "intro";
		scene.enterOnboarding(slug);

		let hasSoul = false;
		try {
			const soul = await fetchSoul(slug);
			hasSoul = soul.exists && soul.content.trim().length > 0;
		} catch {}

		if (hasSoul) {
			await checkKeyThenAsk();
		} else {
			try { soulTemplates = await fetchSoulTemplates(); } catch { soulTemplates = []; }
			if (soulTemplates.length > 0) {
				await typewrite("who should i be for you?");
				stage = "picking-soul";
			} else {
				await askFirstMessage();
			}
		}
	}

	function handleNameKeydown(e: KeyboardEvent) { if (e.key === "Enter") { e.preventDefault(); submitCompanionName(); } }

	async function pickSoul(template: SoulTemplate) {
		stage = "intro";
		await pause(200);
		if (template.id !== "custom") {
			try { await applySoulTemplate(slug, template.id); } catch {}
			await typewrite(`${template.name}. i can be that.`);
		} else {
			await typewrite("a blank canvas. you can shape me later.");
		}
		await pause(400);
		await checkKeyThenAsk();
	}

	async function checkKeyThenAsk() {
		// Check if LLM is already configured (self-hosted users may have it in config.toml)
		try {
			const status = await fetchConfigStatus();
			if (status.llm_configured) {
				await askFirstMessage();
				return;
			}
		} catch {}

		// Ask how they want to connect
		await typewrite("one more thing — how should i think?");
		stage = "picking-provider";
	}

	async function pickProvider(provider: "anthropic" | "openai") {
		selectedProvider = provider;
		apiKeyInput = "";
		apiKeyError = "";
		stage = "intro";
		await typewrite(`got it. i'll need an ${providerLabel} API key.`);
		stage = "waiting-key";
		await pause(100);
		apiKeyInputEl?.focus();
	}

	async function submitApiKey() {
		const key = apiKeyInput.trim();
		if (!key || stage !== "waiting-key") return;
		apiKeyError = "";
		stage = "testing";

		try {
			await saveOnboardingProvider(selectedProvider, key, { updateLlmConfig, updateProvider });
			apiKeyInput = "";
			stage = "intro";
			await pause(200);
			await typewrite("connected.");
			await pause(400);
			await askFirstMessage();
		} catch (e) {
			apiKeyError = e instanceof Error ? e.message : "invalid key";
			stage = "waiting-key";
		}
	}

	function handleKeyKeydown(e: KeyboardEvent) {
		if (e.key === "Enter") { e.preventDefault(); submitApiKey(); }
	}

	async function askFirstMessage() {
		await typewrite("tell me something.");
		stage = "waiting-first";
		await pause(100);
		messageInput?.focus();
	}

	async function submitFirst() {
		const content = firstMessage.trim();
		if (!content) return;
		stage = "sending";
		const langLabel = LANGUAGES.find((l) => l.id === chosenLanguage)?.label ?? chosenLanguage;
		const combined = `${onboardingHandshake(readPreferredName(), langLabel)}\n\n${content}`;
		try {
			await sendMessage(slug, combined);
			await companion.refresh();
		} catch {
			toast.error("setup failed — try sending a message after");
		}
		// Start cinematic intro, then complete
		scene.finishOnboarding();
		stage = "departing";
		await pause(600);
		oncomplete();
	}

	function handleMessageKeydown(e: KeyboardEvent) {
		if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); submitFirst(); }
	}

	$effect(() => { runSequence(); });
</script>

{#if stage === "being-born"}
	<MoonBirth name={companionNameInput.trim() || "Nolune"} oncomplete={finishBirth} />
{/if}
<div class="ob" inert={stage === "being-born"} class:ob-birthing={stage === "being-born"} class:ob-depart={stage === "departing"} class:ob-hidden={stage === "reveal" && !revealed}>
	<div class="ob-content">
		<!-- Typewriter lines -->
		<div class="ob-lines" class:ob-lines-hidden={stage === "reveal"}>
			{#each lines as line, i}
				<div class="ob-line" style="animation-delay: {i * 40}ms">
					{#if i === 0 && line.done}
						<p class="ob-title">{line.revealed}</p>
					{:else if i === 0}
						<p class="ob-title">{line.revealed}<span class="ob-cursor"></span></p>
					{:else if !line.done}
						<p class="ob-text">{line.revealed}<span class="ob-cursor"></span></p>
					{:else}
						<p class="ob-text">{line.revealed}</p>
					{/if}
				</div>
			{/each}
		</div>

		<!-- Interactive sections -->
		<div class="ob-input-area">
			{#if stage === "sending"}
				<div class="ob-enter ob-center">
					<div class="ob-spinner"></div>
					<span class="ob-spinner-label">thinking</span>
				</div>
			{/if}

			{#if stage === "picking-language"}
				<div class="ob-enter">
					<div class="ob-pills ob-pills-lang">
						{#each LANGUAGES as lang}
							<button onclick={() => pickLanguage(lang.id)} class="ob-pill" class:ob-pill-active={chosenLanguage === lang.id}>{lang.label}</button>
						{/each}
					</div>
				</div>
			{/if}

			{#if stage === "naming-companion"}
				<div class="ob-enter">
					<div class="ob-field">
						<label class="ob-label" for="companion-name">Companion name</label>
                        <input id="companion-name" bind:this={nameInputEl} bind:value={companionNameInput} onkeydown={handleNameKeydown} placeholder="A name for your companion" class="ob-input" />
						{#if companionNameInput.trim()}
							<button onclick={submitCompanionName} class="ob-go" aria-label="Confirm"><ArrowRight size={18} aria-hidden="true" /></button>
						{/if}
					</div>
				</div>
			{/if}

			{#if stage === "picking-skin"}
				<div class="ob-enter">
					<div class="ob-pills ob-pills-skin">
						{#each SKINS as skin (skin.id)}
							<button onclick={() => pickSkin(skin.id)} class="ob-pill ob-pill-col ob-pill-skin" class:ob-pill-active={skinStore.skinId === skin.id}>
								<img src={skin.thumbnail} alt={skin.label} class="ob-skin-thumb" />
								<span class="ob-pill-label">{skin.label}</span>
							</button>
						{/each}
					</div>
				</div>
			{/if}

			{#if stage === "picking-soul"}
				<div class="ob-enter">
					<div class="ob-pills ob-pills-soul">
						{#each soulTemplates as template (template.id)}
							<button onclick={() => pickSoul(template)} class="ob-pill ob-pill-col ob-pill-soul">
								<span class="ob-pill-label">{template.name}</span>
								<span class="ob-pill-note">{template.description}</span>
							</button>
						{/each}
					</div>
				</div>
			{/if}

			{#if stage === "picking-provider"}
				<div class="ob-enter">
					<div class="ob-pills ob-pills-soul">
						<button onclick={() => pickProvider("anthropic")} class="ob-pill ob-pill-col ob-pill-soul">
							<span class="ob-pill-label">Anthropic</span>
							<span class="ob-pill-note">pay-per-use</span>
						</button>
						<button onclick={() => pickProvider("openai")} class="ob-pill ob-pill-col ob-pill-soul">
							<span class="ob-pill-label">OpenAI</span>
							<span class="ob-pill-note">pay-per-use</span>
						</button>
					</div>
				</div>
			{/if}

			{#if stage === "waiting-key"}
				<div class="ob-enter">
					<div class="ob-field">
						<label class="ob-label" for="provider-key">{providerLabel} API key</label>
                        <!-- svelte-ignore a11y_autofocus -->
						<input
							id="provider-key"
                            bind:this={apiKeyInputEl}
							bind:value={apiKeyInput}
							onkeydown={handleKeyKeydown}
							placeholder={selectedProvider === "openai" ? "sk-..." : "sk-ant-..."}
							class="ob-input ob-input-mono"
							type="password"
							autofocus
						/>
						{#if apiKeyInput.trim()}
							<button onclick={submitApiKey} class="ob-go" aria-label="Submit"><ArrowRight size={18} aria-hidden="true" /></button>
						{/if}
					</div>
					{#if apiKeyError}
						<p class="ob-error" role="alert">{apiKeyError}</p>
					{/if}
					<a href={providerKeyUrl} target="_blank" rel="noopener" class="ob-hint">
						Get your {providerLabel} API key
					</a>
				</div>
			{/if}

			{#if stage === "testing"}
				<div class="ob-enter ob-center">
					<div class="ob-spinner"></div>
					<span class="ob-spinner-label">connecting</span>
				</div>
			{/if}

			{#if stage === "waiting-first"}
				<div class="ob-enter">
					<div class="ob-field">
						<label class="ob-label" for="first-message">Your first message</label>
                        <textarea id="first-message" bind:this={messageInput} bind:value={firstMessage} onkeydown={handleMessageKeydown} placeholder="What’s on your mind?" rows={3} class="ob-input ob-textarea"></textarea>
						{#if firstMessage.trim()}
							<button onclick={submitFirst} class="ob-go ob-go-textarea" aria-label="Send"><ArrowRight size={18} aria-hidden="true" /></button>
						{/if}
					</div>
				</div>
			{/if}
		</div>
	</div>
</div>

<style>
	.ob-birthing{visibility:hidden;}
	.ob {
		position: relative;
		display: flex;
		height: 100%;
		align-items: center;
		justify-content: center;
		overflow: hidden;
		pointer-events: none;
		z-index: 10;
		transition: opacity 0.6s ease;
	}
	.ob-hidden { opacity: 0; }

	.ob-content {
		position: relative;
		width: 100%;
		max-width: 420px;
		padding: 0 1.5rem;
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1.5rem;
		pointer-events: auto;
	}

	/* ── Lines ── */
	.ob-lines { display: flex; flex-direction: column; gap: 0.625rem; width: 100%; }
	.ob-lines-hidden { visibility: hidden; }

	.ob-line { animation: ob-fade-in 0.4s cubic-bezier(0.16, 1, 0.3, 1) both; }
	@keyframes ob-fade-in {
		from { opacity: 0; transform: translateY(6px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.ob-title {
		font-family: var(--font-display);
		font-size: 1.35rem;
		font-weight: 400;
		font-style: normal;
		letter-spacing: -0.01em;
		color: var(--foreground);
		text-align: center;
	}

	.ob-text {
		font-family: var(--font-body);
		font-size: 0.85rem;
		line-height: 1.6;
		color: var(--text-secondary);
		text-align: center;
	}

	.ob-cursor {
		display: inline-block;
		width: 1.5px;
		height: 1.05em;
		margin-left: 1px;
		vertical-align: text-bottom;
		background: var(--accent);
		animation: blink 0.8s steps(2) infinite;
	}
	@keyframes blink { 0% { opacity: 1; } 100% { opacity: 0; } }

	/* ── Input area ── */
	.ob-input-area { width: 100%; }
	.ob-enter {
		animation: ob-slide-in 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
		animation-delay: 80ms;
	}
	@keyframes ob-slide-in {
		from { opacity: 0; transform: translateY(10px); }
		to { opacity: 1; transform: translateY(0); }
	}

	.ob-center { display: flex; align-items: center; justify-content: center; gap: 0.625rem; }

	/* Companion preferences */
	.ob-pills { display: flex; gap: 0.5rem; flex-wrap: wrap; }
	.ob-pills-lang { display: grid; grid-template-columns: repeat(4, 1fr); gap: 0.375rem; }
	.ob-pills-soul { display: grid; grid-template-columns: repeat(2, 1fr); }

	.ob-pill {
		min-height: 44px;
		display: flex;
		align-items: center;
		justify-content: center;
		flex: 1;
		padding: 0.55rem 0.75rem;
		border-radius: 8px;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--border);
		border-top-color: var(--border);
		font-family: var(--font-body);
		font-size: 0.875rem;
		font-style: normal;
		color: var(--text-secondary);
		cursor: pointer;
		transition: all 0.3s ease;
		box-shadow: none;
	}
	.ob-pill:hover {
		border-color: var(--text-secondary);
		background: var(--accent);
		color: var(--text-secondary);
		box-shadow: none;
	}
	.ob-pill-active {
		border-color: var(--text-secondary);
		background: var(--accent);
		color: var(--text-secondary);
	}

	.ob-pill-col {
		flex-direction: column;
		gap: 0.2rem;
		padding: 0.65rem 0.75rem;
	}
	.ob-pill-soul {
		border-radius: 1rem;
		align-items: flex-start;
		padding: 0.75rem 1rem;
		gap: 0.25rem;
	}
	.ob-pill-label {
		font-family: var(--font-body);
		font-size: 0.875rem;
		font-style: normal;
		color: var(--text-secondary);
	}
	.ob-pill-note {
		font-family: var(--font-body);
		font-size: 0.75rem;
		font-style: normal;
		color: var(--text-secondary);
	}

	/* Inputs */
	.ob-field { position: relative; }

	.ob-input {
		width: 100%;
		padding: 0.75rem 3rem 0.75rem 1.25rem;
		border-radius: 8px;
		background: var(--card);
		backdrop-filter: none;
		-webkit-backdrop-filter: none;
		border: 1px solid var(--border);
		border-top-color: var(--border);
		font-family: var(--font-body);
		font-size: 1rem;
		font-style: normal;
		color: var(--text-secondary);
		outline: none;
		text-align: center;
		transition: all 0.3s ease;
		box-shadow: none;
	}
	.ob-input::placeholder { color: var(--text-secondary); font-style: normal; }
	.ob-input:focus {
		border-color: var(--text-secondary);
		box-shadow: none;
	}
	.ob-input-mono { font-family: var(--font-mono); font-size: 0.8rem; font-style: normal; text-align: left; }

	.ob-textarea {
		border-radius: 1rem;
		resize: none;
		text-align: left;
		font-family: var(--font-body);
		font-size: 0.85rem;
		font-style: normal;
		line-height: 1.6;
		padding: 0.875rem 1.25rem;
	}

	.ob-go {
		position: absolute;
		right: 0.5rem;
		top: 50%;
		transform: translateY(-50%);
		display: flex;
		align-items: center;
		justify-content: center;
		width: 44px;
		height: 44px;
		border-radius: 50%;
		font-size: 0.9rem;
		color: var(--text-secondary);
		transition: all 0.3s ease;
		cursor: pointer;
	}
	.ob-go:hover { color: var(--text-secondary); background: var(--accent); }
	.ob-go-textarea { top: auto; bottom: 0.5rem; transform: none; }

	.ob-error { margin-top: 0.5rem; font-size: 0.72rem; color: var(--destructive); font-style: normal; text-align: center; }

	.ob-hint {
		display: block;
		margin-top: 0.5rem;
		font-size: 0.75rem;
		color: var(--primary);
		text-decoration: none;
		text-align: center;
		transition: color 0.2s ease;
	}
	.ob-hint:hover { color: var(--primary); }

	/* ── Spinner ── */
	.ob-spinner {
		width: 12px; height: 12px;
		border: 1.5px solid var(--border);
		border-top-color: var(--text-secondary);
		border-radius: 50%;
		animation: spin 0.7s linear infinite;
	}
	@keyframes spin { to { transform: rotate(360deg); } }
	.ob-spinner-label {
		font-family: var(--font-display);
		font-size: 0.72rem;
		font-style: normal;
		color: var(--text-secondary);
	}

	/* ── Skin picker ── */
	.ob-pills-skin { display: grid; grid-template-columns: repeat(2, 1fr); gap: 0.5rem; }
	.ob-pill-skin {
		border-radius: 1rem;
		padding: 0.75rem;
		align-items: center;
		gap: 0.5rem;
	}
	.ob-skin-thumb {
		width: 64px;
		height: 64px;
		border-radius: 0.5rem;
		object-fit: cover;
	}

	/* ── Depart ── */
	.ob-depart { animation: depart 0.5s cubic-bezier(0.55, 0, 1, 0.45) forwards; }
	@keyframes depart { to { opacity: 0; transform: scale(0.98); } }

    .ob { overflow-y: auto; padding: 32px 0; }
    .ob-content { max-width: 520px; margin-block: auto; }
    .ob-title { font-size: 2rem; line-height: 1.2; }
    .ob-text { font-size: 1rem; }
    .ob-pill { background: var(--card); border-color: var(--border); box-shadow: none; backdrop-filter: none; }
    .ob-pill:hover { background: var(--popover); border-color: var(--primary); box-shadow: none; color: var(--foreground); }
    .ob-pill-active { background: var(--accent); border-color: var(--primary); color: var(--foreground); }
    .ob-pill-label { color: var(--foreground); }
    .ob-pill-note { color: var(--text-secondary); line-height: 1.5; }
    .ob-label { display: block; margin-bottom: 8px; color: var(--text-secondary); font-size: 14px; }
    .ob-input { min-height: 56px; padding: 12px 60px 12px 16px; background: var(--card); border-color: var(--input); color: var(--foreground); text-align: left; box-shadow: none; backdrop-filter: none; }
    .ob-input:focus { border-color: var(--ring); box-shadow: none; }
    .ob-input::placeholder { color: var(--text-muted); opacity: 1; }
    .ob-input-mono, .ob-textarea { font-size: 1rem; }
    .ob-textarea { padding-bottom: 60px; }
    .ob-go { top: auto; bottom: 6px; right: 6px; transform: none; border-radius: 8px; background: var(--primary); color: var(--primary-foreground); }
    .ob-go:hover { background: var(--primary); color: var(--primary-foreground); filter: brightness(1.06); }
    .ob-error { color: var(--destructive); font-size: 14px; }
    .ob-hint, .ob-hint:hover { color: var(--primary); font-size: 13px; min-height: 44px; padding-top: 12px; text-decoration: underline; text-underline-offset: 3px; }
    .ob-spinner-label { font-family: var(--font-body); font-size: 14px; }
    .ob-spinner { width: 16px; height: 16px; border-color: var(--border); border-top-color: var(--primary); }
    .ob-cursor { background: var(--primary); }
    @media (max-width: 480px) { .ob-pills-lang { grid-template-columns: repeat(2, minmax(0, 1fr)); } .ob-pills-soul { grid-template-columns: 1fr; } }
</style>
