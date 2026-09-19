import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const path = new URL('../src/lib/api/client.ts', import.meta.url);
const source = ts.transpile(readFileSync(path, 'utf8'), { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext }).replace('./legacy-auth-cleanup.js', new URL('../src/lib/api/legacy-auth-cleanup.js', import.meta.url).href);
const api = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
// Since #112 the browser holds no credential: the HttpOnly session cookie is
// attached by the browser itself, so no request may add an Authorization
// header or a query credential, and localStorage is never read for a token.
const secret = 'issue116-browser-control-secret';
const storageReads = [];
globalThis.localStorage = { getItem(key) { storageReads.push(key); return secret; }, removeItem() {} };

function assertNoBrowserCredential(call) {
    assert.equal(call.options?.headers?.Authorization, undefined);
    assert.doesNotMatch(String(call.url), /[?&]token=/);
    assert.ok(!JSON.stringify(call).includes(secret), 'a stored secret leaked into a request');
}

test('upload media requests an exact browser capability with the session cookie only', async () => {
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
    assertNoBrowserCredential(calls[0]);
    assert.deepEqual(JSON.parse(calls[0].options.body), { id: 'id' });
});

test('export fetch returns a blob without any browser credential', async () => {
    const calls = [];
    globalThis.fetch = async (url, options) => {
        calls.push({url, options});
        return new Response('archive');
    };
    const blob = await api.exportInstance('moon');
    assert.equal(await blob.text(), 'archive');
    assert.equal(calls[0].url, '/api/instances/moon/export');
    assertNoBrowserCredential(calls[0]);
});

test('authenticated memory API paths preserve nested Unicode and literal URL delimiters', async () => {
    const calls = [];
    globalThis.fetch = async (url, options) => { calls.push({url, options}); return new Response('memory'); };
    await api.fetchMemoryContent('moon', 'Folder/Résumé #1%.md');
    await api.deleteMemoryFile('moon', 'Folder/Résumé #1%.md');
    for (const call of calls) {
        assert.equal(call.url, '/api/instances/moon/memory/Folder/R%C3%A9sum%C3%A9%20%231%25.md');
        assertNoBrowserCredential(call);
    }
});

test('the client never reads a token out of localStorage', () => {
    assert.ok(!storageReads.includes('nolune_auth_token'), storageReads.join(','));
});
