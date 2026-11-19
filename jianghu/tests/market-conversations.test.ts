import assert from 'node:assert/strict';
import test from 'node:test';
import { required } from './required.ts';
import { baichuanCast, createCharacterTrack, type CityPosition } from '../src/city-characters.ts';
import { createStreetLife } from '../src/street-life.ts';

function walker(id: string, from: CityPosition, to: CityPosition) {
  return { id, market: true, sample: createCharacterTrack({ path: [from, to] }) };
}

test('a chance meeting freezes both route clocks and resumes from the same positions', () => {
  const at = createStreetLife([
    walker('lin', [-4, 3, 0], [4, 3, 0]),
    walker('he', [2, 3, 0], [-4, 3, 0]),
  ], { random: () => 0 });

  assert.equal(required(at(2).get('lin')).mode, 'walk');
  const market = at(3.5);
  assert.equal(required(market.get('lin')).mode, 'talk');
  assert.equal(required(market.get('he')).mode, 'idle');
  assert.deepEqual(required(market.get('lin')).direction, [1, 0]);
  assert.deepEqual(required(market.get('he')).direction, [-1, 0]);
  const stopped = [...required(market.get('lin')).position];
  at(4.8);
  assert.equal(required(market.get('he')).mode, 'talk');
  assert.deepEqual(required(market.get('lin')).position, stopped);

  at(5.9);
  assert.equal(required(market.get('lin')).mode, 'walk');
  assert.deepEqual(required(market.get('lin')).position, stopped);
  at(6.4);
  assert.deepEqual(required(market.get('lin')).position, [-1, 3, 0]);
  assert.equal(required(market.get('he')).mode, 'walk');
});

test('a declined pass only rolls once while the characters remain close', () => {
  let rolls = 0;
  const at = createStreetLife([
    walker('lin', [-4, 3, 0], [4, 3, 0]),
    walker('he', [2, 3, 0], [-4, 3, 0]),
  ], { random: () => { rolls++; return 0.99; } });
  for (const time of [2, 3.3, 3.5, 3.8, 4, 4.4]) {
    assert.notEqual(required(at(time).get('lin')).mode, 'talk');
  }
  assert.equal(rolls, 1);
});

test('passersby on different floors keep walking without rolling for a chat', () => {
  const at = createStreetLife([
    walker('lin', [-4, 3, 0], [4, 3, 0]),
    walker('he', [2, 5, 0], [-4, 5, 0]),
  ], { random: () => assert.fail('different floors are out of range') });
  at(2);
  assert.equal(required(at(3.5).get('lin')).mode, 'walk');
});

test('a vendor resumes the same work sequence and cannot join two conversations at once', () => {
  const vendor = { id: 'vendor', market: true, sample: createCharacterTrack({
    path: [[0, 3, 0]], activity: 'work',
  }) };
  const at = createStreetLife([
    walker('lin', [-3, 3, 0], [4, 3, 0]),
    vendor,
    walker('he', [4, 3, 0], [-4, 3, 0]),
  ], { random: () => 0 });
  const market = at(2.6);
  assert.equal(required(market.get('lin')).mode, 'talk');
  assert.equal(required(market.get('vendor')).mode, 'idle');
  at(3.0);
  assert.equal(required(market.get('he')).mode, 'walk');
  at(5);
  assert.equal(required(market.get('vendor')).mode, 'work');
  assert.equal(required(market.get('vendor')).ticks, 45);
});

test('Baichuan’s authored routes produce encounters between more than one pair', () => {
  const actors = baichuanCast.filter(({ market }) => market);
  const at = createStreetLife(actors.map((actor) => ({
    id: actor.id, market: true, sample: createCharacterTrack(actor),
  })), { random: () => 0 });
  const speakers = new Set();
  for (let tick = 0; tick <= 400; tick++) {
    for (const [id, pose] of at(tick / 20)) {
      if (pose.mode === 'talk') speakers.add(id);
    }
  }
  assert.ok(speakers.size >= 3);
});
