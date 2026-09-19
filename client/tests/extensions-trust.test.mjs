import test from 'node:test';
import assert from 'node:assert/strict';
import { activeToolNames, grantSummary, isCurated, requiresAcknowledgement, trustLabel } from '../src/lib/extensions/trust.js';

const curated = [{ name: 'brave-search', url: 'https://mcp.bravesearch.com/sse' }];

test('only exact catalog matches are curated; everything else needs an acknowledgement', () => {
	assert.equal(isCurated(curated, 'brave-search', 'https://mcp.bravesearch.com/sse'), true);
	assert.equal(isCurated(curated, 'brave-search', 'https://evil.example/sse'), false, 'same name, different URL');
	assert.equal(isCurated(curated, 'my-server', 'https://mcp.bravesearch.com/sse'), false, 'same URL, different name');
	assert.equal(requiresAcknowledgement(curated, 'my-server', 'https://example.com/mcp'), true);
	assert.equal(requiresAcknowledgement(curated, 'brave-search', 'https://mcp.bravesearch.com/sse'), false);
});

test('grants are summarised without exposing anything but names and counts', () => {
	const server = { name: 'x', trust: 'custom', connected: true, tools: [{ name: 'a', enabled: false }, { name: 'b', enabled: true }, { name: 'c', enabled: true }] };
	assert.equal(grantSummary(server), '2 of 3 tools allowed');
	assert.equal(grantSummary({ ...server, tools: [] }), 'no tools discovered');
	assert.equal(grantSummary({ ...server, tools: server.tools.map((t) => ({ ...t, enabled: false })) }), 'no tools allowed (3 available)');
	assert.equal(grantSummary({ ...server, tools: [{ name: 'a', enabled: true }] }), '1 tool allowed');
	assert.equal(trustLabel(server), 'Custom, not reviewed');
	assert.equal(trustLabel({ ...server, trust: 'curated' }), 'Reviewed');
});

test('ordinary chats only see enabled tools of connected servers', () => {
	const servers = [
		{ name: 'one', trust: 'curated', connected: true, tools: [{ name: 'search', enabled: true }, { name: 'fetch', enabled: false }] },
		{ name: 'two', trust: 'custom', connected: false, tools: [{ name: 'run', enabled: true }] },
	];
	assert.deepEqual(activeToolNames(servers), ['one/search']);
});
