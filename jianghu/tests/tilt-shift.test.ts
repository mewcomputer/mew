import assert from 'node:assert/strict';
import test from 'node:test';
import { mewOverview, mewTiltShift } from '../src/mew-story.ts';
import { tiltShiftAperture } from '../src/tilt-shift.ts';

test('tilt shift starts gently and increases as the camera zooms in', () => {
  assert.ok(tiltShiftAperture(1) > 0);
  assert.ok(tiltShiftAperture(2.5) > tiltShiftAperture(1));
  assert.ok(tiltShiftAperture(4.2) > tiltShiftAperture(2.5));
  assert.ok(tiltShiftAperture(4.2) <= 0.4);
  assert.equal(tiltShiftAperture(0.5), tiltShiftAperture(0.75));
  assert.equal(tiltShiftAperture(5), tiltShiftAperture(4.2));
});

test('the homepage has visible tilt shift in the overview and strengthens it on close-ups', () => {
  const overview = tiltShiftAperture(mewOverview.zoom, mewTiltShift);
  const closeup = tiltShiftAperture(2.05, mewTiltShift);
  assert.ok(overview >= 0.27);
  assert.ok(closeup > overview);
  assert.ok(tiltShiftAperture(2.5, mewTiltShift) <= 0.4);
  assert.equal(tiltShiftAperture(3, mewTiltShift), tiltShiftAperture(2.5, mewTiltShift));
});
