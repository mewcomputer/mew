import type * as Three from 'three';
import { createTacticsSprite, flattenOntoSprite, prepareSheet } from '../vendor/tactics/tactics-sprite-three.mjs';
import { spriteFacing } from './agent-sprites.ts';
import { staffAtlas } from './staff-pose.ts';
import type { MovementPoint } from './tactics-movement.ts';

const pixelSize = 0.0375;
const footLift = 0.048;

export function createStaffCharacter(T: typeof Three, sheet: Three.Texture) {
  const rig = createTacticsSprite(staffAtlas, prepareSheet(sheet), {
    animation: 'Walk', scaleX: pixelSize, scaleY: pixelSize,
  });
  rig.itemLayer = 1;
  const profile = [
    [0, -0.34], [0.027, -0.34], [0.037, -0.30], [0.034, -0.25],
    [0.036, -0.15], [0.045, -0.11], [0.045, 0.11], [0.036, 0.15],
    [0.034, 1.07], [0.050, 1.10], [0.050, 1.16],
    [0.069, 1.20], [0.057, 1.27], [0, 1.31],
  ];
  const geometry = new T.LatheGeometry(profile.map(([radius, height]) => new T.Vector2(radius, height)), 10);
  const vertices = geometry.getAttribute('position');
  const colors = new Float32Array(vertices.count * 3);
  const color = new T.Color();
  for (let i = 0; i < vertices.count; i++) {
    const y = vertices.getY(i);
    color.set(y > 1.17 ? '#88d4af' : y > 1.08 || y < -0.26 ? '#dbb77a' : Math.abs(y) < 0.14 ? '#ab602b' : '#785033');
    color.toArray(colors, i * 3);
  }
  geometry.setAttribute('color', new T.BufferAttribute(colors, 3));
  const material = new T.MeshStandardMaterial({
    vertexColors: true, roughness: 0.68, metalness: 0.12, transparent: true, depthWrite: false,
  });
  material.onBeforeRender = flattenOntoSprite;
  const staff = new T.Mesh(geometry, material);
  staff.name = 'rainbell-staff';
  staff.userData.sprite = rig;
  staff.frustumCulled = false;
  rig.group.add(staff);

  const grip = new T.Vector3();
  const up = new T.Vector3(0, 1, 0);
  const right = new T.Vector3();
  const axis = new T.Vector3();
  const rotation = new T.Quaternion();

  return {
    group: rig.group,
    paint({ x, y, z }: MovementPoint, direction: readonly [number, number], camera: Three.Camera, ticks: number) {
      camera.getWorldQuaternion(rotation);
      right.set(1, 0, 0).applyQuaternion(rotation);
      const stand = axis.copy(up).applyQuaternion(rotation).y * rig.standHeight;
      rig.group.position.set(x, y + stand + footLift, z);
      rig.player.facing = spriteFacing(direction);
      const pose = rig.update(ticks / 60, camera);
      staff.visible = rig.hand('last', 0, grip) !== null;
      if (!staff.visible) return;

      staff.position.copy(rig.group.worldToLocal(grip));
      axis.copy(up).addScaledVector(right, -Math.sign(staff.position.x) * 0.34).normalize();
      rig.group.getWorldQuaternion(rotation).invert();
      staff.quaternion.setFromUnitVectors(up, axis).premultiply(rotation);
      staff.renderOrder = pose.tiles[pose.count - 1].order - 0.5;
    },
  };
}
