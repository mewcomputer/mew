import assert from 'node:assert/strict';
import test from 'node:test';
import { required } from './required.ts';
import { baichuanCast } from '../src/city-characters.ts';
import { createCityEncounters } from '../src/city-encounters.ts';

test('the disciples alternate forms and guard on the same practice clock', () => {
  const at = createCityEncounters(baichuanCast);
  const { actors } = at(2);
  const ye = required(actors.get('ye-xia'));
  const ning = required(actors.get('ning-fei'));
  assert.equal(ye.mode, 'work');
  assert.equal(ning.mode, 'guard');
  at(3);
  assert.equal(ye.mode, 'guard');
  assert.equal(ning.mode, 'work');
  assert.ok(ning.position[0] - ye.position[0] >= 1.5);
  assert.equal(ye.position[2], ning.position[2]);
  at(6.5);
  assert.equal(ye.mode, 'idle');
  assert.equal(ning.mode, 'idle');
});

test('the gauge lights only after the ferrykeeper arrives and works the control', () => {
  const at = createCityEncounters(baichuanCast);
  const walking = at(3);
  const keeper = required(walking.actors.get('xu-chun'));
  assert.equal(keeper.mode, 'walk');
  assert.equal(walking.gaugeLit, false);
  assert.equal(at(4.6).gaugeLit, false);
  assert.equal(keeper.mode, 'work');
  assert.equal(at(5.4).gaugeLit, true);
  assert.equal(keeper.mode, 'work');
  assert.equal(at(10).gaugeLit, false);
});
