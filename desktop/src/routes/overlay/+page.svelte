<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  let recording = $state(false);
  let visible = $state(false);
  let serverUrl = $state("");
  let actionQueue = $state<{ id: number; text: string; icon: string }[]>([]);
  let idCounter = 0;
  let hideTimer: ReturnType<typeof setTimeout> | null = null;

  // Little Moon avatar from server
  const avatarSrc = $derived(
    serverUrl ? `${serverUrl}/skins/moon/character.svg` : ""
  );

  function resetHideTimer() {
    if (hideTimer) clearTimeout(hideTimer);
    hideTimer = setTimeout(() => {
      if (!recording) visible = false;
    }, 10000);
  }

  const actionIcons: Record<string, string> = {
    screenshot: "\u{1F4F7}", left_click: "\u{1F5B1}\uFE0F", right_click: "\u{1F5B1}\uFE0F",
    middle_click: "\u{1F5B1}\uFE0F", double_click: "\u{1F5B1}\uFE0F\u{1F5B1}\uFE0F",
    mouse_move: "\u2197\uFE0F", scroll: "\u2195\uFE0F", type: "\u2328\uFE0F", key: "\u2318",
    bash: "\u{1F4BB}", switch_desktop: "\u{1F5A5}\uFE0F",
    start_recording: "\u{1F534}", stop_recording: "\u{23F9}\uFE0F",
    collect_screen_recording: "\u{1F3AC}", get_frame: "\u{1F4F8}",
  };

  const actionLabels: Record<string, string> = {
    screenshot: "screenshot", left_click: "click", right_click: "right click",
    middle_click: "middle click", double_click: "double click", mouse_move: "move",
    scroll: "scroll", type: "typing", key: "key", bash: "command",
    switch_desktop: "switch space", start_recording: "recording started",
    stop_recording: "recording stopped", collect_screen_recording: "collecting",
    get_frame: "frame",
  };

  function flashAction(name: string, detail: string) {
    const id = ++idCounter;
    const icon = actionIcons[name] ?? "\u26A1";
    const label = actionLabels[name] ?? name;
    const text = detail ? `${label}: ${detail}` : label;
    actionQueue = [...actionQueue, { id, text, icon }];
    setTimeout(() => { actionQueue = actionQueue.filter(a => a.id !== id); }, 3000);
  }

  onMount(() => {
    let disposed = false;
    void (async () => {
      try {
        const isRec = await invoke<boolean>("get_screen_recording_allowed");
        if (!disposed && isRec) recording = true;
      } catch {}

      // Get server URL for video source
      try {
        const url = await invoke<string>("get_server_url");
        if (!disposed) serverUrl = url;
      } catch {}
    })();

    const unlistenAction = listen<string>("computer-use-action", (e) => {
      try {
        const data = JSON.parse(e.payload);
        visible = true;
        flashAction(data.action ?? "", data.detail ?? "");
      } catch {
        visible = true;
        flashAction(e.payload, "");
      }
      resetHideTimer();
    });

    const unlistenDone = listen("computer-use-idle", () => {
      if (!recording) visible = false;
    });

    const unlistenRec = listen<boolean>("screen-recording-state", (e) => {
      recording = e.payload;
      if (e.payload) visible = true;
    });

    const unlistenUrl = listen<string>("server-url", (e) => {
      serverUrl = e.payload;
    });

    return () => {
      disposed = true;
      unlistenAction.then(fn => fn());
      unlistenDone.then(fn => fn());
      unlistenRec.then(fn => fn());
      unlistenUrl.then(fn => fn());
    };
  });
</script>

