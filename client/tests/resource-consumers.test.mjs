import test from 'node:test';
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
function files(dir) { return readdirSync(dir, { withFileTypes: true }).flatMap(e => e.isDirectory() ? files(join(dir,e.name)) : [join(dir,e.name)]); }
test('recursive browser HTTP producers never append control-token queries', () => {
    for (const file of files(new URL('../src', import.meta.url).pathname)) {
        if (!/\.(ts|js|svelte)$/.test(file)) continue;
        let source = readFileSync(file, 'utf8');
        if (file.endsWith('/api/client.ts')) {
            const ws = /\/\/ ISSUE-112: sole query-control-token exemption; WebSocket handshake only\.\nexport function createWebSocket\(\): WebSocket \{[^]*?\n\}/;
            source = source.replace(ws, '');
        }
        assert.doesNotMatch(source, /[?&]token=/, file);
    }
});
test('attachment and memory consumers refresh browser grants', () => {
    for (const file of ['chat/MessageBubble.svelte', 'memory/MemoryMapView.svelte']) {
        const source = readFileSync(new URL(`../src/lib/components/${file}`, import.meta.url), 'utf8');
        assert.match(source, /use:resourceMedia/, file);
    }
});
test('saved prose and open viewer resources renew scoped URLs', () => {
    const prose = readFileSync(new URL('../src/lib/components/chat/MessageBubble.svelte', import.meta.url), 'utf8');
    assert.ok(prose.includes('use:resourceProse={html}'));
    const viewer = readFileSync(new URL('../src/lib/stores/fileviewer.svelte.ts', import.meta.url), 'utf8');
    assert.ok(viewer.includes('maintainResource('));
    assert.ok(viewer.includes('resourceFromUrl('));
});
test('drop images use resource identity refresh and capability URLs never open top-level', () => {
    const drop = readFileSync(new URL('../src/lib/components/drops/DropCard.svelte', import.meta.url), 'utf8');
    assert.match(drop, /resourceFromUrl/);
    assert.match(drop, /use:resourceMedia/);
    const viewer = readFileSync(new URL('../src/lib/components/FileViewer.svelte', import.meta.url), 'utf8');
    assert.doesNotMatch(viewer, /window\.open\s*\(/);
    assert.match(viewer, /refreshViewerResource/);
});
