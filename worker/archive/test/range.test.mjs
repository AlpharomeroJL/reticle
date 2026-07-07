// Unit tests for the untrusted-Range parser. The Range header is attacker
// controlled, so the parser is the security boundary: every malformed, backwards,
// or overflowing input must resolve to a single `unsatisfiable` verdict (the caller
// answers 416) and every satisfiable input must clamp to the object size. Run with
// `node --test` (no install, no network); this is the hermetic half of the gate.
import test from 'node:test';
import assert from 'node:assert/strict';
import { parseRange } from '../src/range.js';

test('no Range header serves the whole object', () => {
  assert.deepEqual(parseRange(null, 100), { type: 'full' });
  assert.deepEqual(parseRange('', 100), { type: 'full' });
  assert.deepEqual(parseRange(undefined, 100), { type: 'full' });
});

test('a closed range returns offset and length', () => {
  assert.deepEqual(parseRange('bytes=0-99', 100), { type: 'range', offset: 0, length: 100 });
  assert.deepEqual(parseRange('bytes=0-49', 100), { type: 'range', offset: 0, length: 50 });
  assert.deepEqual(parseRange('bytes=10-19', 100), { type: 'range', offset: 10, length: 10 });
});

test('an open-ended range runs to the end of the object', () => {
  assert.deepEqual(parseRange('bytes=50-', 100), { type: 'range', offset: 50, length: 50 });
  assert.deepEqual(parseRange('bytes=0-', 100), { type: 'range', offset: 0, length: 100 });
});

test('a suffix range returns the last N bytes', () => {
  assert.deepEqual(parseRange('bytes=-20', 100), { type: 'range', offset: 80, length: 20 });
  assert.deepEqual(parseRange('bytes=-1', 100), { type: 'range', offset: 99, length: 1 });
});

test('a suffix larger than the object clamps to the whole object', () => {
  assert.deepEqual(parseRange('bytes=-200', 100), { type: 'range', offset: 0, length: 100 });
});

test('an end past the object clamps to the last byte', () => {
  assert.deepEqual(parseRange('bytes=90-200', 100), { type: 'range', offset: 90, length: 10 });
  assert.deepEqual(parseRange('bytes=0-999999999999', 100), { type: 'range', offset: 0, length: 100 });
});

test('whitespace around the spec is tolerated', () => {
  assert.deepEqual(parseRange('bytes= 0-9 ', 100), { type: 'range', offset: 0, length: 10 });
});

test('a start at or past the object is unsatisfiable', () => {
  assert.deepEqual(parseRange('bytes=100-150', 100), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange('bytes=100-', 100), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange('bytes=200-300', 100), { type: 'unsatisfiable' });
});

test('a backwards range is unsatisfiable', () => {
  assert.deepEqual(parseRange('bytes=50-40', 100), { type: 'unsatisfiable' });
});

test('malformed specs are unsatisfiable, never full', () => {
  for (const h of ['bytes=abc', 'bytes=', 'bytes=-', 'bytes=1-2-3', 'bytes=-0', 'bytes=1.5-2']) {
    assert.deepEqual(parseRange(h, 100), { type: 'unsatisfiable' }, `expected unsatisfiable for ${h}`);
  }
});

test('a non-bytes unit is unsatisfiable', () => {
  assert.deepEqual(parseRange('items=0-10', 100), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange('0-10', 100), { type: 'unsatisfiable' });
});

test('multiple ranges are refused (no multipart support)', () => {
  assert.deepEqual(parseRange('bytes=0-10,20-30', 100), { type: 'unsatisfiable' });
});

test('negative and non-integer bounds are refused', () => {
  assert.deepEqual(parseRange('bytes=-5-10', 100), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange('bytes=0x10-0x20', 100), { type: 'unsatisfiable' });
});

test('an empty object cannot satisfy any range', () => {
  assert.deepEqual(parseRange('bytes=0-0', 0), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange('bytes=-1', 0), { type: 'unsatisfiable' });
  assert.deepEqual(parseRange(null, 0), { type: 'full' });
});
