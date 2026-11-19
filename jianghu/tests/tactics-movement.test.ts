import assert from 'node:assert/strict';
import test from 'node:test';
import { createTacticsMap, type TacticsTile } from '../src/tactics-movement.ts';
import { rainbellGarden, rainbellWorld } from '../src/rainbell-world.ts';
import { required } from './required.ts';

function at(tiles: readonly TacticsTile[], x: number, z: number, surface?: string) {
  return required(tiles.find((tile) => Math.abs(tile.x - x) < 0.01 && Math.abs(tile.z - z) < 0.01 &&
    (!surface || tile.surface === surface)));
}

test('movement uses the shortest four-way walk and cannot cross an occupied tile or a diagonal corner', () => {
  const map = createTacticsMap({
    surfaces: [
      { id: 'plaza', x: 1.5, y: 0, z: 1.5, width: 3, depth: 3 },
      { id: 'corner', x: 3.5, y: 0, z: 3.5, width: 1, depth: 1 },
    ],
    blockers: [{ surface: 'plaza', x: 1.5, z: 0.5, width: 1, depth: 1 }],
    stairs: [],
  });
  const start = at(map.tiles, 0.5, 0.5);
  const around = at(map.tiles, 2.5, 0.5);
  assert.equal(map.tiles.some(({ x, z }) => x === 1.5 && z === 0.5), false);
  assert.deepEqual([...map.reachable(start, 0)], [[start, 0]]);
  assert.equal(map.reachable(start, 3).has(around), false);
  assert.equal(map.reachable(start, 4).get(around), 4);
  assert.equal(map.reachable(start, 9).has(at(map.tiles, 3.5, 3.5)), false);
  assert.equal(map.reachable({ ...start }, 4).size, 0);
});

test('overlapping bridge ends connect same-level landings without duplicate tiles or drops', () => {
  const map = createTacticsMap({
    surfaces: [
      { id: 'west-bank', x: 1.5, y: 2, z: 1.5, width: 3, depth: 3 },
      { id: 'east-bank', x: 7, y: 2, z: 1.5, width: 3, depth: 3 },
      { id: 'bridge', x: 4.25, y: 2, z: 1.5, width: 4, depth: 1 },
      { id: 'below', x: 4.5, y: 1, z: 1.5, width: 3, depth: 1 },
    ],
    blockers: [], stairs: [],
  });
  const start = at(map.tiles, 2.5, 1.5);
  const reached = map.reachable(start, 7);
  assert.ok(reached.has(at(map.tiles, 6, 1.5)));
  assert.equal(reached.has(at(map.tiles, 4.5, 1.5, 'below')), false);
  assert.equal(map.tiles.filter(({ x, y, z }) => y === 2 && x === 2.5 && z === 1.5).length, 1);
});

test('short stairs join their two landings without allowing movement from the exposed sides', () => {
  const map = createTacticsMap({
    surfaces: [
      { id: 'plaza', x: 2.5, y: 0, z: 1.5, width: 3, depth: 3 },
      { id: 'garden', x: -1, y: 1, z: 1.5, width: 2, depth: 3 },
      { id: 'shelf', x: 0.5, y: 0.25, z: 3.5, width: 1, depth: 3 },
    ],
    stairs: [{ id: 'stairs', from: 'plaza', to: 'garden', x: 2, z: 1.5, run: 2, width: 1, direction: 'west', steps: 8 }],
    blockers: [],
  });
  const start = at(map.tiles, 2.5, 1.5);
  const garden = at(map.tiles, -0.5, 1.5);
  assert.equal(map.tiles.some(({ x, z, y }) => x === 1.5 && z === 1.5 && y === 0), false);
  assert.equal(map.tiles.filter((tile) => tile.surface === 'stairs').length, 8);
  assert.equal(map.reachable(start, 2.9).has(garden), false);
  assert.equal(map.reachable(start, 3).get(garden), 3);
  assert.equal(map.reachable(garden, 3).get(start), 3);
  assert.equal(map.reachable(start, 8).has(at(map.tiles, 0.5, 2.5)), false);
});

test('north stairs land at the rendered tread heights and honor a partial obstacle footprint', () => {
  const map = createTacticsMap({
    surfaces: [
      { id: 'square', x: 1.5, y: 0.55, z: 4, width: 3, depth: 2 },
      { id: 'landing', x: 1.5, y: 0.91, z: 1, width: 3, depth: 2 },
    ],
    stairs: [{ id: 'stairs', from: 'square', to: 'landing', x: 1.5, z: 4, run: 2, width: 1, direction: 'north', steps: 2 }],
    blockers: [{ surface: 'landing', x: 1.75, z: 1.5, width: 0.5, depth: 1 }],
  });
  const low = at(map.tiles, 1.5, 3.5, 'stairs');
  const high = at(map.tiles, 1.5, 2.5, 'stairs');
  const clear = at(map.tiles, 1.25, 1.5, 'landing');
  assert.equal(low.y, 0.73);
  assert.equal(high.y, 0.91);
  assert.equal(clear.width, 0.5);
  assert.equal(map.reachable(at(map.tiles, 1.5, 4.5), 3).get(clear), 3);
});

test('a stair names its missing landing in the map error', () => {
  assert.throws(() => createTacticsMap({
    surfaces: [{ id: 'square', x: 0, y: 0, z: 0, width: 4, depth: 4 }],
    stairs: [{ id: 'lookout-stairs', from: 'square', to: 'lookout', x: 0, z: 2, run: 2, width: 1, direction: 'north', steps: 2 }],
    blockers: [],
  }), /Stair "lookout-stairs" references a missing landing/);
});


