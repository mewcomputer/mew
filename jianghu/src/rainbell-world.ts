import { shopFootprint } from './city-shops.ts';
import type { CityPosition } from './city-characters.ts';

const square = { id: 'square', x: -17.6, y: 0.55, z: 49.45, width: 19.6, depth: 13.6, seed: 211 };
const westLanding = { id: 'west-landing', x: -15.6, y: 0.91, z: 39.7, width: 16.4, depth: 6.4, seed: 163 };
const eastLanding = { id: 'east-landing', x: 13.35, y: 0.91, z: 39.95, width: 10.3, depth: 5.45, seed: 171 };
const garden = { id: 'garden', x: -32.05, y: 1.65, z: 50.3, width: 9.4, depth: 12.3 };

export const rainbellSite: CityPosition = [square.x, square.y, square.z];
const pavingWest = westLanding.x + westLanding.width / 2;
const pavingEast = eastLanding.x - eastLanding.width / 2;
export const rainbellBridge = {
  west: -9.0, east: 9.4, y: westLanding.y, z: 41.55, width: 2.45,
  pavingWest, pavingEast, columns: Math.max(1, Math.round((pavingEast - pavingWest) / 0.92)),
};
export const rainbellDecks = [westLanding, eastLanding, square];
const stairZ = square.z - square.depth / 2 + 1.37;
export const rainbellSteps = {
  id: 'landing-stairs', from: square.id, to: westLanding.id,
  x: -13.35, z: stairZ, run: stairZ - (westLanding.z + westLanding.depth / 2), width: 4.2,
  direction: 'north', steps: 3,
} as const;
export const rainbellGarden = {
  deck: garden,
  steps: {
    id: 'garden-stairs', from: square.id, to: garden.id,
    x: -25.6, z: 46.14, run: -25.6 - (garden.x + garden.width / 2), width: 1.86,
    direction: 'west', steps: 7,
  } as const,
  pathX: garden.x + 2.87,
  lookoutZ: 54.54,
};

