import assert from 'node:assert/strict';
import test from 'node:test';
import data from '../vendor/tactics/tactics-sprite-data.json' with { type: 'json' };
import { createAtlas, findAnimation, handPoint, poseAt } from '../vendor/tactics/tactics-sprite.mjs';
import { staffAtlas } from '../src/staff-pose.ts';

test('the staff walk keeps the near hand over the body with a grip in every view and frame', () => {
  const baseAtlas = createAtlas(data);
  const baseWalk = findAnimation(baseAtlas, 'Walk');
  const staffWalk = findAnimation(staffAtlas, 'Walk');
  assert.ok(!baseWalk.idle && !staffWalk.idle);
  for (let view = 0; view < 8; view++) {
    const ready = poseAt(staffAtlas, staffWalk, view, 0);
    const initialBody = ready.tiles.find(({ body }) => body)!;
    const initialGrip = handPoint(staffAtlas, ready);
    assert.ok(initialGrip);
    const totalTicks = baseAtlas.sequences[ready.back ? baseWalk.back : baseWalk.front].totalTicks;
    const bodyTiles = new Set<string>();
    let originalHasOpenHand = false;

    for (let ticks = 0; ticks < totalTicks; ticks++) {
      const base = poseAt(baseAtlas, baseWalk, view, ticks);
      const staff = poseAt(staffAtlas, staffWalk, view, ticks);
      const body = staff.tiles.find(({ body }) => body)!;
      const hand = staff.tiles[staff.count - 1];
      const grip = handPoint(staffAtlas, staff);
      assert.ok(grip, `missing grip at view ${view}, tick ${ticks}`);
      assert.ok(hand.order > body.order);
      assert.equal(hand.id, ready.tiles[ready.count - 1].id);
      assert.equal(grip[0] - body.x, initialGrip[0] - initialBody.x);
      assert.equal(grip[1] - body.y, initialGrip[1] - initialBody.y);
      assert.deepEqual(staff.tiles.slice(0, staff.count - 1), base.tiles.slice(0, base.count - 1));
      bodyTiles.add(body.id);
      originalHasOpenHand ||= handPoint(baseAtlas, base) === null;
    }

    assert.ok(bodyTiles.size > 1);
    assert.ok(originalHasOpenHand, 'the original Walk still uses its native free arm');
  }
});

test('adding the staff walk retains the original frames and all other sequences', () => {
  const walk = findAnimation(staffAtlas, 'Walk');
  assert.ok(!walk.idle);
  assert.deepEqual(staffAtlas.data.frames.slice(0, data.frames.length), data.frames);
  for (const [index, sequence] of data.sequences.entries()) {
    if (index === walk.front || index === walk.back) continue;
    assert.equal(staffAtlas.data.sequences[index], sequence);
  }
});
