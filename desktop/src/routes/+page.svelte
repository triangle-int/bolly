<script lang="ts">
  import { onMount } from "svelte";
  import Moon from "$lib/components/Moon.svelte";
  import { auth, init, saveConnection, testConnection, openConnection, disconnect } from "$lib/auth.svelte";
  import { updater, checkForUpdates, installUpdate, dismissUpdate } from "$lib/updater.svelte";

  let splash = $state(true);
  let splashFading = $state(false);
  let editing = $state(false);
  let shUrl = $state("");
  let shToken = $state("");
  let splashAudio = $state<HTMLAudioElement | null>(null);

  onMount(() => {
    init();
    checkForUpdates();
    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    // The chime strikes as the moon lands and opens its eyes (see the moon-born and eyes-awake keyframes).
    const chime = setTimeout(() => {
      if (!splashAudio) return;
      splashAudio.volume = 0.4;
      splashAudio.play().catch(() => {});
    }, reducedMotion ? 0 : 1300);
    const timer = setTimeout(endSplash, reducedMotion ? 900 : 3000);
    return () => {
      clearTimeout(chime);
      clearTimeout(timer);
    };
  });

  function endSplash() {
    if (splashFading) return;
    splashFading = true;
    setTimeout(() => { splash = false; }, 500);
  }

  function edit() {
    shUrl = auth.connection?.url ?? "";
    shToken = "";
    auth.error = null;
    auth.message = null;
    editing = true;
  }

  function cancelEdit() {
    editing = false;
    shToken = "";
    auth.error = null;
    auth.message = null;
  }

  async function save() {
    if (await saveConnection(shUrl, shToken)) {
      editing = false;
      shToken = "";
    }
  }

  async function forget() {
    shToken = "";
    if (await disconnect()) {
      editing = false;
      shUrl = "";
    }
  }

  const canSubmit = $derived(!auth.loading && !!shUrl.trim() && (!!shToken.trim() || !!auth.connection));
</script>

<!-- Audio lives outside the splash so it is not destroyed on transition; played from onMount, not autoplay -->
<audio bind:this={splashAudio} src="/splash.mp3" preload="auto"></audio>

