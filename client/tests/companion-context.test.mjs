import test from 'node:test';
import assert from 'node:assert/strict';
import {
	DEFAULT_COMPANION_SLUG,
	companionHome,
	needsOnboarding,
	redirectForStaleSlug,
} from '../src/lib/companion/context.js';

test('the client never invents a companion identity: the home route opens the canonical slug', () => {
	assert.equal(DEFAULT_COMPANION_SLUG, 'companion');
	assert.equal(companionHome('companion'), '/companion');
	assert.equal(companionHome(''), '/companion', 'an unloaded slug falls back to the canonical default');
});

test('stale multi-instance URLs redirect to the one companion and keep chat and device context', () => {
	const canonical = 'companion';
	assert.equal(redirectForStaleSlug(canonical, 'alice', '/alice'), '/companion');
	assert.equal(redirectForStaleSlug(canonical, 'alice', '/alice/memory'), '/companion/memory');
	assert.equal(
		redirectForStaleSlug(canonical, 'alice', '/alice/chat/thread-42'),
		'/companion/chat/thread-42',
		'chat context survives the redirect',
	);
	assert.equal(
		redirectForStaleSlug(canonical, 'alice', '/alice/chat/alice'),
		'/companion/chat/alice',
		'only the identity segment is rewritten',
	);
	assert.equal(redirectForStaleSlug(canonical, 'companion', '/companion/chat'), null);
	assert.equal(redirectForStaleSlug(canonical, 'Companion', '/Companion/settings'), '/companion/settings');
});

test('onboarding is offered only once the server says the companion does not exist yet', () => {
	assert.equal(needsOnboarding(null), false, 'unknown state never flashes onboarding');
	assert.equal(needsOnboarding({ slug: 'companion', exists: false, companion_name: '', soul_exists: false }), true);
	assert.equal(needsOnboarding({ slug: 'companion', exists: true, companion_name: 'Luna', soul_exists: true }), false);
});

test('onboarding never greets the user by the companion slug when no name was given', async () => {
	const { introGreeting, onboardingHandshake } = await import('../src/lib/companion/context.js');
	assert.equal(introGreeting('Tim'), 'hey, Tim.');
	assert.equal(introGreeting(''), 'hey.');
	assert.equal(introGreeting(null), 'hey.');
	assert.equal(onboardingHandshake('Tim', 'English'), 'my name is Tim. please speak to me in English.');
	assert.equal(onboardingHandshake(null, 'Deutsch'), 'please speak to me in Deutsch.');
});
