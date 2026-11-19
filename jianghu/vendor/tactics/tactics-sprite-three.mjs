//NOTE(@jimmylee)
//One Three.js unit from tactics-sprite.mjs. A group turned to the camera holds up to 6 planes, one per
//tile, placed in sprite pixels from the stance point. Call `sprite.update(seconds, camera)` once per frame
//before renderer.render, and leave its rotation alone: the group may sit under any parent, and the camera
//under any rig. This is the engine's sequence group with the game parts taken out, and `carry` is the engine's
//held item. tests/sprite-kit.test.js compares the two plane by plane and item by item.

import * as THREE from 'three';
import { FACING_X, FACING_Z, STANCE_HALF_H, cameraStep, createPlayer, frameHand } from './tactics-sprite.mjs';

//NOTE(@jimmylee)
//World units per sprite pixel. The sprite is 1.2 times taller than a square pixel, the same on every tile,
//because a tactical RPG camera looks down on it. A 32px cube is one tile of ground at these numbers.
export const SCALE_X = 0.75;
export const SCALE_Y = 0.9;
//NOTE(@jimmylee)
//A held item is drawn among a frame's tiles. Depth along the view axis changes what covers what and moves
//nothing on an orthographic screen, so the tiles over the item stand ITEM_LAYER_GAP toward the camera and the
//item's depth is squeezed into that gap, ITEM_FLATTEN thick per world unit it reaches.
export const ITEM_LAYER_GAP = 0.4;
export const ITEM_FLATTEN = 0.005;
//NOTE(@jimmylee)
//How squarely an item has to point along the view, as the cosine of the angle between them, to be end-on: then it
//stands in front of every tile, or behind them all, whatever `over` says. The game's views cannot reach this.
export const ITEM_END_ON = 0.8;
//NOTE(@jimmylee)
//An `over` entry for a ball in both hands: the head and the hands stand over it, and the torso does not.
export const AROUND_BODY = -99;
const MAX_TILES = 6;
const MAX_LIFT = 16;
//NOTE(@jimmylee)
//Units sort by view depth as whole groups: three reads a Group's renderOrder before any tile's, so one unit's
//tiles never interleave with another's. SORT_BASE keeps every unit after the scene's other transparent
//objects, which default to 0: a contact shadow or a sheet of water on the ground stays under the unit.
const SORT_BASE = 1e6;

//NOTE(@jimmylee)
//A sheet is read once and never filtered. Without the color space three lifts every dark palette entry
//and the outline between a hand and a body fades. Without nearest filtering neighbouring tiles bleed in.
export function prepareSheet(texture) {
  texture.colorSpace = THREE.SRGBColorSpace;
  texture.magFilter = THREE.NearestFilter;
  texture.minFilter = THREE.NearestFilter;
  texture.generateMipmaps = false;
  texture.needsUpdate = true;
  return texture;
}

//NOTE(@jimmylee)
//Two materials per sheet, shared by every unit. Only the body plane writes depth: coplanar overlays would
//z-fight, and they keep depthTest so the scene still hides them. forceSinglePass draws each plane once.
//toneMapped stays off because a sprite is a palette, not light: a filmic renderer would shift every color.
const MATERIALS = new WeakMap();
function materialsFor(texture) {
  if (!MATERIALS.has(texture)) {
    const base = { map: texture, transparent: true, alphaTest: 0.1, side: THREE.DoubleSide, forceSinglePass: true, depthTest: true, toneMapped: false };
    MATERIALS.set(texture, { body: new THREE.MeshBasicMaterial({ ...base, depthWrite: true }), overlay: new THREE.MeshBasicMaterial({ ...base, depthWrite: false }) });
  }
  return MATERIALS.get(texture);
}

//NOTE(@jimmylee)
//Each plane owns its uv attribute, which is written only when its tile changes, so every unit shares one
//texture and one upload. PlaneGeometry's corners run top left, top right, bottom left, bottom right.
function writePlaneUvs(geometry, tile, sheetW, sheetH) {
  const left = (tile.flipX ? tile.sx + tile.w : tile.sx) / sheetW;
  const right = (tile.flipX ? tile.sx : tile.sx + tile.w) / sheetW;
  const top = 1 - tile.sy / sheetH;
  const bottom = 1 - (tile.sy + tile.h) / sheetH;
  const uv = geometry.attributes.uv;
  uv.setXY(0, left, top);
  uv.setXY(1, right, top);
  uv.setXY(2, left, bottom);
  uv.setXY(3, right, bottom);
  uv.needsUpdate = true;
}

