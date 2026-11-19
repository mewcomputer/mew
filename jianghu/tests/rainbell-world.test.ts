import assert from 'node:assert/strict';
import test from 'node:test';
import { shopFootprint } from '../src/city-shops.ts';
import { operaHouse, rainbellProps, rainbellWorld } from '../src/rainbell-world.ts';
import { createTacticsMap, type TacticsTile } from '../src/tactics-movement.ts';
import { required } from './required.ts';

test('the bridge landing leads to the opera quay and the entrance steps climb to a clear porch', () => {
  const map = createTacticsMap(rainbellWorld);
  const { quay, porch, hall, steps } = operaHouse;
  const landing = required(map.tiles.find(({ surface, x, z }) =>
    surface === 'east-landing' && Math.abs(x - 11) < 1 && Math.abs(z - 41) < 0.5));
  const reached = map.reachable(landing, rainbellWorld.movement);
  assert.ok([...reached.keys()].some(({ surface }) => surface === quay.id));

  const approach = required(map.tiles.find(({ surface, x, z }) =>
    surface === quay.id && Math.abs(x - steps.x) < 0.85 && z > steps.z));
  assert.ok([...map.reachable(approach, rainbellWorld.movement).keys()].some(({ surface }) => surface === porch.id));
  assert.ok(!map.tiles.some(({ surface, x, z }) => surface === quay.id &&
    Math.abs(x - hall.x) < hall.width / 2 && Math.abs(z - hall.z) < hall.depth / 2));
});

test('the opera forecourt has an open center and a connected route around the canal side', () => {
  const map = createTacticsMap(rainbellWorld);
  const { quay, court, steps } = operaHouse;
  const nearest = (surface: string, x: number, z: number) => required(map.tiles
    .filter((tile) => tile.surface === surface)
    .sort((a, b) => Math.hypot(a.x - x, a.z - z) - Math.hypot(b.x - x, b.z - z))[0]);
  const promenade = nearest(quay.id, 15, 54);
  const center = nearest(court.id, court.x, court.z);
  const reachable = map.reachable(promenade, 40);
  assert.ok(reachable.has(center));
  const local = map.reachable(center, rainbellWorld.movement);
  assert.ok([...local.keys()].some((tile) => tile.surface === court.id && tile.x > court.x + 3));
  assert.ok([...local.keys()].some((tile) => tile.surface === court.id && tile.x < court.x - 3));
  assert.ok(map.reachable(center, 20).has(nearest(quay.id, steps.x, steps.z + 0.4)));
});

test('the garden behind the opera house gives the bridge landing another route to the quay', () => {
  const map = createTacticsMap(rainbellWorld);
  const { rearCourt, rearPassage, garden, pool, quay, hall } = operaHouse;
  const passage = required(map.tiles.find((tile) => tile.surface === rearPassage.id && tile.z > 39));
  const reachable = map.reachable(passage, 18);
  assert.ok([...reachable.keys()].some((tile) => tile.surface === 'east-landing'));
  assert.ok([...reachable.keys()].some((tile) => tile.surface === quay.id));
  assert.ok([...reachable.keys()].some((tile) => tile.surface === rearCourt.id && tile.x > hall.x));
  assert.ok([...map.reachable(passage, 40).keys()].some((tile) => tile.surface === garden.id));
  assert.ok(garden.x - garden.width / 2 > hall.x + hall.width / 2 + 5);
  assert.ok(!map.tiles.some((tile) => tile.surface === garden.id &&
    Math.abs(tile.x - pool.x) < pool.width / 2 && Math.abs(tile.z - pool.z) < pool.depth / 2));
});

test('the ticket booth and gallery leave the east-side walk connected to the garden', () => {
  const map = createTacticsMap(rainbellWorld);
  const { quay, garden, ticketBooth, gallery } = operaHouse;
  const approach = required(map.tiles.find((tile) => tile.surface === quay.id &&
    tile.x > 35 && tile.x < 37 && tile.z > 58));
  assert.ok([...map.reachable(approach, 24).keys()].some((tile) => tile.surface === garden.id));
  for (const prop of [ticketBooth, gallery]) {
    assert.ok(!map.tiles.some((tile) => tile.surface === prop.surface &&
      Math.abs(tile.x - prop.x) < prop.width / 2 && Math.abs(tile.z - prop.z) < prop.depth / 2));
  }
});

