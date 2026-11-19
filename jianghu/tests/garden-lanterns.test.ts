import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { createGardenLanterns } from '../src/garden-lanterns.ts';
import { createLanternRound } from '../src/lantern-round.ts';

test('a carried lantern follows the painted hand with a warm local light and switches off by day', () => {
  const makeLantern = createGardenLanterns(T);
  const carrier = new T.Object3D();
  carrier.position.set(1, 2, 3);
  const lantern = makeLantern(carrier, 0);
  const point = lantern.root.getObjectByName('lantern-light');
  assert.ok(point instanceof T.PointLight);
  assert.ok(point.color.r > point.color.g && point.color.g > point.color.b);
  assert.ok(point.distance >= 2 && point.distance < 4);

  lantern.update(null, 0);
  assert.equal(lantern.root.visible, false);
  lantern.update([7.5, 6.5], 0);
  assert.equal(lantern.root.visible, true);
  assert.ok(lantern.root.position.x > carrier.position.x);
  assert.ok(lantern.root.position.y > carrier.position.y);
  const before = point.getWorldPosition(new T.Vector3());

  carrier.position.x += 2;
  lantern.update([7.5, 6.5], 0);
  const traveled = point.getWorldPosition(new T.Vector3()).sub(before);
  assert.ok(traveled.distanceTo(new T.Vector3(2, 0, 0)) < 1e-6);

  carrier.rotation.y = Math.PI;
  lantern.update([-7.5, 6.5], 0);
  assert.ok(lantern.root.position.x > carrier.position.x);
  const hook = [1, 2, 3] as const;
  lantern.shiftToHand(hook, 0);
  assert.deepEqual(lantern.root.position.toArray(), [...hook]);
  assert.equal(point.intensity, 0);
  lantern.store(hook);
  assert.equal(lantern.root.visible, true);
  assert.deepEqual(point.getWorldPosition(new T.Vector3()).toArray(), [1, 1.85, 3]);
  lantern.update([-7.5, 6.5], 1);
  const brightness = point.intensity;
  lantern.update([-7.5, 6.5], 1, false, 0.5);
  assert.equal(point.intensity, brightness / 2);
  lantern.update([-7.5, 6.5], 1, false, 0);
  assert.equal(lantern.root.visible, true);
  assert.equal(point.intensity, 0);
});

test('walkers take a lantern at the rack, carry it on a round, and put it back once', () => {
  const walk = createLanternRound([[0, 0, 0], [2, 0, 0], [4, 0, 0]], 1, 2, [0, 1], 0.5);
  const approach = walk(0, true);
  assert.equal(approach.carrying, false);
  assert.equal(approach.phase, 'fetch');
  const ticks = [];
  for (let time = 0.1; time < 16; time += 0.1) {
    const { pose, phase, carrying, reach } = walk(time, true);
    ticks.push({ x: pose.position[0], phase, carrying, reach });
  }
  const take = ticks.findIndex(({ phase }) => phase === 'take');
  const round = ticks.findIndex(({ phase }) => phase === 'round');
  const hang = ticks.findIndex(({ phase }) => phase === 'hang');
  assert.ok(take >= 0 && take < round && round < hang);
  assert.equal(ticks[take].x, 0);
  assert.equal(ticks[take].carrying, false);
  assert.ok(ticks.slice(take, round).some(({ carrying, reach }) => carrying && reach > 0 && reach < 1));
  assert.ok(ticks.slice(round, hang).some(({ x, carrying }) => x > 3.9 && carrying));
  assert.equal(ticks[hang].x, 0);
  assert.ok(ticks.slice(hang).find(({ phase, carrying }) => phase === 'roam' && !carrying));
  assert.equal(ticks.slice(hang + 1).some(({ phase }) => phase === 'fetch'), false);
});

test('dawn sends a lantern bearer back from their current position', () => {
  const walk = createLanternRound([[0, 0, 0], [2, 0, 0], [4, 0, 0]], 1, 2, [0, 1], 0.5);
  for (let t = 0; t < 3.6; t += 0.1) walk(t, true);
  const last = walk(3.6, true);
  assert.equal(last.phase, 'round');
  const previous = last.pose.position[0];
  const dawn = walk(3.6, false);
  assert.equal(dawn.phase, 'return');
  assert.equal(dawn.pose.position[0], previous);
  assert.equal(dawn.carrying, true);
  const home = walk(7.6, false);
  assert.equal(home.carrying, false);
});
