import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import ts from "typescript";

const source = ts.transpile(readFileSync(new URL("../../client/src/lib/api/client.ts", import.meta.url), "utf8"), {
  target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext,
});
const cleanupSource = readFileSync(
  new URL("../../client/src/lib/api/legacy-auth-cleanup.js", import.meta.url),
  "utf8",
);

async function setup(desktop) {
  const urls = [];
  const context = vm.createContext({
    window: desktop ? { __NOLUNE_DESKTOP_RELAY__: true } : {},
    location: { protocol: "http:", host: "127.0.0.1:1234" },
    document: { cookie: "nolune_token=legacy-cookie" },
    localStorage: { getItem: () => "legacy-storage" },
    WebSocket: class { constructor(url) { urls.push(url); } },
    URL,
    fetch: async (url, options) => {
      assert.equal(options.headers.Authorization, undefined);
      assert.equal(url.includes("token="), false);
      return { ok: true, status: 200, json: async () => ({ url: "/resources/browser/files/a/b?cap=scoped" }) };
    },
  });
  const module = new vm.SourceTextModule(source, { context });
  await module.link((specifier) => {
    if (specifier === "./legacy-auth-cleanup.js") {
      return new vm.SourceTextModule(cleanupSource, { context });
    }
    throw Error(`Unexpected runtime import: ${specifier}`);
  });
  await module.evaluate();
  return { api: module.namespace, urls };
}

test("desktop client never converts legacy cookies/storage into navigation, media or WebSocket query tokens", async () => {
  const { api, urls } = await setup(true);
  assert.equal(api.isDesktopRelay(), true);
  // Since #112 browsers pair for a cookie session; the relay must not, because
  // it authenticates natively and strips cookies upstream.
  await assert.rejects(api.pairBrowser("1234-5678"), /desktop dashboard/);
  for (const url of [
    (await api.mediaUrl("a", "b")).url,
    await api.uploadFileUrl("a", "b"),
    api.exportInstanceUrl("a"),
  ]) {
    assert.equal(url.includes("token="), false);
    assert.equal(url.includes("legacy"), false);
  }
  api.createWebSocket();
  assert.deepEqual(urls, ["ws://127.0.0.1:1234/api/ws"]);
});

test("ordinary browsers hold no credential either: pairing and requests carry nothing from legacy storage", async () => {
  const { api, urls } = await setup(false);
  assert.equal(api.isDesktopRelay(), false);
  // The fetch stub rejects any Authorization header or query token.
  await api.pairBrowser("1234-5678");
  for (const url of [
    (await api.mediaUrl("a", "b")).url,
    await api.uploadFileUrl("a", "b"),
    api.exportInstanceUrl("a"),
  ]) {
    assert.equal(url.includes("token="), false);
    assert.equal(url.includes("legacy"), false);
  }
  api.createWebSocket();
  assert.deepEqual(urls, ["ws://127.0.0.1:1234/api/ws"]);
});
