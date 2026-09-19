import test from 'node:test';
import assert from 'node:assert/strict';
import { clearLegacyBrowserAuth } from '../src/lib/api/legacy-auth-cleanup.js';

// Since #112 the browser never stores a credential: the pre-pairing
// localStorage token is legacy alongside the Bolly-era keys.
const LEGACY_STORAGE_KEYS = ['nolune_auth_token', 'bolly_auth_token', 'bolly_token'];
const LEGACY_COOKIE_NAMES = ['nolune_token', 'bolly_token', 'bolly_auth_token'];

test('legacy browser auth cleanup removes every stored token, including the pre-pairing one', () => {
	const values = new Map(LEGACY_STORAGE_KEYS.map((key) => [key, `old-${key}`]));
	const removed = [];
	const cookieWrites = [];
	const storage = {
		removeItem(key) {
			removed.push(key);
			values.delete(key);
		},
	};
	const cookieDocument = {
		set cookie(value) {
			cookieWrites.push(value);
		},
	};

	clearLegacyBrowserAuth(storage, cookieDocument);

	assert.deepEqual(removed, LEGACY_STORAGE_KEYS);
	assert.equal(values.size, 0);
	assert.deepEqual(
		cookieWrites,
		LEGACY_COOKIE_NAMES.map(
			(name) => `${name}=; Path=/; Max-Age=0; SameSite=Strict`,
		),
	);
	assert.ok(
		!cookieWrites.some((line) => line.startsWith('nolune_session=')),
		'the HttpOnly session cookie is owned by the server and never touched from JavaScript',
	);
});

test('legacy browser auth cleanup tolerates unavailable browser storage', () => {
	assert.doesNotThrow(() => clearLegacyBrowserAuth(undefined, undefined));
});
