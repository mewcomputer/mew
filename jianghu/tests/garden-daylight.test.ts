import assert from 'node:assert/strict';
import test from 'node:test';
import { createGardenDaylight } from '../src/garden-daylight.ts';

test('the garden reaches twilight before night and the lantern cue comes before dark', () => {
  const sky = createGardenDaylight(false);
  assert.equal(sky.advance(0).lighting.night, 0);
  sky.setNight(true);
  const dusk = sky.advance(0.325);
  assert.equal(dusk.twilight, 'dusk');
  assert.ok(dusk.lighting.night > 0.2 && dusk.lighting.night < 0.8);
  assert.ok(dusk.lighting.twilight > 0.75);
  assert.ok(dusk.lanternLevel > 0 && dusk.lanternLevel < 1);
  assert.equal(dusk.lanternsNeeded, true);
  assert.equal(sky.advance(0.35).lighting.night, 1);
});

test('sunrise starts the return while there is still some lantern light', () => {
  const sky = createGardenDaylight(true);
  sky.setNight(false);
  const dawn = sky.advance(0.325);
  assert.equal(dawn.twilight, 'dawn');
  assert.ok(dawn.lighting.night > 0.2 && dawn.lighting.night < 0.8);
  assert.equal(dawn.lanternsNeeded, false);
  assert.ok(dawn.lanternLevel > 0 && dawn.lanternLevel < 1);
  const day = sky.advance(0.35);
  assert.equal(day.lighting.night, 0);
  assert.equal(day.lanternLevel, 0);
});

test('reversing during twilight preserves the current light and reduced motion snaps', () => {
  const sky = createGardenDaylight(false);
  sky.setNight(true);
  const dusk = sky.advance(0.26);
  sky.setNight(false);
  assert.deepEqual(sky.advance(0).lighting, dusk.lighting);
  assert.ok(sky.advance(0.04).lighting.night < dusk.lighting.night);
  assert.equal(sky.advance(0, true).lighting.night, 0);
  sky.setNight(true);
  assert.equal(sky.advance(0, true).lighting.night, 1);
});

test('the Day/Night control settles in under seven tenths of a second in either direction', () => {
  const sky = createGardenDaylight(false);
  sky.setNight(true);
  assert.equal(sky.advance(0.7).lighting.night, 1);
  sky.setNight(false);
  assert.equal(sky.advance(0.7).lighting.night, 0);
});
