<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { auth, init, saveConnection, testConnection, openConnection, disconnect } from "$lib/auth.svelte";
  import { updater, checkForUpdates, installUpdate, dismissUpdate } from "$lib/updater.svelte";


  let splash = $state(true);
  let splashFading = $state(false);
  let editing = $state(false);
  let shUrl = $state("");
  let shToken = $state("");

  onMount(() => {
    init();
    checkForUpdates();
    invoke("get_screen_recording_allowed").catch(() => {});
    const fallback = setTimeout(endSplash, 5000);
    return () => clearTimeout(fallback);
  });

  function endSplash() {
    splashFading = true;
    setTimeout(() => { splash = false; }, 600);
  }

  function edit() {
    shUrl = auth.connection?.url ?? "";
    shToken = "";
    auth.error = null;
    auth.message = null;
    editing = true;
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
</script>

<!-- Audio lives outside splash so it's not destroyed on transition -->
<audio src="/splash.mp3" autoplay></audio>

{#if splash}
  <div class="splash" class:splash-fade={splashFading}>
    <video
      class="splash-video"
      src="/splash.mp4"
      autoplay
      muted
      playsinline
      onended={endSplash}
    ></video>
    <div class="splash-brand">
      <img src="/icon.png" alt="" class="splash-logo" />
      <span class="splash-name">bolly</span>
    </div>
  </div>
{/if}

<div class="dashboard" class:dashboard-enter={!splash}>
    <div class="dashboard-glow"></div>

    <header class="header">
      <div class="brand">
        <img src="/icon.png" alt="" class="logo" />
        <span class="brand-name">bolly</span>
      </div>
      {#if auth.connection || auth.error}
        <button class="sign-out-btn" onclick={forget} disabled={auth.loading}>Disconnect</button>
      {/if}
    </header>

    {#if updater.available}
      <div class="update-banner">
        {#if updater.downloading}
          <div class="update-text">
            Updating to v{updater.version}...
          </div>
          <div class="update-progress-track">
            <div class="update-progress-bar" style:width="{Math.round(updater.progress * 100)}%"></div>
          </div>
        {:else if updater.error}
          <div class="update-text update-error-text">Update failed: {updater.error}</div>
          <div class="update-actions">
            <button class="update-btn" onclick={installUpdate}>Retry</button>
            <button class="update-dismiss" onclick={dismissUpdate}>Dismiss</button>
          </div>
        {:else}
          <div class="update-text">
            v{updater.version} is available
          </div>
          <div class="update-actions">
            <button class="update-btn" onclick={installUpdate}>Update & restart</button>
            <button class="update-dismiss" onclick={dismissUpdate}>Later</button>
          </div>
        {/if}
      </div>
    {/if}

    <main class="content">
      {#if !splash}
        <div class="sign-in-card">
          <h2 class="sign-in-title">connect to your companion</h2>
          {#if auth.error}<p class="sh-error" role="alert">{auth.error}</p>{/if}
          {#if auth.message}<p role="status">{auth.message}</p>{/if}
          {#if auth.connection && !editing}
            <p class="sign-in-desc">{auth.connection.url}</p>
            <div class="sh-form">
              <button class="sign-in-btn" onclick={openConnection} disabled={auth.loading}>Open / Reconnect</button>
              <button class="sign-in-btn" onclick={() => testConnection(auth.connection!.url)} disabled={auth.loading}>Test connection</button>
              <button class="sign-in-btn" onclick={edit} disabled={auth.loading}>Edit connection</button>
            </div>
          {:else}
            <p class="sign-in-desc">Connect to your own Bolly server. Your connection is saved on this computer.</p>
            <form class="sh-form" onsubmit={(event) => { event.preventDefault(); save(); }}>
              <label for="server-url">Server URL</label>
              <input id="server-url" class="paste-input" bind:value={shUrl} placeholder="http://localhost:3000" disabled={auth.loading} required />
              <label for="auth-token">Auth token</label>
              <input id="auth-token" class="paste-input" bind:value={shToken} type="password" autocomplete="off" placeholder={auth.connection ? "Leave blank to keep saved token" : "Auth token from config.toml"} disabled={auth.loading} required={!auth.connection} />
              <button class="sign-in-btn" type="submit" disabled={auth.loading || !shUrl.trim() || (!shToken.trim() && !auth.connection)}>Save connection</button>
              <button class="sign-in-btn" type="button" onclick={() => testConnection(shUrl, shToken)} disabled={auth.loading || !shUrl.trim() || (!shToken.trim() && !auth.connection)}>Test connection</button>
              {#if editing}
                <button class="sign-in-btn" type="button" onclick={() => { editing = false; shToken = ""; auth.error = null; auth.message = null; }} disabled={auth.loading}>Cancel</button>
              {/if}
            </form>
          {/if}
          {#if auth.loading}<p role="status">Please wait…</p>{/if}
        </div>
      {/if}
    </main>
  </div>

<style>
  /* ─── Splash ───────────────────────────────────────────────── */
  .splash {
    position: fixed;
    inset: 0;
    z-index: 100;
    background: var(--background);
    display: flex;
    align-items: center;
    justify-content: center;
    transition: opacity 0.6s ease;
  }

  .splash-fade {
    opacity: 0;
  }

  .splash-video {
    position: absolute;
    width: 420px;
    height: 420px;
    object-fit: contain;
    pointer-events: none;
    opacity: 0.7;
  }

  .splash-brand {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 14px;
    animation: splash-brand-in 1.2s cubic-bezier(0.16, 1, 0.3, 1) both;
    animation-delay: 0.3s;
  }

  .splash-logo {
    width: 40px;
    height: 40px;
    object-fit: contain;
  }

  .splash-name {
    font-family: var(--font-display);
    font-style: italic;
    font-size: 1.8rem;
    color: var(--foreground);
    letter-spacing: -0.02em;
  }

  @keyframes splash-brand-in {
    0% { opacity: 0; transform: translateY(10px) scale(0.95); }
    100% { opacity: 1; transform: translateY(0) scale(1); }
  }

  /* ─── Dashboard ────────────────────────────────────────────── */
  .dashboard {
    display: flex;
    flex-direction: column;
    height: 100vh;
    position: relative;
    overflow: hidden;
    opacity: 0;
  }

  .dashboard-enter {
    animation: dash-in 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  @keyframes dash-in {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .dashboard-glow {
    position: absolute;
    top: 35%;
    left: 50%;
    width: 600px;
    height: 600px;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    background: radial-gradient(circle, oklch(0.55 0.08 240 / 3%) 0%, transparent 60%);
    pointer-events: none;
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 24px;
    position: relative;
    z-index: 1;
    -webkit-app-region: drag;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .logo {
    width: 28px;
    height: 28px;
    object-fit: contain;
  }

  .brand-name {
    font-family: var(--font-display);
    font-style: italic;
    font-size: 1.1rem;
    color: var(--foreground);
  }

  .sign-out-btn {
    -webkit-app-region: no-drag;
    padding: 5px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--muted);
    font-family: var(--font-body);
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.2s;
  }

  .sign-out-btn:hover {
    background: oklch(1 0 0 / 5%);
    color: var(--foreground);
    border-color: oklch(1 0 0 / 14%);
  }

  .content {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px;
    position: relative;
    z-index: 1;
  }

  /* ─── Update banner ─────────────────────────────────────────── */
  .update-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    margin: 0 24px;
    padding: 10px 16px;
    border-radius: 10px;
    background: oklch(0.78 0.12 75 / 8%);
    border: 1px solid oklch(0.78 0.12 75 / 14%);
    position: relative;
    z-index: 1;
    animation: dash-in 0.3s ease both;
  }

  .update-text {
    font-size: 0.78rem;
    color: var(--warm);
    white-space: nowrap;
  }

  .update-error-text {
    color: oklch(0.65 0.15 25 / 80%);
  }

  .update-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }

  .update-btn {
    padding: 5px 14px;
    border-radius: 7px;
    border: 1px solid oklch(0.78 0.12 75 / 22%);
    background: oklch(0.78 0.12 75 / 12%);
    color: var(--warm);
    font-family: var(--font-body);
    font-size: 0.72rem;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.2s;
    white-space: nowrap;
  }

  .update-btn:hover {
    background: oklch(0.78 0.12 75 / 20%);
    border-color: oklch(0.78 0.12 75 / 32%);
  }

  .update-dismiss {
    padding: 5px 10px;
    border-radius: 7px;
    border: none;
    background: transparent;
    color: var(--muted);
    font-family: var(--font-body);
    font-size: 0.72rem;
    cursor: pointer;
    transition: color 0.2s;
  }

  .update-dismiss:hover {
    color: var(--foreground);
  }

  .update-progress-track {
    flex: 1;
    height: 4px;
    border-radius: 2px;
    background: oklch(1 0 0 / 6%);
    overflow: hidden;
  }

  .update-progress-bar {
    height: 100%;
    border-radius: 2px;
    background: var(--warm);
    transition: width 0.3s ease;
  }

  /* ─── Sign in ──────────────────────────────────────────────── */
  .sign-in-card {
    text-align: center;
    max-width: 360px;
    animation: dash-in 0.5s cubic-bezier(0.16, 1, 0.3, 1) both;
  }

  .sign-in-title {
    font-family: var(--font-display);
    font-style: italic;
    font-size: 1.5rem;
    font-weight: 400;
    color: var(--foreground);
    margin: 0 0 12px;
  }

  .sign-in-desc {
    font-size: 0.82rem;
    color: var(--muted);
    margin: 0 0 28px;
    line-height: 1.5;
  }

  .sign-in-btn {
    padding: 10px 28px;
    border-radius: 10px;
    font-size: 0.85rem;
    font-weight: 500;
    font-family: var(--font-body);
    color: var(--warm);
    background: oklch(0.78 0.12 75 / 10%);
    border: 1px solid oklch(0.78 0.12 75 / 18%);
    border-top-color: oklch(0.78 0.12 75 / 28%);
    cursor: pointer;
    transition: all 0.3s ease;
  }

  .sign-in-btn:hover:not(:disabled) {
    background: oklch(0.78 0.12 75 / 16%);
    border-color: oklch(0.78 0.12 75 / 30%);
    box-shadow: 0 0 40px oklch(0.78 0.12 75 / 8%);
  }

  .sign-in-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .sh-error {
    font-size: 0.78rem;
    color: oklch(0.65 0.15 25 / 80%);
    margin: 0 0 8px;
    padding: 8px 12px;
    border-radius: 8px;
    background: oklch(0.65 0.15 25 / 8%);
    border: 1px solid oklch(0.65 0.15 25 / 14%);
  }

  .sh-form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .paste-input {
    flex: 1;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: oklch(1 0 0 / 3%);
    color: var(--foreground);
    font-family: monospace;
    font-size: 0.75rem;
    outline: none;
    transition: border-color 0.2s;
  }

  .paste-input:focus {
    border-color: oklch(1 0 0 / 16%);
  }

  .paste-input::placeholder {
    color: oklch(0.50 0.03 240);
  }



</style>
