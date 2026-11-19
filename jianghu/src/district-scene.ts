import type * as Three from 'three';
import { random, type District, type LandmarkId } from './district.ts';

type Box = [number, number, number, number, number, number, Three.ColorRepresentation, number];

export function createDistrictScene(
  T: typeof Three, scene: Three.Scene, { landmarks: stations, layout }: District,
) {
  const unit = new T.BoxGeometry(1, 1, 1);
  const dummy = new T.Object3D();
  const color = new T.Color();
  const solids: Box[] = [];
  const ornaments: Box[] = [];
  const lamps: Box[] = [];
  function box(
    x: number, y: number, z: number, w: number, h: number, d: number,
    tint: Three.ColorRepresentation, group = solids, angle = 0,
  ) {
    group.push([x, y, z, w, h, d, tint, angle]);
  }
  function flushBoxes(items: Box[], material: Three.Material, shadow: boolean) {
    const result = new T.InstancedMesh(unit, material, items.length);
    items.forEach(([x, y, z, w, h, d, tint, angle], index) => {
      dummy.position.set(x, y + h * 0.5, z);
      dummy.scale.set(w, h, d);
      dummy.rotation.set(0, angle, 0);
      dummy.updateMatrix();
      result.setMatrixAt(index, dummy.matrix);
      result.setColorAt(index, color.set(tint));
    });
    result.receiveShadow = shadow;
    result.castShadow = shadow;
    scene.add(result);
    return result;
  }
  const pale = '#bab9a2',
    trim = '#3a5a50',
    brick = '#a24b39',
    metal = '#596f67',
    roofColor = '#3f796b',
    ceramic = '#ded6b9',
    jade = '#8ee0bb',
    bronze = '#b67a46';
  function roof(x: number, y: number, z: number, w: number, d: number, h: number) {
    const steps = 8;
    for (let i = 0; i < steps; i++) {
      const f = i / (steps - 1);
      const inset = 0.74 * f;
      box(
        x,
        y + (i * h) / steps,
        z,
        w * (1 - inset),
        h / steps + 0.018,
        d * (1 - 0.52 * f),
        i % 2 ? '#478373' : roofColor,
      );
    }
    box(x, y - 0.05, z, w + 0.13, 0.082, d + 0.14, '#244d43');
    box(x, y + h, z, w * 0.34, 0.1, 0.13, '#8eb3a0');
    for (const a of [-1, 1])
      for (const b of [-1, 1]) {
        box(x + a * (w * 0.5 - 0.11), y - 0.045, z + b * (d * 0.5 - 0.13), 0.22, 0.21, 0.25, '#7daa94');
        box(x + a * (w * 0.5 + 0.025), y + 0.1, z + b * (d * 0.5 - 0.045), 0.16, 0.08, 0.16, '#a7bc9b');
      }
    for (let k = 0; k < 17; k++) {
      const off = (k - 8) * w * 0.05;
      for (let i = 0; i < steps - 1; i++) {
        const f = i / (steps - 1);
        const width = w * (1 - 0.74 * f);
        if (Math.abs(off) > width * 0.47) continue;
        const ext = d * (1 - 0.52 * f) * 0.5;
        for (const sign of [-1, 1]) {
          box(
            x + off,
            y + (i * h) / steps + h / steps,
            z + sign * (ext - 0.028),
            0.023,
            0.021,
            0.096,
            '#699786',
            ornaments,
          );
        }
      }
    }
  }

  function pavilion(id: LandmarkId) {
    const { x, y, z } = stations[id];
    const tall = id === 'north';
    const modern = id === 'east';
    const h = tall ? 1.75 : 1.3;
    box(x, y - 0.06, z, layout.pavilion.width, 0.16, layout.pavilion.depth, pale);
    box(x, y + 0.1, z, 1.21, h, 1.05, pale);
    for (const sx of [-1, 1])
      for (const sz of [-1, 1]) box(x + sx * 0.6, y + 0.1, z + sz * 0.53, 0.116, h, 0.123, brick);
    for (const sign of [-1, 1]) {
      box(x, y + 0.43, z + sign * 0.535, 0.87, 0.7, 0.033, trim);
      box(x, y + 0.49, z + sign * 0.56, 0.74, 0.57, 0.014, '#8ccaa7', lamps);
      for (let i = -2; i <= 2; i++)
        box(x + i * 0.145, y + 0.48, z + sign * 0.572, 0.025, 0.59, 0.018, trim);
    }
    for (const sign of [-1, 1]) {
      box(x + sign * 0.61, y + 0.39, z, 0.038, 0.72, 0.75, trim);
      for (let i = -2; i <= 2; i++)
        box(x + sign * 0.635, y + 0.46, z + i * 0.134, 0.018, 0.57, 0.078, '#7fae95', lamps);
    }
    box(x, y + h + 0.08, z, 1.44, 0.13, 1.25, metal);
    if (modern) {
      box(x, y + h + 0.2, z, 1.87, 0.16, 1.68, '#23473e');
      box(x, y + h + 0.28, z, 1.8, 0.035, 1.6, jade, lamps);
      box(x, y + h + 0.36, z, 1.64, 0.1, 1.46, ceramic);
      box(x, y + h + 0.46, z, 1.34, 0.038, 1.08, '#315f5a');
      for (let i = -3; i <= 3; i++)
        box(x + i * 0.19, y + h + 0.5, z, 0.021, 0.012, 1.1, '#95ad9b', ornaments);
      box(x, y + h + 0.5, z, 1.34, 0.012, 0.025, '#95ad9b', ornaments);
      box(x + 0.49, y + h + 0.49, z - 0.42, 0.27, 0.11, 0.29, ceramic);
      box(x + 0.49, y + h + 0.57, z - 0.42, 0.08, 0.65, 0.1, bronze);
      box(x + 0.49, y + h + 1.04, z - 0.42, 0.53, 0.07, 0.1, '#284b43');
      box(x + 0.49, y + h + 1.11, z - 0.42, 0.39, 0.04, 0.084, jade, lamps);
    } else {
      roof(x, y + h + 0.21, z, 1.82, 1.59, tall ? 0.58 : 0.49);
    }
    if (tall) {
      box(x, y + h + 0.82, z, 0.12, 0.55, 0.12, metal);
      box(x, y + h + 1.32, z, 0.48, 0.07, 0.08, brick);
      box(x, y + h + 1.41, z, 0.095, 0.086, 0.095, '#f5c880', lamps);
    }
    const anchor = new T.Vector3(x, y + h + 0.3, z + 0.68);
    const ring = new T.Mesh(
      new T.RingGeometry(0.79, 0.85, 32),
      new T.MeshBasicMaterial({ color: '#f3bd77', side: T.DoubleSide, transparent: true, opacity: 0.92 }),
    );
    ring.position.set(x, y + 0.017, z);
    ring.rotation.x = -Math.PI / 2;
    scene.add(ring);
    const marker = new T.Mesh(
      new T.BoxGeometry(0.16, 0.16, 0.16),
      new T.MeshBasicMaterial({ color: '#ffd795' }),
    );
    marker.position.set(x, y + h + (tall ? 1.54 : 0.79), z);
    scene.add(marker);
    return { ...stations[id], anchor, ring, marker };
  }
  const landmarks = {
    west: pavilion('west'),
    north: pavilion('north'),
    east: pavilion('east'),
  } satisfies Record<LandmarkId, ReturnType<typeof pavilion>>;

  const { z: sz, pierOffset, pierWidth, depth } = layout.hall;
  for (const sign of [-1, 1]) {
    const x = sign * pierOffset;
    box(x, 0.18, sz, 1.2, 3.48, 1.72, ceramic);
    box(x, 0.13, sz, pierWidth, 0.34, depth, trim);
    box(x, 1.53, sz, 1.3, 0.17, 1.82, metal);
    box(x, 3.38, sz, 1.36, 0.24, 1.92, brick);
    for (const face of [-1, 1]) {
      for (let i = -2; i <= 2; i++) {
        box(x + i * 0.192, 1.85, sz + face * 0.872, 0.118, 1.18, 0.043, trim);
        box(x + i * 0.192, 1.97, sz + face * 0.9, 0.054, 1.01, 0.022, '#a7d4ad', lamps);
      }
      box(x + face * 0.614, 1.84, sz, 0.042, 1.17, 1.3, metal);
      for (const offset of [-0.44, 0, 0.44]) {
        box(x + face * 0.641, 1.98, sz + offset, 0.022, 0.88, 0.075, jade, lamps);
      }
    }
  }
  for (let i = -4; i <= 4; i++) box(i * 0.114, 0.29, sz + 0.55, 0.047, 2.59, 0.092, trim);
  box(0, 2.12, sz + 0.58, 1.48, 0.28, 0.38, brick);
  box(0, 2.37, sz + 0.79, 0.93, 0.069, 0.035, '#e7ac67', lamps);
  box(0, 3.6, sz, 3.9, 0.42, 2.13, trim);
  for (let i = -9; i <= 9; i++) {
    const x = i * 0.182;
    for (const sign of [-1, 1]) {
      box(x, 3.75, sz + sign * 1.089, 0.111, 0.16, 0.07, i % 3 ? '#6b9180' : '#d79d72');
      box(x, 3.95, sz + sign * 1.029, 0.086, 0.16, 0.12, brick);
    }
  }
  roof(0, 4.0, sz, 4.46, 2.58, 0.58);
  box(0, 3.98, sz - 1.35, 4.36, 0.036, 0.042, jade, lamps);
  box(0, 3.98, sz + 1.35, 4.36, 0.036, 0.042, jade, lamps);
  for (const sign of [-1, 1]) {
    const x = sign * 0.98;
    box(x, 4.48, sz, 0.64, 0.22, 0.87, '#284b43');
    box(x, 4.63, sz, 0.42, 0.64, 0.62, brick);
    box(x, 5.24, sz, 0.52, 0.11, 0.71, ceramic);
    box(x, 4.79, sz + 0.32, 0.23, 0.24, 0.035, '#e7ab63', lamps);
  }
  const crownY = 5.71;
  const crownZ = sz + 0.06;
  box(0, crownY - 0.04, crownZ, 2.16, 0.08, 0.11, bronze);
  box(0, crownY - 0.14, crownZ, 0.46, 0.31, 0.54, '#284b43');
  box(0, crownY - 0.082, crownZ + 0.29, 0.34, 0.19, 0.09, ceramic);
  box(0, crownY - 0.043, crownZ + 0.345, 0.24, 0.11, 0.04, jade, lamps);

  const bridge = layout.bridge;
  const deckY = bridge.surface - 0.068;
  box(0, deckY - 0.22, bridge.z, bridge.width, 0.22, bridge.depth, pale);
  for (let i = -5; i <= 5; i++) {
    box(i * (bridge.width - 0.27) / 10, deckY, bridge.z, 0.18, 0.068, bridge.depth - 0.04, pale);
  }
  for (const side of [-1, 1]) {
    const z = bridge.z + side * (bridge.depth / 2 - 0.02);
    for (const offset of [-1, -0.5, 0, 0.5, 1]) {
      box(offset * (bridge.width / 2 - 0.11), deckY + 0.02, z, 0.069, 0.45, 0.08, brick);
    }
    box(0, deckY + 0.41, z, bridge.width - 0.08, 0.075, 0.059, brick);
    box(0, deckY + 0.16, z, bridge.width - 0.15, 0.045, 0.043, metal);
  }
  for (const sign of [-1, 1]) {
    // The service conduits run along the canal wall to the spillway collars.
    box(sign * 0.53, 0.73, 3.57, 0.105, 0.13, 3.04, '#223c36');
    box(sign * 0.485, 0.789, 3.57, 0.025, 0.032, 3.07, jade, lamps);
    box(sign * 0.56, -1.02, 5.085, 0.17, 1.92, 0.16, '#415e52');
    box(sign * 0.56, -0.89, 5.18, 0.045, 1.65, 0.028, jade, lamps);
    for (let j = 0; j < 5; j++) {
      const z = 2.12 + j * 0.61;
      box(sign * 0.53, 0.65, z, 0.16, 0.26, 0.11, bronze);
      box(sign * 0.62, 0.97, z, 0.082, 0.2, 0.077, metal);
      box(sign * 0.62, 1.14, z, 0.074, 0.046, 0.073, '#ddaa72', lamps);
      box(sign * 0.57, 0.83, z, 0.035, 0.055, 0.34, '#8c9b86');
    }
  }

  function pine(x: number, y: number, z: number, seed: number) {
    const h = 0.72 + random(x, z, seed) * 0.61;
    box(x, y, z, 0.095, h, 0.09, '#735f4c');
    for (let i = 0; i < 11; i++) {
      const a = random(x, z, i + 9) * Math.PI * 2;
      const r = random(x, z, i + 51) * 0.4;
      const w = 0.26 + random(x, z, i + 29) * 0.28;
      const tint = ['#35624b', '#47785b', '#638663', '#5b886b'][i % 4];
      box(
        x + Math.cos(a) * r,
        y + h * 0.51 + i * 0.047,
        z + Math.sin(a) * r,
        w,
        0.16,
        w * 0.73,
        tint,
        solids,
        a,
      );
    }
  }
  [
    [-5.6, 1.13, 0.2],
    [-4.7, 1.14, 2.98],
    [-4.7, 1.41, -2.7],
    [-2.22, 0.89, 4.6],
    [-1.18, 1.1, 3.2],
    [2.0, 1.39, -3.8],
    [4.54, 1.43, -2.8],
    [5.4, 1.14, 1.9],
    [4.0, 1.13, 3.9],
    [2.16, 0.89, 4.75],
    [-4.27, 1.12, -0.32],
  ].forEach(([x, y, z], i) => pine(x, y, z, i));
  [
    [-1.8, 1.13, 1.25],
    [2.6, 1.13, 2.09],
    [-2.9, 1.14, 0.62],
    [3.55, 1.13, 1.94],
  ].forEach(([x, y, z]) => {
    box(x, y, z, 0.25, 0.23, 0.27, '#9d7652');
    for (const i of [-1, 1]) box(x + i * 0.086, y + 0.23, z, 0.031, 0.011, 0.27, '#604f42', ornaments);
  });
  for (const side of [-1, 1]) {
    for (let i = 0; i < 28; i++) {
      const z = 1.65 + random(i, side, 5) * 3.5;
      const x = side * (0.54 + random(i, side, 9) * 0.24);
      const h = 0.08 + random(i, side, 3) * 0.23;
      box(x, 0.96, z, 0.023, h, 0.026, i % 2 ? '#81966b' : '#abc286', ornaments, random(i, side, 2));
    }
  }
  const architecture = flushBoxes(solids, new T.MeshStandardMaterial({ roughness: 0.87 }), true);
  flushBoxes(ornaments, new T.MeshStandardMaterial({ roughness: 0.9 }), false);
  flushBoxes(lamps, new T.MeshBasicMaterial(), false);

  // The relay crown pairs a moon gate silhouette with a segmented ceramic housing.
  const crown = new T.Group();
  crown.position.set(0, crownY, crownZ);
  const housing = new T.Mesh(
    new T.TorusGeometry(1.18, 0.22, 4, 16),
    new T.MeshStandardMaterial({ color: ceramic, roughness: 0.52 }),
  );
  housing.scale.z = 1.9;
  housing.castShadow = housing.receiveShadow = true;
  crown.add(housing);
  const lining = new T.Mesh(
    new T.TorusGeometry(1.03, 0.057, 4, 16),
    new T.MeshStandardMaterial({ color: '#417f68', roughness: 0.5, emissive: '#376451' }),
  );
  lining.scale.z = 6.3;
  crown.add(lining);
  const ribs = new T.InstancedMesh(
    unit,
    new T.MeshStandardMaterial({ color: bronze, metalness: 0.42, roughness: 0.48 }),
    8,
  );
  for (let i = 0; i < ribs.count; i++) {
    const angle = (i * Math.PI) / 4;
    dummy.position.set(Math.cos(angle) * 1.19, Math.sin(angle) * 1.19, 0);
    dummy.rotation.set(0, 0, angle);
    dummy.scale.set(0.43, 0.10, 0.52);
    dummy.updateMatrix();
    ribs.setMatrixAt(i, dummy.matrix);
  }
  ribs.castShadow = ribs.receiveShadow = true;
  crown.add(ribs);
  architecture.add(crown);

  return { architecture, landmarks };
}
