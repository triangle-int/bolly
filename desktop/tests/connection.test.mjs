import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";
import ts from "typescript";
import { compileModule } from "svelte/compiler";

const source = compileModule(ts.transpile(
  readFileSync(new URL("../src/lib/auth.svelte.ts", import.meta.url), "utf8"),
  { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext },
), { generate: "server", filename: "auth.svelte.js" }).js.code;

async function setup(options = {}) {
  const calls = [];
  let savedOrigin = options.savedOrigin ?? null;
  const invoke = async (name, args) => {
    calls.push({ name, args });
    if (options.fail === name) throw Error("TOP_SECRET");
    if (name === "initialize_saved_connection") return savedOrigin;
    if (name === "save_connection") {
      savedOrigin = args.url;
      return savedOrigin;
    }
    if (name === "delete_saved_connection") savedOrigin = null;
  };
  const context = vm.createContext({ URL });
  const module = new vm.SourceTextModule(source, { context });
  await module.link((name) => {
    if (name === "svelte/internal/server") {
      return new vm.SyntheticModule([], function () {}, { context });
    }
    assert.equal(name, "@tauri-apps/api/core", "dashboard must not import mutable plugin storage");
    return new vm.SyntheticModule(["invoke"], function () { this.setExport("invoke", invoke); }, { context });
  });
  await module.evaluate();
  return { api: module.namespace, calls, savedOrigin: () => savedOrigin };
}

const init = async (options = {}) => {
  const fixture = await setup(options);
  await fixture.api.init();
  fixture.calls.length = 0;
  return fixture;
};

test("clean install initializes without saved metadata", async () => {
  const { api } = await setup(); await api.init();
  assert.equal(api.auth.connection, null);
});

test("restart exposes only the native-owned canonical origin", async () => {
  const { api } = await setup({ savedOrigin: "https://example.org" }); await api.init();
  assert.equal(JSON.stringify(api.auth.connection), JSON.stringify({ url: "https://example.org" }));
  assert.deepEqual(Object.keys(api.auth.connection), ["url"]);
});

test("initialization clears legacy browser auth before native credential lookup", async () => {
  const { api, calls } = await setup(); await api.init();
  assert.deepEqual(calls.map(({ name }) => name), ["clear_legacy_browser_auth", "initialize_saved_connection"]);
});

test("initialization errors never expose native error text", async () => {
  const { api } = await setup({ fail: "initialize_saved_connection" }); await api.init();
  assert.equal(api.auth.error.includes("TOP_SECRET"), false);
});

test("normalization accepts root HTTP origins", async () => {
  const { api } = await setup();
  assert.equal(api.normalizeConnection(" localhost:3000/ ", " x ").url, "http://localhost:3000");
});

test("normalization accepts root HTTPS origins with ports", async () => {
  const { api } = await setup();
  assert.equal(api.normalizeConnection("https://example.org:8443/", "x").url, "https://example.org:8443");
});

test("normalization rejects non-HTTP URLs", async () => {
  const { api } = await setup(); assert.throws(() => api.normalizeConnection("file:///tmp/x", "x"));
});

test("normalization rejects embedded credentials", async () => {
  const { api } = await setup(); assert.throws(() => api.normalizeConnection("https://u:p@example.org", "x"));
});

test("normalization rejects base paths", async () => {
  const { api } = await setup(); assert.throws(() => api.normalizeConnection("https://example.org/base", "x"));
});

test("normalization rejects queries and fragments", async () => {
  const { api } = await setup();
  assert.throws(() => api.normalizeConnection("https://example.org/?token=x", "x"));
  assert.throws(() => api.normalizeConnection("https://example.org/#x", "x"));
});

test("normalization rejects header injection", async () => {
  const { api } = await setup(); assert.throws(() => api.normalizeConnection("https://example.org", "x\r\nY: z"));
});

test("manual test sends entered token only to native validation", async () => {
  const { api, calls } = await init();
  assert.equal(await api.testConnection("localhost:3000", "TOP_SECRET"), true);
  assert.equal(JSON.stringify(calls), JSON.stringify([{ name: "test_connection", args: { url: "http://localhost:3000", token: "TOP_SECRET" } }]));
  assert.equal(JSON.stringify(api.auth).includes("TOP_SECRET"), false);
});

test("saved test sends no URL reference or credential metadata", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000" });
  assert.equal(await api.testConnection("http://localhost:3000", ""), true);
  assert.deepEqual(calls, [{ name: "test_saved_connection", args: undefined }]);
});

test("blank-token test cannot retarget the native saved credential", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000" });
  assert.equal(await api.testConnection("https://attacker.invalid", ""), false);
  assert.equal(calls.length, 0);
});

test("save delegates canonical binding to one native command", async () => {
  const { api, calls } = await init();
  assert.equal(await api.saveConnection("localhost:3000", "TOP_SECRET"), true);
  assert.equal(JSON.stringify(calls), JSON.stringify([{ name: "save_connection", args: { url: "http://localhost:3000", token: "TOP_SECRET" } }]));
  assert.equal(JSON.stringify(api.auth.connection), JSON.stringify({ url: "http://localhost:3000" }));
});

test("blank-token save only permits the existing origin", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000" });
  assert.equal(await api.saveConnection("https://attacker.invalid", ""), false);
  assert.equal(calls.length, 0);
});

test("reconnect invokes native saved open with no caller-controlled arguments", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000" });
  assert.equal(await api.openConnection(), true);
  assert.deepEqual(calls, [{ name: "open_saved_connection", args: undefined }]);
});

test("failed reconnect cleans transport and reports a safe error", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000", fail: "open_saved_connection" });
  assert.equal(await api.openConnection(), false);
  assert.equal(calls.at(-1).name, "disconnect_computer_use");
  assert.equal(api.auth.error.includes("TOP_SECRET"), false);
});

test("disconnect deletes native credentials despite transport failure", async () => {
  const { api, calls } = await init({ savedOrigin: "http://localhost:3000", fail: "disconnect_computer_use" });
  assert.equal(await api.disconnect(), false);
  assert.equal(calls.some(({ name }) => name === "delete_saved_connection"), true);
  assert.equal(api.auth.connection, null);
});
