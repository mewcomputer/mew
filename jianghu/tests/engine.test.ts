import test from 'node:test';
import assert from 'node:assert/strict';
import { createWorld, meshChunk } from '../src/world.ts';
import { createWaterSimulation } from '../src/fluid.ts';
import { createDistrict } from '../src/district.ts';

const tint = Array.from({ length: 16 }, () => ({ r: 1, g: 1, b: 1 }));

test('chunk mesh hides shared faces and an edit invalidates both sides of the boundary', () => {
  const world = createWorld({ size: [4, 2, 4], origin: [0, 0, 0], cell: 1, chunkSize: 2 });
  world.set(1, 0, 1, 1);
  const affected = world.set(2, 0, 1, 1);
  assert.deepEqual(affected, [
    [0, 0],
    [0, 1],
    [1, 0],
    [1, 1],
  ]);
  assert.equal(meshChunk(world, 0, 0, tint).indices.length, 30);
  assert.equal(meshChunk(world, 1, 0, tint).indices.length, 30);
  world.set(2, 0, 1, 0);
  assert.equal(meshChunk(world, 0, 0, tint).indices.length, 36);
  assert.equal(meshChunk(world, 1, 0, tint).indices.length, 0);
  assert.equal(world.get(-1, 0, 0), 0);
  assert.throws(() => world.set(4, 0, 0, 1), RangeError);
});

type Water = ReturnType<typeof createWaterSimulation>;

const run = (water: Water, seconds: number, external = true) => {
  for (let i = 0; i < Math.round(seconds / water.dt); i++) water.step(external);
};
const volume = (water: Water) => water.depth.reduce((sum, value) => sum + value, 0);

test('internal water transfers conserve volume and prevent negative depth', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  const initial = volume(water);
  run(water, 9, false);
  assert.ok(Math.abs(volume(water) - initial) < 1e-8);
  assert.ok(Math.min(...water.depth) >= -1e-10);
});

test('sluice closure backs up the canal and reopening restores the spillway', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  run(water, 13);
  const initial = water.measure();
  water.setGate(true);
  run(water, 10);
  const closed = water.measure();
  assert.ok(closed.upstream > initial.upstream + 0.2);
  assert.ok(closed.discharge < initial.discharge * 0.5);
  water.setGate(false);
  run(water, 12);
  assert.ok(water.measure().discharge > initial.discharge * 0.8);
});

test('voxel edits update the fluid bed, displace water, and change discharge', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  run(water, 13);
  const initial = water.measure().discharge;
  const beforeEdit = volume(water);
  const z = 35;
  for (let x = hydraulics.channelStart; x < hydraulics.channelEnd; x++) {
    for (let y = 4; y <= 12; y++) world.set(x, y, z, 15);
  }
  assert.equal(volume(water), beforeEdit);
  assert.equal(water.bed[water.index(hydraulics.channelStart, z)], world.origin[1] + 13 * world.cell);
  run(water, 8);
  assert.ok(water.measure().discharge < initial * 0.55);
});

test('rendering can read current speed in flowing, blocked, and dry columns', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  const x = hydraulics.channelStart + 1;
  const upstream = hydraulics.gateEdge - 1;
  run(water, 13);
  const openSpeed = water.speedAt(x, upstream);
  assert.ok(openSpeed > 0.2);
  assert.ok(water.speedAt(x, water.length - 1) > 0.2);
  assert.equal(water.speedAt(0, 0), 0);

  water.setGate(true);
  run(water, 10);
  assert.ok(water.speedAt(x, upstream) < openSpeed * 0.1);
});

test('a throwing edit observer cannot prevent the water bed or later observers from updating', (t) => {
  const world = createWorld({ size: [3, 4, 3], origin: [0, 0, 0], cell: 0.5 });
  const errors = t.mock.method(console, 'error', () => {});
  world.onChange(() => {
    throw new Error('broken observer');
  });
  const water = createWaterSimulation(world, {
    channelStart: 0,
    channelEnd: 2,
    gateEdge: 1,
    sill: 0.55,
    inletLevel: 0.5,
  });
  assert.deepEqual(world.set(0, 0, 0, 1), [[0, 0]]);
  assert.equal(water.bed[water.index(0, 0)], 0.5);
  assert.equal(errors.mock.callCount(), 1);
  const error: unknown = errors.mock.calls[0].arguments[1];
  assert.ok(error instanceof Error);
  assert.equal(error.message, 'broken observer');
  water.dispose();
});

test('disposing water twice releases its subscription to the world', () => {
  const { world, hydraulics } = createDistrict();
  const water = createWaterSimulation(world, hydraulics);
  const x = hydraulics.channelStart;
  const before = water.bed[water.index(x, 0)];
  water.dispose();
  water.dispose();
  assert.ok(world.set(x, 16, 0, 1).length);
  assert.equal(water.bed[water.index(x, 0)], before);
});
