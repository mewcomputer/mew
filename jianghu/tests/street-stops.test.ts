import assert from 'node:assert/strict';
import test from 'node:test';
import { required } from './required.ts';
import { createCharacterTrack } from '../src/city-characters.ts';
import { createStreetLife } from '../src/street-life.ts';

function passer(id: string, z = 0) {
  return { id, sample: createCharacterTrack({ path: [[0, 2, z], [6, 2, z]] }) };
}

test('one passerby can stop for tea, face the counter, and resume from the same tile', () => {
  const at = createStreetLife([passer('first'), passer('second', 0.55)], {
    random: () => 0,
    stops: [{ mode: 'sip', near: [2, 2, 0], face: [2, 2, -1], radius: 0.85 }],
  });
  const views = at(2.5);
  assert.equal(required(views.get('first')).mode, 'sip');
  assert.deepEqual(required(views.get('first')).direction, [0, -1]);
  assert.equal(required(views.get('second')).mode, 'walk');
  const stopped = [...required(views.get('first')).position];
  at(4);
  assert.deepEqual(required(views.get('first')).position, stopped);
  at(5.5);
  assert.equal(required(views.get('first')).mode, 'walk');
  assert.deepEqual(required(views.get('first')).position, stopped);
  at(6);
  assert.deepEqual(required(views.get('first')).position, [2, 2, 0]);
});

test('a passerby only stops to watch while the court is practicing', () => {
  const at = createStreetLife([passer('visitor')], {
    random: () => 0,
    stops: [{ mode: 'watch', near: [2, 2, 0], face: [2, 2, 2], radius: 1.1 }],
  });
  const views = at(2.5, false);
  assert.equal(required(views.get('visitor')).mode, 'walk');
  at(2.7, true);
  assert.equal(required(views.get('visitor')).mode, 'watch');
  assert.deepEqual(required(views.get('visitor')).direction, [0, 1]);
  at(5.7, false);
  assert.equal(required(views.get('visitor')).mode, 'walk');
});

test('a toll can interrupt a walk or work nearby, and the walk resumes from the same tile', () => {
  const willing = { ...passer('walker'), bellAttention: 1.5 };
  const worker = {
    id: 'worker', bellAttention: 1,
    sample: createCharacterTrack({ path: [[2, 2, 0.7]], activity: 'work' }),
  };
  const at = createStreetLife([
    willing, worker, passer('unbothered'),
    { ...passer('distant', 12), bellAttention: 1.5 },
    { id: 'higher', bellAttention: 1.5, sample: createCharacterTrack({ path: [[2, 3, 0]] }) },
  ]);
  assert.equal(required(at(2.4).get('worker')).mode, 'work');
  const views = at(2.5, false, [2, 2, 2]);
  assert.equal(required(views.get('walker')).mode, 'watch');
  assert.deepEqual(required(views.get('walker')).direction, [0, 1]);
  assert.deepEqual(required(views.get('walker')).position, [1.5, 2, 0]);
  assert.equal(required(views.get('worker')).mode, 'watch');
  assert.equal(required(views.get('unbothered')).mode, 'walk');
  assert.equal(required(views.get('distant')).mode, 'walk');
  assert.equal(required(views.get('higher')).mode, 'idle');
  at(3.5);
  assert.equal(required(views.get('worker')).mode, 'work');
  assert.equal(required(views.get('walker')).mode, 'watch');
  at(4);
  assert.equal(required(views.get('walker')).mode, 'walk');
  assert.deepEqual(required(views.get('walker')).position, [1.5, 2, 0]);
  at(4.5);
  assert.deepEqual(required(views.get('walker')).position, [2, 2, 0]);
});

test('a toll leaves an existing conversation intact', () => {
  const at = createStreetLife([
    { ...passer('first'), market: true, bellAttention: 2 },
    { ...passer('second', 0.5), market: true, bellAttention: 2 },
  ], { random: () => 0 });
  assert.equal(required(at(2).get('first')).mode, 'talk');
  assert.equal(required(at(2.2, false, [2, 2, 2]).get('first')).mode, 'talk');
});
