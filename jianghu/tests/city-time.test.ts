import assert from 'node:assert/strict';
import test from 'node:test';
import { sampleCityTime } from '../src/city-lighting.ts';

test('midnight wraps, dawn and dusk are warm, and lights brighten at night', () => {
  const midnight = sampleCityTime(0);
  assert.deepEqual(midnight, sampleCityTime(1440));
  assert.equal(midnight.night, 1);
  assert.equal(sampleCityTime(720).night, 0);
  assert.ok(sampleCityTime(390).twilight > 0.8);
  assert.ok(sampleCityTime(1110).twilight > 0.8);
  assert.equal(sampleCityTime(720).twilight, 0);
  assert.ok(sampleCityTime(360).night > sampleCityTime(405).night);
  assert.ok(sampleCityTime(1065).night < sampleCityTime(1140).night);
});

test('the sun crosses the city from morning to afternoon', () => {
  const morning = sampleCityTime(9 * 60).direction;
  const noon = sampleCityTime(12 * 60).direction;
  const afternoon = sampleCityTime(16 * 60).direction;
  assert.ok(morning[0] > afternoon[0]);
  assert.ok(noon[1] > afternoon[1]);
});
