import type { World } from './world.ts';

type Hydraulics = {
  channelStart: number;
  channelEnd: number;
  gateEdge: number;
  sill: number;
  inletLevel: number;
};

export function createWaterSimulation(
  world: World,
  { channelStart, channelEnd, gateEdge, sill, inletLevel }: Hydraulics,
) {
  const { size, origin, cell, get } = world;
  const waterWidth = size[0],
    waterLength = size[2],
    waterCount = waterWidth * waterLength;
  const depth = new Float64Array(waterCount);
  const bed = new Float64Array(waterCount);
  const xFlow = new Float64Array(waterCount);
  const zFlow = new Float64Array(waterCount);
  const outgoing = new Float64Array(waterCount);
  const limiter = new Float64Array(waterCount);
  const outlet = new Float64Array(channelEnd - channelStart);
  const dt = 1 / 90;
  const wet = 0.001;
  let gateClosed = false;
  function index(x: number, z: number) {
    return z * waterWidth + x;
  }
  function updateBed(x: number, z: number) {
    let y = size[1] - 1;
    while (y >= 0 && !get(x, y, z)) y--;
    bed[index(x, z)] = origin[1] + (y + 1) * cell;
  }
  const dispose = world.onChange((x, _y, z) => updateBed(x, z));
  for (let z = 0; z < waterLength; z++)
    for (let x = 0; x < waterWidth; x++) {
      updateBed(x, z);
      if (x >= channelStart && x < channelEnd) {
        const i = index(x, z);
        const h = 0.17 + 0.29 * (1 - z / (waterLength - 1)) ** 0.55;
        depth[i] = h;
      }
    }

  // Each edge has one signed discharge. The same transfer leaves one column and enters the next.
  function edgeFlow(previous: number, a: number, b: number, sill = -Infinity) {
    const headA = bed[a] + depth[a],
      headB = bed[b] + depth[b];
    const openDepth = Math.max(headA, headB) - Math.max(bed[a], bed[b], sill);
    if (openDepth <= wet) return 0;
    const velocityLimit = (2.7 * openDepth) / cell;
    const pressure = (dt * 9.81 * openDepth * (headA - headB)) / (cell * cell);
    return Math.max(-velocityLimit, Math.min(velocityLimit, (previous + pressure) * 0.985));
  }
  function gateBlocks(x: number, z: number) {
    return gateClosed && z === gateEdge && x >= channelStart && x < channelEnd;
  }
  function step(external = true) {
    for (let x = channelStart; x < channelEnd; x++) {
      const i = index(x, 0);
      if (!external) continue;
      const inlet = Math.min(dt * 0.82, Math.max(0, inletLevel - bed[i] - depth[i]));
      depth[i] += inlet;
    }

    outgoing.fill(0);
    for (let z = 0; z < waterLength; z++)
      for (let x = 0; x < waterWidth; x++) {
        const i = index(x, z);
        const a =
          x < waterWidth - 1 && (xFlow[i] || depth[i] > wet || depth[i + 1] > wet)
            ? edgeFlow(xFlow[i], i, i + 1)
            : 0;
        const b =
          z < waterLength - 1 && (zFlow[i] || depth[i] > wet || depth[i + waterWidth] > wet)
            ? edgeFlow(zFlow[i], i, i + waterWidth, gateBlocks(x, z) ? sill : -Infinity)
            : 0;
        xFlow[i] = a;
        zFlow[i] = b;
        if (a) outgoing[a > 0 ? i : i + 1] += Math.abs(a);
        if (b) outgoing[b > 0 ? i : i + waterWidth] += Math.abs(b);
      }
    for (let i = 0; i < waterCount; i++)
      limiter[i] = outgoing[i] > 0 ? Math.min(1, depth[i] / (outgoing[i] * dt)) : 1;
    for (let i = 0; i < waterCount; i++) {
      const a = xFlow[i],
        b = zFlow[i];
      if (a) {
        const transfer = a * dt * limiter[a > 0 ? i : i + 1];
        depth[i] -= transfer;
        depth[i + 1] += transfer;
      }
      if (b) {
        const transfer = b * dt * limiter[b > 0 ? i : i + waterWidth];
        depth[i] -= transfer;
        depth[i + waterWidth] += transfer;
      }
    }
    for (let x = channelStart; x < channelEnd; x++) {
      const i = index(x, waterLength - 1);
      const release = external ? Math.min(depth[i], depth[i] * dt * 6.8) : 0;
      depth[i] -= release;
      outlet[x - channelStart] = (release * cell * cell) / dt;
    }
    if (external) {
      // Empty columns outside the island drain out of the diorama.
      for (let i = 0; i < waterCount; i++) {
        if (bed[i] !== origin[1] || depth[i] <= 0) continue;
        const falling = Math.min(depth[i], depth[i] * dt * 11);
        depth[i] -= falling;
      }
    }
  }

  function speedAt(x: number, z: number) {
    const i = index(x, z);
    if (depth[i] <= wet) return 0;
    const east = xFlow[i];
    const west = x > 0 ? xFlow[i - 1] : 0;
    const north = z > 0 ? zFlow[i - waterWidth] : 0;
    const south =
      z === waterLength - 1 && x >= channelStart && x < channelEnd
        ? outlet[x - channelStart] / (cell * cell)
        : zFlow[i];
    // Convert depth transfer per second into horizontal speed at the column.
    return (Math.hypot((east + west) * 0.5, (north + south) * 0.5) * cell) / Math.max(0.04, depth[i]);
  }

  function measure() {
    return {
      upstream: depth[index(channelStart + 1, gateEdge - 1)],
      discharge: outlet.reduce((total, flow) => total + flow, 0) * 1000,
    };
  }

  return {
    dispose,
    width: waterWidth,
    length: waterLength,
    count: waterCount,
    dt,
    wet,
    gateEdge,
    channelStart,
    channelEnd,
    bed,
    depth,
    outlet,
    index,
    gateBlocks,
    step,
    measure,
    speedAt,
    setGate(closed: boolean) {
      gateClosed = closed;
    },
    get gateClosed() {
      return gateClosed;
    },
  };
}

export type WaterSimulation = ReturnType<typeof createWaterSimulation>;
