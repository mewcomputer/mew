import { stairLanes, surfaceGrid } from './surface-grid.ts';

const epsilon = 1e-6;
const minimumTileWidth = 0.125;

type Footprint = {
  readonly x: number;
  readonly z: number;
  readonly width: number;
  readonly depth: number;
};

type Surface = Footprint & {
  readonly id: string;
  readonly y: number;
  readonly columns?: number;
  readonly rows?: number;
};
type Stair = {
  readonly id: string;
  readonly from: string;
  readonly to: string;
  readonly x: number;
  readonly z: number;
  readonly run: number;
  readonly width: number;
  readonly direction: 'north' | 'west';
  readonly steps: number;
};
type Blocker = Footprint & { readonly surface: string };

export type TacticsTile = Footprint & {
  readonly surface: string;
  readonly y: number;
  readonly cell?: Footprint;
  readonly parts?: readonly Footprint[];
};

export type MovementPoint = {
  readonly x: number;
  readonly y: number;
  readonly z: number;
};

type Bounds = { minX: number; maxX: number; minZ: number; maxZ: number };
type FloorTile = { parts: readonly Bounds[]; tile: TacticsTile };
type Position = Pick<MovementPoint, 'x' | 'z'>;
type TilePortal = { tile: TacticsTile; portal: Position };
type TileCost = { tile: TacticsTile; cost: number };
type Edge = TilePortal & { cost: number };

function bounds({ x, z, width, depth }: Footprint): Bounds {
  return { minX: x - width / 2, maxX: x + width / 2, minZ: z - depth / 2, maxZ: z + depth / 2 };
}

function footprint({ minX, maxX, minZ, maxZ }: Bounds): Footprint {
  return { x: (minX + maxX) / 2, z: (minZ + maxZ) / 2, width: maxX - minX, depth: maxZ - minZ };
}

function subtract(area: Bounds, cut: Bounds): Bounds[] | null {
  const minX = Math.max(area.minX, cut.minX);
  const maxX = Math.min(area.maxX, cut.maxX);
  const minZ = Math.max(area.minZ, cut.minZ);
  const maxZ = Math.min(area.maxZ, cut.maxZ);
  if (maxX - minX <= epsilon || maxZ - minZ <= epsilon) return null;

  return [
    { ...area, maxX: minX },
    { ...area, minX: maxX },
    { minX, maxX, minZ: area.minZ, maxZ: minZ },
    { minX, maxX, minZ: maxZ, maxZ: area.maxZ },
  ].filter((part) => part.maxX - part.minX >= minimumTileWidth && part.maxZ - part.minZ >= minimumTileWidth);
}

const close = (a: number, b: number) => Math.abs(a - b) <= epsilon;
const overlaps = (a: number, b: number, c: number, d: number) =>
  Math.min(b, d) - Math.max(a, c) >= minimumTileWidth - epsilon;

const midpoint = (a: number, b: number, c: number, d: number) => (Math.max(a, c) + Math.min(b, d)) / 2;

function sharedPortal(a: Bounds, b: Bounds): Position | undefined {
  if (overlaps(a.minZ, a.maxZ, b.minZ, b.maxZ)) {
    if (close(a.maxX, b.minX)) return { x: a.maxX, z: midpoint(a.minZ, a.maxZ, b.minZ, b.maxZ) };
    if (close(a.minX, b.maxX)) return { x: a.minX, z: midpoint(a.minZ, a.maxZ, b.minZ, b.maxZ) };
  }
  if (overlaps(a.minX, a.maxX, b.minX, b.maxX)) {
    if (close(a.maxZ, b.minZ)) return { x: midpoint(a.minX, a.maxX, b.minX, b.maxX), z: a.maxZ };
    if (close(a.minZ, b.maxZ)) return { x: midpoint(a.minX, a.maxX, b.minX, b.maxX), z: a.minZ };
  }
}

function tilePortal(a: readonly Bounds[], b: readonly Bounds[]) {
  for (const part of a) {
    for (const neighbor of b) {
      const portal = sharedPortal(part, neighbor);
      if (portal) return portal;
    }
  }
}