//NOTE(@jimmylee)
//A material's onBeforeRender, for anything carried on a sprite: three has just worked out the mesh's
//modelViewMatrix, and this flattens that matrix's depth into the gap between the tiles under the item and the
//tiles over it, around the view depth of the unit in `object.userData.sprite`. The normals stay the mesh's own,
//so a metal still reads as 3D, and its position and matrixWorld keep saying where it really is. `overTiles`
//sits past the gap instead, over the tiles drawn over the item it goes with, as an arrow on a string does.
export function flattenOntoSprite(renderer, scene, camera, geometry, object) {
  const sprite = object.userData.sprite;
  if (!sprite) return;
  const view = camera.matrixWorldInverse.elements;
  const { x, y, z } = sprite.stance;
  const plane = view[2] * x + view[6] * y + view[10] * z + view[14] + sprite.lift + sprite.layerGap * (object.userData.overTiles ? 1.5 : 0.5);
  const e = object.modelViewMatrix.elements;
  e[2] *= ITEM_FLATTEN;
  e[6] *= ITEM_FLATTEN;
  e[10] *= ITEM_FLATTEN;
  e[14] = plane + (e[14] - plane) * ITEM_FLATTEN;
}

const firstTileOverItem = (count, layer, end) => (end > 0 ? count : end < 0 ? 0 : layer < 0 ? -layer : Math.max(0, count - layer));
const tileOverItem = (index, isBody, count, layer, end) => (layer === AROUND_BODY && end === 0 ? index >= count - 2 || (index === 0 && !isBody) : index >= firstTileOverItem(count, layer, end));

const _forward = new THREE.Vector3();
const _toCamera = new THREE.Quaternion();
const _parentRotation = new THREE.Quaternion();
const _point = new THREE.Vector3();
const _hand = [0, 0];
const _itemForward = new THREE.Vector3();
const _itemAcross = new THREE.Vector3();
const _itemUp = new THREE.Vector3();
const _itemTurned = new THREE.Vector3();
const _itemToCamera = new THREE.Vector3();
const _itemAimed = new THREE.Vector3();
const _itemBasis = new THREE.Matrix4();

