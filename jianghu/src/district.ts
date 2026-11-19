import { createWorld } from './world.ts';

export const districtCast = [
  { id: 'lin', role: 'Research', position: [-2.3, 1.35, -1.8] },
  { id: 'mei', role: 'Code', position: [-2.75, 1.1, 2.45] },
  { id: 'shen', role: 'Browse', position: [3.25, 1.12, 2.25] },
  { id: 'yu', role: 'Write', position: [2.7, 1.34, -1.8] },
] as const;

const map = [
  '  2233~223   ',
  ' 23333~23332 ',
  ' 23332~23332 ',
  '223222~223332',
  '222222~233322',
  '122222~222222',
  '122222~222221',
  '112212~221211',
  ' 11212~21111 ',
  '  1112~2111  ',
  '   112~211   ',
];

export const materials = [
  '#000000',
  '#5f7068',
  '#667970',
  '#52655e',
  '#68766b',
  '#586b62',
  '#75896a',
  '#81906c',
  '#688470',
  '#6b8268',
  '#859373',
  '#b2b2a0',
  '#a7aa96',
  '#bab7a2',
  '#999f91',
  '#81998a',
  '#4b8072',
  '#a75743',
];

export const brushes = { stone: 15, jade: 16, cinnabar: 17 };

export function random(x: number, y: number, seed: number) {
  const value = Math.sin(x * 127.1 + y * 311.7 + seed * 73.3) * 43758.5453;
  return value - Math.floor(value);
}

const paved = (column: number, row: number) =>
  (row === 6 && column >= 2 && column <= 10) ||
  (column === 3 && row >= 2 && row <= 6) ||
  (column === 10 && row >= 4 && row <= 6) ||
  (row === 3 && column >= 3 && column <= 5) ||
  (row === 4 && column >= 8 && column <= 10) ||
  (column === 4 && row >= 6 && row <= 8);

export function createDistrict() {
  const world = createWorld({ origin: [-6.5, -2.4, -5.5], size: [52, 39, 44], cell: 0.25 });
  map.forEach((row, j) =>
    [...row].forEach((tile, i) => {
      if (tile === ' ') return;
      const water = tile === '~';
      const surface = water ? -0.15 - Math.min(3, Math.floor(j / 3)) * 0.25 : 0.65 + Number(tile) * 0.24;
      const height = Math.round((surface - world.origin[1]) / world.cell);
      for (let dx = 0; dx < 4; dx++) {
        for (let dz = 0; dz < 4; dz++) {
          const x = i * 4 + dx,
            z = j * 4 + dz;
          const path = paved(i, j);
          for (let y = 0; y < height; y++) {
            const variant = random(x, z, y);
            let material = 1 + Math.floor(variant * 5);
            if (y === height - 1 && !water) {
              material = path
                ? 11 + Math.floor(random(i * 2 + Number(dx > 1), j * 2 + Number(dz > 1), 18) * 4)
                : 6 + Math.floor(variant * 5);
            }
            world.set(x, y, z, material);
          }
        }
      }
    }),
  );
  return {
    world,
    hydraulics: { gateEdge: 15, channelStart: 24, channelEnd: 28, sill: 0.55, inletLevel: 0.5 },
    layout: {
      bridge: { z: 1.01, width: 2.18, depth: 0.9, surface: 1.128 },
      hall: { z: -2.3, pierOffset: 1.18, pierWidth: 1.34, depth: 1.9 },
      pavilion: { width: 1.48, depth: 1.38 },
    },
    landmarks: {
      west: {
        title: 'West Workshop',
        copy: 'The workshop terrace sits above the water on the west bank.',
        x: -3.6,
        z: 1.6,
        y: 1.14,
      },
      north: {
        title: 'North Relay',
        copy: 'The relay tower overlooks the head of the canal.',
        x: -2.85,
        z: -3.5,
        y: 1.39,
      },
      east: {
        title: 'East Pavilion',
        copy: 'A glazed pavilion across the bridge from the workshop.',
        x: 4.05,
        z: 0.42,
        y: 1.16,
      },
    },
  };
}

export type District = ReturnType<typeof createDistrict>;
export type LandmarkId = keyof District['landmarks'];
