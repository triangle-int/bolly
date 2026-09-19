import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
const source = ts.transpile(readFileSync(new URL('../src/lib/api/resource-media.ts', import.meta.url), 'utf8'), { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext }).replace(/^import .*;$/gm, '');
const { resourceFromUrl } = await import('data:text/javascript;base64,' + Buffer.from(source).toString('base64'));
test('refresh identifies upload versus root/nested memory including literal hash and Unicode', () => {
    for (const [kind, path] of [['files','id'], ['memory','Résumé #1.png'], ['memory','Folder/Résumé #1.png']]) {
        const encoded = path.split('/').map(encodeURIComponent).join('/');
        assert.deepEqual(resourceFromUrl(`/resources/model-provider/${kind}/moon/${encoded}?cap=expired`, 'moon'), {slug:'moon',kind,path});
    }
});
test('refresh rejects noncanonical aliases, encoded separators, foreign origins and another instance', () => {
    for (const url of [
        '/resources/browser/files/moon/a%2Fb',
        '/resources/browser/memory/moon/a%2Fb',
        '/resources/browser/memory/moon/R%c3%a9.png',
        '/resources/browser/memory/moon/e%CC%81.png',
        '/resources/browser/files/%6Doon/id',
        '/resources/browser/files/other/id',
        'https://foreign.invalid/resources/browser/files/moon/id',
    ]) {
        assert.equal(resourceFromUrl(url, 'moon', 'https://nolune.local'), null, url);
    }
});
test('historical authenticated API file links are renewed instead of losing attachments', () => {
    assert.deepEqual(resourceFromUrl('/api/instances/moon/uploads/id/file?token=old-control', 'moon'), {slug:'moon',kind:'files',path:'id'});
    assert.deepEqual(resourceFromUrl('/api/instances/moon/memory/Folder/R%C3%A9sum%C3%A9.png?token=old-control', 'moon'), {slug:'moon',kind:'memory',path:'Folder/Résumé.png'});
});

test('destroy invalidates queued prose binds before they can issue resources', async () => {
    const lifecycleSource = ts.transpile(readFileSync(new URL('../src/lib/api/resource-media.ts', import.meta.url), 'utf8'), { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext })
        .replace(/^import .*;$/gm, '')
        .replace("return resource.kind === 'files' ? mediaUrl(resource.slug, resource.path) : memoryMediaUrl(resource.slug, resource.path);", 'issued += 1; return Promise.resolve({ url: "fresh", refresh_after_seconds: 60 });');
    const instrumented = `let issued = 0; const maintainResource = (issue) => { issue(); return () => { globalThis.__stopped += 1; }; };\n${lifecycleSource}\nexport const counts = () => ({ issued, stopped: globalThis.__stopped });`;
    globalThis.__stopped = 0;
    const lifecycle = await import('data:text/javascript;base64,' + Buffer.from(instrumented).toString('base64'));
    const child = { dataset: { resource: JSON.stringify({ slug: 'moon', kind: 'files', path: 'id' }) }, tagName: 'IMG', setAttribute() {}, removeAttribute() {} };
    const node = { querySelectorAll: () => [child] };

    const action = lifecycle.resourceProse(node, '');
    action.update();
    action.destroy();
    await Promise.resolve();
    assert.deepEqual(lifecycle.counts(), { issued: 1, stopped: 1 });

    const again = lifecycle.resourceProse(node, '');
    again.update();
    again.update();
    again.destroy();
    await Promise.resolve();
    assert.deepEqual(lifecycle.counts(), { issued: 2, stopped: 2 });
});
