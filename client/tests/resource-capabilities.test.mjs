import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const path = new URL('../src/lib/api/client.ts', import.meta.url);
const source = ts.transpile(readFileSync(path, 'utf8'), { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext }).replace('./legacy-auth-cleanup.js', new URL('../src/lib/api/legacy-auth-cleanup.js', import.meta.url).href);
const api = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
const secret = 'issue116-browser-control-secret';
globalThis.localStorage = { getItem: () => secret, removeItem() {} };

test('upload media requests an exact browser capability using Bearer headers', async () => {
    const calls = [];
    globalThis.fetch = async (url, options) => {
        calls.push({url, options});
        return Response.json({ url: '/resources/browser/files/moon/id?cap=scoped', refresh_after_seconds: 600 });
    };
    assert.deepEqual(await api.mediaUrl('moon', 'id'), {
        url: '/resources/browser/files/moon/id?cap=scoped',
        refresh_after_seconds: 600,
    });
    assert.equal(calls[0].url, '/api/instances/moon/resource-capabilities/files');
    assert.equal(calls[0].options.headers.Authorization, `Bearer ${secret}`);
    assert.deepEqual(JSON.parse(calls[0].options.body), { id: 'id' });
});

test('export fetch uses Authorization and returns a blob without query credentials', async () => {
    const calls = [];
    globalThis.fetch = async (url, options) => {
        calls.push({url, options});
        return new Response('archive');
    };
    const blob = await api.exportInstance('moon');
    assert.equal(await blob.text(), 'archive');
    assert.equal(calls[0].url, '/api/instances/moon/export');
    assert.equal(calls[0].options.headers.Authorization, `Bearer ${secret}`);
});

test('authenticated memory API paths preserve nested Unicode and literal URL delimiters', async () => {
    const calls = [];
    globalThis.fetch = async (url, options) => { calls.push({url, options}); return new Response('memory'); };
    await api.fetchMemoryContent('moon', 'Folder/Résumé #1%.md');
    await api.deleteMemoryFile('moon', 'Folder/Résumé #1%.md');
    for (const call of calls) {
        assert.equal(call.url, '/api/instances/moon/memory/Folder/R%C3%A9sum%C3%A9%20%231%25.md');
        assert.equal(call.options.headers.Authorization, `Bearer ${secret}`);
    }
});
