import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { createHippedRoofGeometry, createOperaCanopyGeometry } from '../src/city-roof.ts';

test('a hipped roof covers its footprint, turns up at the corners, and closes at the ridge', () => {
  const roof = createHippedRoofGeometry(T, 7, 6, 1.4);
  roof.computeBoundingBox();
  const bounds = roof.boundingBox!;
  assert.equal(bounds.min.x, -3.5);
  assert.equal(bounds.max.x, 3.5);
  assert.equal(bounds.min.z, -3);
  assert.equal(bounds.max.z, 3);

  const positions = roof.getAttribute('position');
  const eave: number[] = [];
  const corner: number[] = [];
  for (let i = 0; i < positions.count; i++) {
    if (positions.getZ(i) !== 3) continue;
    if (Math.abs(positions.getX(i)) < 0.25) eave.push(positions.getY(i));
    if (positions.getX(i) === 3.5) corner.push(positions.getY(i));
  }
  assert.ok(Math.max(...corner) > Math.max(...eave) + 0.12);
  assert.ok(Math.abs(bounds.max.y - 1.4) < 0.001);
  const ray = new T.Raycaster(new T.Vector3(0, 3, 0), new T.Vector3(0, -1, 0));
  assert.ok(ray.intersectObject(new T.Mesh(roof)).some(({ point }) => Math.abs(point.y - 1.4) < 0.001));
});

test('the opera canopy sweeps upward toward the canal and has a closed soffit', () => {
  const roof = new T.Mesh(createOperaCanopyGeometry(T, 14, 9, 2.4));
  const cast = (x: number, y: number, dy: number) => {
    const ray = new T.Raycaster(new T.Vector3(x, y, 0.03), new T.Vector3(0, dy, 0));
    const hit = ray.intersectObject(roof)[0];
    assert.ok(hit, 'the visible shell covers the lobby');
    return hit.point.y;
  };
  const crest = cast(-5, 6, -1);
  assert.ok(crest > cast(5, 6, -1) + 0.7);
  assert.ok(crest - cast(-5, -2, 1) > 0.2);
});
