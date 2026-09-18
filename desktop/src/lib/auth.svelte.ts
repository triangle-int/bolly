import { invoke } from "@tauri-apps/api/core";

export type Connection = { url: string };
type Draft = { url: string; token: string };
class ConnectionError extends Error {}

export const auth = $state({
  connection: null as Connection | null,
  loading: true,
  error: null as string | null,
  message: null as string | null,
});

export function normalizeConnection(url: string, token: string): Draft {
  let parsed: URL;
  try {
    const input = url.trim();
    parsed = new URL(input.includes("://") ? input : `http://${input}`);
  } catch {
    throw new ConnectionError("Enter a valid server URL.");
  }
  if (!["http:", "https:"].includes(parsed.protocol) || !parsed.hostname ||
      parsed.username || parsed.password || parsed.search || parsed.hash || parsed.pathname !== "/") {
    throw new ConnectionError("Use a root HTTP or HTTPS server URL without a base path, credentials, query parameters, or a fragment.");
  }
  if (/\r|\n/.test(token)) throw new ConnectionError("Enter a valid auth token.");
  return { url: parsed.origin, token: token.trim() };
}

export async function init() {
  auth.loading = true;
  auth.connection = null;
  auth.error = null;
  try {
    await invoke("clear_legacy_browser_auth");
    const url = await invoke<string | null>("initialize_saved_connection");
    auth.connection = url ? { url } : null;
  } catch {
    auth.error = "Could not restore the saved connection. Unlock the OS credential store and retry.";
  } finally {
    auth.loading = false;
  }
}

async function run(action: () => Promise<void>) {
  if (auth.loading) return false;
  auth.loading = true;
  auth.error = null;
  auth.message = null;
  try {
    await action();
    return true;
  } catch (error) {
    auth.error = error instanceof ConnectionError ? error.message : "Connection operation failed. Please retry.";
    return false;
  } finally {
    auth.loading = false;
  }
}

function normalizedSavedUrl(url: string) {
  const normalized = normalizeConnection(url, "").url;
  if (!auth.connection || auth.connection.url !== normalized) {
    throw new ConnectionError("Enter the auth token when changing the server URL.");
  }
  return normalized;
}

export async function testConnection(url: string, token = "") {
  return run(async () => {
    const config = normalizeConnection(url, token);
    if (config.token) {
      await invoke("test_connection", config);
    } else {
      normalizedSavedUrl(config.url);
      await invoke("test_saved_connection");
    }
    auth.message = "Connection test succeeded.";
  });
}

export async function saveConnection(url: string, token: string) {
  return run(async () => {
    const config = normalizeConnection(url, token);
    if (!config.token) normalizedSavedUrl(config.url);
    const savedUrl = await invoke<string>("save_connection", config);
    auth.connection = { url: savedUrl };
    auth.message = "Connection saved.";
  });
}

export async function openConnection() {
  return run(async () => {
    if (!auth.connection) return;
    try {
      await invoke("open_saved_connection");
    } catch {
      await invoke("disconnect_computer_use").catch(() => {});
      throw new ConnectionError("Could not open the companion. Please reconnect.");
    }
  });
}

export async function disconnect() {
  return run(async () => {
    let failed = false;
    try { await invoke("disconnect_computer_use"); } catch { failed = true; }
    try { await invoke("delete_saved_connection"); } catch { failed = true; }
    auth.connection = null;
    if (failed) throw new ConnectionError("Some credentials could not be removed. Unlock the OS credential store and retry Disconnect.");
  });
}
