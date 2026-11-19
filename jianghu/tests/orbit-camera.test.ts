import assert from 'node:assert/strict';
import test from 'node:test';
import * as Three from 'three';
import { createOrbitCamera } from '../src/scene-view.ts';

const settings = {
  target: [0, 2, 0],
  yaw: 0.4,
  pitch: 0.6,
  width: 92,
  height: 98,
  radius: 145,
  turn: 0.12,
  zoomFactor: 1.15,
  minPitch: 0.17,
  maxPitch: 0.78,
  minZoom: 0.75,
  maxZoom: 2.4,
  yawLimits: [-0.82, 0.82],
  pan: { x: [-58, 58], z: [-70, 58] },
} as const;

test('the shared orbit fits a narrow viewport and resets after a bounded pan and zoom', () => {
  const camera = new Three.OrthographicCamera();
  const rig = createOrbitCamera(Three, camera, settings);
  rig.update(400, 800, 0, false);
  assert.equal(camera.right, 46);
  assert.equal(camera.top, 92);
  rig.zoomBy(10);
  rig.drag(20000, -20000, 800, true);
  const moved = rig.snapshot();
  assert.equal(moved.zoom, 2.4);
  assert.ok(moved.target.x >= -58 && moved.target.x <= 58);
  assert.equal(moved.target.z, 58);
  rig.control('reset');
  assert.deepEqual(rig.snapshot().target.toArray(), [0, 2, 0]);
  assert.equal(rig.snapshot().zoom, 1);
});

test('the shared camera allows quadrant reset and completes a focus under reduced motion', () => {
  const camera = new Three.OrthographicCamera();
  const rig = createOrbitCamera(Three, camera, {
    ...settings,
    yawLimits: undefined,
    turn: Math.PI / 2,
    snapReset: true,
  });
  rig.control('right');
  rig.control('reset');
  assert.equal(rig.yaw, settings.yaw + Math.PI / 2);
  const destination = new Three.Vector3(3, 4, 5);
  rig.move(destination, 1.7, 10, false);
  rig.update(900, 600, 20, true);
  assert.equal(rig.transitioning, false);
  assert.ok(rig.snapshot().target.equals(destination));
  assert.equal(rig.snapshot().zoom, 1.7);
});

test('camera-relative pan follows the current yaw and clamps to the map bounds', () => {
  const rig = createOrbitCamera(Three, new Three.OrthographicCamera(), settings);
  rig.pan(0, 10);
  const forward = rig.snapshot().target;
  assert.ok(Math.abs(forward.x + Math.sin(settings.yaw) * 10) < 1e-6);
  assert.ok(Math.abs(forward.z + Math.cos(settings.yaw) * 10) < 1e-6);
  rig.pan(10, 0);
  const right = rig.snapshot().target.sub(forward);
  assert.ok(Math.abs(right.x - Math.cos(settings.yaw) * 10) < 1e-6);
  assert.ok(Math.abs(right.z + Math.sin(settings.yaw) * 10) < 1e-6);
  rig.pan(0, 10_000);
  assert.deepEqual(rig.snapshot().target.toArray(), [-58, 2, -70]);
  assert.equal(rig.pan(0, 1), false);
});

test('landmark focus takes the shortest orbit across the yaw seam and settles on the saved view', () => {
  const camera = new Three.OrthographicCamera();
  const rig = createOrbitCamera(Three, camera, { ...settings, yawLimits: undefined, yaw: 3 });
  const target = new Three.Vector3(3, 5, 1);
  rig.move(target, 1.8, 0, false, { yaw: -3, pitch: 0.4 });
  rig.update(900, 600, 90, false);
  assert.ok(rig.yaw > 3 && rig.yaw < 3.29);
  assert.ok(camera.position.z < 0, 'the camera passes behind the target rather than circling in front');
  assert.ok(rig.snapshot().pitch < settings.pitch);
  rig.update(900, 600, 340, false);
  assert.deepEqual(rig.snapshot(), { target, zoom: 1.8, yaw: -3, pitch: 0.4 });
  assert.equal(rig.transitioning, false);
});

test('focus respects camera limits and can snap or yield to manual control', () => {
  const camera = new Three.OrthographicCamera();
  const rig = createOrbitCamera(Three, camera, settings);
  rig.move(new Three.Vector3(4, 2, -3), 2, 0, true, { yaw: 2, pitch: 1.1 });
  assert.equal(rig.snapshot().yaw, settings.yawLimits[1]);
  assert.equal(rig.snapshot().pitch, settings.maxPitch);

  rig.move(new Three.Vector3(), 1, 10, false, { yaw: -0.8, pitch: 0.3 });
  rig.update(900, 600, 100, false);
  rig.cancelTransition();
  const manual = rig.snapshot();
  rig.update(900, 600, 500, false);
  assert.deepEqual(rig.snapshot(), manual);
  rig.move(new Three.Vector3(), 1, 510, false);
  rig.update(900, 600, 1000, false);
  assert.equal(rig.snapshot().yaw, manual.yaw);
  assert.equal(rig.snapshot().pitch, manual.pitch);
});

test('off-axis framing shifts the projected target without changing the orbit', () => {
  const camera = new Three.OrthographicCamera();
  const rig = createOrbitCamera(Three, camera, settings);
  rig.setFrame(0.14, -0.06);
  rig.update(1100, 800, 0, false);
  const projected = new Three.Vector3(...settings.target).project(camera);
  assert.ok(Math.abs(projected.x - 0.28) < 1e-8);
  assert.ok(Math.abs(projected.y + 0.12) < 1e-8);
  assert.equal(rig.yaw, settings.yaw);
});

test('identical camera poses and offsets do not need another render', () => {
  const rig = createOrbitCamera(Three, new Three.OrthographicCamera(), settings);
  assert.equal(rig.setView(settings.target, settings.yaw, settings.pitch, 1), false);
  assert.equal(rig.setFrame(0, 0), false);
  assert.equal(rig.setView([1, 2, 0], settings.yaw, settings.pitch, 1), true);
  assert.equal(rig.setView([1, 2, 0], settings.yaw, settings.pitch, 1), false);
  assert.equal(rig.setFrame(0.14, -0.06), true);
  assert.equal(rig.setFrame(0.14, -0.06), false);
});
