import assert from 'node:assert/strict';
import test from 'node:test';
import { createTacticsWalk } from '../src/tactics-walk.ts';

test('a walk follows the stair portal before it climbs and lands exactly at the destination', () => {
  const walk = createTacticsWalk([
    { x: 0, y: 0, z: 0 }, { x: 1, y: 0, z: 0 },
    { x: 1, y: 0.5, z: 0 }, { x: 1, y: 0.5, z: -1 },
  ]);
  assert.equal(walk.advance(0.1).y, 0);
  const rising = walk.advance(0.27);
  assert.equal(rising.x, 1);
  assert.ok(rising.y > 0 && rising.y < 0.5);
  assert.deepEqual(rising.direction, [1, 0]);
  const end = walk.advance(5);
  assert.deepEqual([end.x, end.y, end.z], [1, 0.5, -1]);
  assert.deepEqual(end.direction, [0, -1]);
  assert.equal(end.done, true);
});

test('the walk can finish in one update for reduced motion', () => {
  const walk = createTacticsWalk([{ x: 2, y: 1, z: 1 }, { x: 2, y: 0, z: 1 }, { x: 3, y: 0, z: 1 }]);
  assert.equal(walk.advance(Infinity).done, true);
  assert.deepEqual(walk.advance(0).direction, [1, 0]);
});