const operaQuayNorth = eastLanding.z + eastLanding.depth / 2;
const operaCourtNorth = 63.225;
const operaQuay = {
  id: 'opera-quay', x: 26.3, y: eastLanding.y, z: (operaQuayNorth + operaCourtNorth) / 2,
  width: 29, depth: operaCourtNorth - operaQuayNorth, seed: 241,
};
const operaCourt = {
  id: 'opera-court', x: operaQuay.x, y: operaQuay.y, z: operaCourtNorth + 5.75,
  width: operaQuay.width, depth: 11.5, seed: 246,
};
const operaHall = { x: 24.6, z: 50.2, width: 18.2, depth: 14.5 };
const porchDepth = 2.55;
const operaPorch = {
  id: 'opera-porch', x: operaHall.x, y: 2.21, z: operaHall.z + (operaHall.depth + porchDepth) / 2,
  width: operaHall.width, depth: porchDepth,
};
const operaStairStart = 62.22;
const operaSteps = {
  id: 'opera-steps', from: operaQuay.id, to: operaPorch.id,
  x: operaHall.x, z: operaStairStart, run: operaStairStart - (operaPorch.z + operaPorch.depth / 2),
  width: 8.1, direction: 'north', steps: 7,
} as const;
const operaScreen = { x: 12.67, z: 50.95, width: 0.39, depth: 10.18 };
const operaRearCourt = {
  id: 'opera-rear-court', x: 31.8, y: operaQuay.y, z: (34.25 + operaQuayNorth) / 2,
  width: 18, depth: operaQuayNorth - 34.25, seed: 294,
};
const operaRearPassage = {
  id: 'opera-rear-passage', x: 20.65, y: operaQuay.y, z: (37.895 + operaQuayNorth) / 2,
  width: 4.3, depth: operaQuayNorth - 37.895, seed: 291,
};
const operaGarden = {
  id: 'opera-garden', x: 45.35, y: operaQuay.y, z: 45.05, width: 9.1, depth: 18.1, seed: 297,
};
export const operaHouse = {
  quay: operaQuay,
  court: operaCourt,
  porch: operaPorch,
  hall: operaHall,
  entrance: { x: operaHall.x, y: operaPorch.y, z: operaHall.z + operaHall.depth / 2, width: 2.04 },
  backstage: { x: operaHall.x + 6.55, y: operaRearCourt.y, z: operaHall.z - operaHall.depth / 2, width: 2.46 },
  rearCourt: operaRearCourt,
  rearPassage: operaRearPassage,
  garden: operaGarden,
  ticketBooth: { surface: operaQuay.id, x: 38.7, z: 56.9, width: 3.2, depth: 2.21 },
  gallery: { surface: operaRearCourt.id, x: 34.78, z: 35.45, width: 5.65, depth: 0.82 },
  pool: { x: 46.1, z: 43.0, width: 4.45, depth: 3.05 },
  gardenBench: { x: 46.85, z: 37.95, width: 3.59, depth: 1.15 },
  rearRail: { x: 34.75, z: 34.48, width: 11.78, depth: 0.18 },
  gardenRail: { x: operaGarden.x, z: 36.18, width: 8.78, depth: 0.18 },
  steps: operaSteps,
  stairCurbs: [-1, 1].map((side) => ({
    x: operaSteps.x + side * (operaSteps.width / 2 + 0.19),
    z: operaSteps.z - operaSteps.run / 2, width: 0.38, depth: operaSteps.run,
  })),
  facadePosts: Array.from({ length: 8 }, (_, column) => ({
    x: operaHall.x + (column - 3.5) * 2.37,
    z: operaHall.z + operaHall.depth / 2 + 0.06, width: 0.48, depth: 0.53,
  })),
  railing: { x: operaQuay.x - operaQuay.width / 2 + 0.31, z: 57.45, width: 0.19, depth: 28.75 },
  screen: operaScreen,
  screenPosts: [-4.8, -1.6, 1.6, 4.8].map((offset) => ({ ...operaScreen, z: operaScreen.z + offset, depth: 0.39 })),
  planters: [
    { surface: operaQuay.id, x: 14.60, z: 59.85, width: 2.1, depth: 1.95, tree: 'willow' },
    { surface: operaCourt.id, x: 38.0, z: 67.45, width: 3.5, depth: 4.5, tree: 'ginkgo' },
    { surface: operaCourt.id, x: 14.03, z: 71.65, width: 2.8, depth: 3.5, tree: 'pine' },
    { surface: operaRearCourt.id, x: 24.85, z: 36.28, width: 2.85, depth: 2.10, tree: 'ginkgo' },
    { surface: operaGarden.id, x: 46.9, z: 50.7, width: 3.26, depth: 2.47, shrub: 'flowering' },
    { surface: operaRearCourt.id, x: 35.6, z: 37.2, width: 4.3, depth: 1.45, shrub: 'mound' },
    { surface: operaGarden.id, x: 49.23, z: 43.95, width: 1.01, depth: 5.2, shrub: 'flowering' },
  ],
} as const;

export const rainbellProps = {
  bellFoundation: { x: square.x, z: square.z, width: 5.92, depth: 4.42 },
  shops: [
    { x: -24.5, z: 51.55, width: 3.68, depth: 2.77, tint: '#706a50', variant: 'market', sign: '雨具' },
    { x: -20.7, z: 54.15, width: 3.55, depth: 2.48, tint: '#4d645a', variant: 'station', sign: '渡券' },
  ] as const,
  planters: [[-24.8, 45.1, 2.34, 'willow'], [-10.4, 53.38, 2.65, 'ginkgo'], [-25.28, 54.6, 1.86, 'broadleaf']] as const,
  bambooPlanters: [
    { surface: square.id, x: -15.7, y: square.y, z: 52.65, width: 0.92, depth: 0.94, height: 2.16, seed: 151 },
    { surface: westLanding.id, x: -16.30, y: westLanding.y, z: 36.91, width: 0.90, depth: 0.82, height: 1.96, seed: 262 },
  ],
  reedBeds: [
    { x: -8.43, y: square.y + 0.16, z: 46.40, width: 1.00, depth: 1.29, height: 1.50, seed: 79 },
    { x: -8.46, y: square.y + 0.16, z: 48.38, width: 1.10, depth: 1.85, height: 2.21, seed: 125 },
    { x: -8.46, y: square.y + 0.16, z: 50.59, width: 1.10, depth: 1.49, height: 1.87, seed: 284 },
  ],
  benches: [[-22.95, 47.86], [-12.4, 50.4], [-14.1, 54.2]],
  stalls: [[-25.65, 47.49, '#a37554'], [-10.25, 48.52, '#628a77']] as const,
  pagoda: { x: garden.x - 1.12, z: garden.z + 0.02, width: 4.52, depth: 4.58 },
  gardenRocks: [
    [-34.94, 45.83, 1.72], [-32.52, 45.42, 0.84], [-30.96, 47.03, 0.56],
    [-35.64, 48.27, 1.26], [-35.82, 53.37, 1.36], [-33.29, 54.29, 1.14],
    [-31.86, 55.28, 0.71], [-28.11, 52.31, 0.61],
  ],
  gardenTrees: [
    [-35.92, 46.04, 3.05, 'pine'], [-32.48, 46.81, 2.69, 'goldenrain'],
    [-36.1, 53.27, 2.88, 'willow'], [-34.28, 54.96, 2.19, 'pine'],
  ] as const,
  maple: [-31.6, 54.88, 2.36] as const,
};

