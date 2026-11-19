import type * as Three from 'three';
import type { SpriteArtwork } from './agent-sprites.ts';
import type { CityDirection } from './city-characters.ts';
import { rainbellWorld } from './rainbell-world.ts';
import { createStaffCharacter } from './staff-character.ts';
import { pavingJoint } from './surface-grid.ts';
import { createTacticsMap, type MovementPoint, type TacticsTile } from './tactics-movement.ts';
import { createTacticsWalk } from './tactics-walk.ts';

const tileLift = 0.012;

export function createRainbellPlaytest(T: typeof Three, scene: Three.Scene, artwork: SpriteArtwork) {
  const map = createTacticsMap(rainbellWorld);
  const occluders = scene.children.filter((object) => object instanceof T.Mesh);
  const root = new T.Group();
  root.name = 'rainbell-playtest';
  root.visible = false;
  scene.add(root);

  const tileGeometry = new T.PlaneGeometry(1, 1).rotateX(-Math.PI / 2);
  const pose = new T.Object3D();
  const footprints = new Map(map.tiles.map((tile) => [tile, overlayParts(tile)]));
  let capacity = 0;
  for (const parts of footprints.values()) capacity += parts.length;
  const grid = overlay('#ddd9be', 0.18, 2);
  const range = overlay('#72edd0', 0.57, 3);
  const start = overlay('#f0bd68', 0.85, 4);
  start.name = 'movement-start';
  const destination = overlay('#fff4b9', 0.65, 4);
  const route = new T.InstancedMesh(new T.CylinderGeometry(1, 1, 1, 8),
    new T.MeshBasicMaterial({ color: '#fff4b9', toneMapped: false }), capacity * 3);
  route.name = 'movement-route';
  route.frustumCulled = false;
  route.count = 0;
  root.add(route);

  const costume = artwork.get('Disciple');
  if (!costume) throw new Error('The movement playtest needs the disciple costume.');
  const unit = createStaffCharacter(T, new T.CanvasTexture(costume));
  unit.group.name = 'movement-test-character';
  unit.group.visible = false;
  root.add(unit.group);
  for (const tile of map.tiles) drawTile(grid, tile);
  grid.instanceMatrix.needsUpdate = true;

  const ray = new T.Raycaster();
  const pointer = new T.Vector2();
  const floor = new T.Plane(new T.Vector3(0, 1, 0));
  const hit = new T.Vector3();
  const routeUp = new T.Vector3(0, 1, 0);
  const routeSpan = new T.Vector3();
  const routePose = new T.Object3D();
  const facing: CityDirection = [0, 1];
  const collisions: Three.Intersection[] = [];
  let current: TacticsTile | undefined;
  let hovered: TacticsTile | undefined;
  let plan: ReturnType<typeof map.plan> | undefined;
  let walk: { animation: ReturnType<typeof createTacticsWalk>; destination: TacticsTile } | undefined;

  function overlay(color: string, opacity: number, order: number) {
    const material = new T.MeshBasicMaterial({ color, transparent: true, opacity, depthWrite: false, toneMapped: false });
    const mesh = new T.InstancedMesh(tileGeometry, material, capacity);
    mesh.count = 0;
    mesh.frustumCulled = false;
    mesh.renderOrder = order;
    root.add(mesh);
    return mesh;
  }

  function drawTile(mesh: Three.InstancedMesh, tile: TacticsTile) {
    for (const { x, z, width, depth } of footprints.get(tile)!) {
      pose.position.set(x, tile.y + tileLift, z);
      pose.scale.set(width, 1, depth);
      pose.updateMatrix();
      mesh.setMatrixAt(mesh.count++, pose.matrix);
    }
  }

  function tileAt(camera: Three.Camera, x: number, y: number, bounds: DOMRect) {
    pointer.set((x - bounds.left) / bounds.width * 2 - 1, 1 - (y - bounds.top) / bounds.height * 2);
    ray.setFromCamera(pointer, camera);
    let nearest: TacticsTile | undefined;
    let distance = Infinity;
    for (const [tile, parts] of footprints) {
      floor.constant = -tile.y - tileLift;
      if (!ray.ray.intersectPlane(floor, hit)) continue;
      const length = ray.ray.origin.distanceTo(hit);
      if (length >= distance) continue;
      if (!parts.some(({ x, z, width, depth }) =>
        Math.abs(hit.x - x) <= width / 2 && Math.abs(hit.z - z) <= depth / 2)) continue;
      nearest = tile;
      distance = length;
    }
    if (!nearest) return;
    ray.far = distance - 0.085;
    ray.intersectObjects(occluders, false, collisions);
    ray.far = Infinity;
    const obstructed = collisions.length > 0;
    collisions.length = 0;
    return obstructed ? undefined : nearest;
  }

  function showRoute(points: readonly MovementPoint[], target: TacticsTile) {
    route.count = 0;
    for (let i = 1; i < points.length; i++) {
      const a = points[i - 1];
      const b = points[i];
      routeSpan.set(b.x - a.x, b.y - a.y, b.z - a.z);
      const length = routeSpan.length();
      if (length < 1e-6) continue;
      routePose.position.set((a.x + b.x) / 2, (a.y + b.y) / 2 + 0.085, (a.z + b.z) / 2);
      routePose.quaternion.setFromUnitVectors(routeUp, routeSpan.divideScalar(length));
      routePose.scale.set(0.055, length + 0.06, 0.055);
      routePose.updateMatrix();
      route.setMatrixAt(route.count++, routePose.matrix);
    }
    route.instanceMatrix.needsUpdate = true;
    destination.count = 0;
    drawTile(destination, target);
    destination.instanceMatrix.needsUpdate = true;
  }

  function select(tile: TacticsTile) {
    current = tile;
    hovered = undefined;
    route.count = destination.count = 0;
    plan = map.plan(tile, rainbellWorld.movement);
    range.count = start.count = 0;
    for (const next of plan.reachable.keys()) {
      if (next !== tile) drawTile(range, next);
    }
    range.instanceMatrix.needsUpdate = true;
    drawTile(start, tile);
    start.instanceMatrix.needsUpdate = true;
    unit.group.visible = true;
    return plan.reachable.size - 1;
  }

  return {
    root,
    clear() {
      current = hovered = undefined;
      plan = walk = undefined;
      unit.group.visible = false;
      start.count = range.count = route.count = destination.count = 0;
    },
    clearHover() {
      if (hovered) {
        hovered = undefined;
        route.count = destination.count = 0;
      }
    },
    hover(camera: Three.Camera, x: number, y: number, bounds: DOMRect) {
      if (!plan || walk) return;
      const tile = tileAt(camera, x, y, bounds);
      if (tile === hovered) return;
      hovered = tile;
      const points = tile && tile !== current ? plan.pathTo(tile) : undefined;
      if (tile && points) showRoute(points, tile);
      else route.count = destination.count = 0;
    },
    pick(camera: Three.Camera, x: number, y: number, bounds: DOMRect) {
      if (walk) return;
      const tile = tileAt(camera, x, y, bounds);
      if (!tile) return;
      if (!current) return select(tile);
      if (tile === current) return;
      const points = plan?.pathTo(tile);
      if (!points) return;
      hovered = undefined;
      range.count = start.count = 0;
      showRoute(points, tile);
      walk = { animation: createTacticsWalk(points), destination: tile };
      return 'walking' as const;
    },
    update(camera: Three.Camera, delta: number, instant: boolean) {
      if (!current) return;
      if (walk) {
        const pose = walk.animation.advance(instant ? Infinity : delta);
        facing[0] = pose.direction[0];
        facing[1] = pose.direction[1];
        if (!pose.done) {
          unit.paint(pose, facing, camera, pose.ticks);
          return;
        }
        const walkDestination = walk.destination;
        const count = select(walkDestination);
        walk = undefined;
        unit.paint(walkDestination, facing, camera, 0);
        return count;
      }
      unit.paint(current, facing, camera, 0);
    },
  };
}

function overlayParts(tile: TacticsTile) {
  const cell = tile.cell ?? tile;
  const halfWidth = (cell.width - pavingJoint) / 2;
  const halfDepth = (cell.depth - pavingJoint) / 2;
  // An obstacle can split a tile's outline into rectangles. Only inset the paving joint.
  return (tile.parts ?? [tile]).flatMap(({ x, z, width, depth }) => {
    const left = Math.max(cell.x - halfWidth, x - width / 2);
    const right = Math.min(cell.x + halfWidth, x + width / 2);
    const back = Math.max(cell.z - halfDepth, z - depth / 2);
    const front = Math.min(cell.z + halfDepth, z + depth / 2);
    if (right <= left || front <= back) return [];
    return [{ x: (left + right) / 2, z: (back + front) / 2, width: right - left, depth: front - back }];
  });
}
