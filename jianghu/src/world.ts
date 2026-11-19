export type Coordinates = readonly [number, number, number];
export type Chunk = readonly [column: number, row: number];

type WorldOptions = {
  size: Coordinates;
  origin: Coordinates;
  cell: number;
  chunkSize?: number;
};

type WorldListener = (x: number, y: number, z: number) => void;

export function createWorld({ size, origin, cell, chunkSize = 16 }: WorldOptions) {
  const [width, height, length] = size;
  const blocks = new Uint8Array(width * height * length);
  const chunkColumns = Math.ceil(width / chunkSize);
  const chunkRows = Math.ceil(length / chunkSize);
  const listeners = new Set<WorldListener>();
  const inside = (x: number, y: number, z: number) =>
    Number.isInteger(x) &&
    Number.isInteger(y) &&
    Number.isInteger(z) &&
    x >= 0 &&
    x < width &&
    y >= 0 &&
    y < height &&
    z >= 0 &&
    z < length;
  const index = (x: number, y: number, z: number) => (y * length + z) * width + x;

  function get(x: number, y: number, z: number) {
    return inside(x, y, z) ? blocks[index(x, y, z)] : 0;
  }

  function set(x: number, y: number, z: number, value: number): Chunk[] {
    if (!inside(x, y, z)) throw new RangeError('Voxel coordinates are outside the world.');
    if (!Number.isInteger(value) || value < 0 || value > 255)
      throw new RangeError('Invalid voxel material.');
    const i = index(x, y, z);
    if (blocks[i] === value) return [];
    blocks[i] = value;
    const affected: Chunk[] = [];
    const left = Math.max(0, Math.floor((x - 1) / chunkSize));
    const right = Math.min(chunkColumns - 1, Math.floor((x + 1) / chunkSize));
    const back = Math.max(0, Math.floor((z - 1) / chunkSize));
    const front = Math.min(chunkRows - 1, Math.floor((z + 1) / chunkSize));
    for (let cx = left; cx <= right; cx++) {
      for (let cz = back; cz <= front; cz++) affected.push([cx, cz]);
    }
    for (const listener of listeners) {
      try {
        listener(x, y, z);
      } catch (error) {
        console.error('Jianghu world change observer failed', error);
      }
    }
    return affected;
  }

  return {
    size,
    origin,
    cell,
    chunkSize,
    chunkColumns,
    chunkRows,
    get,
    set,
    inside,
    onChange(listener: WorldListener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

export type World = ReturnType<typeof createWorld>;

const corners = [
  [0, 0],
  [1, 0],
  [1, 1],
  [0, 1],
];
const splitAcross = [0, 1, 3, 1, 2, 3];
const splitAlong = [0, 1, 2, 0, 2, 3];

type MeshColor = { r: number; g: number; b: number };

export function meshChunk(world: World, cx: number, cz: number, palette: readonly MeshColor[]) {
  const { size, origin, cell, chunkSize, get } = world;
  const positions: number[] = [];
  const normals: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  const lowerX = cx * chunkSize,
    lowerZ = cz * chunkSize;
  const upperX = Math.min(lowerX + chunkSize, size[0]);
  const upperZ = Math.min(lowerZ + chunkSize, size[2]);
  const block = [0, 0, 0],
    edge = [0, 0, 0],
    neighbor: [number, number, number] = [0, 0, 0];
  const occlusion = [0, 0, 0, 0];

  for (let y = 0; y < size[1]; y++) {
    for (let z = lowerZ; z < upperZ; z++) {
      for (let x = lowerX; x < upperX; x++) {
        const material = get(x, y, z);
        if (!material) continue;
        block[0] = x;
        block[1] = y;
        block[2] = z;
        const base = palette[material];
        for (let axis = 0; axis < 3; axis++) {
          const u = (axis + 1) % 3;
          const v = (axis + 2) % 3;
          for (const sign of [-1, 1]) {
            neighbor[0] = x;
            neighbor[1] = y;
            neighbor[2] = z;
            neighbor[axis] += sign;
            if (get(...neighbor)) continue;
            const first = positions.length / 3;
            for (let corner = 0; corner < corners.length; corner++) {
              const [du, dv] = corners[corner];
              edge[axis] = block[axis] + (sign > 0 ? 1 : 0);
              edge[u] = block[u] + du;
              edge[v] = block[v] + dv;
              positions.push(
                origin[0] + edge[0] * cell,
                origin[1] + edge[1] * cell,
                origin[2] + edge[2] * cell,
              );
              normals.push(axis === 0 ? sign : 0, axis === 1 ? sign : 0, axis === 2 ? sign : 0);

              neighbor[u] = block[u] + (du ? 1 : -1);
              neighbor[v] = block[v];
              const sideU = Number(Boolean(get(...neighbor)));
              neighbor[u] = block[u];
              neighbor[v] = block[v] + (dv ? 1 : -1);
              const sideV = Number(Boolean(get(...neighbor)));
              neighbor[u] = block[u] + (du ? 1 : -1);
              const diagonal = Number(Boolean(get(...neighbor)));
              const dark = sideU && sideV ? 3 : sideU + sideV + diagonal;
              const shade = 1 - dark * 0.13;
              occlusion[corner] = dark;
              colors.push(base.r * shade, base.g * shade, base.b * shade);
            }
            // Split toward the brighter corners to avoid a visible diagonal in the face lighting.
            const split =
              occlusion[0] + occlusion[2] > occlusion[1] + occlusion[3] ? splitAcross : splitAlong;
            for (let i = 0; i < 6; i += 3) {
              indices.push(
                first + split[i],
                first + split[i + (sign > 0 ? 1 : 2)],
                first + split[i + (sign > 0 ? 2 : 1)],
              );
            }
          }
        }
      }
    }
  }
  return { positions, normals, colors, indices };
}
