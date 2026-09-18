import test from 'node:test';
import assert from 'node:assert/strict';
import { embeddingStatusText } from '../src/lib/embedding-status.js';

test('unavailable embeddings explain BM25 and preserve unknown status honestly', () => {
    assert.match(embeddingStatusText(undefined), /status unavailable/i);
    const text = embeddingStatusText({ status: 'unavailable', provider: 'openai', model: 'text-embedding-3-small', reason: 'OpenAI embedding API key is missing' });
    assert.match(text, /OpenAI/);
    assert.match(text, /API key is missing/);
    assert.match(text, /BM25/);
});
test('unverified is not presented as connected and pending config requires a restart', () => {
    const text = embeddingStatusText({ status: 'unverified', needs_restart: true });
    assert.match(text, /not yet verified/i);
    assert.match(text, /restart/i);
    assert.doesNotMatch(text, /connected/i);
});
test('working embeddings report the configured backend, without claiming arbitrary provider support', () => {
    const text = embeddingStatusText({ status: 'available', provider: 'openai_compatible', model: 'custom-model' });
    assert.match(text, /OpenAI-compatible/);
    assert.match(text, /custom-model/);
    assert.match(text, /local/i);
    assert.match(text, /unauthenticated/i);
    assert.match(text, /available/);
});
test('an unsupported configuration is not labeled as an OpenAI connection', () => {
    const text = embeddingStatusText({ status: 'unavailable', provider: 'unsupported', reason: 'embedding provider is unconfigured or unsupported' });
    assert.doesNotMatch(text, /OpenAI/);
    assert.match(text, /unsupported/);
});
