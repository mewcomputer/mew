import assert from 'node:assert/strict';
import test from 'node:test';
import { gaugeCrown, type JoineryPiece } from '../src/city-joinery.ts';

function contains({ min, max }: JoineryPiece, point: readonly number[]) {
  return point.every((value, axis) => value > min[axis] && value < max[axis]);
}

test('orthogonal crown beams keep opposite webs at the crossing', () => {
  const { alongX, alongZ } = gaugeCrown();
  const has = (pieces: readonly JoineryPiece[], ...point: [number, number, number]) => pieces.some((piece) => contains(piece, point));
  assert.ok(has(alongX, 0.23, 0.21, 0));
  assert.equal(has(alongX, 0.23, 0.70, 0), false);
  assert.ok(has(alongZ, 0, 0.70, 0.23));
  assert.equal(has(alongZ, 0, 0.21, 0.23), false);
  assert.ok(has(alongX, 2.5, 0.70, 0));
  assert.ok(has(alongZ, 0, 0.21, 1.85));
});

test('a vertical key passes through both webs with assembly clearance', () => {
  const { alongX, alongZ, key } = gaugeCrown();
  assert.ok(contains(key, [0, 0.21, 0]));
  assert.ok(contains(key, [0, 0.70, 0]));
  for (const [first, second] of [[alongX, alongZ], [alongX, [key]], [alongZ, [key]]]) {
    for (const a of first) {
      for (const b of second) {
        const intersects = a.min.every((value, axis) => value < b.max[axis] && a.max[axis] > b.min[axis]);
        assert.equal(intersects, false, 'separate members must not occupy the same volume');
      }
    }
  }
});
