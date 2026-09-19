import test from 'node:test';
import assert from 'node:assert/strict';
import { saveOnboardingProvider } from '../src/lib/components/onboarding/provider.js';

for (const [provider, field] of [['openai', 'openai'], ['anthropic', 'api_key']]) {
	test(`${provider} onboarding saves only its own credential and activates it afterward`, async () => {
		const calls = [];
		await saveOnboardingProvider(provider, 'test-only-key', {
			updateLlmConfig: async payload => { calls.push(['save', payload]); },
			updateProvider: async value => { calls.push(['activate', value]); },
		});
		assert.deepEqual(calls, [['save', { [field]: 'test-only-key' }], ['activate', provider]]);
	});
}

test('failed credential save does not activate a provider', async () => {
	let activated = false;
	await assert.rejects(saveOnboardingProvider('openai', 'test-only-key', {
		updateLlmConfig: async () => { throw new Error('Save failed'); },
		updateProvider: async () => { activated = true; },
	}), /Save failed/);
	assert.equal(activated, false);
});

test('activation failure is returned to onboarding instead of reporting success', async () => {
	await assert.rejects(saveOnboardingProvider('openai', 'test-only-key', {
		updateLlmConfig: async () => {},
		updateProvider: async () => { throw new Error('Activation failed'); },
	}), /Activation failed/);
});