test('a surface uses its own paving grid and connects to a neighboring offset grid', () => {
  const map = createTacticsMap({
    surfaces: [
      { id: 'market', x: 3.25, y: 0, z: 4.35, width: 5, depth: 2.5 },
      { id: 'causeway', x: 6.95, y: 0, z: 4.1, width: 2.4, depth: 1.2, columns: 3, rows: 1 },
    ],
    stairs: [],
    blockers: [],
  });
  const market = map.tiles.filter(({ surface }) => surface === 'market');
  const causeway = map.tiles.filter(({ surface }) => surface === 'causeway');
  assert.equal(market.length, 8);
  assert.equal(causeway.length, 3);
  const start = at(map.tiles, 5.125, 3.725, 'market');
  assert.equal(start.width, 1.25);
  assert.equal(start.depth, 1.25);
  const first = at(map.tiles, 6.15, 4.1, 'causeway');
  const last = at(map.tiles, 7.75, 4.1, 'causeway');
  assert.equal(map.reachable(start, 1).get(first), 1);
  assert.equal(map.reachable(start, 3).get(last), 3);
});

test('contiguous fragments around an obstacle share a single move and keep an unobstructed spawn point', () => {
  const map = createTacticsMap({
    surfaces: [{ id: 'street', x: 1.5, y: 0, z: 0.5, width: 3, depth: 1 }],
    stairs: [],
    blockers: [{ surface: 'street', x: 1.5, z: 0.5, width: 0.4, depth: 0.4 }],
  });
  assert.equal(map.tiles.length, 3);
  const start = at(map.tiles, 0.5, 0.5);
  const finish = at(map.tiles, 2.5, 0.5);
  const middle = required(map.tiles.find((tile) => tile.parts));
  assert.equal(middle.cell?.x, 1.5);
  assert.equal(middle.cell?.width, 1);
  assert.equal(middle.parts?.length, 4);
  assert.ok(Math.abs(middle.x - 1.5) >= 0.2 || Math.abs(middle.z - 0.5) >= 0.2);
  assert.equal(map.reachable(start, 1).get(middle), 1);
  assert.equal(map.reachable(start, 2).get(finish), 2);
});

test('a full-width blocker keeps fragments of the same paving cell disconnected', () => {
  const map = createTacticsMap({
    surfaces: [{ id: 'street', x: 1.5, y: 0, z: 0.5, width: 3, depth: 1 }],
    stairs: [],
    blockers: [{ surface: 'street', x: 1.5, z: 0.5, width: 0.2, depth: 1 }],
  });
  assert.equal(map.tiles.filter(({ cell }) => cell?.x === 1.5).length, 2);
  const start = at(map.tiles, 0.5, 0.5);
  const finish = at(map.tiles, 2.5, 0.5);
  assert.equal(map.reachable(start, 10).has(finish), false);
});

test('a planned turn detours inside a clipped tile without spending another movement point', () => {
  const map = createTacticsMap({
    surfaces: [{ id: 'street', x: 1.5, y: 0, z: 0.5, width: 3, depth: 1 }],
    stairs: [],
    blockers: [{ surface: 'street', x: 1.5, z: 0.5, width: 0.4, depth: 0.4 }],
  });
  const start = at(map.tiles, 0.5, 0.5);
  const finish = at(map.tiles, 2.5, 0.5);
  const plan = map.plan(start, 2);
  const path = required(plan.pathTo(finish));
  assert.deepEqual(plan.reachable, map.reachable(start, 2));
  assert.equal(plan.reachable.get(finish), 2);
  assert.deepEqual(path[0], { x: 0.5, y: 0, z: 0.5 });
  assert.deepEqual(path.at(-1), { x: 2.5, y: 0, z: 0.5 });
  assert.deepEqual(plan.pathTo(start), [path[0]]);
  assert.equal(map.plan(start, 1).pathTo(finish), undefined);
  assert.ok(path.some(({ z }) => z < 0.3 || z > 0.7));
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1];
    const b = path[i];
    for (let step = 0; step <= 20; step++) {
      const x = a.x + (b.x - a.x) * step / 20;
      const z = a.z + (b.z - a.z) * step / 20;
      assert.ok(!(1.300001 < x && x < 1.699999 && 0.300001 < z && z < 0.699999));
    }
  }
});

test('a route up the Rainbell garden stairs lifts vertically at each tread edge', () => {
  const map = createTacticsMap(rainbellWorld);
  const start = required(map.tiles.find(({ surface, x, z }) =>
    surface === 'square' && Math.abs(x + 24.5) < 0.8 && Math.abs(z - 46.14) < 0.7));
  const plan = map.plan(start, rainbellWorld.movement);
  const destination = required([...plan.reachable.keys()].find(({ surface }) => surface === 'garden'));
  const path = required(plan.pathTo(destination));
  let rises = 0;
  for (let i = 1; i < path.length; i++) {
    const a = path[i - 1];
    const b = path[i];
    assert.ok(b.y >= a.y);
    if (b.y === a.y) continue;
    assert.equal(a.x, b.x);
    assert.equal(a.z, b.z);
    rises++;
  }
  assert.equal(path[0].y, start.y);
  assert.equal(path.at(-1)?.y, rainbellGarden.deck.y);
  assert.equal(rises, rainbellGarden.steps.steps);
});
