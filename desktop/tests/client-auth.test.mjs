import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import ts from "typescript";

const source = ts.transpile(readFileSync(new URL("../../client/src/lib/api/client.ts", import.meta.url), "utf8"), {
  target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext,
});

async function setup(desktop) {
  const urls = [];
  const context = vm.createContext({
    window: desktop ? { __NOLUNE_DESKTOP_RELAY__: true } : {},
    location: { protocol: "http:", host: "127.0.0.1:1234" },
    document: { cookie: "nolune_token=legacy-cookie" },
    localStorage: { getItem: () => "legacy-storage" },
    WebSocket: class { constructor(url) { urls.push(url); } },
    URL,
  });
  const module = new vm.SourceTextModule(source, { context });
  await module.link(() => { throw Error("Unexpected runtime import"); });
  await module.evaluate();
  return { api: module.namespace, urls };
}

test("desktop client never converts legacy cookies/storage into navigation, media or WebSocket query tokens", async () => {
  const { api, urls } = await setup(true);
  assert.equal(api.getAuthToken(), null);
  assert.throws(() => api.setAuthToken("new-secret"), /desktop dashboard/);
  for (const url of [api.mediaUrl("a", "b"), api.uploadFileUrl("a", "b"), api.exportInstanceUrl("a")]) {
    assert.equal(url.includes("token="), false);
    assert.equal(url.includes("legacy"), false);
  }
  api.createWebSocket();
  assert.deepEqual(urls, ["ws://127.0.0.1:1234/api/ws"]);
});

test("ordinary browser token behavior remains unchanged", async () => {
  const { api } = await setup(false);
  assert.equal(api.getAuthToken(), "legacy-storage");
});
