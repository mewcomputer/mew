import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { createStaffCharacter } from '../src/staff-character.ts';

test('the equipped staff draws between the body and gripping arm throughout a walk', () => {
  const texture = new T.Texture();
  const character = createStaffCharacter(T, texture);
  const scene = new T.Scene();
  scene.add(character.group);
  const camera = new T.OrthographicCamera();
  for (const [x, y, z] of [[4, 5, 5], [-4, 5, -5]]) {
    camera.position.set(x, y, z);
    camera.lookAt(0, 0, 0);
    camera.updateMatrixWorld();
    for (let tick = 0; tick < 64; tick++) {
      character.paint({ x: 0, y: 1, z: 0 }, [0, 1], camera, tick);
      const meshes = character.group.children.filter((object): object is T.Mesh<T.BufferGeometry, T.Material> =>
        object instanceof T.Mesh && object.visible && !Array.isArray(object.material));
      const staff = meshes.find((mesh) => mesh.name === 'rainbell-staff');
      const body = meshes.find((mesh) => mesh.material.depthWrite);
      assert.ok(staff && body);
      const arm = meshes.reduce((top, mesh) => mesh.renderOrder > top.renderOrder ? mesh : top);
      assert.ok(body.renderOrder < staff.renderOrder && staff.renderOrder < arm.renderOrder);
      assert.ok(body.position.z < arm.position.z);
      assert.ok(Number.isFinite(staff.position.lengthSq()));
      assert.equal(staff.material.depthWrite, false);
      if (!staff.geometry.boundingBox) staff.geometry.computeBoundingBox();
      const foot = staff.localToWorld(new T.Vector3(0, staff.geometry.boundingBox!.min.y, 0));
      assert.ok(foot.y >= 1, 'the bottom of the carried staff stays above the tile');
    }
  }
});
