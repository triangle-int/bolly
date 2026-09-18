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
		<div class="auth-icon">~</div>
		{#if isDesktopRelay()}
			<p class="auth-label">Return to the desktop dashboard to update your token and reconnect.</p>
		{:else}
		<p class="auth-label">this companion requires a token</p>
		<input
			type="password"
			bind:value={token}
			onkeydown={handleKeydown}
			placeholder="auth token..."
			class="auth-input"
		/>
		{#if error}
			<p class="auth-error">invalid token</p>
		{/if}
		<button onclick={submit} class="auth-button">connect</button>
		{/if}
	</div>
</div>

<style>
	.auth-gate {
		display: flex;
		align-items: center;
		justify-content: center;
		height: 100%;
	}

	.auth-card {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 1rem;
		padding: 2.5rem 3rem;
		border-radius: 1rem;
		background: oklch(0.09 0.018 278 / 60%);
		border: 1px solid oklch(var(--ink) / 4%);
		backdrop-filter: blur(20px);
		animation: auth-enter 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
	}

	@keyframes auth-enter {
		from { opacity: 0; transform: translateY(12px) scale(0.97); }
		to { opacity: 1; transform: translateY(0) scale(1); }
	}

	.auth-icon {
		font-family: var(--font-mono);
		font-size: 1.5rem;
		color: oklch(0.78 0.12 75 / 35%);
	}

	.auth-label {
		font-family: var(--font-body);
		font-size: 0.8rem;
		color: oklch(0.78 0.12 75 / 45%);
	}

	.auth-input {
		width: 16rem;
		padding: 0.5rem 0.75rem;
		border-radius: 0.5rem;
		background: oklch(0.065 0.015 280);
		border: 1px solid oklch(var(--ink) / 6%);
		color: var(--foreground);
		font-family: var(--font-mono);
		font-size: 0.8rem;
		outline: none;
		transition: border-color 0.2s ease;
	}

	.auth-input:focus {
		border-color: oklch(0.78 0.12 75 / 35%);
	}

	.auth-error {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		color: oklch(0.65 0.15 20 / 70%);
	}

	.auth-button {
		font-family: var(--font-mono);
		font-size: 0.7rem;
		letter-spacing: 0.05em;
		color: oklch(0.78 0.12 75 / 60%);
		background: oklch(0.78 0.12 75 / 6%);
		border: 1px solid oklch(0.78 0.12 75 / 12%);
		padding: 0.4rem 1.25rem;
		border-radius: 2rem;
		cursor: pointer;
		transition: all 0.2s ease;
	}

	.auth-button:hover {
		background: oklch(0.78 0.12 75 / 12%);
		color: oklch(0.78 0.12 75 / 80%);
	}
</style>
