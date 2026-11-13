import test from 'node:test';
import assert from 'node:assert/strict';
import { comparePrice, alreadyMet } from './price.js';
test('到价预提示按精确十进制比较，包含等号且上下行对称', () => {
  assert.equal(comparePrice('0.1000000000000000001', '0.1'), 1);
  assert.equal(comparePrice('001.1000', '1.1'), 0);
  assert.equal(alreadyMet('100', '100.00', 'up'), true);
  assert.equal(alreadyMet('100', '100.00', 'down'), true);
  assert.equal(alreadyMet('99.99', '100', 'up'), false);
  for (const value of ['0', '-1', '1e2', 'NaN', '', '1.']) assert.throws(() => comparePrice(value, '1'));
});
