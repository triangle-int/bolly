import test from 'node:test';
import assert from 'node:assert/strict';
import {
	displayName,
	filterEntries,
	folderOf,
	formatSize,
	groupByFolder,
	mediaKind,
	relatedPaths,
} from '../src/lib/memory/library.js';

const entries = [
	{ path: 'people/tim.md', summary: 'Tim likes oolong', size: 120 },
	{ path: 'about/self.md', summary: 'who i am', size: 300 },
	{ path: 'notes.md', summary: 'loose note', size: 40 },
	{ path: 'people/anna.md', summary: 'Anna, colleague', size: 80 },
	{ path: 'media/sky.png', summary: 'a blue sky', size: 5000 },
];

test('library groups by folder with root files last and entries sorted', () => {
	const groups = groupByFolder(entries);
	assert.deepEqual(groups.map((g) => g.folder), ['about', 'media', 'people', '']);
	assert.deepEqual(groups[2].entries.map((e) => e.path), ['people/anna.md', 'people/tim.md']);
	assert.equal(groups[2].size, 200);
	assert.equal(folderOf('notes.md'), '');
	assert.equal(displayName('people/tim.md'), 'tim');
	assert.equal(displayName('media/sky.png'), 'sky.png');
});

test('media kinds and sizes are derived from the path, never from index internals', () => {
	assert.equal(mediaKind('media/sky.PNG'), 'image');
	assert.equal(mediaKind('clips/talk.mov'), 'video');
	assert.equal(mediaKind('voice/memo.m4a'), 'audio');
	assert.equal(mediaKind('docs/paper.pdf'), 'pdf');
	assert.equal(mediaKind('people/tim.md'), 'text');
	assert.equal(formatSize(512), '512 B');
	assert.equal(formatSize(2048), '2.0 KB');
	assert.equal(formatSize(3 * 1024 * 1024), '3.0 MB');
});

test('explicit links come from the memory graph and local filtering is case-insensitive', () => {
	const graph = { edges: [['about/self.md', 'people/tim.md'], ['people/anna.md', 'people/tim.md'], ['a.md', 'b.md']] };
	assert.deepEqual(relatedPaths(graph, 'people/tim.md'), ['about/self.md', 'people/anna.md']);
	assert.deepEqual(relatedPaths(graph, 'notes.md'), []);
	assert.deepEqual(relatedPaths(null, 'people/tim.md'), []);
	assert.deepEqual(filterEntries(entries, 'OOLONG').map((e) => e.path), ['people/tim.md']);
	assert.equal(filterEntries(entries, '  ').length, entries.length);
});
