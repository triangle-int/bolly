<script lang="ts">
	import { setAuthToken, isDesktopRelay } from "$lib/api/client.js";

	let { onauth }: { onauth: () => void } = $props();

	let token = $state("");
	let error = $state(false);

	function submit() {
		if (!token.trim()) return;
		setAuthToken(token.trim());
		error = false;
		onauth();
	}

	function handleKeydown(e: KeyboardEvent) {
		if (e.key === "Enter") submit();
	}
</script>

<div class="auth-gate">
	<div class="auth-card">
		<img class="auth-icon" src="/skins/moon/character.svg" alt="Nolune" />
        <h1>Welcome back</h1>
		{#if isDesktopRelay()}
			<p class="auth-label">Return to the desktop dashboard to update your token and reconnect.</p>
		{:else}
		<label for="auth-token" class="auth-label">Connect with your access token</label>
		<input
			id="auth-token"
            type="password"
            autocomplete="current-password"
			bind:value={token}
			onkeydown={handleKeydown}
			placeholder="Access token"
			class="auth-input"
		/>
		{#if error}
			<p class="auth-error" role="alert">Invalid token</p>
		{/if}
		<button onclick={submit} class="auth-button" disabled={!token.trim()}>Connect</button>
		{/if}
	</div>
</div>

<style>
 .auth-gate { display: flex; align-items: center; justify-content: center; min-height: 100%; padding: 24px; overflow: auto; }
 .auth-card { display: flex; flex-direction: column; align-items: stretch; gap: 16px; width: 100%; max-width: 420px; padding: 32px; border-radius: 16px; background: var(--card); border: 1px solid var(--border); }
 .auth-icon { width: 64px; height: 64px; align-self: center; }
 h1 { font-family: var(--font-display); font-size: 32px; font-weight: 400; text-align: center; color: var(--foreground); }
 .auth-label { font-size: 14px; line-height: 1.6; color: var(--text-secondary); }
 .auth-input { width: 100%; min-height: 48px; padding: 12px; border-radius: 8px; background: var(--background); border: 1px solid var(--input); color: var(--foreground); font-size: 16px; }
 .auth-input::placeholder { color: var(--text-muted); opacity: 1; }
 .auth-input:focus { border-color: var(--ring); }
 .auth-error { color: var(--destructive); font-size: 14px; }
 .auth-button { min-height: 44px; padding: 12px 16px; border-radius: 8px; background: var(--primary); color: var(--primary-foreground); font-size: 14px; font-weight: 500; cursor: pointer; }
 .auth-button:hover:not(:disabled) { filter: brightness(1.06); }
 .auth-button:disabled { opacity: 0.5; cursor: not-allowed; }
 @media (max-width: 480px) { .auth-card { padding: 24px; } }
</style>