test('a Rainbell turn climbs to the garden and stops at the pagoda footprint', () => {
  const map = createTacticsMap(rainbellWorld);
  const start = required(map.tiles.find(({ surface, x, z }) =>
    surface === 'square' && Math.abs(x + 24.5) < 0.8 && Math.abs(z - 46.14) < 0.7));
  const reached = map.reachable(start, rainbellWorld.movement);
  assert.ok([...reached.keys()].some(({ surface }) => surface === 'garden'));
  assert.ok(!map.tiles.some(({ surface, x, z }) =>
    surface === 'garden' && Math.abs(x + 33.17) < 1.7 && Math.abs(z - 50.32) < 1.7));
  assert.ok(!map.tiles.some(({ surface, x, z }) =>
    surface === 'square' && Math.hypot(x + 17.6, z - 49.45) < 1));
});

test('a Rainbell turn reaches the canal bridge from its landing within the movement budget', () => {
  const map = createTacticsMap(rainbellWorld);
  const start = required(map.tiles.find(({ surface, x, z }) =>
    surface === 'west-landing' && Math.abs(x + 9.5) < 1 && Math.abs(z - 41.55) < 0.4));
  const reached = map.reachable(start, rainbellWorld.movement);
  assert.ok([...reached.keys()].some(({ surface }) => surface === 'canal-bridge'));
  assert.ok(![...reached.keys()].some(({ surface }) => surface === 'east-landing'));
  assert.ok([...map.reachable(start, 40).keys()].some(({ surface }) => surface === 'east-landing'));
});

test('different storefronts share an authored base footprint and leave the lane between them open', () => {
  const map = createTacticsMap(rainbellWorld);
  const squareTiles: readonly TacticsTile[] = map.tiles.filter(({ surface }) => surface === 'square');
  const variants = new Set(rainbellProps.shops.map(({ variant }) => variant));
  assert.equal(variants.size, 2);

  for (const shop of rainbellProps.shops) {
    const base = shopFootprint(shop);
    assert.ok(rainbellWorld.blockers.some((blocker) =>
      blocker.surface === 'square' && blocker.x === shop.x && blocker.z === shop.z &&
      blocker.width === base.width && blocker.depth === base.depth));
    for (const tile of squareTiles) {
      for (const part of tile.parts ?? [tile]) {
        const xOverlap = Math.min(part.x + part.width / 2, shop.x + base.width / 2) -
          Math.max(part.x - part.width / 2, shop.x - base.width / 2);
        const zOverlap = Math.min(part.z + part.depth / 2, shop.z + base.depth / 2) -
          Math.max(part.z - part.depth / 2, shop.z - base.depth / 2);
        assert.ok(xOverlap < 1e-6 || zOverlap < 1e-6);
      }
    }
  }

  assert.ok(squareTiles.some(({ x, z }) => x > -24 && x < -22.65 && z > 48.5 && z < 50));
});

test('the planted canal edge leaves a walking lane beside the reeds', () => {
  const map = createTacticsMap(rainbellWorld);
  const promenade = map.tiles.filter(({ surface, x }) => surface === 'square' && x > -9.85 && x < -9.1);
  const start = required(promenade.find(({ z }) => z > 47 && z < 48.6));
  const reached = map.reachable(start, rainbellWorld.movement);
  assert.ok(promenade.some((tile) => tile.z > 50.5 && tile.z < 52 && reached.has(tile)));
  for (const { x, z, width, depth } of rainbellProps.reedBeds) {
    assert.ok(rainbellWorld.blockers.some((p) =>
      p.surface === 'square' && p.x === x && p.z === z && p.width === width && p.depth === depth));
    assert.ok(!map.tiles.some((tile) => tile.surface === 'square' && Math.abs(tile.x - x) < width / 2 &&
      Math.abs(tile.z - z) < depth / 2));
  }
});

test('the bamboo planter stays clear of the routes around Rainbell Pavilion', () => {
  const map = createTacticsMap(rainbellWorld);
  const [pot] = rainbellProps.bambooPlanters;
  const square = map.tiles.filter((tile) => tile.surface === 'square');
  assert.ok(!square.some(({ x, z }) =>
    Math.abs(x - pot.x) < pot.width / 2 && Math.abs(z - pot.z) < pot.depth / 2));
  const west = required(square.find(({ x, z }) => x < pot.x - 0.6 && x > pot.x - 2.5 &&
    Math.abs(z - pot.z) < 0.65));
  assert.ok([...map.reachable(west, rainbellWorld.movement).keys()].some(({ x, z }) =>
    x > pot.x + 0.65 && Math.abs(z - pot.z) < 1));
});
