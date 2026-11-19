import assert from 'node:assert/strict';
import test from 'node:test';
import { tiltShiftAperture } from '../src/tilt-shift.ts';

test('tilt shift starts gently and increases as the camera zooms in', () => {
  assert.ok(tiltShiftAperture(1) > 0);
  assert.ok(tiltShiftAperture(2.5) > tiltShiftAperture(1));
  assert.ok(tiltShiftAperture(4.2) > tiltShiftAperture(2.5));
  assert.ok(tiltShiftAperture(4.2) <= 0.4);
  assert.equal(tiltShiftAperture(0.5), tiltShiftAperture(0.75));
  assert.equal(tiltShiftAperture(5), tiltShiftAperture(4.2));
});