function connectedParts(parts: Bounds[]) {
  if (parts.length === 1) return [parts];
  const components: Bounds[][] = [];
  while (parts.length) {
    const component = [parts.pop()!];
    for (let i = 0; i < component.length; i++) {
      for (let j = parts.length - 1; j >= 0; j--) {
        if (sharedPortal(component[i], parts[j])) {
          component.push(parts[j]);
          parts.splice(j, 1);
        }
      }
    }
    components.push(component);
  }
  return components;
}

function stairBounds(stair: Stair): Bounds {
  const { x, z, run, width, direction } = stair;
  return direction === 'west'
    ? { minX: x - run, maxX: x, minZ: z - width / 2, maxZ: z + width / 2 }
    : { minX: x - width / 2, maxX: x + width / 2, minZ: z - run, maxZ: z };
}

function landingPortal(stair: Bounds, tile: Bounds, alongX: boolean, top: boolean): Position | undefined {
  if (alongX) {
    const x = top ? stair.minX : stair.maxX;
    if (close(x, top ? tile.maxX : tile.minX) && overlaps(stair.minZ, stair.maxZ, tile.minZ, tile.maxZ)) {
      return { x, z: midpoint(stair.minZ, stair.maxZ, tile.minZ, tile.maxZ) };
    }
  } else {
    const z = top ? stair.minZ : stair.maxZ;
    if (close(z, top ? tile.maxZ : tile.minZ) && overlaps(stair.minX, stair.maxX, tile.minX, tile.maxX)) {
      return { x: midpoint(stair.minX, stair.maxX, tile.minX, tile.maxX), z };
    }
  }
}

function contains({ minX, maxX, minZ, maxZ }: Bounds, { x, z }: Position) {
  return x >= minX - epsilon && x <= maxX + epsilon && z >= minZ - epsilon && z <= maxZ + epsilon;
}

function appendPoint(path: MovementPoint[], { x, z }: Position, y: number) {
  const last = path.at(-1);
  if (!last || !close(last.x, x) || !close(last.y, y) || !close(last.z, z)) path.push({ x, y, z });
}

function appendWithinTile(path: MovementPoint[], tile: TacticsTile, destination: Position) {
  if (tile.parts) {
    const parts = tile.parts.map(bounds);
    const from = path.at(-1)!;
    const first = parts.find((part) => contains(part, from));
    const last = parts.find((part) => contains(part, destination));
    if (!first || !last) return false;
    const parents = new Map<Bounds, { part: Bounds; portal: Position } | null>([[first, null]]);
    const queue = [first];
    for (let i = 0; i < queue.length && !parents.has(last); i++) {
      for (const part of parts) {
        if (parents.has(part)) continue;
        const portal = sharedPortal(queue[i], part);
        if (!portal) continue;
        parents.set(part, { part: queue[i], portal });
        queue.push(part);
      }
    }
    if (!parents.has(last)) return false;

    // Route through shared edges so each segment lies inside one open rectangle.
    const portals: Position[] = [];
    for (let entry = parents.get(last); entry; entry = parents.get(entry.part)) portals.push(entry.portal);
    for (let i = portals.length - 1; i >= 0; i--) appendPoint(path, portals[i], tile.y);
  }
  appendPoint(path, destination, tile.y);
  return true;
}

