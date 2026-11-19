import assert from 'node:assert/strict';
import test from 'node:test';
import { required } from './required.ts';
import { createAgentTraffic } from '../src/agent-traffic.ts';
import { createDistrict } from '../src/district.ts';
import { demoAgents } from '../src/agents.ts';

test('world routes cross the bridge and reject a point in the canal', () => {
  const traffic = createAgentTraffic(createDistrict(), demoAgents);
  assert.equal(traffic.walk('shen', 0, 3), false);
  let arrived = false;
  assert.equal(
    traffic.walk('shen', -1.25, 1.25, () => {
      arrived = true;
    }),
    true,
  );
  let crossed = false;
  for (let i = 0; i < 240 && traffic.moving; i++) {
    traffic.step(1 / 60);
    const [x, y, z] = required(traffic.get('shen')).position;
    if (Math.abs(x) < 0.5) {
      crossed = true;
      assert.ok(y > 1 && z > 0.5 && z < 1.5);
    }
  }
  assert.ok(arrived && crossed);
  assert.equal(traffic.moving, false);
});

test('handoff range uses world positions and agents reserve distinct destinations', () => {
  const traffic = createAgentTraffic(createDistrict(), demoAgents);
  assert.equal(traffic.near('lin', 'shen'), false);
  assert.equal(traffic.walk('lin', -0.75, 1.25), true);
  assert.equal(traffic.walk('shen', -0.75, 1.25), false);
  assert.equal(traffic.walk('shen', 0.25, 1.25), true);
  for (let i = 0; i < 360 && traffic.moving; i++) traffic.step(1 / 60);
  assert.equal(traffic.moving, false);
  assert.equal(traffic.near('lin', 'shen'), true);
});

test('traffic follows the authored bridge deck height', () => {
  const district = createDistrict();
  district.layout.bridge.surface = 1.25;
  const traffic = createAgentTraffic(district, demoAgents);
  assert.equal(traffic.walk('shen', -1.25, 1.25), true);
  let crossed = false;
  for (let i = 0; i < 240 && traffic.moving; i++) {
    traffic.step(1 / 60);
    const [x, y] = required(traffic.get('shen')).position;
    if (Math.abs(x) > 0.25) continue;
    assert.equal(y, district.layout.bridge.surface);
    crossed = true;
  }
  assert.equal(crossed, true);
});
