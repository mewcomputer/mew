import assert from 'node:assert/strict';
import test from 'node:test';
import { createOperaShow, operaCast } from '../src/opera-show.ts';
import { operaHouse } from '../src/rainbell-world.ts';

test('the audience gathers in the court, enters through the lobby, and comes back out', () => {
  const update = createOperaShow();
  const audience = operaCast.filter(({ entrance }) => entrance === 'public');
  assert.ok(audience.length >= 6);
  const gather = update(21);
  assert.ok(audience.filter(({ id }) => {
    const { position, mode, visible } = gather.actors.get(id)!;
    return visible && position[2] >= operaHouse.court.z - operaHouse.court.depth / 2 &&
      (mode === 'idle' || mode === 'talk');
  }).length >= 5);
  assert.ok(Array.from({ length: 40 }, (_, i) => update(23 + i / 4).frontDoor).some((open) => open > 0.7));
  assert.ok(audience.every(({ id }) => !update(43).actors.get(id)!.visible));
  assert.ok(audience.some(({ id }) => update(60).actors.get(id)!.visible));
});

test('ticket buyers line up, buy one at a time, and continue toward the forecourt', () => {
  const at = createOperaShow();
  const booth = operaHouse.ticketBooth;
  const buyers = ['qiu-han', 'lin-xia', 'xie-jin'];
  const queue = at(8.5);
  const places = buyers.map((id) => queue.actors.get(id)!);
  assert.equal(queue.ticketBuyer, null);
  assert.ok(places.every(({ position, direction, mode }) =>
    Math.abs(position[0] - booth.x) < 0.65 && direction[1] === -1 && mode === 'idle'));
  assert.ok(places[0].position[2] < places[1].position[2] && places[1].position[2] < places[2].position[2]);

  const sales = [9, 11.1, 13.3].map((time) => {
    const frame = at(time);
    const buyer = frame.ticketBuyer;
    assert.ok(buyer && frame.actors.get(buyer)?.mode === 'work');
    assert.deepEqual(buyers.filter((id) => frame.actors.get(id)?.mode === 'work'), [buyer]);
    assert.ok(frame.actors.get(buyer)!.position[2] < booth.z + booth.depth / 2 + 1);
    assert.equal(frame.frontDoor, 0);
    return buyer;
  });
  assert.deepEqual(sales, buyers);
  const gathering = at(22);
  assert.equal(gathering.ticketBuyer, null);
  assert.ok(buyers.every((id) => gathering.actors.get(id)!.position[2] > 63.225));
});

test('performers gather beyond the hall roofline and use the backstage entrance', () => {
  const update = createOperaShow();
  const performers = operaCast.filter(({ entrance }) => entrance === 'backstage');
  assert.ok(performers.length >= 2);
  const gather = update(10.5);
  const { garden } = operaHouse;
  for (const { id } of performers) {
    const { position: [x, , z], visible, mode } = gather.actors.get(id)!;
    assert.ok(visible && mode === 'idle');
    assert.ok(Math.abs(x - garden.x) < garden.width / 2 && Math.abs(z - garden.z) < garden.depth / 2);
  }
  const entries = Array.from({ length: 28 }, (_, i) => {
    const { rearDoor, frontDoor } = update(16 + i / 4);
    return { rearDoor, frontDoor };
  });
  assert.ok(entries.some(({ rearDoor }) => rearDoor > 0.7));
  assert.ok(entries.every(({ frontDoor }) => frontDoor === 0));
  const ready = update(25);
  assert.equal(ready.frontDoor, 0);
  assert.ok(performers.every(({ id }) => !ready.actors.get(id)!.visible));
});

test('returning to an earlier point in the show restores positions and doors', () => {
  const update = createOperaShow();
  const arrival = structuredClone(update(9.63));
  update(76);
  const replay = update(9.63);
  assert.equal(replay.frontDoor, arrival.frontDoor);
  assert.equal(replay.rearDoor, arrival.rearDoor);
  assert.equal(replay.ticketBuyer, arrival.ticketBuyer);
  assert.equal(replay.ticketProgress, arrival.ticketProgress);
  for (const [id, pose] of replay.actors) {
    const expected = arrival.actors.get(id)!;
    assert.equal(pose.visible, expected.visible);
    assert.equal(pose.mode, expected.mode);
    assert.ok(pose.position.every((point, i) => Math.abs(point - expected.position[i]) < 1e-8));
  }
});
