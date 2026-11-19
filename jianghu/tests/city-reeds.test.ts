import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { mountCityReeds } from '../src/city-reeds.ts';

const bank = { x: -8, y: 0.55, z: 48, width: 0.9, depth: 1.8, height: 1.95, seed: 311 };

function reeds(patches: Parameters<typeof mountCityReeds>[2]) {
  const scene = new T.Scene();
  mountCityReeds(T, scene, patches);
  assert.equal(scene.children.length, 1);
  const mesh = scene.children[0];
  assert.ok(mesh instanceof T.Mesh);
  return mesh;
}

test('reed patches batch deterministic geometry rooted at their authored heights', () => {
  const patches = [bank, { ...bank, x: 4, y: 1.2, seed: 171 }];
  const mesh = reeds(patches);
  const position = mesh.geometry.getAttribute('position');
  const colors = mesh.geometry.getAttribute('color');
  assert.ok(position.count > 400);
  assert.equal(colors.count, position.count);
  assert.deepEqual(position.array, reeds(patches).geometry.getAttribute('position').array);
  assert.notDeepEqual(position.array,
    reeds([{ ...bank, seed: 501 }, patches[1]]).geometry.getAttribute('position').array);
  mesh.geometry.computeBoundingBox();
  const box = mesh.geometry.boundingBox!;
  assert.ok(Math.abs(box.min.y - bank.y) < 1e-5);
  assert.ok(Math.abs(box.max.y - (patches[1].y + bank.height)) < 1e-5);
  assert.ok(box.min.x >= bank.x - bank.width / 2 - 0.121);
  assert.ok(box.max.x <= patches[1].x + bank.width / 2 + 0.121);
  assert.ok(box.min.z >= bank.z - bank.depth / 2 - 0.121);
  assert.ok(box.max.z <= bank.z + bank.depth / 2 + 0.121);
});

test('reeds write depth from both sides and leave open space between the blades', () => {
  const mesh = reeds([bank]);
  assert.ok(mesh.material instanceof T.MeshStandardMaterial);
  assert.equal(mesh.material.side, T.DoubleSide);
  assert.equal(mesh.material.depthWrite, true);
  assert.equal(mesh.material.transparent, false);
  const ray = new T.Raycaster();
  const forward = new T.Vector3(0, 0, -1);
  let covered = 0;
  let open = 0;
  for (let i = 0; i < 31; i++) {
    ray.set(new T.Vector3(bank.x + (i / 30 - 0.5) * bank.width,
      bank.y + bank.height * 0.72, bank.z + 3), forward);
    if (ray.intersectObject(mesh).length) covered++;
    else open++;
  }
  assert.ok(covered > 5);
  assert.ok(open > 2);
});

test('an empty planting adds no scene objects', () => {
  const scene = new T.Scene();
  mountCityReeds(T, scene, []);
  assert.equal(scene.children.length, 0);
});
