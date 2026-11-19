import test from 'node:test';
import assert from 'node:assert/strict';
import { createWaterClock } from '../src/water-clock.ts';
import { createDistrict } from '../src/district.ts';
import { createWaterSimulation } from '../src/fluid.ts';

test('the water clock steps deterministically and discards paused wall time', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  const clock = createWaterClock(water);
  clock.advance(1000, true);
  clock.advance(1045, true);
  assert.equal(clock.simulated, 4 * water.dt);
  const surface = water.depth.slice();
  clock.advance(1545, false);
  clock.advance(8045, true);
  assert.deepEqual(water.depth, surface);
  clock.advance(8079, true);
  assert.equal(clock.simulated, 7 * water.dt);
  water.dispose();
});