export function createTacticsSprite(atlas, texture, { facing = 'SE', animation = 'Idle', seconds = 0, scaleX = SCALE_X, scaleY = SCALE_Y } = {}) {
  const player = createPlayer(atlas, { facing, animation, seconds });
  const { width: sheetW, height: sheetH } = atlas.data.sheet;
  const materials = materialsFor(texture);
  const group = new THREE.Group();
  const planes = [];
  //NOTE(@jimmylee)
  //Everything measured in world units follows the world's scale, so a unit in a world of metres keeps its shape.
  const worldScale = scaleX / SCALE_X;
  for (let i = 0; i < MAX_TILES; i++) {
    const mesh = new THREE.Mesh(new THREE.PlaneGeometry(1, 1), i === 0 ? materials.body : materials.overlay);
    mesh.visible = false;
    mesh.frustumCulled = false;
    planes.push({ mesh, id: '', flipX: false });
    group.add(mesh);
  }
  let carried = null;
  let carriedEnd = 0;

  //NOTE(@jimmylee)
  //Writes world values, then moves them into the mesh's parent, so an item can live anywhere in the scene.
  function placeCarried(pose) {
    const { mesh, hold, animation: held } = carried;
    const showing = player.animation === held;
    const entry = atlas.sequences[pose.back ? held.back : held.front];
    const frame = hold.steady ?? pose.frame;
    const hand = !showing ? null : hold.hands ? hold.hands[pose.back ? 'back' : 'front'][frame] : frameHand(atlas, entry.frames[frame], pose.back ? hold.back : hold.front, _hand);
    mesh.visible = Boolean(hand);
    if (!hand) return;
    const flip = pose.mirrored ? -1 : 1;
    sprite.pointAt(hand[0] * flip, hand[1], (hold.depth ?? 0) * worldScale, mesh.position);
    if (!hold.tilt) {
      if (mesh.parent) mesh.parent.worldToLocal(mesh.position);
      return;
    }
    const side = (pose.back ? hold.back : hold.front) === 'last' ? flip : -flip;
    //NOTE(@jimmylee)
    //A frame a stage aims (`hold.aims`) points where the stage left the mesh, so the end-on test reads that, and
    //any other reads the tilt it is about to be given.
    const aimed = hold.aims?.[pose.frame];
    if (aimed) _itemAimed.set(1, 0, 0).applyQuaternion(mesh.quaternion);
    const dirX = FACING_X[player.facing];
    const dirZ = FACING_Z[player.facing];
    const tilt = hold.tilt[pose.frame];
    const cos = Math.cos(tilt);
    if (hold.across) {
      _itemForward.set(-dirZ * side * cos, Math.sin(tilt), dirX * side * cos);
      _itemAcross.set(-dirX * side, 0, -dirZ * side);
    } else {
      _itemForward.set(dirX * cos, Math.sin(tilt), dirZ * cos);
      _itemAcross.set(-dirZ, 0, dirX);
    }
    _itemUp.crossVectors(_itemAcross, _itemForward);
    if (hold.roll) {
      const sin = Math.sin(hold.roll) * side;
      _itemTurned.copy(_itemUp).multiplyScalar(Math.cos(hold.roll)).addScaledVector(_itemAcross, sin);
      _itemAcross.multiplyScalar(Math.cos(hold.roll)).addScaledVector(_itemUp, -sin);
      _itemUp.copy(_itemTurned);
    }
    mesh.quaternion.setFromRotationMatrix(_itemBasis.makeBasis(_itemForward, _itemUp, _itemAcross));
    const toward = (aimed ? _itemAimed : _itemForward).dot(_itemToCamera.set(0, 0, 1).applyQuaternion(sprite.rotation));
    carriedEnd = toward > ITEM_END_ON ? 1 : toward < -ITEM_END_ON ? -1 : 0;
    const [x, y, z] = hold.offset[pose.frame];
    mesh.position.addScaledVector(_itemForward, (x + hold.slide[pose.frame]) * worldScale).addScaledVector(_itemUp, y * worldScale).addScaledVector(_itemAcross, z * side * worldScale);
    if (mesh.parent) {
      mesh.parent.worldToLocal(mesh.position);
      mesh.quaternion.premultiply(mesh.parent.getWorldQuaternion(_parentRotation).invert());
    }
  }

  const sprite = {
    group,
    player,
    //NOTE(@jimmylee)
    //How far the stance point sits above the soles, in world units: stand the group this high over the ground.
    standHeight: STANCE_HALF_H * scaleY,
    //NOTE(@jimmylee)
    //The stance point in the world and the camera's world rotation at the last update, and how far this frame's
    //planes stand toward the camera. `itemLayer` says how many tiles, from the top of the stack, draw over a mesh
    //the caller placed by hand: a number, or { front, back }. A negative number counts tiles from the bottom that
    //stay under it, and AROUND_BODY leaves only the torso under it. `carry` sets it for you, and works out when an
    //item points along the view.
    stance: new THREE.Vector3(),
    rotation: new THREE.Quaternion(),
    lift: 0,
    layerGap: ITEM_LAYER_GAP * worldScale,
    itemLayer: null,
    play: player.play,

    //NOTE(@jimmylee)
    //Where sprite pixel (u, v) from the stance point is in the world this frame, as a mirrored tile places it,
    //with the frame's SEQ motion, `depth` world units toward the camera. Call it after update.
    pointAt(u, v, depth = 0, out = new THREE.Vector3()) {
      const distance = player.pose.fb * scaleX;
      group.getWorldPosition(out);
      out.x += FACING_X[player.facing] * distance;
      out.z += FACING_Z[player.facing] * distance;
      return out.add(_point.set(u * scaleX, -(v + player.pose.du) * scaleY, depth).applyQuaternion(sprite.rotation));
    },

    //NOTE(@jimmylee)
    //The hand `arm` draws this frame in the world, or null when that tile has no grip.
    hand(arm = 'last', depth = 0, out = new THREE.Vector3()) {
      const { pose } = player;
      const dry = player.water ? pose.shp - atlas.data.waterOffset : pose.shp;
      const hand = frameHand(atlas, dry, arm, _hand);
      return hand ? sprite.pointAt(pose.mirrored ? -hand[0] : hand[0], hand[1], depth, out) : null;
    },

    //NOTE(@jimmylee)
    //Puts `mesh` in this unit's hand while `name` plays, as data.holds says: on the hand of each frame, at the
    //item's tilt, among the frame's tiles. `update` moves it, and hides it while another animation plays or the
    //frame has no hand. Pair it with `release`, which leaves the mesh where it is, for a throw.
    carry(mesh, name = player.animation.name) {
      const key = name.toLowerCase();
      const row = Object.keys(atlas.data.holds ?? {}).find((held) => held.toLowerCase() === key);
      if (!row) throw new Error(`Nothing is held in "${name}". Read data.holds for the animations that hold something.`);
      const hold = atlas.data.holds[row];
      carried = { mesh, hold, animation: atlas.animations.get(key) };
      mesh.scale.setScalar((hold.tilt ? atlas.data.heldItemScale : 1) * worldScale);
      mesh.frustumCulled = false;
      if (hold.over) {
        mesh.userData.sprite = sprite;
        mesh.material.onBeforeRender = flattenOntoSprite;
      }
      return mesh;
    },
    release() {
      if (carried) carried.mesh.userData.sprite = null;
      carried = null;
    },

    update(at, camera) {
      camera.getWorldQuaternion(sprite.rotation);
      group.quaternion.copy(sprite.rotation);
      if (group.parent) group.quaternion.premultiply(group.parent.getWorldQuaternion(_parentRotation).invert());
      const e = camera.matrixWorld.elements;
      const pose = player.update(at, cameraStep(e[8], e[10]));

      group.updateWorldMatrix(true, false);
      sprite.stance.setFromMatrixPosition(group.matrixWorld);
      group.renderOrder = SORT_BASE + (sprite.stance.x - e[12]) * e[8] + (sprite.stance.y - e[13]) * e[9] + (sprite.stance.z - e[14]) * e[10];

      //NOTE(@jimmylee)
      //SEQ motion. Forward travels along the unit's world facing and is projected into the plane, so a lunge
      //goes the right way at every camera. Down is screen-vertical, in sprite pixels.
      let moveX = 0;
      let moveY = -pose.du * scaleY;
      if (pose.fb !== 0) {
        const distance = pose.fb * scaleX;
        _forward.set(FACING_X[player.facing] * distance, 0, FACING_Z[player.facing] * distance).applyQuaternion(_toCamera.copy(sprite.rotation).invert());
        moveX = _forward.x;
        moveY += _forward.y;
      }

      //NOTE(@jimmylee)
      //A frame reaching below the soles (Dead) would sink into the ground, so its planes move toward the
      //camera along the view axis, which does not move them on an orthographic screen.
      let below = 0;
      for (let i = 0; i < pose.count; i++) below = Math.max(below, pose.tiles[i].y + pose.tiles[i].h - STANCE_HALF_H);
      const bottom = sprite.standHeight - (below + STANCE_HALF_H) * scaleY * e[5] + moveY * e[5];
      const lift = bottom < 0 && e[9] > 1e-6 ? Math.min(-bottom / e[9], (MAX_LIFT * worldScale) / Math.max(Math.sqrt(1 - e[9] * e[9]), 1e-6)) : 0;
      sprite.lift = lift;

      carriedEnd = 0;
      if (carried) placeCarried(pose);
      const carriedNow = carried?.hold.over && player.animation === carried.animation;
      const layer = carriedNow ? carried.hold.over[pose.back ? 'back' : 'front'][pose.frame] : sprite.itemLayer === null ? null : typeof sprite.itemLayer === 'number' ? sprite.itemLayer : sprite.itemLayer[pose.back ? 'back' : 'front'];

      let next = 1;
      for (let i = 0; i < pose.count; i++) {
        const tile = pose.tiles[i];
        const plane = planes[tile.body ? 0 : next++];
        plane.mesh.visible = true;
        plane.mesh.renderOrder = tile.order;
        plane.mesh.position.set((tile.x + tile.w / 2) * scaleX + moveX, -(tile.y + tile.h / 2) * scaleY + moveY, layer !== null && tileOverItem(i, tile.body, pose.count, layer, carriedEnd) ? lift + sprite.layerGap : lift);
        plane.mesh.scale.set(tile.w * scaleX, tile.h * scaleY, 1);
        if (plane.id !== tile.id || plane.flipX !== tile.flipX) {
          writePlaneUvs(plane.mesh.geometry, tile, sheetW, sheetH);
          plane.id = tile.id;
          plane.flipX = tile.flipX;
        }
      }
      for (let i = next; i < MAX_TILES; i++) planes[i].mesh.visible = false;
      return pose;
    },
    dispose() {
      for (const { mesh } of planes) mesh.geometry.dispose();
      group.removeFromParent();
    },
  };
  return sprite;
}
