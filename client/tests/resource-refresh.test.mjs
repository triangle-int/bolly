import test from 'node:test';
import assert from 'node:assert/strict';
import { maintainResource } from '../src/lib/api/resource-refresh.js';

const tick = () => new Promise(resolve => setImmediate(resolve));

test('browser media honors server refresh timing and discards stale responses after cleanup', async () => {
    const timers = [];
    const seen = [];
    let count = 0;
    const stop = maintainResource(async () => ({ url: `cap-${++count}`, refresh_after_seconds: 17 }), url => seen.push(url), {
        schedule: (fn, ms) => { timers.push({ fn, ms }); return timers.length; }, cancel: () => {},
    });
    await tick();
    assert.deepEqual(seen, ['cap-1']);
    assert.equal(timers[0].ms, 17000);
    timers[0].fn();
    await tick();
    assert.deepEqual(seen, ['cap-1', 'cap-2']);
    stop();
    timers[1].fn();
    await tick();
    assert.deepEqual(seen, ['cap-1', 'cap-2']);
});

test('media failure requests an immediate bounded refresh without parallel issuance', async () => {
    const timers = [];
    const seen = [];
    let resolveSecond;
    let count = 0;
    const stop = maintainResource(async () => {
        count += 1;
        if (count === 2) return new Promise(resolve => { resolveSecond = resolve; });
        return { url: `cap-${count}`, refresh_after_seconds: 60 };
    }, url => seen.push(url), {
        schedule: (fn, ms) => { timers.push({ fn, ms }); return timers.length; }, cancel: () => {},
    });
    await tick();
    stop.refreshNow();
    stop.refreshNow();
    assert.equal(count, 2);
    resolveSecond({ url: 'cap-2', refresh_after_seconds: 60 });
    await tick();
    assert.deepEqual(seen, ['cap-1', 'cap-2']);
    stop.refreshNow();
    await tick();
    assert.equal(count, 3);
    stop.refreshNow();
    await tick();
    assert.equal(count, 3, 'immediate retries are bounded');
    stop();
});

test('failed issuance clears media and retries with backoff', async () => {
    const timers = [];
    const seen = [];
    const stop = maintainResource(async () => { throw Error('rotated'); }, url => seen.push(url), {
        schedule: (fn, ms) => { timers.push({ fn, ms }); return 1; }, cancel: () => {},
    });
    await tick();
    assert.deepEqual(seen, ['']);
    assert.equal(timers[0].ms, 30000);
    stop();
});
