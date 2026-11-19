import assert from 'node:assert/strict';
import test from 'node:test';
import { baichuanCast, createCharacterTrack } from '../src/city-characters.ts';
import {
  createCityEncounters, rainbellPeriod, rainbellSignal, rainbellSignalStart, rainbellStrikeBetween, rainbellSwing,
} from '../src/city-encounters.ts';
import { rainbellBridge, rainbellDecks, rainbellGarden, rainbellSite, rainbellSteps } from '../src/rainbell-world.ts';
import { createStreetLife } from '../src/street-life.ts';
import { required } from './required.ts';

test('the courier can get from the east quay to Rainbell Square along supported ground', () => {
  const courier = required(baichuanCast.find(({ id }) => id === 'yu-lin'));
  const sample = createCharacterTrack(courier);
  let crossed = false;
  let reachedSquare = false;
  for (let tick = 0; tick < 550; tick += 1) {
    const [x, y, z] = sample(tick / 20).position;
    const bridge = x >= rainbellBridge.west && x <= rainbellBridge.east &&
      Math.abs(z - rainbellBridge.z) <= rainbellBridge.width / 2 && Math.abs(y - rainbellBridge.y) < 0.02;
    const deck = rainbellDecks.some(({ x: cx, y: cy, z: cz, width, depth }) =>
      Math.abs(x - cx) <= width / 2 && Math.abs(z - cz) <= depth / 2 && Math.abs(y - cy) < 0.02);
    const stairs = Math.abs(x - rainbellSteps.x) <= rainbellSteps.width / 2 &&
      z >= rainbellSteps.z - rainbellSteps.run && z <= rainbellSteps.z &&
      y >= rainbellSite[1] && y <= rainbellBridge.y;
    const existingQuay = x >= 6.56 && x <= 22.75 && z >= 24.325 && z <= 37.895 && Math.abs(y - 0.91) < 0.02;
    assert.ok(bridge || deck || stairs || existingQuay, `unsupported route position ${x},${y},${z}`);
    crossed ||= bridge && Math.abs(x) < 3;
    reachedSquare ||= deck && z > rainbellSteps.z;
  }
  assert.ok(crossed && reachedSquare);
});

test('the rain bell signals during the steward’s operation', () => {
  const sample = createCityEncounters(baichuanCast);
  const arrival = required(sample(rainbellSignalStart - 2).actors.get('pei-shu'));
  assert.equal(arrival.mode, 'walk');
  const waiting = required(sample(rainbellSignalStart - 0.25).actors.get('pei-shu'));
  assert.equal(waiting.mode, 'idle');
  assert.equal(rainbellSignal(rainbellSignalStart - 0.25), false);
  const ringing = required(sample(rainbellSignalStart + 0.35).actors.get('pei-shu'));
  assert.equal(ringing.mode, 'work');
  assert.equal(rainbellSignal(rainbellSignalStart + 0.35), true);
  assert.equal(rainbellSignal(rainbellSignalStart + 5), false);
});

test('the garden visitor walks up from the square and reaches the pagoda lookout', () => {
  const visitor = required(baichuanCast.find(({ id }) => id === 'zhao-lan'));
  const sample = createCharacterTrack(visitor);
  const { deck, steps, pathX, lookoutZ } = rainbellGarden;
  let climbed = false;
  let reachedLookout = false;
  let returned = false;
  for (let tick = 0; tick <= 900; tick++) {
    const [x, y, z] = sample(tick / 20).position;
    const square = rainbellDecks.some(({ x: cx, y: cy, z: cz, width, depth }) =>
      Math.abs(x - cx) <= width / 2 && Math.abs(z - cz) <= depth / 2 && Math.abs(y - cy) < 0.01);
    const stair = x <= steps.x && x >= steps.x - steps.run - 0.01 &&
      Math.abs(z - steps.z) <= steps.width / 2 && y >= rainbellSite[1] && y <= deck.y;
    const garden = Math.abs(x - deck.x) <= deck.width / 2 && Math.abs(z - deck.z) <= deck.depth / 2 &&
      Math.abs(y - deck.y) < 0.01;
    assert.ok(square || stair || garden, `unsupported garden route position ${x},${y},${z}`);
    climbed ||= stair && y > rainbellSite[1] + 0.2 && y < deck.y - 0.2;
    reachedLookout ||= climbed && garden && Math.hypot(x - pathX, z - lookoutZ) < 0.1;
    returned ||= reachedLookout && square && x > -24;
  }
  assert.ok(climbed && reachedLookout && returned);
});

test('the bell sits in the middle of the square and strikes once per cycle', () => {
  const square = required(rainbellDecks.find(({ y }) => y === rainbellSite[1]));
  assert.ok(Math.hypot(rainbellSite[0] - square.x, rainbellSite[2] - square.z) < 0.5);
  for (const cycle of [0, 1, 7]) {
    const strike = rainbellPeriod * cycle + rainbellSignalStart;
    assert.equal(rainbellStrikeBetween(strike - 0.01, strike), true);
    assert.equal(rainbellStrikeBetween(strike, strike + 0.01), false);
    assert.equal(rainbellStrikeBetween(strike, strike), false);
    assert.equal(rainbellSwing(strike - 2), 0);
    assert.ok(rainbellSwing(strike - 0.3) < 0, 'pulls back before impact');
    assert.ok(rainbellSwing(strike) > rainbellSwing(strike + 0.2), 'rebounds after impact');
  }
});

test('Rainbell Square has a walker and a worker who look up at the first toll', () => {
  const at = createStreetLife(baichuanCast.map((character) => ({
    id: character.id, bellAttention: character.bellAttention, sample: createCharacterTrack(character),
  })));
  const before = at(rainbellSignalStart - 0.05);
  assert.equal(required(before.get('song-hui')).mode, 'walk');
  assert.equal(required(before.get('qin-zhu')).mode, 'work');
  const after = at(rainbellSignalStart, false, rainbellSite);
  assert.equal(required(after.get('song-hui')).mode, 'watch');
  assert.equal(required(after.get('qin-zhu')).mode, 'watch');
  assert.equal(required(after.get('zhao-lan')).mode, 'walk');
  at(rainbellSignalStart + 2.25);
  assert.equal(required(after.get('song-hui')).mode, 'walk');
  assert.equal(required(after.get('qin-zhu')).mode, 'work');
});
