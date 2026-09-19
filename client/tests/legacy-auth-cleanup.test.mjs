import test from 'node:test';
import assert from 'node:assert/strict';
import { clearLegacyBrowserAuth } from '../src/lib/api/legacy-auth-cleanup.js';

const CURRENT_TOKEN_KEY = 'nolune_auth_token';
const LEGACY_STORAGE_KEYS = ['bolly_auth_token', 'bolly_token'];
const LEGACY_COOKIE_NAMES = ['nolune_token', 'bolly_token', 'bolly_auth_token'];

test('legacy browser auth cleanup preserves the current Nolune localStorage token', () => {
	const values = new Map([
		[CURRENT_TOKEN_KEY, 'keep-until-112'],
		...LEGACY_STORAGE_KEYS.map((key) => [key, `old-${key}`]),
	]);
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
	assert.equal(values.get(CURRENT_TOKEN_KEY), 'keep-until-112');
	assert.deepEqual(
		cookieWrites,
		LEGACY_COOKIE_NAMES.map(
			(name) => `${name}=; Path=/; Max-Age=0; SameSite=Strict`,
		),
	);
});

test('legacy browser auth cleanup tolerates unavailable browser storage', () => {
	assert.doesNotThrow(() => clearLegacyBrowserAuth(undefined, undefined));
});
