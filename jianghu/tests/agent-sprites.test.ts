import assert from 'node:assert/strict';
import test from 'node:test';
import { spriteView, spritePose } from '../src/agent-sprites.ts';

test('world direction and orbit select front and back views from the tactics sheet', () => {
  const views = Array.from({ length: 8 }, (_, step) => spriteView([1, 0], (step * Math.PI) / 4));
  assert.equal(new Set(views).size, 8);
  const poses = views.map((view) => spritePose('Research', 'idle', view, 0));
  assert.ok(poses.every((pose) => pose.count > 0));
  assert.ok(poses.some((pose) => pose.back) && poses.some((pose) => !pose.back));
});

test('walk and role-specific work poses resolve to visible, changing tiles', () => {
  for (const role of ['Research', 'Code', 'Browse', 'Write'] as const) {
    for (const mode of ['walk', 'work'] as const) {
      for (let view = 0; view < 8; view++) {
        const frames = Array.from({ length: 24 }, (_, step) => spritePose(role, mode, view, step * 6));
        assert.ok(frames.every((pose) => pose.count > 0), `${role} ${mode} view ${view}`);
        assert.ok(new Set(frames.map((pose) => pose.shp)).size > 1, `${role} ${mode} view ${view}`);
      }
    }
  }
});
