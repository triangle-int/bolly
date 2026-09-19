import test from 'node:test';
import assert from 'node:assert/strict';
import {
	canCancel,
	canRetry,
	outcomeSummary,
	relativeTime,
	statusLabel,
	triggerLabel,
	upsertRun,
} from '../src/lib/activity/receipts.js';

const base = {
	id: 'run_100_a',
	trigger: { kind: 'heartbeat', agent: 'companion' },
	reason: 'periodic check-in',
	target: { kind: 'companion' },
	status: { kind: 'completed' },
	attempt: 1,
	started_at: 100,
	finished_at: 105,
	approvals: [],
	outcome: { actions: [{ tool: 'reach_out', summary: 'sent a message' }, { tool: 'memory_write', summary: 'writing notes/tea.md' }], messages_sent: 1, tokens: 10 },
};

test('receipts describe trigger and status without exposing model text', () => {
	assert.equal(triggerLabel({ kind: 'heartbeat', agent: 'companion' }), 'Check-in');
	assert.equal(triggerLabel({ kind: 'heartbeat', agent: 'reflection' }), 'Reflection');
	assert.equal(triggerLabel({ kind: 'machine_connected', machine_id: 'mac' }), 'Computer connected · mac');
	assert.equal(statusLabel({ kind: 'skipped', reason: { kind: 'quiet_hours' } }), 'Held for quiet hours');
	assert.equal(statusLabel({ kind: 'failed', error: 'x', retryable: true }), 'Failed, can retry');
	assert.equal(statusLabel({ kind: 'failed', error: 'x', retryable: false }), 'Failed');
	assert.equal(outcomeSummary(base), 'sent you a message, 1 action');
	assert.equal(outcomeSummary({ ...base, outcome: { actions: [], messages_sent: 0, tokens: 0 } }), 'nothing to do');
	assert.equal(outcomeSummary({ ...base, outcome: null }), '');
});

test('cancel and retry follow the run status exactly', () => {
	assert.equal(canCancel({ ...base, status: { kind: 'running' } }), true);
	assert.equal(canCancel(base), false);
	assert.equal(canRetry({ ...base, status: { kind: 'cancelled' } }), true);
	assert.equal(canRetry({ ...base, status: { kind: 'failed', error: 'x', retryable: true } }), true);
	assert.equal(canRetry({ ...base, status: { kind: 'failed', error: 'x', retryable: false } }), false);
	assert.equal(canRetry(base), false);
});

test('live updates replace runs in place and keep newest first', () => {
	const older = { ...base, id: 'run_50_b', started_at: 50, status: { kind: 'running' } };
	let runs = upsertRun([base], older);
	assert.deepEqual(runs.map((r) => r.id), ['run_100_a', 'run_50_b']);
	runs = upsertRun(runs, { ...older, status: { kind: 'completed' } });
	assert.equal(runs.length, 2);
	assert.equal(runs[1].status.kind, 'completed');
	assert.equal(relativeTime(1000, 1030), 'just now');
	assert.equal(relativeTime(1000, 1000 + 120), '2m ago');
	assert.equal(relativeTime(1000, 1000 + 7200), '2h ago');
	assert.equal(relativeTime(1000, 1000 + 3 * 86400), '3d ago');
});