{#if splash}
  <div class="splash" class:splash-fade={splashFading} aria-label="Nolune is starting">
    <div class="splash-moon"><Moon size="100%" /></div>
    <div class="splash-brand">
      <span class="splash-name">nolune</span>
      <p class="nl-eyebrow">A little presence. Entirely yours.</p>
    </div>
    <button class="splash-skip nl-button-secondary" onclick={endSplash}>Continue</button>
  </div>
{/if}

<div class="app" class:app-enter={!splash}>
  <header class="header">
    <div class="brand">
      <Moon size={28} />
      <span class="brand-name">nolune</span>
    </div>
    {#if auth.connection || auth.error}
      <button class="header-action nl-button-secondary" onclick={forget} disabled={auth.loading}>Disconnect</button>
    {/if}
  </header>

  {#if updater.available}
    <div class="update-banner" role="status">
      {#if updater.downloading}
        <span class="update-text">Updating to v{updater.version}…</span>
        <div class="update-progress-track" aria-hidden="true">
          <div class="update-progress-bar" style:width="{Math.round(updater.progress * 100)}%"></div>
        </div>
      {:else if updater.error}
        <span class="update-text update-error-text">Update failed: {updater.error}</span>
        <div class="update-actions">
          <button class="nl-button" onclick={installUpdate}>Retry</button>
          <button class="nl-button-secondary" onclick={dismissUpdate}>Dismiss</button>
        </div>
      {:else}
        <span class="update-text">v{updater.version} is available</span>
        <div class="update-actions">
          <button class="nl-button" onclick={installUpdate}>Update and restart</button>
          <button class="nl-button-secondary" onclick={dismissUpdate}>Later</button>
        </div>
      {/if}
    </div>
  {/if}

  <main class="content">
    {#if !splash}
      <section class="connect nl-panel" aria-labelledby="connect-title">
        <p class="nl-eyebrow">Desktop companion</p>
        <h2 id="connect-title" class="connect-title">Connect to your companion</h2>

        {#if auth.error}<p class="form-error" role="alert">{auth.error}</p>{/if}
        {#if auth.message}<p class="form-message" role="status">{auth.message}</p>{/if}

        {#if auth.connection && !editing}
          <p class="connect-desc">Your companion is saved on this computer. Open it to keep talking.</p>
          <div class="server">
            <span class="nl-label">Server</span>
            <code class="server-url">{auth.connection.url}</code>
          </div>
          <div class="actions">
            <button class="nl-button" onclick={openConnection} disabled={auth.loading}>Open companion</button>
            <button class="nl-button-secondary" onclick={() => testConnection(auth.connection!.url)} disabled={auth.loading}>Test connection</button>
            <button class="nl-button-secondary" onclick={edit} disabled={auth.loading}>Edit connection</button>
          </div>
        {:else}
          <p class="connect-desc">Connect to your own Nolune server. Your connection is saved on this computer.</p>
          <form class="form" onsubmit={(event) => { event.preventDefault(); save(); }}>
            <div class="field">
              <label class="nl-label" for="server-url">Server URL</label>
              <input id="server-url" class="nl-input" bind:value={shUrl} placeholder="http://localhost:3000" disabled={auth.loading} required autocomplete="url" spellcheck="false" />
            </div>
            <div class="field">
              <label class="nl-label" for="auth-token">Auth token</label>
              <input id="auth-token" class="nl-input" bind:value={shToken} type="password" autocomplete="off" placeholder={auth.connection ? "Leave blank to keep the saved token" : "Token from config.toml"} disabled={auth.loading} required={!auth.connection} />
            </div>
            <div class="actions">
              <button class="nl-button" type="submit" disabled={!canSubmit}>Save connection</button>
              <button class="nl-button-secondary" type="button" onclick={() => testConnection(shUrl, shToken)} disabled={!canSubmit}>Test connection</button>
              {#if editing}
                <button class="nl-button-secondary" type="button" onclick={cancelEdit} disabled={auth.loading}>Cancel</button>
              {/if}
            </div>
          </form>
        {/if}

        {#if auth.loading}
          <p class="pending" role="status"><span class="spinner" aria-hidden="true"></span>Please wait…</p>
        {/if}
      </section>
    {/if}
  </main>
</div>

<style>
  /* ─── Splash ───────────────────────────────────────────────── */
  .splash {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 32px;
    padding: 24px;
    background: var(--background);
    text-align: center;
    transition: opacity 0.5s ease;
  }

  .splash-fade {
    opacity: 0;
    pointer-events: none;
  }

  .splash-moon {
    width: clamp(140px, 30vmin, 240px);
    transform-origin: center;
    animation: moon-born 2.2s cubic-bezier(0.22, 1, 0.36, 1) both;
  }

  .splash-moon :global(.moon-eyes) {
    transform-box: fill-box;
    transform-origin: center;
    animation: eyes-awake 2.6s ease both;
  }

  .splash-brand {
    display: flex;
    flex-direction: column;
    gap: 8px;
    animation: fade-up 0.7s ease 1.5s both;
  }

  .splash-name {
    font: 400 40px/1.1 var(--font-display);
    letter-spacing: -0.02em;
    color: var(--foreground);
  }

  .splash-skip {
    position: absolute;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    -webkit-app-region: no-drag;
  }

  @keyframes moon-born {
    0% { opacity: 0; transform: translateY(24px) scale(0.1) rotate(-30deg); }
    20% { opacity: 1; }
    65% { transform: translateY(-6px) scale(1.04) rotate(4deg); }
    100% { opacity: 1; transform: translateY(0) scale(1) rotate(0); }
  }

  @keyframes eyes-awake {
    0%, 48% { transform: scaleY(0.08); }
    62%, 78% { transform: scaleY(1); }
    83% { transform: scaleY(0.08); }
    89%, 100% { transform: scaleY(1); }
  }

  @keyframes fade-up {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  @media (prefers-reduced-motion: reduce) {
    .splash-moon,
    .splash-moon :global(.moon-eyes),
    .splash-brand {
      animation: none;
    }
  }

  /* ─── App shell ────────────────────────────────────────────── */
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    opacity: 0;
  }

  .app-enter {
    animation: fade-up 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 16px 24px;
    -webkit-app-region: drag;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .brand-name {
    font: 400 20px/1.1 var(--font-display);
    letter-spacing: -0.02em;
    color: var(--foreground);
  }

  .header-action {
    -webkit-app-region: no-drag;
  }

  .content {
    flex: 1;
    display: flex;
    padding: 24px 40px 40px;
    overflow-y: auto;
  }

  /* ─── Update banner ─────────────────────────────────────────── */
  .update-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 12px;
    margin: 0 24px;
    padding: 12px 16px;
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    background: var(--card);
    animation: fade-up 0.3s ease both;
  }

  .update-text {
    font-size: 14px;
    color: var(--text-secondary);
  }

  .update-error-text {
    color: var(--destructive);
  }

  .update-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }

  .update-progress-track {
    flex: 1;
    min-width: 120px;
    height: 4px;
    border-radius: 2px;
    background: var(--accent);
    overflow: hidden;
  }

  .update-progress-bar {
    height: 100%;
    border-radius: 2px;
    background: var(--primary);
    transition: width 0.3s ease;
  }

  /* ─── Connect panel ─────────────────────────────────────────── */
  .connect {
    width: 100%;
    max-width: 440px;
    margin: auto; /* centers when there is room, scrolls from the top when there is not */
    animation: fade-up 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  .connect-title {
    font: 400 28px/1.15 var(--font-display);
    letter-spacing: -0.02em;
    color: var(--foreground);
    margin: 8px 0 12px;
  }

  .connect-desc {
    font-size: 14px;
    line-height: 1.6;
    color: var(--text-secondary);
    margin: 0 0 24px;
  }

  .form-error,
  .form-message {
    font-size: 14px;
    line-height: 1.5;
    margin: 0 0 16px;
  }

  .form-error {
    color: var(--destructive);
  }

  .form-message {
    color: var(--text-secondary);
  }

  .server {
    margin-bottom: 24px;
  }

  .server-url {
    display: block;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-control);
    background: var(--popover);
    font: 400 14px/1.5 var(--font-mono);
    color: var(--foreground);
    overflow-x: auto;
    white-space: nowrap;
    user-select: text;
  }

  .form {
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 8px;
  }

  .actions > * {
    flex: 1 1 auto;
  }

  .pending {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 16px 0 0;
    font-size: 14px;
    color: var(--text-muted);
  }

  .spinner {
    width: 16px;
    height: 16px;
    border: 2px solid var(--border);
    border-top-color: var(--primary);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }

  @media (max-width: 560px) {
    .content {
      padding: 16px 20px 24px;
    }
    .connect {
      padding: 20px;
    }
    .splash-name {
      font-size: 32px;
    }
  }
</style>
