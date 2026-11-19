import assert from 'node:assert/strict';
import test from 'node:test';
import { frameMewStory } from '../src/mew-story.ts';

const overview = { target: [0, 2, 0] as const, yaw: 3, pitch: 0.4, zoom: 0.92 };
const hall = { target: [0, 4, -3] as const, yaw: -3, pitch: 0.52, zoom: 1.85 };
const left = { stop: overview, center: 400, side: 'left', hero: true } as const;
const right = { stop: hall, center: 1200, side: 'right' } as const;

test('the landing camera moves to the opposite side of the copy as the story scrolls', () => {
  const hero = frameMewStory(left, right, 400, false, false);
  assert.deepEqual(hero.target, overview.target);
  assert.equal(hero.shiftX, 0.14);
  assert.equal(hero.shiftY, -0.1);
  const midpoint = frameMewStory(left, right, 800, false, false);
  assert.ok(midpoint.yaw > 3 && midpoint.yaw < 3.29);
  assert.ok(midpoint.shiftX < 0.14 && midpoint.shiftX > -0.22);
  const next = frameMewStory(left, right, 1200, false, false);
  assert.deepEqual(next.target, hall.target);
  assert.ok(Math.abs(next.shiftX + 0.22) < 1e-12);
});

test('mobile framing places the garden below the copy and reduced motion picks a complete frame', () => {
  const frame = frameMewStory(left, right, 850, true, true);
  assert.deepEqual(frame.target, hall.target);
  assert.equal(frame.shiftX, 0);
  assert.equal(frame.shiftY, -0.24);
});
