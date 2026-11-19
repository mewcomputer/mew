import type * as Three from 'three';

type ReedPatch = {
  x: number;
  y: number;
  z: number;
  width: number;
  depth: number;
  height: number;
  seed: number;
};

const bladeProfile = [[0, 0.36], [0.24, 1], [0.52, 0.88], [0.76, 0.52]];
const spill = 0.12;
const bladeTints = ['#435934', '#57663a', '#69793f', '#7d8245', '#8d8950'];

function random(seed: number) {
  const value = Math.sin(seed * 127.1 + 81.7) * 43758.5453;
  return value - Math.floor(value);
}

export function mountCityReeds(T: typeof Three, scene: Three.Scene, patches: readonly ReedPatch[]) {
  if (!patches.length) return;

  const positions: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  const palette = bladeTints.map(tint => new T.Color(tint));
  const stalk = new T.Color('#686c3a');
  const seedHead = new T.Color('#614634');

  function vertex(x: number, y: number, z: number, tint: Three.Color, shade: number) {
    positions.push(x, y, z);
    colors.push(tint.r * shade, tint.g * shade, tint.b * shade);
  }

  for (const patch of patches) {
    const area = patch.width * patch.depth;
    const bladeCount = Math.max(16, Math.round(area * 39));
    const stemCount = Math.max(3, Math.round(area * 6));

    for (let i = 0; i < bladeCount; i++) {
      const seed = patch.seed + i * 23;
      const x = patch.x + (random(seed) - 0.5) * patch.width * 0.86;
      const z = patch.z + (random(seed + 1) - 0.5) * patch.depth * 0.86;
      const angle = random(seed + 2) * 2 * Math.PI;
      const acrossX = Math.cos(angle);
      const acrossZ = Math.sin(angle);
      const height = patch.height * (0.42 + random(seed + 3) * 0.54);
      const halfWidth = Math.min(0.071, patch.height * 0.044) * (0.7 + random(seed + 4) * 0.49);
      const lean = (0.11 + random(seed + 5) * 0.15) * (random(seed + 6) < 0.5 ? -1 : 1);
      const maxX = patch.width / 2 + spill - Math.abs(x - patch.x);
      const maxZ = patch.depth / 2 + spill - Math.abs(z - patch.z);
      const spread = Math.min(1, (maxX - halfWidth) / Math.abs(acrossZ * lean),
        (maxZ - halfWidth) / Math.abs(acrossX * lean));
      const bendX = -acrossZ * lean * spread;
      const bendZ = acrossX * lean * spread;
      const color = palette[Math.floor(random(seed + 7) * palette.length)];
      const start = positions.length / 3;

      for (let section = 0; section < bladeProfile.length; section++) {
        const [level, taper] = bladeProfile[section];
        const half = halfWidth * taper;
        const fold = half * 0.41;
        const cx = x + bendX * level ** 2;
        const cz = z + bendZ * level ** 2;
        const y = patch.y + height * level;
        const shade = 0.65 + level * 0.43;

        vertex(cx - acrossX * half, y, cz - acrossZ * half, color, shade * 0.78);
        vertex(cx - acrossZ * fold, y, cz + acrossX * fold, color, shade);
        vertex(cx + acrossX * half, y, cz + acrossZ * half, color, shade * 0.92);
        if (section === 0) continue;
        const at = start + (section - 1) * 3;
        indices.push(at, at + 1, at + 4, at, at + 4, at + 3);
        indices.push(at + 1, at + 2, at + 5, at + 1, at + 5, at + 4);
      }
      const tip = positions.length / 3;
      vertex(x + bendX, patch.y + height, z + bendZ, color, 1.08);
      indices.push(tip - 3, tip - 2, tip, tip - 2, tip - 1, tip);
    }

    for (let i = 0; i < stemCount; i++) {
      const seed = patch.seed + 1873 + i * 29;
      const x = patch.x + (random(seed) - 0.5) * patch.width * 0.79;
      const z = patch.z + (random(seed + 1) - 0.5) * patch.depth * 0.79;
      const height = patch.height * (i === 0 ? 1 : 0.68 + random(seed + 4) * 0.31);
      const radius = 0.033 + random(seed + 5) * 0.018;
      const leanX = (random(seed + 2) - 0.5) * Math.min(0.27, 2 * (patch.width * 0.105 + spill - radius));
      const leanZ = (random(seed + 3) - 0.5) * Math.min(0.27, 2 * (patch.depth * 0.105 + spill - radius));
      const head = Math.min(0.27, height * 0.18);
      const segments = 6;
      const rings = [0, height - head, height - head * 0.85, height - head * 0.70, height - head * 0.24];
      const radii = [0.014, 0.009, radius * 0.76, radius, radius];
      const start = positions.length / 3;

      for (let ring = 0; ring < rings.length; ring++) {
        const atY = rings[ring];
        const atRadius = radii[ring];
        const cx = x + leanX * atY / height;
        const cz = z + leanZ * atY / height;
        const tint = ring < 2 ? stalk : seedHead;
        for (let side = 0; side < segments; side++) {
          const angle = side * 2 * Math.PI / segments;
          vertex(cx + Math.cos(angle) * atRadius, patch.y + atY, cz + Math.sin(angle) * atRadius,
            tint, 0.82 + 0.08 * ((side + i) % 3));
          if (ring === 0) continue;
          const lower = start + (ring - 1) * segments;
          const a = lower + side;
          const b = lower + (side + 1) % segments;
          indices.push(a, a + segments, b, b, a + segments, b + segments);
        }
      }
      const tip = positions.length / 3;
      vertex(x + leanX, patch.y + height, z + leanZ, seedHead, 0.9);
      for (let side = 0; side < segments; side++) {
        indices.push(tip - segments + side, tip, tip - segments + (side + 1) % segments);
      }
    }
  }

  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  const mesh = new T.Mesh(geometry,
    new T.MeshStandardMaterial({ vertexColors: true, side: T.DoubleSide, roughness: 0.88 }));
  mesh.name = 'canal-reeds';
  mesh.castShadow = mesh.receiveShadow = true;
  scene.add(mesh);
}
