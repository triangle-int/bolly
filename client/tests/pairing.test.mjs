import test from 'node:test';
import assert from 'node:assert/strict';
import { formatPairingCodeInput, isCompletePairingCode, pairingErrorText } from '../src/lib/api/pairing.js';

test('pairing input is normalised to 1234-5678 as it is typed or pasted', () => {
	assert.equal(formatPairingCodeInput(''), '');
	assert.equal(formatPairingCodeInput('12'), '12');
	assert.equal(formatPairingCodeInput('1234'), '1234');
	assert.equal(formatPairingCodeInput('12345'), '1234-5');
	assert.equal(formatPairingCodeInput('1234-5678'), '1234-5678');
	assert.equal(formatPairingCodeInput(' 1234 5678 '), '1234-5678');
	assert.equal(formatPairingCodeInput('Pairing code:  4821-9037'), '4821-9037');
	assert.equal(formatPairingCodeInput('123456789'), '1234-5678', 'extra digits are dropped');
	assert.equal(formatPairingCodeInput(undefined), '');
});

test('completeness needs exactly eight digits', () => {
	assert.equal(isCompletePairingCode('1234-5678'), true);
	assert.equal(isCompletePairingCode('1234-567'), false);
	assert.equal(isCompletePairingCode('abcd-efgh'), false);
});

test('every server reason has a sentence and unknown reasons are honest', () => {
	for (const reason of ['invalid_code', 'rate_limited', 'cross_origin', 'auth_disabled']) {
		assert.ok(pairingErrorText(reason).length > 20, reason);
	}
	assert.match(pairingErrorText('rate_limited'), /10 minutes/);
	assert.match(pairingErrorText('invalid_code'), /5 minutes/);
	assert.match(pairingErrorText('unknown'), /server/i);
	assert.doesNotMatch(pairingErrorText('unknown'), /token/i, 'never tells a browser user to find the API token');
});
