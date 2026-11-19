import type { CityCharacter, CityPosition } from './city-characters.ts';
import { createTacticsMap } from './tactics-movement.ts';

const surfaceY = 1.35;
const upperY = 2.70;
const towerLanding = { id: 'tower-landing', x: 12.15, z: -13.5, width: 5.2, depth: 3.5, y: 3.84 };

export const gardenBuildings = {
  hall: { surface: 'court', x: 0, z: -2.1, width: 7.6, depth: 5.45 },
  pavilion: { surface: 'court', x: 5.05, z: 5.15, width: 3.65, depth: 3.65 },
  gallery: { surface: 'west', x: -13.2, z: -1.1, width: 2.2, depth: 11.8 },
  tower: { surface: 'tower-landing', x: 12.15, z: -13.58, width: 3.42, depth: 2.82 },
  gate: { x: 0, z: 13.2, width: 6.1 },
  wall: { x: -0.75, z: -13.4, width: 15.8 },
} as const;

export const gardenPlanting = [
  { surface: 'west', x: -13.1, z: -8.14, width: 1.85, depth: 1.65, size: 2.4, kind: 'ginkgo' },
  { surface: 'west', x: -13.7, z: 7.4, width: 1.85, depth: 2.15, size: 2.65, kind: 'willow' },
  { surface: 'east', x: 13.5, z: 7.9, width: 1.95, depth: 1.95, size: 3.2, kind: 'pine' },
  { surface: 'east', x: 13.1, z: -5.5, width: 1.8, depth: 1.8, size: 2.55, kind: 'broadleaf' },
  { surface: 'south', x: -10.5, z: 13.1, width: 1.9, depth: 1.7, size: 2.3, kind: 'ginkgo' },
  { surface: 'south', x: 9.7, z: 13.1, width: 1.9, depth: 1.75, size: 2.6, kind: 'pine' },
  { surface: 'north', x: -12.7, z: -13.2, width: 3.55, depth: 3.15, size: 3.1, kind: 'pine' },
  { surface: 'north', x: 6.9, z: -11.45, width: 1.55, depth: 1.35, size: 2.45, kind: 'maple' },
  { surface: 'court', x: -5.15, z: 4.6, width: 2, depth: 1.9, size: 2.25, kind: 'pine' },
] as const;

const { hall, pavilion, gallery, tower, gate, wall } = gardenBuildings;
export const mewGarden = {
  surfaces: [
    { id: 'court', x: 0, z: 0, width: 16, depth: 16, y: upperY, columns: 12, rows: 12 },
    { id: 'north', x: 0, z: -12.875, width: 31, depth: 4.75, y: upperY, columns: 19, rows: 4 },
    { id: 'south', x: 0, z: 12.875, width: 28.5, depth: 4.75, y: surfaceY, columns: 18, rows: 4 },
    { id: 'west', x: -12.8, z: 0, width: 4.6, depth: 21, y: upperY, columns: 4, rows: 14 },
    { id: 'east', x: 12.8, z: 0, width: 4.6, depth: 21, y: surfaceY, columns: 4, rows: 14 },
    { id: 'north-bridge', x: 0, z: -9.25, width: 2.5, depth: 2.5, y: upperY },
    { id: 'west-bridge', x: -9.25, z: 1.8, width: 2.5, depth: 2.25, y: upperY },
    towerLanding,
  ],
  blockers: [
    hall, pavilion, gallery, tower, ...gardenPlanting,
    { ...towerLanding, surface: 'north' },
    ...[-1, 1].map((side) => ({ surface: 'south', x: gate.x + side * 2.2, z: gate.z, width: 0.55, depth: 0.55 })),
    ...[-1, 1].map((side) => ({ surface: 'north', x: wall.x + side * 4.65, z: wall.z, width: 6.5, depth: 0.44 })),
  ],
  stairs: [
    { id: 'gate-steps', from: 'south', to: 'court', x: 0, z: 10.5, width: 3.2, run: 2.5, direction: 'north', steps: 6 },
    { id: 'pond-steps', from: 'east', to: 'court', x: 10.5, z: 0.55, width: 2.65, run: 2.5, direction: 'west', steps: 6 },
    { id: 'gallery-steps', from: 'south', to: 'west', x: -12.65, z: 12.2, width: 2.25, run: 1.7, direction: 'north', steps: 6 },
    { id: 'wall-steps', from: 'east', to: 'north', x: 12.65, z: -8.8, width: 2.4, run: 1.7, direction: 'north', steps: 6 },
    { id: 'tower-steps', from: 'north', to: towerLanding.id, x: 12.15, z: -10.76, width: 2.12, run: 0.99, direction: 'north', steps: 5 },
  ] as const,
};

type GardenStop = { target: CityPosition; zoom: number };
export const gardenStops = {
  garden: { target: [0, 1.72, 0], zoom: 0.86 },
  hall: { target: [0, 4.4, -1], zoom: 1.92 },
  pavilion: { target: [5.3, 3.8, 5.3], zoom: 2.08 },
  gallery: { target: [-12.1, 3.85, -0.65], zoom: 1.85 },
  wall: { target: [-0.2, 4.3, -12.5], zoom: 1.8 },
  gate: { target: [0, 2.85, 12.15], zoom: 1.98 },
  tower: { target: [11.7, 8.15, -12.4], zoom: 1.8 },
} satisfies Record<string, GardenStop>;

export function gardenCast(): CityCharacter[] {
  const map = createTacticsMap(mewGarden);
  const residents = [
    ['daemon', 'Merchant', [0, 4], [3, -6]],
    ['agent', 'Research', [3, 5.7], [-11.25, 0.65]],
    ['provider', 'Write', [-11.5, -6], [-5.5, 1.2]],
    ['tui', 'Code', [-4.3, -10.95], [-3.4, -5.85]],
    ['web', 'Browse', [11.7, 2.4], [7.4, -3.4]],
    ['desktop', 'Warden', [10, -13.8], [11.65, 4.55]],
    ['mobile', 'Ferrykeeper', [-4.2, 12.1], [1.8, 5.9]],
  ] as const;
  return residents.map(([id, outfit, from, to], index) => {
    const nearest = ([x, z]: readonly [number, number]) => map.tiles.reduce((best, tile) =>
      Math.hypot(tile.x - x, tile.z - z) < Math.hypot(best.x - x, best.z - z) ? tile : best);
    const points = map.plan(nearest(from), 100).pathTo(nearest(to));
    if (!points || points.length < 2) throw new Error(`Missing garden route for ${id}.`);
    const [start, ...rest] = points.map(({ x, y, z }): CityPosition => [x, y, z]);
    return { id, outfit, path: [start, ...rest], speed: 1.18 + index * 0.12, phase: index * 0.83 };
  });
}
