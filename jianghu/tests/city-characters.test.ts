import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { createCharacterBillboards } from '../src/character-billboard.ts';
import { baichuanCast, createCharacterTrack } from '../src/city-characters.ts';
import { spriteCanvasSize, spriteGround, spritePose, spriteView } from '../src/agent-sprites.ts';

test('a walking character pauses, traverses its route, then returns facing the other way', () => {
  const sample = createCharacterTrack({
    path: [[0, 1, 2], [2, 2, 2], [2, 2, 4]], speed: 1, phase: 0,
  });
  let pose = sample(0);
  assert.deepEqual(pose.position, [0, 1, 2]);
  assert.equal(pose.mode, 'idle');
  pose = sample(2);
  assert.deepEqual(pose.position, [1, 1.5, 2]);
  assert.deepEqual(pose.direction, [1, 0]);
  assert.equal(pose.mode, 'walk');
  pose = sample(7);
  assert.deepEqual(pose.position, [2, 2, 3]);
  assert.deepEqual(pose.direction, [0, -1]);
});

test('the cast has unique identities and city costumes with real animation frames', () => {
  const identities = baichuanCast.map(({ id }) => id);
  assert.equal(new Set(identities).size, identities.length);
  for (const definition of baichuanCast) {
    const { outfit } = definition;
    const sample = createCharacterTrack(definition);
    const { mode, direction, ticks } = sample(3);
    const view = spriteView(direction, 0.39);
    assert.ok(spritePose(outfit, mode, view, ticks).tiles.length, definition.id);
  }
  const trainee = createCharacterTrack({
    path: [[5, 2, 8]], direction: [1, 0], activity: 'work', phase: 0,
  });
  assert.equal(trainee(0).mode, 'idle');
  assert.equal(trainee(1.6).mode, 'work');
  assert.deepEqual(trainee(2.5).position, [5, 2, 8]);

  for (const yaw of [0.39, -0.64]) {
    const view = spriteView([1, 0], yaw);
    const frames = new Set();
    for (let tick = 0; tick < 60; tick += 4) {
      frames.add(spritePose('Disciple', 'work', view, tick).shp);
    }
    assert.ok(frames.size > 1, 'sword practice changes poses from either view');
  }
});

test('lit character billboards share pixel-aligned geometry and the scene tone mapping', () => {
  const billboard = createCharacterBillboards(T);
  const texture = new T.Texture();
  const first = billboard(texture);
  const second = billboard(new T.Texture());
  assert.equal(first.geometry, second.geometry);
  assert.equal(first.material.map, texture);
  assert.equal(first.material.alphaTest, 0.48);
  assert.equal(first.material.toneMapped, true);
  assert.equal(first.receiveShadow, true);
  assert.equal(first.castShadow, false);
  first.geometry.computeBoundingBox();
  const size = spriteCanvasSize * 0.0375;
  assert.ok(Math.abs(first.geometry.boundingBox!.min.y + (1 - spriteGround) * size) < 1e-6);
  assert.ok(Math.abs(first.geometry.boundingBox!.max.y - spriteGround * size) < 1e-6);
});
