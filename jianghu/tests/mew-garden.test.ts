import assert from 'node:assert/strict';
import test from 'node:test';
import { gardenCast, gardenStops, mewGarden } from '../src/mew-garden.ts';
import { createCharacterTrack } from '../src/city-characters.ts';
import { createTacticsMap } from '../src/tactics-movement.ts';

test('every part of the garden connects to the hall without opening a route through the moat', () => {
  const map = createTacticsMap(mewGarden);
  const court = map.tiles.find(({ surface }) => surface === 'court')!;
  const reachable = map.reachable(court, 150);
  for (const { id } of mewGarden.surfaces) {
    assert.ok([...reachable.keys()].some(({ surface }) => surface === id), id);
  }
  assert.ok(!map.tiles.some(({ x, z }) => x > 8 && x < 10.5 && z > 4 && z < 8));
  assert.ok(!map.tiles.some(({ x, z }) => Math.abs(x) < 2 && z > -4 && z < -1));
});

test('seven figures have connected routes and the garden has six landmark views', () => {
  const cast = gardenCast();
  assert.equal(cast.length, 7);
  for (const actor of cast) {
    assert.ok(actor.path.length > 2, actor.id);
    const sample = createCharacterTrack(actor);
    for (let time = 0; time < 50; time += 0.25) {
      assert.ok(sample(time).position.every(Number.isFinite), actor.id);
    }
  }
  assert.ok(cast.some(({ path }) => new Set(path.map(([, y]) => y)).size > 3));
  assert.equal(new Set(cast.map(({ id }) => id)).size, cast.length);
  assert.deepEqual(Object.keys(gardenStops), ['garden', 'hall', 'pavilion', 'gallery', 'wall', 'gate', 'tower']);
});

test('the forecourt and tower plinth are reached by modeled stairs', () => {
  const map = createTacticsMap(mewGarden);
  const low = map.tiles.find(({ surface }) => surface === 'south')!;
  const high = map.tiles.find(({ surface }) => surface === 'tower-landing')!;
  const path = map.plan(low, 150).pathTo(high)!;
  assert.ok(high.y > low.y + 2);
  assert.ok(path.some(({ y }) => y > low.y && y < high.y));
  assert.ok(path.some((point, i) => i > 0 &&
    point.x === path[i - 1].x && point.z === path[i - 1].z && point.y !== path[i - 1].y));
});