const { west, east, y, z, width, columns } = rainbellBridge;
const { bellFoundation, shops, planters, bambooPlanters, reedBeds, benches, stalls, pagoda, gardenRocks, gardenTrees, maple } = rainbellProps;
export const rainbellWorld = {
  movement: 5,
  surfaces: [
    ...rainbellDecks, garden, operaHouse.quay, operaHouse.court, operaHouse.porch,
    operaHouse.rearCourt, operaHouse.rearPassage, operaHouse.garden,
    { id: 'canal-bridge', x: (pavingWest + pavingEast) / 2, y, z,
      width: pavingEast - pavingWest, depth: width - 0.36, columns, rows: 1 },
  ],
  stairs: [rainbellSteps, rainbellGarden.steps, operaHouse.steps],
  blockers: [
    { surface: operaHouse.quay.id, ...operaHouse.hall },
    { ...operaHouse.porch, surface: operaHouse.quay.id },
    { surface: operaHouse.quay.id, ...operaHouse.railing },
    { surface: operaHouse.court.id, ...operaHouse.railing },
    { surface: operaHouse.quay.id, ...operaHouse.screen },
    operaHouse.ticketBooth,
    operaHouse.gallery,
    { surface: operaHouse.rearCourt.id, ...operaHouse.rearRail },
    ...[operaHouse.pool, operaHouse.gardenBench, operaHouse.gardenRail]
      .map((prop) => ({ surface: operaHouse.garden.id, ...prop })),
    ...operaHouse.stairCurbs.map((prop) => ({ surface: operaHouse.quay.id, ...prop })),
    ...operaHouse.planters,
    ...operaHouse.facadePosts.map((post) => ({ surface: operaHouse.porch.id, ...post })),
    { surface: square.id, ...bellFoundation },
    ...[-1, 1].flatMap((side) => {
      const railZ = z + side * (width / 2 - 0.12);
      return [
        { surface: westLanding.id, x: (west + pavingWest) / 2, z: railZ, width: pavingWest - west, depth: 0.12 },
        { surface: eastLanding.id, x: (east + pavingEast) / 2, z: railZ, width: east - pavingEast, depth: 0.12 },
      ];
    }),
    { surface: square.id, x: square.x - 1.69, z: square.z + 2.38, width: 0.45, depth: 0.56 },
    ...[-1, 1].flatMap((side) => [-1, 1].map((end) => ({
      surface: square.id, x: square.x + side * 2.49, z: square.z + end * 1.93, width: 0.83, depth: 0.83,
    }))),
    ...shops.map((shop) => ({ surface: square.id, x: shop.x, z: shop.z, ...shopFootprint(shop) })),
    ...planters.map(([x, z]) => ({ surface: square.id, x, z, width: 1.22, depth: 1.23 })),
    ...bambooPlanters.map(({ surface, x, z, width, depth }) => ({ surface, x, z, width, depth })),
    ...reedBeds.map(({ x, z, width, depth }) => ({ surface: square.id, x, z, width, depth })),
    ...benches.map(([x, z]) => ({ surface: square.id, x, z, width: 1.17, depth: 0.42 })),
    ...stalls.map(([x, z]) => ({ surface: square.id, x, z, width: 1.09, depth: 0.83 })),
    { surface: garden.id, ...pagoda },
    ...gardenRocks.filter(([, , size]) => size > 1).map(([x, z, size]) => ({
      surface: garden.id, x, z, width: size * 0.88, depth: size * 0.73,
    })),
    ...[...gardenTrees, maple].map(([x, z, size]) => ({
      surface: garden.id, x, z, width: size * 0.19, depth: size * 0.19,
    })),
  ],
};
