<script lang="ts">
	import { pairBrowser, PairingError, isDesktopRelay } from "$lib/api/client.js";
	import { formatPairingCodeInput, isCompletePairingCode, pairingErrorText } from "$lib/api/pairing.js";

	let { onauth }: { onauth: () => void } = $props();

	let code = $state("");
	let error = $state("");
	let busy = $state(false);

	const ready = $derived(isCompletePairingCode(code) && !busy);

	function onInput(e: Event) {
		code = formatPairingCodeInput((e.currentTarget as HTMLInputElement).value);
		error = "";
	}

	async function submit() {
		if (!ready) return;
		busy = true;
		error = "";
		try {
			await pairBrowser(code);
			code = "";
			onauth();
		} catch (e) {
			if (e instanceof PairingError) {
				if (e.reason === "auth_disabled") {
					onauth();
					return;
				}
				error = pairingErrorText(e.reason);
			} else {
				error = pairingErrorText("unknown");
			}
		} finally {
			busy = false;
		}
	}

	function onSubmit(e: SubmitEvent) {
		e.preventDefault();
		submit();
	}
</script>

<div class="auth-gate">
	<div class="auth-card">
		<img class="auth-icon" src="/skins/moon/character.svg" alt="Nolune" />
		<h1>Pair this browser</h1>
		{#if isDesktopRelay()}
			<p class="auth-label">Return to the desktop dashboard to update your token and reconnect.</p>
		{:else}
			<p class="auth-label">
				Nolune only lets in browsers you have paired. Get a one-time code from somewhere that already has access:
			</p>
			<ul class="auth-steps">
				<li>On the computer where Nolune is installed, run <code>nolune pair</code>.</li>
				<li>Or, on a device that is already connected, open <strong>Settings → Server → Pair another browser</strong>.</li>
			</ul>
			<form class="auth-form" onsubmit={onSubmit}>
				<label for="pairing-code" class="auth-label">Enter the code</label>
				<input
					id="pairing-code"
					type="text"
					inputmode="numeric"
					autocomplete="one-time-code"
					spellcheck="false"
					value={code}
					oninput={onInput}
					placeholder="1234-5678"
					class="auth-input"
					aria-invalid={error ? "true" : undefined}
					aria-describedby={error ? "pairing-error" : undefined}
				/>
				{#if error}
					<p id="pairing-error" class="auth-error" role="alert">{error}</p>
				{/if}
				<button type="submit" class="auth-button" disabled={!ready}>
					{busy ? "Connecting…" : "Connect"}
				</button>
			</form>
			<p class="auth-fineprint">Codes expire after 5 minutes and work once. Paired browsers can be reviewed and revoked from Settings.</p>
		{/if}
	</div>
</div>

<style>
 .auth-gate { display: flex; align-items: center; justify-content: center; min-height: 100%; padding: 24px; overflow: auto; }
 .auth-card { display: flex; flex-direction: column; align-items: stretch; gap: 16px; width: 100%; max-width: 440px; padding: 32px; border-radius: 16px; background: var(--card); border: 1px solid var(--border); }
 .auth-icon { width: 64px; height: 64px; align-self: center; }
 h1 { font-family: var(--font-display); font-size: 32px; font-weight: 400; text-align: center; color: var(--foreground); }
 .auth-label { font-size: 14px; line-height: 1.6; color: var(--text-secondary); }
 .auth-form { display: flex; flex-direction: column; gap: 16px; }
 .auth-steps { display: flex; flex-direction: column; gap: 8px; margin: 0; padding-left: 20px; font-size: 14px; line-height: 1.6; color: var(--text-secondary); }
 .auth-steps strong { color: var(--foreground); font-weight: 500; }
 .auth-steps code { font-family: var(--font-mono, monospace); font-size: 13px; padding: 1px 6px; border-radius: 6px; background: var(--background); border: 1px solid var(--border); color: var(--foreground); }
 .auth-input { width: 100%; min-height: 48px; padding: 12px; border-radius: 8px; background: var(--background); border: 1px solid var(--input); color: var(--foreground); font-size: 20px; font-family: var(--font-mono, monospace); letter-spacing: 0.12em; text-align: center; }
 .auth-input::placeholder { color: var(--text-muted); opacity: 1; letter-spacing: 0.12em; }
 .auth-input:focus { border-color: var(--ring); }
 .auth-error { color: var(--destructive); font-size: 14px; line-height: 1.5; }
 .auth-button { min-height: 44px; padding: 12px 16px; border-radius: 8px; background: var(--primary); color: var(--primary-foreground); font-size: 14px; font-weight: 500; cursor: pointer; }
 .auth-button:hover:not(:disabled) { filter: brightness(1.06); }
 .auth-button:disabled { opacity: 0.5; cursor: not-allowed; }
 .auth-fineprint { font-size: 12px; line-height: 1.5; color: var(--text-muted); text-align: center; }
 @media (max-width: 480px) { .auth-card { padding: 24px; } }
</style>
