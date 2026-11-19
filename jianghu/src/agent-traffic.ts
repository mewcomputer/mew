import type { District } from './district.ts';

type Point = [number, number, number];
type Direction = [number, number];

interface NavigationTile {
  ix: number;
  iz: number;
  position: Point;
}

interface Actor {
  tile: NavigationTile;
  position: Point;
  path: NavigationTile[];
  direction: Direction;
  walked: number;
  arrive?: () => void;
}

const stride = 0.5;
const speed = 2.35;
const neighbors = [
  [1, 0],
  [-1, 0],
  [0, 1],
  [0, -1],
];

export function createAgentTraffic(
  { world, landmarks, layout }: District,
  agents: readonly { id: string; position: Readonly<Point> }[],
) {
  const startX = world.origin[0] + stride / 2;
  const startZ = world.origin[2] + stride / 2;
  const tiles = new Map<string, NavigationTile>();
  const key = (x: number, z: number) => x + ':' + z;
  const indices = (x: number, z: number): [number, number] => [
    Math.round((x - startX) / stride), Math.round((z - startZ) / stride),
  ];
  const { cell, size, origin } = world;

  const { bridge, hall, pavilion } = layout;
  const stations = Object.values(landmarks);
  const hallHalfWidth = hall.pierOffset + hall.pierWidth / 2;

  for (let iz = 0; iz < (size[2] * cell) / stride; iz++) {
    for (let ix = 0; ix < (size[0] * cell) / stride; ix++) {
      const x = startX + ix * stride;
      const z = startZ + iz * stride;
      const onBridge = Math.abs(x) < bridge.width / 2 && Math.abs(z - bridge.z) < bridge.depth / 2;
      const inHall = Math.abs(x) < hallHalfWidth && Math.abs(z - hall.z) < hall.depth / 2;
      const inPavilion = stations.some(
        (station) => Math.abs(x - station.x) < pavilion.width / 2 && Math.abs(z - station.z) < pavilion.depth / 2,
      );
      if (inHall || inPavilion) continue;
      let top = size[1] - 1;
      const vx = Math.floor((x - origin[0]) / cell);
      const vz = Math.floor((z - origin[2]) / cell);
      while (top >= 0 && !world.get(vx, top, vz)) top--;
      const y = onBridge ? bridge.surface : origin[1] + (top + 1) * cell;
      if (y < 0.69) continue;
      tiles.set(key(ix, iz), { ix, iz, position: [x, y, z] });
    }
  }

  const closest = (x: number, z: number) => {
    let best: NavigationTile | undefined;
    let distance = Infinity;
    for (const tile of tiles.values()) {
      const [tx, , tz] = tile.position;
      const next = Math.hypot(x - tx, z - tz);
      if (next < distance) {
        best = tile;
        distance = next;
      }
    }
    return best;
  };
  const actors = new Map<string, Actor>();
  for (const agent of agents) {
    const tile = closest(agent.position[0], agent.position[2]);
    if (!tile) throw new Error(`No walkable tile for ${agent.id}`);
    actors.set(agent.id, { tile, position: [...tile.position], path: [], direction: [1, 0], walked: 0 });
  }
  const get = (id: string) => actors.get(id);

  function walk(id: string, x: number, z: number, onArrive?: () => void) {
    const actor = get(id);
    const destination = tiles.get(key(...indices(x, z)));
    if (!actor || !destination || actor.path.length) return false;
    const occupied = new Set<NavigationTile>();
    for (const other of actors.values()) {
      if (other === actor) continue;
      occupied.add(other.tile);
      for (const stop of other.path) occupied.add(stop);
    }
    if (occupied.has(destination)) return false;

    const queue = [actor.tile];
    const cameFrom = new Map<NavigationTile, NavigationTile | null>([[actor.tile, null]]);
    for (let i = 0; i < queue.length && !cameFrom.has(destination); i++) {
      const tile = queue[i];
      for (const [dx, dz] of neighbors) {
        const next = tiles.get(key(tile.ix + dx, tile.iz + dz));
        if (
          !next ||
          cameFrom.has(next) ||
          occupied.has(next) ||
          Math.abs(tile.position[1] - next.position[1]) > 0.3
        )
          continue;
        queue.push(next);
        cameFrom.set(next, tile);
      }
    }
    if (!cameFrom.has(destination)) return false;
    const path: NavigationTile[] = [];
    for (let tile: NavigationTile | null | undefined = destination; tile && tile !== actor.tile; tile = cameFrom.get(tile)) {
      path.push(tile);
    }
    actor.path = path.reverse();
    actor.arrive = onArrive;
    if (!path.length) {
      actor.arrive = undefined;
      onArrive?.();
    }
    return true;
  }

  function step(seconds: number) {
    for (const actor of actors.values()) {
      let remaining = seconds * speed;
      while (remaining > 0 && actor.path.length) {
        const next = actor.path[0];
        const [x, y, z] = actor.position;
        const dx = next.position[0] - x;
        const dy = next.position[1] - y;
        const dz = next.position[2] - z;
        const distance = Math.hypot(dx, dy, dz);
        actor.direction = [Math.sign(dx), Math.sign(dz)];
        const moved = Math.min(remaining, distance);
        actor.walked += moved;
        remaining -= moved;
        if (moved < distance) {
          const fraction = moved / distance;
          actor.position = [x + dx * fraction, y + dy * fraction, z + dz * fraction];
          break;
        }
        actor.position = [...next.position];
        actor.tile = next;
        actor.path.shift();
        if (!actor.path.length) {
          const callback = actor.arrive;
          actor.arrive = undefined;
          callback?.();
        }
      }
    }
  }

  return {
    get,
    walk,
    step,
    near(a: string, b: string) {
      const one = get(a)?.position;
      const two = get(b)?.position;
      return Boolean(one && two && Math.hypot(one[0] - two[0], one[1] - two[1], one[2] - two[2]) <= 1.05);
    },
    get moving() {
      return [...actors.values()].some((actor) => actor.path.length);
    },
  };
}