export function createTacticsMap({ surfaces, stairs, blockers }: {
  readonly surfaces: readonly Surface[];
  readonly stairs: readonly Stair[];
  readonly blockers: readonly Blocker[];
}) {
  const tiles: TacticsTile[] = [];
  const edges = new Map<TacticsTile, Edge[]>();
  const floor = new Map<string, FloorTile[]>();
  const cells = new Map<string, FloorTile[]>();
  const previousSurfaces: { y: number; bounds: Bounds }[] = [];
  const surfaceById = new Map(surfaces.map((surface) => [surface.id, surface]));
  const flights = stairs.map((stair) => {
    const from = surfaceById.get(stair.from);
    const to = surfaceById.get(stair.to);
    if (!from || !to) throw new Error(`Stair "${stair.id}" references a missing landing.`);
    if (to.y <= from.y || stair.run <= 0 || stair.width <= 0 || !Number.isInteger(stair.steps) || stair.steps < 1
      || ![stair.x, stair.z, stair.run, stair.width].every(Number.isFinite)) {
      throw new Error(`Stair "${stair.id}" needs a positive rise, run, width, and tread count.`);
    }
    return { stair, from, to, footprint: stairBounds(stair) };
  });

  function addTile(tile: TacticsTile) {
    tiles.push(tile);
    edges.set(tile, []);
    return tile;
  }

  function connect(a: TacticsTile, b: TacticsTile, cost: number, portal: Position) {
    edges.get(a)!.push({ tile: b, cost, portal });
    edges.get(b)!.push({ tile: a, cost, portal });
  }

  for (const surface of surfaces) {
    if (![surface.x, surface.y, surface.z, surface.width, surface.depth].every(Number.isFinite)
      || surface.width <= 0 || surface.depth <= 0) {
      throw new Error(`Surface "${surface.id}" needs a finite position and positive dimensions.`);
    }
    if ([surface.columns, surface.rows].some((count) => count !== undefined && (!Number.isInteger(count) || count < 1))) {
      throw new Error(`Surface "${surface.id}" needs positive whole numbers of rows and columns.`);
    }
    const area = bounds(surface);
    const { columns, rows, cellWidth, cellDepth } = surfaceGrid(surface.width, surface.depth, surface.columns, surface.rows);
    const cuts = [
      ...previousSurfaces.filter(({ y }) => close(surface.y, y)).map((other) => other.bounds),
      ...blockers.filter((blocker) => blocker.surface === surface.id).map(bounds),
      ...flights.filter(({ from, to }) => from === surface || to === surface).map(({ footprint }) => footprint),
    ];
    const surfaceTiles: FloorTile[] = [];
    floor.set(surface.id, surfaceTiles);
    previousSurfaces.push({ y: surface.y, bounds: area });

    for (let row = 0; row < rows; row++) {
      for (let column = 0; column < columns; column++) {
        const cell = {
          minX: area.minX + column * cellWidth, maxX: area.minX + (column + 1) * cellWidth,
          minZ: area.minZ + row * cellDepth, maxZ: area.minZ + (row + 1) * cellDepth,
        };
        const parts = [cell];
        for (const cut of cuts) {
          for (let i = parts.length - 1; i >= 0; i--) {
            const remainder = subtract(parts[i], cut);
            if (remainder) parts.splice(i, 1, ...remainder);
          }
        }
        if (!parts.length) continue;

        const buckets: FloorTile[][] = [];
        const nearby = new Set<FloorTile>();
        for (let iz = Math.floor(cell.minZ - epsilon); iz <= Math.floor(cell.maxZ + epsilon); iz++) {
          for (let ix = Math.floor(cell.minX - epsilon); ix <= Math.floor(cell.maxX + epsilon); ix++) {
            const key = `${ix},${iz}`;
            let bucket = cells.get(key);
            if (!bucket) cells.set(key, bucket = []);
            for (const other of bucket) nearby.add(other);
            buckets.push(bucket);
          }
        }

        const original = footprint(cell);
        for (const component of connectedParts(parts)) {
          const largest = component.reduce((best, next) =>
            (next.maxX - next.minX) * (next.maxZ - next.minZ) > (best.maxX - best.minX) * (best.maxZ - best.minZ) ? next : best);
          const tile = addTile({
            ...(largest === cell ? original : footprint(largest)),
            y: surface.y,
            surface: surface.id,
            cell: component[0] !== cell ? original : undefined,
            parts: component.length > 1 ? component.map(footprint) : undefined,
          });
          const current = { parts: component, tile };
          for (const other of nearby) {
            if (!close(tile.y, other.tile.y)) continue;
            const portal = tilePortal(component, other.parts);
            if (portal) connect(tile, other.tile, 1, portal);
          }
          for (const bucket of buckets) bucket.push(current);
          surfaceTiles.push(current);
        }
      }
    }
  }

  for (const { stair, from, to, footprint: flight } of flights) {
    const { direction, id, run, steps, width } = stair;
    const alongX = direction === 'west';
    const tread = run / steps;
    const treadCost = Math.max(1, run) / steps;
    const laneCount = stairLanes(width);
    const laneWidth = width / laneCount;
    const landingCost = 0.5 + treadCost / 2;

    function connectLanding(tile: TacticsTile, area: Bounds, landingId: string, top: boolean) {
      for (const landing of floor.get(landingId)!) {
        for (const part of landing.parts) {
          const portal = landingPortal(area, part, alongX, top);
          if (!portal) continue;
          connect(tile, landing.tile, landingCost, portal);
          break;
        }
      }
    }

    let previousRow: TacticsTile[] = [];
    for (let step = 0; step < steps; step++) {
      const row: TacticsTile[] = [];
      for (let lane = 0; lane < laneCount; lane++) {
        const top = (step + 1) * tread;
        const bottom = step * tread;
        const offset = lane * laneWidth;
        const area = alongX
          ? { minX: flight.maxX - top, maxX: flight.maxX - bottom,
              minZ: flight.minZ + offset, maxZ: flight.minZ + offset + laneWidth }
          : { minX: flight.minX + offset, maxX: flight.minX + offset + laneWidth,
              minZ: flight.maxZ - top, maxZ: flight.maxZ - bottom };
        const tile = addTile({ ...footprint(area), y: from.y + (to.y - from.y) * (step + 1) / steps, surface: id });
        if (lane > 0) connect(tile, row[lane - 1], 1, alongX ? { x: tile.x, z: area.minZ } : { x: area.minX, z: tile.z });
        if (step > 0) connect(tile, previousRow[lane], treadCost,
          alongX ? { x: area.maxX, z: tile.z } : { x: tile.x, z: area.maxZ });
        if (step === 0) connectLanding(tile, area, from.id, false);
        if (step === steps - 1) connectLanding(tile, area, to.id, true);
        row.push(tile);
      }
      previousRow = row;
    }
  }

  function findReachable(start: TacticsTile, budget: number, parents?: Map<TacticsTile, TilePortal>) {
    const costs = new Map<TacticsTile, number>();
    if (!edges.has(start) || !(budget >= 0)) return costs;
    costs.set(start, 0);
    const queue: TileCost[] = [{ tile: start, cost: 0 }];
    for (let i = 0; i < queue.length; i++) {
      const { tile, cost } = queue[i];
      if (cost !== costs.get(tile)) continue;
      for (const next of edges.get(tile)!) {
        const total = cost + next.cost;
        if (total > budget + epsilon || total >= (costs.get(next.tile) ?? Infinity) - epsilon) continue;
        costs.set(next.tile, total);
        parents?.set(next.tile, { tile, portal: next.portal });
        queue.push({ tile: next.tile, cost: total });
      }
    }
    return costs;
  }

  return {
    tiles: tiles as readonly TacticsTile[],
    reachable(start: TacticsTile, budget: number) {
      return findReachable(start, budget);
    },
    plan(start: TacticsTile, budget: number) {
      const parents = new Map<TacticsTile, TilePortal>();
      const reachable = findReachable(start, budget, parents);
      const validStart = reachable.has(start);
      return {
        reachable,
        pathTo(target: TacticsTile): readonly MovementPoint[] | undefined {
          if (!validStart) return;
          const steps: TilePortal[] = [];
          for (let tile = target; tile !== start;) {
            const previous = parents.get(tile);
            if (!previous) return;
            steps.push({ tile, portal: previous.portal });
            tile = previous.tile;
          }
          const path: MovementPoint[] = [{ x: start.x, y: start.y, z: start.z }];
          let tile = start;
          for (let i = steps.length - 1; i >= 0; i--) {
            const { tile: next, portal } = steps[i];
            if (!appendWithinTile(path, tile, portal)) return;
            appendPoint(path, portal, next.y);
            tile = next;
          }
          if (!appendWithinTile(path, tile, target)) return;
          return path;
        },
      };
    },
  };
}
