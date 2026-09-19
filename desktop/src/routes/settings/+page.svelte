<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";

  type Permissions = {
    screen_recording: boolean;
    accessibility: boolean;
  };

  let permissions = $state<Permissions | null>(null);
  let checking = $state(false);

  onMount(async () => {
    refresh();
  });

  async function refresh() {
    checking = true;
    try {
      permissions = await invoke<Permissions>("check_permissions");
    } catch (e) {
      console.error("check_permissions failed", e);
    } finally {
      checking = false;
    }
  }

  async function openSettings(permission: string) {
    await invoke("open_permission_settings", { permission });
    setTimeout(refresh, 3000);
  }
</script>

<div class="settings">
  <header class="header">
    <h1 class="title">Settings</h1>
  </header>

  <main class="content">
    <section class="section">
      <h2 class="section-title">Permissions</h2>
      <p class="section-desc">
        Nolune needs these macOS permissions to control your computer when you ask it to.
      </p>

      {#if permissions}
        <div class="perm-list">
          <div class="perm-row">
            <div class="perm-info">
              <div class="perm-icon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/>
                </svg>
              </div>
              <div>
                <span class="perm-name">Screen Recording</span>
                <span class="perm-desc">Take screenshots of your screen</span>
              </div>
            </div>
            <div class="perm-status">
              {#if permissions.screen_recording}
                <span class="badge badge-granted">Granted</span>
              {:else}
                <button class="grant-btn" onclick={() => openSettings("screen_recording")}>Grant</button>
              {/if}
            </div>
          </div>

          <div class="perm-row">
            <div class="perm-info">
              <div class="perm-icon">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"/>
                </svg>
              </div>
              <div>
                <span class="perm-name">Accessibility</span>
                <span class="perm-desc">Control mouse and keyboard</span>
              </div>
            </div>
            <div class="perm-status">
              {#if permissions.accessibility}
                <span class="badge badge-granted">Granted</span>
              {:else}
                <button class="grant-btn" onclick={() => openSettings("accessibility")}>Grant</button>
              {/if}
            </div>
          </div>
        </div>

        <button class="refresh-btn" onclick={refresh} disabled={checking}>
          {checking ? "Checking..." : "Refresh status"}
        </button>
      {:else}
        <div class="loading">
          <div class="spinner"></div>
        </div>
      {/if}
    </section>
  </main>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    height: 100vh;
    background: var(--background);
  }

  .header {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 16px 24px;
    -webkit-app-region: drag;
  }

  .title {
    font-family: var(--font-display);
    font-style: italic;
    font-size: 1rem;
    font-weight: 400;
    color: var(--foreground);
    margin: 0;
  }

  .content {
    flex: 1;
    padding: 0 32px 32px;
    overflow-y: auto;
  }

  .section {
    max-width: 480px;
    margin: 0 auto;
  }

  .section-title {
    font-family: var(--font-display);
    font-style: italic;
    font-size: 0.95rem;
    font-weight: 400;
    color: var(--foreground);
    margin: 0 0 6px;
  }

  .section-desc {
    font-size: 0.75rem;
    color: var(--muted);
    margin: 0 0 20px;
    line-height: 1.5;
  }

  .perm-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
    background: var(--border);
    border-radius: 12px;
    overflow: hidden;
    margin-bottom: 16px;
  }

  .perm-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 16px;
    background: oklch(1 0 0 / 3%);
  }

  .perm-info {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .perm-icon {
    width: 32px;
    height: 32px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: 8px;
    background: oklch(1 0 0 / 5%);
    color: var(--muted);
  }

  .perm-name {
    display: block;
    font-size: 0.82rem;
    font-weight: 500;
    color: var(--foreground);
  }

  .perm-desc {
    display: block;
    font-size: 0.68rem;
    color: var(--muted);
    margin-top: 1px;
  }

  .badge {
    font-size: 0.68rem;
    padding: 3px 10px;
    border-radius: 20px;
    font-family: var(--font-body);
  }

  .badge-granted {
    background: oklch(0.72 0.17 142 / 12%);
    color: oklch(0.72 0.17 142);
    border: 1px solid oklch(0.72 0.17 142 / 20%);
  }

  .grant-btn {
    padding: 5px 14px;
    border-radius: 8px;
    border: 1px solid oklch(0.78 0.12 75 / 20%);
    background: oklch(0.78 0.12 75 / 10%);
    color: var(--warm);
    font-family: var(--font-body);
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.2s;
  }

  .grant-btn:hover {
    background: oklch(0.78 0.12 75 / 18%);
    border-color: oklch(0.78 0.12 75 / 35%);
  }

  .refresh-btn {
    display: block;
    margin: 0 auto;
    padding: 6px 16px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--muted);
    font-family: var(--font-body);
    font-size: 0.72rem;
    cursor: pointer;
    transition: all 0.2s;
  }

  .refresh-btn:hover:not(:disabled) {
    background: oklch(1 0 0 / 4%);
    color: var(--foreground);
  }

  .refresh-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .loading {
    display: flex;
    justify-content: center;
    padding: 24px;
  }

  .spinner {
    width: 20px;
    height: 20px;
    border: 2px solid oklch(1 0 0 / 10%);
    border-top-color: var(--warm);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin {
    to { transform: rotate(360deg); }
  }
</style>