<div class="overlay" class:overlay-visible={visible || recording}>
  <div class="pip" class:pip-recording={recording}>
    {#if recording}
      <div class="pip-ring"></div>
      <div class="pip-ring pip-ring-2"></div>
    {/if}

    {#if avatarSrc}
      <img src={avatarSrc} class="pip-avatar" alt="Nolune" />
    {:else}
      <div class="pip-placeholder"></div>
    {/if}

    {#if recording}
      <div class="pip-rec">
        <div class="pip-rec-dot"></div>
      </div>
    {/if}
  </div>

  <div class="flash-stack">
    {#each actionQueue as flash (flash.id)}
      <div class="flash">
        <span class="flash-icon">{flash.icon}</span>
        <span class="flash-text">{flash.text}</span>
      </div>
    {/each}
  </div>
</div>

<style>
  :global(html), :global(body) {
    background: transparent !important;
    margin: 0;
    padding: 0;
    overflow: hidden;
  }

  .overlay {
    position: fixed;
    inset: 0;
    pointer-events: none;
    z-index: 99999;
    opacity: 0;
    transition: opacity 0.5s ease;
  }
  .overlay-visible { opacity: 1; }

  .pip {
    position: absolute;
    bottom: 16px;
    right: 16px;
    width: 56px;
    height: 56px;
    border-radius: 50%;
    display: flex;
    align-items: center;
    justify-content: center;
    animation: pip-in 0.5s cubic-bezier(0.34, 1.56, 0.64, 1) both;
  }

  @keyframes pip-in {
    from { opacity: 0; transform: scale(0.5) translateY(10px); }
    to { opacity: 1; transform: scale(1) translateY(0); }
  }

  .pip-avatar {
    width: 100%;
    height: 100%;
    object-fit: contain;
    border-radius: 50%;
    border: 2px solid #B7A9E7;
  }

  .pip-placeholder {
    width: 100%;
    height: 100%;
    border-radius: 50%;
    border: 2px solid #51485F;
  }

  /* Recording rings */
  .pip-ring {
    position: absolute;
    inset: -4px;
    border-radius: 50%;
    border: 2px solid oklch(0.65 0.22 25 / 60%);
    animation: ring-pulse 2s ease-in-out infinite;
    z-index: 2;
  }
  .pip-ring-2 {
    inset: -8px;
    border-color: oklch(0.65 0.22 25 / 25%);
    animation-delay: 0.5s;
  }
  @keyframes ring-pulse {
    0%, 100% { transform: scale(1); opacity: 0.6; }
    50% { transform: scale(1.08); opacity: 0.2; }
  }

  .pip-recording .pip-avatar {
    border-color: oklch(0.65 0.22 25 / 40%);
  }

  /* REC dot */
  .pip-rec {
    position: absolute;
    top: -2px;
    right: -2px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    z-index: 3;
  }
  .pip-rec-dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: oklch(0.62 0.25 25);
    box-shadow: 0 0 10px oklch(0.62 0.25 25 / 90%);
    animation: rec-pulse 1.5s ease-in-out infinite;
  }
  @keyframes rec-pulse {
    0%, 100% { opacity: 1; transform: scale(1); }
    50% { opacity: 0.4; transform: scale(0.85); }
  }

  /* Flashes */
  .flash-stack {
    position: absolute;
    bottom: 78px;
    right: 12px;
    display: flex;
    flex-direction: column-reverse;
    gap: 6px;
    align-items: flex-end;
  }
  .flash {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 5px 12px;
    border-radius: 12px;
    background: oklch(0.10 0.02 260 / 88%);
    border: 1px solid oklch(1 0 0 / 10%);
    box-shadow: 0 4px 16px oklch(0 0 0 / 40%);
    animation: flash-in 3s ease both;
    white-space: nowrap;
  }
  @keyframes flash-in {
    0% { opacity: 0; transform: translateX(16px); }
    8% { opacity: 1; transform: translateX(0); }
    80% { opacity: 1; }
    100% { opacity: 0; transform: translateX(8px); }
  }
  .flash-icon { font-size: 13px; line-height: 1; }
  .flash-text {
    font-family: "SF Mono", "JetBrains Mono", ui-monospace, monospace;
    font-size: 11px;
    font-weight: 500;
    color: oklch(0.85 0.03 75);
    letter-spacing: 0.02em;
  }
</style>
