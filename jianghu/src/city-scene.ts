import type * as Three from 'three';
import { gaugeCrown, type JoineryPiece } from './city-joinery.ts';
import { createHippedRoofGeometry, createOperaCanopyGeometry } from './city-roof.ts';
import { shopFootprint, shopVariants, type ShopVariant } from './city-shops.ts';
import { createCityFoliage } from './city-foliage.ts';
import { mountCityReeds } from './city-reeds.ts';
import { pavingJoint, stairLanes, surfaceGrid } from './surface-grid.ts';
import {
  gaugeSite, practiceSite, rainbellPeriod, rainbellSignalEnd, rainbellSignalStart, rainbellSwing, teaStand,
} from './city-encounters.ts';
import { operaHouse, rainbellBridge, rainbellDecks, rainbellGarden, rainbellProps, rainbellSite, rainbellSteps } from './rainbell-world.ts';

const stone = '#b5a588';
const porcelain = '#ecdbad';
const glass = '#31565a';
const lacquer = '#a94f3c';
const copper = '#c78853';
const garden = '#668a66';
const dark = '#31494b';
export const spillwayCrest = 20.49;
export const canalEnd = 75;
export const reservoirProfile: readonly [readonly [number, number], ...readonly [number, number][]] = [
  [-10.12, 8.22],
  [-13.20, 16.1],
  [-15.80, 34.0],
  [-27.7, 35.4],
  [-42.6, 33.2],
  [-54.8, 27.2],
  [-66.9, 19.2],
];
export const feederProfile: readonly (readonly [z: number, center: number, y: number, width: number])[] = [
  [reservoirProfile[reservoirProfile.length - 1][0], -2.80, spillwayCrest + 0.02, 2.60],
  [-69.50, -2.38, spillwayCrest + 0.18, 2.45],
  [-72.10, -1.02, spillwayCrest + 1.14, 2.25],
  [-76.90, 0.75, spillwayCrest + 1.70, 2.15],
  [-82.00, -0.65, spillwayCrest + 2.10, 2.20],
  [-87.85, -4.60, spillwayCrest + 3.15, 2.30],
  [-94.00, -7.25, spillwayCrest + 4.05, 2.10],
  [-99.30, -5.80, spillwayCrest + 4.08, 1.80],
];

export function canalHalfWidth(z: number) {
  const narrows = Math.min(1, Math.max(0, (z - 17.8) / 6.0));
  const valley = Math.max(0, z - 40) * 0.16;
  return 13.05 - 6.5 * narrows + valley;
}

function random(i: number) {
  const value = 43758.5453 * Math.sin(i * 127.1 + 311.7);
  return value - Math.floor(value);
}

type InstanceBox = [
  x: number, y: number, z: number, width: number, height: number, depth: number,
  color: Three.ColorRepresentation, yaw: number,
];

export function buildCity(T: typeof Three, scene: Three.Scene, time: Three.IUniform<number>) {
  const unit = new T.BoxGeometry(1, 1, 1);
  const pose = new T.Object3D();
  const blocks: InstanceBox[] = [];
  const lights: InstanceBox[] = [];
  const nightLamps: { lamp: Three.PointLight; intensity: number }[] = [];
  const foliage = createCityFoliage(T);
  const { tree, shrub, bamboo } = foliage;
  function block(
    x: number, y: number, z: number, width: number, height: number, depth: number,
    color: Three.ColorRepresentation, yaw = 0,
  ) {
    blocks.push([x, y + height / 2, z, width, height, depth, color, yaw]);
  }
  function light(
    x: number, y: number, z: number, width: number, height: number, depth: number,
    color: Three.ColorRepresentation,
  ) {
    lights.push([x, y + height / 2, z, width, height, depth, color, 0]);
  }
  function nightLamp(x: number, y: number, z: number, color: string, intensity: number, range: number) {
    const lamp = new T.PointLight(color, intensity, range);
    lamp.position.set(x, y, z);
    lamp.visible = false;
    nightLamps.push({ lamp, intensity });
    scene.add(lamp);
  }
  function batch(items: InstanceBox[], material: Three.Material) {
    const mesh = new T.InstancedMesh(unit, material, items.length);
    const color = new T.Color();
    items.forEach(([x, y, z, w, h, d, tint, yaw], i) => {
      pose.position.set(x, y, z);
      pose.scale.set(w, h, d);
      pose.rotation.y = yaw;
      pose.updateMatrix();
      mesh.setMatrixAt(i, pose.matrix);
      mesh.setColorAt(i, color.set(tint));
    });
    scene.add(mesh);
    return mesh;
  }

  function tower(
    x: number, z: number, base: number, width: number, depth: number,
    floors: number, style: number, seed: number,
  ) {
    const storey = 0.91;
    const ground = base + 0.43;
    const roof = ground + floors * storey;
    const shell = [porcelain, '#a0b8a8', '#d3c1a0'][style];
    const facade = ['#405e5c', '#385454', '#48645d'][style];
    const trim = ['#7b9783', '#64867e', copper][style];
    const setback = Math.max(2, Math.floor(floors / 3));
    let crownWidth = width;
    let crownDepth = depth;
    let crownX = x;

    block(x, base - 0.05, z, width + 1.1, 0.48, depth + 0.9, stone);

    for (let floor = 0; floor < floors; floor++) {
      const tier = style === 0 ? Math.min(2, Math.floor(floor / setback)) : 0;
      const deck = style === 0 && tier > 0 && floor % setback === 0;
      const w = width * (style === 1 ? 0.87 : 1 - tier * 0.15);
      const d = depth * (style === 1 ? 0.92 : 1 - tier * 0.12);
      const cx = x - tier * width * 0.044;
      const y = ground + floor * storey;
      crownWidth = w;
      crownDepth = d;
      crownX = cx;
      block(cx, y, z, w, storey, d, floor === 0 ? dark : facade);

      if (style === 0 || (style === 2 && floor % 3 === 0)) {
        const overhang = deck ? 0.48 : 0.11;
        block(cx, y, z, w + overhang, deck ? 0.18 : 0.09, d + overhang, shell);
        if (deck) {
          block(cx + w / 2 + 0.13, y + 0.14, z + d * 0.18, 0.27, 0.19, d * 0.44, '#607c5d');
          block(cx + w / 2 + 0.13, y + 0.30, z + d * 0.18, 0.19, 0.15, d * 0.38, '#7c8d66');
        }
      }

      for (const side of [-1, 1]) {
        const front = z + side * d / 2;
        const flank = cx + side * w / 2;
        const sill = y + (deck ? 0.27 : 0.19);
        const windows = style === 1 ? 3 : 4;
        const bay = w * 0.76 / windows;
        for (let pane = 0; pane < windows; pane++) {
          const mid = cx + (pane - (windows - 1) / 2) * bay;
          const daylight = random(seed + floor * 17 + pane * 29 + side * 71);
          const paneWidth = bay * (style === 1 && pane === 1 ? 0.42 : 0.77);
          const tint = daylight > 0.49 ? '#668e81' : '#2a4749';
          block(mid, sill, front + side * 0.024, paneWidth, 0.49, 0.052, tint);
          if (daylight > 0.85) {
            light(mid, sill + 0.055, front + side * 0.054, paneWidth * 0.66,
              0.25, 0.026, daylight > 0.95 ? '#e9c98e' : '#97beaa');
          }
        }
        for (const bayZ of [-0.24, 0.24]) {
          block(flank + side * 0.018, y + 0.20, z + d * bayZ, 0.052, 0.44, d * 0.29,
            style === 1 ? '#2b484a' : '#58796f');
        }
        block(cx, y + 0.70, front + side * 0.046, w * 0.84, 0.073, 0.084, trim);
        if (style === 2 && floor % 2 === 1) {
          block(flank + side * 0.025, y + 0.72, z, 0.23, 0.12, d * 0.91, shell);
        }
      }
    }

    for (const side of [-1, 1]) {
      block(x + side * width * 0.28, ground + 0.03, z + depth * 0.50,
        0.16, 0.81, 0.17, shell);
      if (style !== 0) {
        for (const end of [-1, 1]) {
          block(x + side * crownWidth * 0.46, ground, z + end * crownDepth * 0.46,
            style === 1 ? 0.23 : 0.34, floors * storey + 0.10, 0.32, shell);
        }
      }
    }
    block(crownX, roof, z, crownWidth + 0.26, 0.20, crownDepth + 0.24, shell);
    if (style === 0) {
      block(crownX, roof + 0.20, z, crownWidth * 0.77, 0.12, crownDepth * 0.68, garden);
      block(crownX - crownWidth * 0.13, roof + 0.32, z, crownWidth * 0.41, 0.55, crownDepth * 0.51, facade);
      block(crownX - crownWidth * 0.13, roof + 0.87, z, crownWidth * 0.60, 0.14, crownDepth * 0.66, dark);
      block(crownX - crownWidth * 0.13, roof + 1.01, z, crownWidth * 0.48, 0.16, crownDepth * 0.38, '#527065');
    } else if (style === 1) {
      for (const side of [-1, 1]) {
        block(x + side * width * 0.27, roof + 0.26, z, width * 0.19, 1.56, depth * 0.43, shell);
        block(x + side * width * 0.27, roof + 0.39, z + depth * 0.22, width * 0.076, 1.25, 0.056, trim);
      }
      block(x, roof + 1.61, z, width * 0.79, 0.17, depth * 0.54, dark);
      block(x, roof + 1.82, z, width * 0.66, 0.11, depth * 0.35, shell);
      light(x, roof + 1.43, z + depth * 0.054, width * 0.29, 0.085, 0.09, '#a4d5bd');
      block(x + width * 0.27, roof + 1.93, z, 0.075, 0.43, 0.076, copper);
    } else {
      block(x, roof + 0.20, z, width * 0.68, 0.36, depth * 0.68, facade);
      for (const side of [-1, 1]) {
        block(x, roof + 0.57, z + side * depth * 0.23, width + 0.43, 0.12, depth * 0.56, dark);
        block(x + side * width * 0.46, roof + 0.66, z + side * depth * 0.28, 0.37, 0.15, 0.34, trim);
      }
      block(x, roof + 0.69, z, width * 0.85, 0.19, depth * 0.64, '#4c6b60');
      block(x, roof + 0.88, z, width * 0.75, 0.16, 0.20, '#688472');
    }
  }

  const paving = ['#a38f72', '#b0a185', '#b7ad91', '#c8b596', '#b7a788'];
  function terrace(
    x: number, z: number, width: number, depth: number, top: number, seed: number, planted = false,
  ) {
    block(x, -4.6, z, width, top + 4.23, depth, '#434d46');
    block(x, top - 0.90, z, width + 0.08, 0.24, depth + 0.08, '#707a63');
    block(x, top - 0.64, z, width + 0.13, 0.28, depth + 0.13, stone);
    const { columns, rows, cellWidth: w, cellDepth: d } = surfaceGrid(width, depth);
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < columns; col++) {
        const tint = paving[Math.floor(random(seed + row * 89 + col * 19) * paving.length)];
        block(x + (col + 0.5) * w - width / 2, top - 0.36, z + (row + 0.5) * d - depth / 2,
          w - pavingJoint, 0.36, d - pavingJoint, planted ? garden : tint);
      }
    }
    for (let course = 0; course < 3; course++) {
      for (let col = 0; col < columns; col++) {
        const offset = (col + (course % 2 ? 0.7 : 0.5)) * w - width / 2;
        const cellWidth = col === columns - 1 && course % 2 ? w * 0.57 : w * 0.85;
        const tint = ['#767762', '#898675', '#8d8b72'][Math.floor(random(seed + course * 45 + col * 17) * 3)];
        block(x + offset, top - 1.23 - course * 0.36, z + depth / 2 + 0.043,
          cellWidth, 0.31, 0.10, tint);
      }
    }
  }

  function eaves(x: number, y: number, z: number, width: number, depth: number) {
    for (let row = 0; row < 9; row++) {
      const slice = (row - 4) / 9;
      block(x, y + 0.78 * (1 - Math.abs(slice) * 2), z + slice * depth,
        width, 0.22, depth / 9 + 0.02,
        ['#283e3c', '#374f48', '#2e4541'][row % 3]);
    }
    for (let seam = -Math.floor(width * 1.2); seam <= Math.floor(width * 1.2); seam++) {
      const ribX = x + seam * 0.39;
      for (const edge of [-1, 1]) {
        block(ribX, y + 0.25, z + edge * depth * 0.432, 0.043, 0.065, 0.30, '#51665a');
        block(ribX, y + 0.40, z + edge * depth * 0.322, 0.043, 0.055, 0.24, '#536c61');
      }
    }
    block(x, y + 0.98, z, width + 0.2, 0.15, 0.18, '#527065');
    for (const side of [-1, 1]) {
      block(x, y + 0.02, z + side * (depth / 2 - 0.12), width + 0.37, 0.15, 0.23, dark);
      for (const edge of [-1, 1]) {
        for (let curl = 0; curl < 3; curl++) {
          block(x + edge * (width / 2 - 0.40 + curl * 0.22), y + 0.15 + curl * 0.11,
            z + side * (depth / 2 - 0.12), 0.26, 0.19, 0.25, '#5b7768');
        }
      }
    }
  }

  const glazedTiles = new T.MeshStandardMaterial({ vertexColors: true, roughness: 0.77 });
  function hippedRoof(x: number, y: number, z: number, width: number, depth: number, rise: number) {
    const roof = new T.Mesh(createHippedRoofGeometry(T, width, depth, rise), glazedTiles);
    roof.position.set(x, y, z);
    roof.castShadow = roof.receiveShadow = true;
    scene.add(roof);
    block(x, y + rise - 0.035, z, width * 0.2 + 0.14, 0.16, 0.18, '#587667');
    for (const side of [-1, 1]) {
      block(x + side * width * 0.104, y + rise + 0.10, z, 0.18, 0.15, 0.24, copper);
    }
  }

  function shop(
    x: number, y: number, z: number, w: number, d: number, tint: string, variant: ShopVariant, label?: string,
  ) {
    const { wall, frame, glass, lit, height } = shopVariants[variant];
    const foundation = shopFootprint({ width: w, depth: d });
    const front = z + d / 2;
    const lintel = y + 0.24 + height;
    block(x, y, z, foundation.width, 0.24, foundation.depth, stone);
    block(x, y + 0.21, z, w + 0.1, 0.14, d + 0.1, '#737966');
    block(x, y + 0.24, z - 0.10, w, height, d - 0.20, wall);
    block(x, y + 0.34, front - 0.16, w - 0.12, height - 0.19, 0.075, dark);
    block(x, y + 0.46, front - 0.015, w - 0.03, 0.13, 0.16, frame);

    const bay = w * 0.27;
    for (let opening = -1; opening <= 1; opening++) {
      const pane = x + opening * bay;
      const door = opening === 0;
      const sill = y + (door ? 0.34 : 0.70);
      const paneHeight = lintel - sill - 0.28;
      block(pane, sill, front - 0.035, bay - 0.09, paneHeight, 0.075, glass);
      light(pane, sill + 0.08, front + 0.008, bay - 0.22, paneHeight - 0.20, 0.035, lit);
      for (const bar of [-1, 0, 1]) {
        block(pane + bar * (bay / 4), sill + 0.05, front + 0.043, 0.048, paneHeight - 0.085, 0.041, frame);
      }
      block(pane, sill + paneHeight * 0.67, front + 0.058, bay - 0.075, 0.073, 0.062, frame);
      if (door) {
        for (const edge of [-1, 1]) block(pane + edge * 0.095, sill + 0.48, front + 0.078, 0.04, 0.10, 0.04, copper);
      } else {
        block(pane, sill - 0.055, front + 0.05, bay - 0.03, 0.07, 0.21, stone);
        block(pane, y + 0.37, front + 0.015, bay - 0.08, 0.23, 0.055, frame);
      }
    }

    for (const side of [-1, 1]) {
      const column = x + side * w * 0.45;
      for (const atZ of [z - d * 0.44, front - 0.04]) {
        block(column, y + 0.24, atZ, 0.24, 0.23, 0.21, stone);
        block(column, y + 0.45, atZ, 0.16, height - 0.19, 0.16, frame);
      }
      for (const atZ of [z - d * 0.27, z, z + d * 0.27]) {
        const wallX = x + side * (w / 2 + 0.017);
        block(wallX, y + 0.67, atZ, 0.07, 1.02, d * 0.178, frame);
        block(wallX + side * 0.026, y + 0.74, atZ, 0.05, 0.87, d * 0.143, glass);
        for (const bar of [-1, 0, 1]) {
          block(wallX + side * 0.058, y + 0.87 + bar * 0.24, atZ, 0.04, 0.039, d * 0.15, copper);
        }
      }
      block(column, lintel - 0.25, front + 0.04, 0.39, 0.18, 0.36, tint);
    }
    block(x, lintel - 0.08, z, w + 0.52, 0.18, d + 0.51, dark);
    block(x, lintel + 0.04, front + 0.25, w + 0.44, 0.085, 0.06, copper);
    const roof = lintel + 0.19;

    switch (variant) {
      case 'market':
        eaves(x, roof, z, w + 0.87, d + 0.92);
        for (let strip = 0; strip < 11; strip++) {
          block(x + (strip - 5) * w * 0.072, lintel - 0.43, front + 0.36,
            w * 0.073, strip % 3 === 0 ? 0.16 : 0.10, 0.69, strip % 3 ? tint : '#b99a6d');
        }
        for (const side of [-1, 1]) {
          const stall = x + side * w * 0.30;
          block(stall, y + 0.27, front + 0.11, 0.52, 0.31, 0.21, '#8b6c48');
          block(stall, y + 0.55, front + 0.11, 0.55, 0.08, 0.22, '#bc9b6a');
          block(stall - side * 0.12, y + 0.61, front + 0.09, 0.141, 0.14, 0.14, '#9a6843');
        }
        break;
      case 'teahouse':
        hippedRoof(x, roof, z, w + 0.95, d + 0.92, 0.94);
        for (const side of [-1, 1]) {
          lantern(x + side * w * 0.37, lintel - 0.92, front + 0.34);
          block(x + side * w * 0.36, lintel - 0.43, front + 0.12, 0.10, 0.46, 0.11, dark);
        }
        break;
      case 'workshop':
        block(x, y + 0.36, front + 0.093, bay - 0.11, height - 0.44, 0.09, '#455a52');
        for (let slat = 0; slat < 6; slat++) {
          block(x, y + 0.48 + slat * 0.228, front + 0.148, bay - 0.14, 0.044, 0.035, '#79897a');
        }
        block(x - bay * 0.21, y + 0.89, front + 0.157, 0.07, 0.26, 0.047, copper);
        block(x, roof - 0.04, z, w + 0.51, 0.17, d + 0.65, '#3e5755');
        for (let strip = 0; strip < 4; strip++) {
          const atZ = z + (strip - 1.62) * d * 0.24;
          block(x, roof + strip * 0.115, atZ, w + 0.60, 0.15, d * 0.25, '#597168');
          light(x, roof + strip * 0.115 + 0.045, atZ + d * 0.125, w * 0.79, 0.056, 0.055, '#9fbdac');
        }
        block(x - w * 0.28, roof + 0.48, z - d * 0.24, 0.32, 0.43, 0.34, frame);
        block(x - w * 0.28, roof + 0.89, z - d * 0.24, 0.43, 0.11, 0.43, '#7f9482');
        break;
      case 'station':
        block(x, roof - 0.04, z, w + 0.91, 0.18, d + 0.94, '#48645b');
        block(x, roof + 0.13, z - 0.27, w + 0.69, 0.16, d + 0.20, '#637d70');
        block(x, roof + 0.28, z - 0.37, w * 0.89, 0.64, d * 0.49, dark);
        eaves(x, roof + 0.82, z - 0.39, w * 0.94, d * 0.67);
        light(x, roof + 0.45, z - 0.37 + d * 0.25, w * 0.73, 0.14, 0.061, '#a6cfa8');
        light(x, lintel - 0.27, front + 0.30, w * 0.85, 0.072, 0.06, '#acd5ac');
        for (const side of [-1, 1]) {
          block(x + side * w * 0.42, y + 0.29, front + 0.12, 0.22, 0.68, 0.17, dark);
          light(x + side * w * 0.42, y + 0.78, front + 0.21, 0.13, 0.097, 0.035, '#b5deae');
        }
        break;
    }
    if (label) {
      const plaqueWidth = w * 0.57;
      for (const side of [-1, 1]) {
        block(x + side * plaqueWidth * 0.38, lintel - 0.27, front + 0.22, 0.08, 0.34, 0.36, frame);
      }
      sign(x, lintel - 0.21, front + 0.42, plaqueWidth, label, tint);
    }
  }

  function sign(x: number, y: number, z: number, width: number, label: string, tint: string) {
    const canvas = document.createElement('canvas');
    canvas.width = 160;
    canvas.height = 42;
    const paint = canvas.getContext('2d');
    if (!paint) throw new Error('Could not draw Baichuan textures.');
    paint.fillStyle = tint;
    paint.fillRect(0, 0, canvas.width, canvas.height);
    paint.strokeStyle = '#e1c799';
    paint.strokeRect(3.5, 3.5, 153, 35);
    paint.font = 'bold 26px "PingFang SC", "Noto Sans CJK SC", sans-serif';
    paint.fillStyle = '#f3dbab';
    paint.textBaseline = 'middle';
    paint.textAlign = 'center';
    paint.fillText(label, 80, 22, 145);
    const map = new T.CanvasTexture(canvas);
    map.colorSpace = T.SRGBColorSpace;
    const plate = new T.Mesh(
      new T.PlaneGeometry(width, width * 42 / 160),
      new T.MeshStandardMaterial({ map, roughness: 0.8 }),
    );
    plate.position.set(x, y, z);
    scene.add(plate);
  }

  function stairs(
    x: number, z: number, lower: number, upper: number, run: number, width: number,
    direction: 'east' | 'west' | 'north', steps: number, lanes = 1,
  ) {
    const alongX = direction !== 'north';
    const sign = direction === 'east' ? 1 : -1;
    const tread = run / steps;
    const treadSpan = lanes === 1 ? tread + 0.01 : tread;
    const laneWidth = width / lanes;
    const laneSpan = lanes === 1 ? width : laneWidth - pavingJoint;
    for (let i = 0; i < steps; i++) {
      const distance = (i + 0.5) * tread;
      for (let lane = 0; lane < lanes; lane++) {
        const offset = (lane + 0.5) * laneWidth - width / 2;
        block(x + (alongX ? sign * distance : offset), lower - 0.14, z + (alongX ? offset : -distance),
          alongX ? treadSpan : laneSpan, 0.14 + (upper - lower) * (i + 1) / steps,
          alongX ? laneSpan : treadSpan, porcelain);
      }
    }
  }

  function banner(
    x: number, top: number, z: number, width: number, length: number,
    color: string, inscription: string,
  ) {
    const canvas = document.createElement('canvas');
    canvas.width = 64;
    canvas.height = 128;
    const paint = canvas.getContext('2d');
    if (!paint) throw new Error('Could not draw Baichuan textures.');
    paint.fillStyle = color;
    paint.fillRect(0, 0, 64, 128);
    for (let thread = 0; thread < 64; thread += 2) {
      paint.fillStyle = thread % 6 ? '#271c2018' : '#edc87927';
      paint.fillRect(thread, 0, 1, 128);
    }
    paint.strokeStyle = '#dbac68';
    paint.lineWidth = 2.2;
    paint.strokeRect(5, 6, 54, 111);
    paint.strokeRect(9, 11, 46, 101);
    paint.textAlign = 'center';
    paint.textBaseline = 'middle';
    paint.fillStyle = '#f0d5a0';
    paint.font = 'bold 25px "Kaiti SC", "Songti SC", serif';
    [...inscription].forEach((character, index) => paint.fillText(character, 32, 40 + index * 34));
    const map = new T.CanvasTexture(canvas);
    map.colorSpace = T.SRGBColorSpace;
    const fabric = new T.MeshStandardMaterial({ map, side: T.DoubleSide, roughness: 0.92 });
    fabric.onBeforeCompile = (shader) => {
      shader.uniforms.windTime = time;
      shader.vertexShader = shader.vertexShader.replace('#include <common>', `
        #include <common>
        uniform float windTime;
      `).replace('#include <begin_vertex>', `
        #include <begin_vertex>
        float hem = 1.0 - uv.y;
        float phase = windTime * 2.2 + position.x * 4.4 - hem * 2.6;
        transformed.z += hem * (.10 + .15 * sin(phase));
        transformed.x += .095 * hem * hem * sin(phase * .83);
      `);
    };
    const geometry = new T.PlaneGeometry(width, length, 9, 12).translate(0, -length / 2, 0);
    const cloth = new T.Mesh(geometry, fabric);
    cloth.position.set(x, top, z);
    cloth.receiveShadow = true;
    scene.add(cloth);
    block(x, top - 0.06, z, width + 0.26, 0.07, 0.081, '#ad8151');
  }

  function lantern(x: number, y: number, z: number, kind: 'silk' | 'garden' | 'signal' = 'silk') {
    if (kind === 'garden') {
      block(x, y, z, 0.32, 0.09, 0.32, '#858e7c');
      light(x, y + 0.09, z, 0.23, 0.22, 0.23, '#f4d997');
      for (const side of [-1, 1]) {
        block(x + side * 0.112, y + 0.08, z - 0.112, 0.046, 0.24, 0.046, '#546960');
        block(x + side * 0.112, y + 0.08, z + 0.112, 0.046, 0.24, 0.046, '#546960');
      }
      block(x, y + 0.31, z, 0.39, 0.065, 0.39, '#415851');
      block(x, y + 0.375, z, 0.29, 0.068, 0.29, '#6b8375');
      block(x, y + 0.443, z, 0.18, 0.07, 0.18, '#9ea891');
      return;
    }
    if (kind === 'signal') {
      block(x, y, z, 0.26, 0.055, 0.21, '#384d49');
      block(x, y + 0.055, z, 0.27, 0.35, 0.21, '#425b54');
      block(x, y + 0.38, z, 0.33, 0.06, 0.27, '#667f6d');
      block(x, y + 0.44, z, 0.08, 0.14, 0.08, '#97724e');
      for (const side of [-1, 1]) {
        light(x, y + 0.13, z + side * 0.111, 0.188, 0.20, 0.023, '#c4dfab');
        block(x, y + 0.21, z + side * 0.125, 0.20, 0.024, 0.02, '#607960');
        light(x + side * 0.137, y + 0.16, z, 0.019, 0.14, 0.11, '#ebbd74');
      }
      return;
    }

    block(x, y + 0.50, z, 0.045, 0.27, 0.045, dark);
    block(x, y + 0.46, z, 0.26, 0.07, 0.25, '#6c4636');
    light(x, y + 0.09, z, 0.40, 0.31, 0.35, '#edb768');
    light(x, y + 0.02, z, 0.29, 0.07, 0.28, '#db8949');
    light(x, y + 0.40, z, 0.29, 0.06, 0.28, '#e49d53');
    for (const side of [-1, 1]) {
      block(x + side * 0.138, y + 0.09, z + 0.179, 0.023, 0.31, 0.018, lacquer);
      block(x + side * 0.138, y + 0.09, z - 0.179, 0.023, 0.31, 0.018, lacquer);
    }
    block(x, y - 0.025, z, 0.25, 0.045, 0.24, '#624335');
    block(x, y - 0.18, z, 0.029, 0.15, 0.029, '#c99752');
    block(x, y - 0.25, z, 0.079, 0.091, 0.066, lacquer);
  }

  function gaugeTower(x: number, y: number, z: number) {
    block(x, y, z, 3.94, 0.28, 3.74, porcelain);
    block(x, y + 0.28, z, 3.18, 1.43, 3.02, stone);
    block(x, y + 1.69, z, 3.47, 0.29, 3.34, dark);
    block(x, y + 1.98, z, 1.82, 4.77, 1.82, '#557269');
    block(x, y + 2.12, z + 0.93, 0.72, 4.09, 0.16, dark);
    light(x, y + 2.30, z + 1.02, 0.14, 3.77, 0.05, '#a5d5ac');
    for (let tick = 0; tick < 9; tick++) {
      block(x + 0.23, y + 2.39 + tick * 0.42, z + 1.03, 0.19, 0.049, 0.045, porcelain);
    }
    for (const side of [-1, 1]) {
      block(x + side * 1.04, y + 2.03, z - 0.38, 0.31, 4.43, 0.64, stone);
      block(x + side * 0.82, y + 2.10, z + 0.94, 0.12, 4.18, 0.16, '#a77b47');
      block(x + side * 0.74, y + 0.41, z + 1.53, 0.24, 0.51, 0.07, '#9a8059');
      for (let band = 0; band < 3; band++) {
        block(x + side * 1.04, y + 2.22 + band * 1.76, z - 0.38, 0.40, 0.13, 0.71, '#a77b47');
      }
    }
    block(x, y + 6.43, z, 2.38, 0.26, 2.35, lacquer);
    block(x, y + 6.69, z, 1.77, 0.30, 1.77, porcelain);
    block(x, y + 6.99, z, 0.82, 0.65, 0.86, dark);
    sign(x, y + 1.33, z + 1.54, 1.83, '水位臺', '#4b665f');
    block(x, y + 0.37, z + 1.53, 0.68, 0.61, 0.16, dark);
    block(x, y + 0.48, z + 1.63, 0.19, 0.16, 0.10, '#9a8059');
    block(x + 0.53, y + 5.66, z + 0.99, 0.59, 0.51, 0.30, dark);
    const signalMaterial = new T.MeshStandardMaterial({ color: '#b18954', roughness: 0.51 });
    for (const [dx, dy, dz, size] of [[0, 0.81, 1.65, 0.22], [0.53, 5.91, 1.17, 0.35]]) {
      const lens = new T.Mesh(new T.BoxGeometry(size, size, size * 0.28), signalMaterial);
      lens.position.set(x + dx, y + dy, z + dz);
      scene.add(lens);
    }

    const crownBase = y + 7.64;
    const { alongX, alongZ, key } = gaugeCrown();
    function member({ min, max }: JoineryPiece, tint: string) {
      block(x + (min[0] + max[0]) / 2, crownBase + min[1], z + (min[2] + max[2]) / 2,
        max[0] - min[0], max[1] - min[1], max[2] - min[2], tint);
    }
    for (const piece of alongX) member(piece, '#9e4933');
    for (const piece of alongZ) member(piece, '#345f5a');
    member(key, '#d6a65d');
    block(x, crownBase + 1.46, z, 0.43, 0.15, 0.43, porcelain);

    // Fittings make each projecting end read as a separate member.
    block(x + 2.74, crownBase + 0.24, z, 0.13, 0.44, 0.89, '#c59152');
    block(x + 3.12, crownBase + 0.11, z, 0.10, 0.67, 0.70, '#d9b075');
    block(x - 1.94, crownBase + 0.11, z, 0.10, 0.67, 0.70, '#d9b075');
    for (const lug of [-1.37, 1.41, 2.31]) {
      block(x + lug, crownBase + 0.67, z + 0.415, 0.083, 0.083, 0.03, '#d2a469');
      block(x + lug, crownBase + 0.16, z + 0.415, 0.083, 0.083, 0.03, '#d2a469');
    }
    block(x, crownBase + 0.12, z + 1.78, 0.9, 0.60, 0.12, '#a4ba9d');
    lantern(x, crownBase - 0.81, z + 1.78, 'signal');
    banner(x + 2.22, crownBase - 0.04, z + 0.41, 0.69, 1.96, '#783f39', '聽潮');
    return (lit: boolean) => {
      signalMaterial.color.set(lit ? '#a4e5af' : '#b18954');
      signalMaterial.emissive.set(lit ? '#83d996' : '#000000');
    };
  }

  function practiceCourt(x: number, y: number, z: number) {
    block(x, y, z, 4.9, 0.16, 5.08, '#556c5e');
    for (let row = 0; row < 5; row++) {
      for (let column = 0; column < 5; column++) {
        const center = row === 2 || column === 2;
        block(x + (column - 2) * 0.92, y + 0.14, z + (row - 2) * 0.96,
          0.895, 0.065, 0.935, center ? '#ac9874' : '#8e8d76');
      }
    }
    for (const side of [-1, 1]) {
      const post = x + side * 2.07;
      block(post, y, z - 2.19, 0.43, 0.44, 0.48, stone);
      block(post, y + 0.4, z - 2.19, 0.15, 2.64, 0.15, '#59483a');
      block(post, y + 3.03, z - 2.19, 0.86, 0.105, 0.091, copper);
      banner(post, y + 2.96, z - 2.11, 0.64, 1.53, '#425e58', '劍堂');
      lantern(post, y + 1.36, z + 2.07, 'garden');
      block(post, y + 0.14, z + 2.07, 0.16, 2.01, 0.16, '#64503c');
    }
    const rack = x - 1.62;
    block(rack, y + 0.21, z - 0.57, 0.32, 0.45, 1.58, '#705339');
    for (let i = 0; i < 4; i++) {
      const slot = z - 1.07 + i * 0.32;
      block(rack, y + 0.66, slot, 0.07, 0.94, 0.08, '#a1a89a');
      block(rack, y + 1.55, slot, 0.18, 0.06, 0.07, copper);
      block(rack, y + 1.61, slot, 0.06, 0.18, 0.065, '#695349');
    }
  }

  function teaCounter(x: number, y: number, z: number) {
    block(x, y + 0.66, z, 1.06, 0.14, 1.43, '#816343');
    for (const side of [-1, 1]) {
      block(x + side * 0.40, y, z, 0.13, 0.67, 1.18, '#534739');
    }
    block(x - 0.25, y + 0.80, z - 0.33, 0.29, 0.24, 0.29, '#4c6861');
    block(x - 0.25, y + 1.03, z - 0.33, 0.23, 0.05, 0.24, '#af9564');
    block(x - 0.04, y + 0.96, z - 0.33, 0.19, 0.065, 0.09, '#4c6861');
    for (const cup of [-0.04, 0.20, 0.44]) {
      block(x + 0.19, y + 0.79, z + cup, 0.12, 0.09, 0.10, '#d8c59b');
    }
    for (const side of [-1, 1]) block(x - 0.44, y, z + side * 0.62, 0.10, 2.07, 0.11, dark);
    block(x - 0.38, y + 2.05, z, 0.16, 0.10, 1.41, copper);
    banner(x - 0.38, y + 1.91, z + 0.81, 0.39, 0.71, '#783f39', '茶');
  }

  function rainbellSquare() {
    for (const { x, y, z, width, depth, seed } of rainbellDecks) terrace(x, z, width, depth, y, seed);
    const { west, east, y: bridgeY, z: bridgeZ, width, columns, pavingWest, pavingEast } = rainbellBridge;
    const span = east - west;
    const bridgeX = (east + west) / 2;
    block(bridgeX, bridgeY - 0.48, bridgeZ, span, 0.33, width, '#697970');
    const { cellWidth: boardWidth } = surfaceGrid(pavingEast - pavingWest, width, columns, 1);
    for (let tread = 0; tread < columns; tread++) {
      block(pavingWest + (tread + 0.5) * boardWidth, bridgeY - 0.15, bridgeZ,
        boardWidth - pavingJoint, 0.15, width - pavingJoint, tread % 3 ? stone : porcelain);
    }
    for (const pier of [-4.45, 4.45]) {
      block(pier, -1.95, bridgeZ, 1.28, 2.39, 1.44, '#76847b');
      block(pier, 0.1, bridgeZ, 1.46, 0.43, 1.63, stone);
      block(pier, -0.71, bridgeZ, 1.37, 0.15, 1.54, '#909280');
      block(pier, -0.55, bridgeZ, 1.45, 0.13, 1.60, '#688077');
    }
    for (const side of [-1, 1]) {
      const railZ = bridgeZ + side * (width / 2 - 0.12);
      const outboard = bridgeZ + side * (width / 2 + 0.17);
      block(bridgeX, bridgeY - 0.30, railZ, span, 0.093, 0.12, '#435e54');
      block(bridgeX, bridgeY + 0.75, railZ, span, 0.13, 0.12, '#425f56');
      block(bridgeX, bridgeY + 0.16, railZ, span - 0.26, 0.085, 0.09, '#516e61');
      const bayWidth = (span - 0.28) / 12;
      for (let post = 0; post <= 12; post++) {
        const x = west + 0.14 + post * bayWidth;
        const joint = post % 3 === 0;
        block(x, bridgeY, railZ, joint ? 0.20 : 0.105, 0.78, 0.11, joint ? '#44574b' : dark);
        if (joint) {
          block(x, bridgeY + 0.01, railZ, 0.27, 0.16, 0.12, '#989278');
          block(x, bridgeY + 0.65, railZ, 0.27, 0.12, 0.12, '#a48150');
          block(x, bridgeY + 0.88, railZ, 0.25, 0.055, 0.12, '#8f9880');
        }
        if (post === 12) continue;
        const mid = x + bayWidth / 2;
        block(mid, bridgeY + 0.42, railZ, bayWidth - 0.18, 0.06, 0.075, '#57746a');
        for (const direction of [-1, 1]) {
          block(mid + direction * bayWidth * 0.23, bridgeY + 0.23, railZ, 0.075, 0.42, 0.082, '#4d695d');
        }
        block(mid, bridgeY + 0.43, railZ + side * 0.046, 0.16, 0.16, 0.029, copper);
      }
      for (const x of [pavingWest + 0.24, pavingEast - 0.24]) {
        block(x, bridgeY - 0.14, outboard, 0.18, 1.15, 0.19, '#506b60');
        block(x, bridgeY + 0.82, railZ + side * 0.12, 0.15, 0.15, 0.29, '#a98352');
        lantern(x, bridgeY + 1.00, outboard, 'signal');
        if (side === 1) nightLamp(x, bridgeY + 1.24, outboard, '#ffca91', 14, 5.5);
      }
    }
    const [x, y, z] = rainbellSite;
    stairs(rainbellSteps.x, rainbellSteps.z, y, bridgeY, rainbellSteps.run, rainbellSteps.width,
      rainbellSteps.direction, rainbellSteps.steps, stairLanes(rainbellSteps.width));

    for (let tile = 0; tile < 24; tile++) {
      const angle = tile * Math.PI / 12;
      const dx = Math.cos(angle);
      const dz = Math.sin(angle);
      block(x + dx * 2.77, y + 0.009, z + dz * 2.77,
        0.67, 0.028, 0.21, tile % 6 ? '#6d8173' : '#ad8952', -angle - Math.PI / 2);
      if (tile % 6 === 0) {
        block(x + dx * 3.09, y + 0.01, z + dz * 3.09, 0.38, 0.032, 0.10, copper, -angle);
      }
    }
    block(x, y, z, rainbellProps.bellFoundation.width, 0.13, rainbellProps.bellFoundation.depth, stone);
    block(x, y + 0.12, z, 5.69, 0.075, 4.22, '#708174');
    const plinth = new T.Mesh(
      new T.CylinderGeometry(1.72, 1.89, 0.24, 12),
      new T.MeshStandardMaterial({ color: '#576d63', roughness: 0.89 }),
    );
    plinth.position.set(x, y + 0.28, z);
    plinth.receiveShadow = true;
    scene.add(plinth);
    for (const side of [-1, 1]) {
      const postX = x + side * 2.49;
      const postZ = z + side * 1.93;
      block(x, y + 4.61, postZ, 6.23, 0.39, 0.44, '#634a3d');
      block(x, y + 4.95, postZ, 6.37, 0.11, 0.52, copper);
      block(postX, y + 4.66, z, 0.43, 0.39, 5.18, '#634a3d');
      block(postX, y + 4.96, z, 0.51, 0.095, 5.27, copper);
      for (const end of [-1, 1]) {
        const cornerZ = z + end * 1.93;
        block(postX, y + 0.19, cornerZ, 0.83, 0.38, 0.83, porcelain);
        block(postX, y + 0.53, cornerZ, 0.46, 4.13, 0.46, '#784e3c');
        block(postX, y + 0.58, cornerZ, 0.51, 0.20, 0.51, dark);
        block(postX, y + 4.18, cornerZ, 0.64, 0.22, 0.63, lacquer);
        block(postX, y + 4.35, cornerZ, 1.01, 0.16, 0.64, '#586e60');
        block(postX, y + 4.51, cornerZ, 0.64, 0.19, 1.03, '#586e60');
        block(postX - side * 0.57, y + 4.48, cornerZ, 0.41, 0.15, 0.29, copper);
      }
      banner(postX - side * 0.48, y + 4.40, z - 1.62, 0.53, 1.39, '#4c6b63', side < 0 ? '聽雨' : '水信');
      const lanternX = postX + side * 0.53;
      const lanternZ = z + 1.94;
      lantern(lanternX, y + 3.37, lanternZ);
      const lanternLight = new T.PointLight('#ffc083', 13, 4.5);
      lanternLight.position.set(lanternX, y + 3.61, lanternZ);
      scene.add(lanternLight);
      block(postX + side * 0.28, y + 4.10, z + 1.94, 0.71, 0.12, 0.11, copper);
    }
    block(x, y + 4.74, z, 0.53, 0.26, 4.34, dark);
    block(x, y + 4.84, z, 5.19, 0.16, 0.36, '#5b493b');
    block(x, y + 5.02, z, 6.02, 0.13, 4.64, '#55695a');
    hippedRoof(x, y + 5.14, z, 7.42, 5.90, 1.43);
    sign(x, y + 4.76, z + 2.22, 1.97, '聽雨坪', '#496256');
    block(x, y + 4.02, z, 0.37, 0.72, 0.37, copper);

    const bellProfile = [
      [1.16, 0], [1.24, 0.10], [1.18, 0.28], [1.03, 0.56], [0.91, 0.91],
      [0.87, 1.44], [0.71, 1.73], [0.41, 1.93], [0.32, 2.08],
      [0.25, 2.08], [0.33, 1.84], [0.63, 1.65], [0.79, 1.39], [0.82, 0.88],
      [0.96, 0.51], [1.09, 0.23], [1.11, 0.10], [1.16, 0],
    ];
    const bronze = new T.MeshStandardMaterial({ color: '#a67b4d', metalness: 0.36, roughness: 0.55 });
    const bell = new T.Mesh(new T.LatheGeometry(bellProfile.map(([r, h]) => new T.Vector2(r, h)), 16), bronze);
    bell.position.set(x, y + 2.09, z);
    bell.castShadow = bell.receiveShadow = true;
    scene.add(bell);
    const bands = new T.MeshBasicMaterial({ color: '#b6e4c0' });
    bands.onBeforeCompile = (shader) => {
      shader.uniforms.bellTime = time;
      shader.fragmentShader = shader.fragmentShader.replace('#include <common>', `
        #include <common>
        uniform float bellTime;
      `).replace('#include <color_fragment>', `
        #include <color_fragment>
        float beat = mod(bellTime, ${rainbellPeriod.toFixed(2)});
        float pulse = step(${rainbellSignalStart.toFixed(2)}, beat) *
          (1.0 - step(${rainbellSignalEnd.toFixed(2)}, beat));
        diffuseColor.rgb *= 0.34 + pulse * (0.67 + 0.26 * sin(beat * 12.0));
      `);
    };
    for (const [height, radius] of [[0.2, 1.22], [0.47, 1.076], [1.52, 0.83]]) {
      const hoop = new T.Mesh(new T.TorusGeometry(radius, 0.036, 5, 16), bands);
      hoop.rotation.x = Math.PI / 2;
      hoop.position.set(x, y + 2.09 + height, z);
      scene.add(hoop);
    }
    const strikerX = x + 0.39;
    const strikerZ = z + 2.02;
    block(strikerX, y + 4.65, z + 1.09, 0.27, 0.21, 3.04, dark);
    block(strikerX, y + 4.73, z + 2.66, 0.33, 0.13, 0.20, copper);

    const striker = new T.Group();
    striker.name = 'rainbell-striker';
    striker.position.set(strikerX, y + 4.65, strikerZ);
    const timber = new T.MeshStandardMaterial({ color: '#8c5634', roughness: 0.85, flatShading: true });
    const iron = new T.MeshStandardMaterial({ color: '#445a54', roughness: 0.71 });
    const rope = new T.MeshStandardMaterial({ color: '#bb9b65', roughness: 1 });
    const beam = new T.Mesh(new T.CylinderGeometry(0.18, 0.19, 1.70, 8), timber);
    beam.rotation.x = Math.PI / 2;
    beam.position.set(0, -2.23, 0.18);
    beam.castShadow = true;
    striker.add(beam);
    for (const end of [-0.48, 0.48]) {
      const cord = new T.Mesh(unit, rope);
      cord.position.set(0, -1.04, end);
      cord.scale.set(0.045, 2.08, 0.045);
      striker.add(cord);
      const collar = new T.Mesh(new T.CylinderGeometry(0.198, 0.198, 0.115, 8), iron);
      collar.rotation.x = beam.rotation.x;
      collar.position.set(0, beam.position.y, end);
      striker.add(collar);
    }
    scene.add(striker);

    block(x - 1.69, y, z + 2.38, 0.45, 0.85, 0.56, '#55665b');
    block(x - 1.69, y + 0.84, z + 2.38, 0.55, 0.13, 0.68, copper);
    light(x - 1.69, y + 0.98, z + 2.38, 0.27, 0.054, 0.24, '#b6daba');

    for (const prop of rainbellProps.shops) {
      shop(prop.x, y, prop.z, prop.width, prop.depth, prop.tint, prop.variant, prop.sign);
      nightLamp(prop.x, y + 1.36, prop.z + prop.depth / 2 + 0.24, '#ffd098', 13, 5);
    }
    // Brass inlays lead from the landing to the bell's rain dial.
    for (let i = 0; i < 12; i++) {
      const px = -22.4 + 0.75 * i;
      block(px, y + 0.008, 45.54, 0.57, 0.014, 0.20, i % 3 ? '#847f66' : copper);
    }
    for (const markerZ of [44.52, 52.3]) {
      const markerX = -8.95;
      block(markerX, y, markerZ, 1.41, 0.21, 0.76, '#6c8472');
      for (const side of [-1, 1]) {
        block(markerX + side * 0.59, y + 0.18, markerZ, 0.12, 1.95, 0.13, dark);
        lantern(markerX + side * 0.58, y + 1.25, markerZ + 0.2, 'signal');
      }
      block(markerX, y + 2.07, markerZ, 1.57, 0.10, 0.85, '#a37a50');
      block(markerX, y + 2.15, markerZ, 1.73, 0.11, 1.06, '#506d64');
    }
    for (const [px, pz, size, kind] of rainbellProps.planters) {
      block(px, y, pz, 1.22, 0.28, 1.23, '#858d71');
      block(px, y + 0.26, pz, 1.02, 0.06, 1.02, garden);
      tree(px, y + 0.3, pz, size, px * pz, kind);
    }
    for (const pot of rainbellProps.bambooPlanters) {
      const { x, y, z, width, depth, height, seed } = pot;
      block(x, y, z, width * 0.83, 0.14, depth * 0.83, '#596e63');
      block(x, y + 0.11, z, width, 0.32, depth, '#697e70');
      block(x, y + 0.37, z, width, 0.065, depth, '#b49b70');
      block(x, y + 0.43, z, width - 0.09, 0.027, depth - 0.09, '#4e5942');
      bamboo(x, y + 0.455, z, height, seed);
    }
    for (const bed of rainbellProps.reedBeds) {
      block(bed.x, y, bed.z, bed.width, 0.13, bed.depth, '#7e8a73');
      block(bed.x, y + 0.12, bed.z, bed.width - 0.13, 0.045, bed.depth - 0.13, '#536a4e');
    }
    // A shallow apron gives the canal a stone bed between the quay and the deeper channel.
    const plaza = rainbellDecks[2];
    const shore = plaza.x + plaza.width / 2;
    const stones = ['#828c72', '#748471', '#909776'];
    for (let flag = 0; flag < 14; flag++) {
      const along = plaza.z + (flag - 6.5) * 0.93;
      const edge = shore + 0.24 + random(flag * 31) * 0.16;
      block(edge, -0.65, along, 1.05, 0.42, 0.94, stones[flag % 3]);
      block(edge + 0.63, -1.01, along, 0.70, 0.53, 0.98, '#596f63');
      if (flag % 3 !== 1) {
        block(edge + 0.34, -0.27, along + 0.10, 0.39, 0.12, 0.44, '#657c5b', (random(flag) - 0.5) * 0.6);
      }
    }
    for (const [px, pz] of rainbellProps.benches) {
      block(px, y + 0.43, pz, 1.17, 0.11, 0.42, '#806445');
      for (const side of [-1, 1]) block(px + side * 0.41, y, pz, 0.14, 0.43, 0.34, stone);
      block(px + 0.91, y, pz - 0.43, 0.37, 0.41, 0.4, '#706a58');
    }
    for (const [px, pz, tint] of rainbellProps.stalls) {
      block(px, y + 0.54, pz, 1.09, 0.105, 0.83, '#876948');
      for (const side of [-1, 1]) block(px + side * 0.43, y, pz, 0.11, 1.66, 0.1, dark);
      block(px, y + 1.59, pz, 1.46, 0.075, 1.13, tint);
      for (const side of [-1, 1]) {
        block(px + side * 0.28, y + 0.64, pz, 0.22, 0.125, 0.26, '#caac7b');
      }
    }
    return striker;
  }

  function plantRainbellGarden() {
    const { deck, steps, pathX, lookoutZ } = rainbellGarden;
    const { x, y, z, width, depth } = deck;
    terrace(x, z, width, depth, y, 223, true);
    block(x, y - 0.35, z, width - 0.06, 0.34, depth - 0.06, '#657f5c');
    stairs(steps.x, steps.z, rainbellSite[1], y, steps.run, steps.width,
      steps.direction, steps.steps, stairLanes(steps.width));
    for (const side of [-1, 1]) {
      const curbZ = steps.z + side * steps.width * 0.51;
      for (let tread = 0; tread < 4; tread++) {
        block(steps.x - (tread + 0.5) * steps.run / 4, rainbellSite[1] - 0.1, curbZ,
          steps.run / 4, 0.36 + tread * 0.24, 0.16, '#8a8972');
      }
    }
    const stairTopX = steps.x - steps.run;
    block((pathX + stairTopX) / 2, y - 0.19, steps.z, stairTopX - pathX, 0.19, 1.86, stone);
    const slabs = 10;
    const spacing = (lookoutZ - steps.z) / (slabs - 1);
    for (let i = 0; i < slabs; i++) {
      const column = pathX + (random(i * 79) - 0.5) * 0.19;
      block(column, y - 0.12, steps.z + i * spacing, 1.54 + random(i + 9) * 0.2, 0.12,
        spacing - 0.055, paving[i % paving.length]);
    }

    const { x: pagodaX, z: pagodaZ, width: pagodaWidth, depth: pagodaDepth } = rainbellProps.pagoda;
    const base = y + 0.56;
    block(pagodaX, y, pagodaZ, pagodaWidth, 0.18, pagodaDepth, '#7c8470');
    block(pagodaX, y + 0.18, pagodaZ, 4.12, 0.25, 4.16, stone);
    block(pagodaX, y + 0.43, pagodaZ, 4.35, 0.13, 4.39, porcelain);
    stairs(-30.35, pagodaZ, y, base, 0.65, 1.37, 'west', 3);
    block(-30.07, y - 0.08, pagodaZ, 0.58, 0.08, 1.48, stone);

    let floor = base;
    for (const [body, height, roof, rise] of [[3.33, 1.86, 4.55, 0.94], [2.44, 1.49, 3.70, 0.81], [1.63, 1.16, 2.82, 0.70]]) {
      const eave = floor + height;
      block(pagodaX, floor, pagodaZ, body - 0.30, height, body - 0.30, '#524d3f');
      block(pagodaX, floor + 0.10, pagodaZ, body + 0.1, 0.13, body + 0.1, '#977657');
      for (const side of [-1, 1]) {
        const faceX = pagodaX + side * body * 0.485;
        const faceZ = pagodaZ + side * body * 0.485;
        for (const lateral of [-1, 1]) {
          block(faceX, floor, pagodaZ + lateral * body * 0.485, 0.19, height, 0.19, '#8f5744');
        }
        block(pagodaX, eave - 0.24, faceZ, body + 0.2, 0.22, 0.29, '#7b5040');
        block(faceX, eave - 0.24, pagodaZ, 0.29, 0.22, body + 0.2, '#7b5040');
        for (const panel of [-1, 0, 1]) {
          block(pagodaX + panel * body * 0.252, floor + 0.24, faceZ, body * 0.214, height - 0.60, 0.08, '#b1996e');
          block(faceX, floor + 0.24, pagodaZ + panel * body * 0.252, 0.08, height - 0.60, body * 0.214, '#b1996e');
          block(pagodaX + panel * body * 0.252, floor + 0.25, faceZ + side * 0.05, 0.06, height - 0.61, 0.04, dark);
          block(faceX + side * 0.05, floor + 0.25, pagodaZ + panel * body * 0.252, 0.04, height - 0.61, 0.06, dark);
        }
      }
      hippedRoof(pagodaX, eave, pagodaZ, roof, roof, rise);
      floor = eave + rise * 0.78;
    }
    block(pagodaX, floor + 0.15, pagodaZ, 0.40, 0.19, 0.40, copper);
    block(pagodaX, floor + 0.34, pagodaZ, 0.18, 0.38, 0.18, '#a7854e');
    sign(pagodaX, base + 1.52, pagodaZ + 1.69, 1.25, '棲雲', '#536455');
    for (const lampZ of [steps.z + 1.12, lookoutZ]) {
      block(-27.83, y, lampZ, 0.33, 0.21, 0.35, stone);
      block(-27.83, y + 0.19, lampZ, 0.11, 1.27, 0.12, dark);
      block(-28.01, y + 1.42, lampZ, 0.55, 0.10, 0.13, dark);
      lantern(-28.15, y + 0.68, lampZ, 'garden');
      nightLamp(-28.15, y + 0.94, lampZ, '#ffc385', 10, 4.6);
    }
    block(-30.37, y + 0.38, lookoutZ, 0.43, 0.12, 1.37, '#785b43');
    for (const side of [-1, 1]) block(-30.37, y, lookoutZ + side * 0.48, 0.28, 0.38, 0.18, stone);

    for (const [sx, sz, size] of rainbellProps.gardenRocks) {
      const angle = random(sx * sz) - 0.5;
      block(sx, y - 0.05, sz, size * 1.5, size * 0.22, size * 1.01, '#748459', angle);
      block(sx, y + size * 0.12, sz, size * 0.88, size * 0.34, size * 0.73, '#6d7666', angle + 0.23);
      block(sx - size * 0.26, y + size * 0.42, sz + size * 0.11, size * 0.49, size * 0.11, size * 0.49, '#899b66', angle);
    }
    const beds = [
      [-34.68, 45.18, 1.48, 'mound'], [-32.01, 45.17, 0.97, 'flowering'], [-30.91, 47.67, 0.78, 'mound'],
      [-35.91, 49.71, 1.12, 'mound'], [-35.77, 52.92, 1.50, 'mound'], [-34.58, 55.15, 1.24, 'flowering'],
      [-32.37, 54.72, 1.20, 'flowering'], [-30.48, 52.62, 0.73, 'mound'], [-28.07, 53.03, 0.57, 'flowering'],
    ] as const;
    for (const [bx, bz, size, kind] of beds) {
      shrub(bx, y, bz, size, bx * bz, kind);
    }
    const west = x - width / 2;
    for (const [rz, run, cap] of [[45.58, 1.55, 0.89], [49.22, 2.14, 0.22], [53.67, 1.84, 0.71], [56.04, 1.11, 0.08]]) {
      block(west - 1.00, -5.19, rz, 2.29, 2.52, run * 1.52, '#4c5c50', 0.09);
      block(west - 0.66, -3.18, rz - 0.17, 1.89, 2.35, run * 1.31, '#53675a', -0.14);
      block(west - 0.39, -1.15, rz + 0.13, 1.36, 1.15 + cap, run, '#677662', 0.16);
      block(west - 0.43, -0.025 + cap, rz + 0.16, 1.25, 0.17, run * 0.88, '#71885d', 0.16);
    }
    for (const [tx, tz, size, kind] of rainbellProps.gardenTrees) tree(tx, y, tz, size, tx * tz, kind);
    const [mapleX, mapleZ, mapleSize] = rainbellProps.maple;
    tree(mapleX, y, mapleZ, mapleSize, 73, 'maple');
  }

  function slidingOperaDoors(
    { x, y, z, width }: { x: number; y: number; z: number; width: number },
    height: number, material: Three.Material,
  ) {
    const panelGeometry = new T.BoxGeometry(width / 2 - 0.018, height, 0.073);
    const handleGeometry = new T.BoxGeometry(0.042, height * 0.45, 0.12);
    const handleMaterial = new T.MeshStandardMaterial({ color: '#b79b71', roughness: 0.54 });
    const panels = [-1, 1].map((side) => {
      const panel = new T.Mesh(panelGeometry, material);
      const handle = new T.Mesh(handleGeometry, handleMaterial);
      handle.position.set(-side * (width / 4 - 0.12), -height * 0.09, 0);
      panel.add(handle);
      panel.position.set(x + side * width / 4, y + height / 2, z);
      scene.add(panel);
      return { panel, side };
    });
    return (open: number) => {
      for (const { panel, side } of panels) panel.position.x = x + side * width * (0.25 + 0.5 * open);
    };
  }

  function buildOperaHouse() {
    const { quay, court, porch, hall, steps, railing, planters, screen, screenPosts } = operaHouse;
    const { x, z, width, depth } = hall;
    const y = porch.y;
    const front = z + depth / 2;
    const limestone = '#d1cec0';
    const frame = '#465859';
    const bronze = '#9e805e';
    terrace(quay.x, quay.z, quay.width, quay.depth, quay.y, quay.seed);
    terrace(court.x, court.z, court.width, court.depth, court.y, court.seed);
    const courtGrid = surfaceGrid(court.width, court.depth);
    for (const offset of [-5, 5]) {
      const lineX = court.x + offset * courtGrid.cellWidth;
      block(lineX, court.y + 0.002, court.z, 0.07, 0.012, court.depth - 0.12, '#817c69');
    }
    const { x: screenX, z: screenZ } = screen;
    block(screenX + 1.13, quay.y + 2.94, screenZ, 2.99, 0.24, 10.65, limestone);
    block(screenX + 1.13, quay.y + 2.87, screenZ, 2.87, 0.075, 10.49, '#a5906e');
    for (const post of screenPosts) {
      block(post.x, quay.y, post.z, post.width, 2.99, post.depth, frame);
    }
    for (let slat = 0; slat < 20; slat++) {
      block(screenX - 0.10, quay.y + 0.73, screenZ + (slat - 9.5) * 0.49,
        0.12, 2.16, 0.062, slat % 4 ? bronze : '#635a4c', 0.26);
    }
    for (const along of [-3.25, 3.25]) {
      light(screenX + 1.1, quay.y + 2.848, screenZ + along, 1.55, 0.03, 0.041, '#e4c18f');
      nightLamp(screenX + 1.1, quay.y + 2.68, screenZ + along, '#ffd19a', 12, 5.4);
    }
    const { entrance, backstage } = operaHouse;
    const openingWest = backstage.x - backstage.width / 2;
    const openingEast = backstage.x + backstage.width / 2;
    const stageWings = [[x - width / 2, openingWest], [openingEast, x + width / 2]];
    for (const [west, east] of stageWings) {
      block((west + east) / 2, quay.y, z, east - west, y - quay.y - 0.19, depth, stone);
      block((west + east) / 2, y - 0.19, z, east - west, 0.19, depth, limestone);
    }
    const vestibuleDepth = 0.76;
    block(backstage.x, quay.y, z + vestibuleDepth / 2, backstage.width, y - quay.y - 0.19, depth - vestibuleDepth, stone);
    block(backstage.x, y - 0.19, z + vestibuleDepth / 2, backstage.width, 0.19, depth - vestibuleDepth, limestone);
    block(porch.x, quay.y, porch.z, porch.width, y - quay.y - 0.18, porch.depth, '#787563');
    const { columns, rows, cellWidth, cellDepth } = surfaceGrid(porch.width, porch.depth);
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < columns; col++) {
        block(porch.x - porch.width / 2 + (col + 0.5) * cellWidth, y - 0.18,
          porch.z - porch.depth / 2 + (row + 0.5) * cellDepth,
          cellWidth - pavingJoint, 0.18, cellDepth - pavingJoint, col % 3 === 1 ? porcelain : stone);
      }
    }
    stairs(steps.x, steps.z, quay.y, y, steps.run, steps.width, steps.direction, steps.steps, stairLanes(steps.width));
    for (const curb of operaHouse.stairCurbs) {
      for (let i = 0; i < steps.steps; i++) {
        const tread = steps.run / steps.steps;
        block(curb.x, quay.y - 0.07, steps.z - (i + 0.5) * tread,
          curb.width, 0.33 + (y - quay.y) * (i + 1) / steps.steps, tread + 0.012, bronze);
      }
    }

    const auditoriumHeight = 6.9;
    const auditoriumX = x + 0.83;
    const auditoriumZ = z - 0.25;
    const auditoriumWidth = 7.55;
    const auditoriumDepth = 5.83;
    const auditorium = new T.Mesh(
      new T.CylinderGeometry(1, 1, auditoriumHeight, 24),
      new T.MeshStandardMaterial({ color: '#9f5543', roughness: 0.83 }),
    );
    auditorium.position.set(auditoriumX, y + auditoriumHeight / 2, auditoriumZ);
    auditorium.scale.set(auditoriumWidth, 1, auditoriumDepth);
    auditorium.castShadow = auditorium.receiveShadow = true;
    scene.add(auditorium);
    block(x + 0.97, y + 5.85, z - 1.8, 11.9, 1.42, 6.91, '#9faaa5');
    for (let course = 0; course < 5; course++) {
      block(x + 0.97, y + 6.25 + course * 0.18, z + 1.68, 11.9, 0.051, 0.10, frame);
    }
    for (const [west, east] of stageWings) {
      block((west + east) / 2, y + 0.15, backstage.z + 0.22, east - west, 6.24, 0.46, limestone);
    }
    const rearDoorTop = backstage.y + 2.51;
    block(backstage.x, rearDoorTop, backstage.z + 0.22, backstage.width, y + 6.39 - rearDoorTop, 0.46, limestone);
    for (let rib = -14; rib <= 14; rib++) {
      const angle = rib * Math.PI / 27;
      const ribX = auditoriumX + auditoriumWidth * Math.sin(angle);
      const ribZ = auditoriumZ + auditoriumDepth * Math.cos(angle);
      block(ribX, y + 0.35, ribZ, 0.085, 6.01, 0.13, rib % 3 ? '#b77150' : '#d39a69', angle);
    }

    const lobbyHeight = 4.98;
    const lobbyGlass = new T.MeshPhysicalMaterial({
      color: '#86a9ad', roughness: 0.22, metalness: 0.22,
      transparent: true, opacity: 0.38, depthWrite: false, side: T.DoubleSide,
    });
    const doorHeight = 2.62;
    const flank = (width - 0.08 - entrance.width) / 2;
    for (const side of [-1, 1]) {
      const facade = new T.Mesh(new T.PlaneGeometry(flank, lobbyHeight), lobbyGlass);
      facade.position.set(x + side * (entrance.width + flank) / 2, y + 0.18 + lobbyHeight / 2, front - 0.04);
      scene.add(facade);
    }
    const transomHeight = lobbyHeight + 0.18 - doorHeight;
    const transom = new T.Mesh(new T.PlaneGeometry(entrance.width, transomHeight), lobbyGlass);
    transom.position.set(x, y + doorHeight + transomHeight / 2, front - 0.04);
    scene.add(transom);
    for (let pane = 0; pane <= 16; pane++) {
      const paneX = x + (pane / 16 - 0.5) * (width - 0.08);
      const overDoor = Math.abs(paneX - x) < entrance.width / 2;
      block(paneX, y + (overDoor ? doorHeight : 0.16), front - 0.018, 0.047,
        overDoor ? transomHeight : lobbyHeight, 0.12, frame);
    }
    block(x, y + 3.76, front - 0.012, width, 0.055, 0.10, frame);
    block(x, y + 5.1, front - 0.08, width + 0.2, 0.19, 0.24, limestone);
    for (const post of operaHouse.facadePosts) {
      block(post.x, y, post.z, post.width, 0.22, post.depth, limestone);
      block(post.x, y + 0.18, post.z, 0.20, 5.24, 0.26, frame);
    }
    for (const side of [-1, 1]) {
      const edge = x + side * (width / 2 - 0.04);
      const glassReturn = new T.Mesh(new T.PlaneGeometry(depth - 0.32, lobbyHeight), lobbyGlass);
      glassReturn.rotation.y = side * Math.PI / 2;
      glassReturn.position.set(edge, y + 0.18 + lobbyHeight / 2, z + 0.12);
      scene.add(glassReturn);
      for (let pane = 0; pane <= 12; pane++) {
        block(edge, y + 0.16, front - 0.07 - pane * (depth - 0.3) / 12, 0.13, lobbyHeight, 0.059, frame);
      }
      block(edge, y + 3.76, z + 0.1, 0.11, 0.055, depth - 0.25, frame);
      block(x + side * 5.61, y + 0.2, front - 1.12, 2.2, 0.57, 0.72, '#a9805e');
      light(x + side * 4.66, y + 4.91, front - 1.10, 6.41, 0.035, 0.075, '#edc08a');
      light(x + side * 5.45, y + 0.14, front + 0.82, 4.25, 0.038, 0.046, '#e6c28a');
      nightLamp(x + side * 5.35, y + 3.08, front + 0.7, '#ffc788', 22, 7.5);
    }
    nightLamp(x - width / 2 - 0.16, y + 2.05, z + 0.7, '#e2ad72', 14, 6.6);
    sign(x - 5.7, y + 3.9, front + 0.155, 2.35, '聽潮戲樓', '#414c52');

    const canopy = new T.MeshStandardMaterial({ vertexColors: true, roughness: 0.62 });
    for (const [cx, cy, cz, w, d, rise] of [
      [x + 0.6, y + 7.35, z - 0.91, 20.4, 13.75, 2.7],
      [x - 0.19, y + 5.34, front - 0.18, 19.3, 4.95, 1.25],
    ]) {
      const shell = new T.Mesh(createOperaCanopyGeometry(T, w, d, rise), canopy);
      shell.name = 'opera-canopy';
      shell.position.set(cx, cy, cz);
      shell.castShadow = shell.receiveShadow = true;
      scene.add(shell);
    }

    block(railing.x, quay.y + 0.71, railing.z, railing.width, 0.11, railing.depth, frame);
    const railPosts = Math.ceil(railing.depth / 1.18);
    for (let i = 0; i < railPosts; i++) {
      const railZ = railing.z + ((i + 0.5) / railPosts - 0.5) * railing.depth;
      block(railing.x, quay.y, railZ, railing.width, 0.78, 0.18, stone);
      block(railing.x, quay.y + 0.77, railZ, 0.21, 0.09, 0.22, bronze);
    }
    for (const [index, planter] of planters.entries()) {
      block(planter.x, quay.y, planter.z, planter.width, 0.37, planter.depth, limestone);
      block(planter.x, quay.y + 0.37, planter.z, planter.width - 0.40, 0.055, planter.depth - 0.42, '#68745a');
      if ('tree' in planter) {
        tree(planter.x, quay.y + 0.43, planter.z, index ? 3.5 : 2.72, 712 + index * 40, planter.tree);
      } else {
        const longSide = Math.max(planter.width, planter.depth);
        const lengthwise = planter.depth > planter.width;
        const patches = Math.ceil(longSide / 1.4);
        for (let patch = 0; patch < patches; patch++) {
          const offset = ((patch + 0.5) / patches - 0.5) * longSide;
          shrub(planter.x + (lengthwise ? 0 : offset), quay.y + 0.43, planter.z + (lengthwise ? offset : 0),
            Math.min(1.16, Math.min(planter.width, planter.depth) * 0.85), 712 + index * 40 + patch, planter.shrub);
        }
      }
      if (planter.surface !== quay.id) {
        const facing = planter.x > quay.x ? -1 : 1;
        const benchX = planter.x + facing * (planter.width / 2 - 0.2);
        block(benchX, quay.y + 0.37, planter.z, 0.39, 0.085, planter.depth - 0.18, '#9e7753');
        if ('tree' in planter) shrub(planter.x, quay.y + 0.43, planter.z + planter.depth * 0.29, 1.12, 274 + index, 'mound');
        const lampX = planter.x + facing * (planter.width / 2 - 0.45);
        light(lampX, quay.y + 0.43, planter.z - 0.65, 0.11, 0.14, 1.02, '#e0ba82');
        nightLamp(lampX, quay.y + 0.66, planter.z - 0.65, '#eac18b', 13, 5.6);
      }
    }
    return slidingOperaDoors(entrance, doorHeight, lobbyGlass);
  }

  function buildOperaStreetFurniture() {
    const { ticketBooth: booth, gallery, quay } = operaHouse;
    const y = quay.y;
    const front = booth.z + booth.depth / 2;
    const wall = '#526360';
    const concrete = '#bdb7a5';
    const aluminum = '#303e42';
    block(booth.x, y, booth.z, booth.width, 0.12, booth.depth, concrete);
    block(booth.x, y + 0.12, booth.z - booth.depth / 2 + 0.13, booth.width - 0.12, 2.26, 0.23, aluminum);
    const glassSide = new T.PlaneGeometry(booth.depth - 0.27, 1.86);
    const glazing = new T.MeshPhysicalMaterial({
      color: '#91b6bc', roughness: 0.22, metalness: 0.19,
      transparent: true, opacity: 0.35, depthWrite: false, side: T.DoubleSide,
    });
    for (const side of [-1, 1]) {
      const edge = booth.x + side * (booth.width / 2 - 0.075);
      block(edge, y + 0.12, booth.z, 0.11, 0.39, booth.depth, aluminum);
      block(edge, y + 2.28, booth.z, 0.11, 0.13, booth.depth, aluminum);
      for (const jamb of [-1, 1]) {
        block(edge, y + 0.39, booth.z + jamb * (booth.depth / 2 - 0.075), 0.11, 1.93, 0.13, aluminum);
      }
      const pane = new T.Mesh(glassSide, glazing);
      pane.position.set(edge, y + 1.38, booth.z);
      pane.rotation.y = side * Math.PI / 2;
      scene.add(pane);
    }
    block(booth.x, y + 0.12, front - 0.31, booth.width - 0.27, 0.82, 0.48, '#677d79');
    block(booth.x, y + 0.92, front - 0.31, booth.width - 0.16, 0.085, 0.62, '#dfddd1');
    block(booth.x + 0.87, y + 1.005, front - 0.4, 0.36, 0.24, 0.14, aluminum);
    light(booth.x + 0.87, y + 1.055, front - 0.323, 0.26, 0.126, 0.016, '#a9d3be');

    const halfWidth = (booth.width + 0.42) / 2;
    const halfDepth = (booth.depth + 0.62) / 2;
    const cut = 0.17;
    const canopyOutline = new T.Shape([
      new T.Vector2(-halfWidth + cut, -halfDepth), new T.Vector2(halfWidth - cut, -halfDepth),
      new T.Vector2(halfWidth, -halfDepth + cut), new T.Vector2(halfWidth, halfDepth - cut),
      new T.Vector2(halfWidth - cut, halfDepth), new T.Vector2(-halfWidth + cut, halfDepth),
      new T.Vector2(-halfWidth, halfDepth - cut), new T.Vector2(-halfWidth, -halfDepth + cut),
    ]);
    const canopy = new T.Mesh(new T.ExtrudeGeometry(canopyOutline, { depth: 0.115, bevelEnabled: false }),
      new T.MeshStandardMaterial({ color: '#d5d7d0', roughness: 0.62 }));
    canopy.rotation.x = -Math.PI / 2;
    canopy.position.set(booth.x, y + 2.41, booth.z + 0.12);
    canopy.castShadow = canopy.receiveShadow = true;
    scene.add(canopy);
    block(booth.x, y + 2.285, front - 0.04, booth.width - 0.04, 0.125, 0.16, aluminum);
    light(booth.x, y + 2.39, front + 0.28, booth.width - 0.38, 0.028, 0.034, '#f0d0a7');
    nightLamp(booth.x, y + 2.18, front - 0.05, '#ffcf9e', 14, 5.3);

    const artwork = document.createElement('canvas');
    artwork.width = 384;
    artwork.height = 62;
    const paint = artwork.getContext('2d');
    if (!paint) throw new Error('Could not draw the opera ticket display.');
    paint.fillStyle = '#243437';
    paint.fillRect(0, 0, artwork.width, artwork.height);
    paint.fillStyle = '#a4d3b5';
    paint.fillRect(15, 15, 4, 32);
    paint.fillStyle = '#f1e6cf';
    paint.textBaseline = 'middle';
    paint.font = 'bold 38px "PingFang SC", "Noto Sans CJK SC", sans-serif';
    paint.fillText('售票', 38, 33);
    paint.font = '600 24px system-ui, sans-serif';
    paint.fillText('TICKETS', 139, 33);
    const displayTexture = new T.CanvasTexture(artwork);
    displayTexture.colorSpace = T.SRGBColorSpace;
    const display = new T.Mesh(new T.PlaneGeometry(2.28, 0.368), new T.MeshBasicMaterial({ map: displayTexture }));
    display.name = 'opera-ticket-display';
    display.position.set(booth.x - 0.2, y + 0.63, front - 0.064);
    scene.add(display);

    block(gallery.x, y, gallery.z, gallery.width, 0.23, gallery.depth, concrete);
    block(gallery.x, y + 0.2, gallery.z, gallery.width - 0.26, 1.39, gallery.depth - 0.31, wall);
    block(gallery.x, y + 1.55, gallery.z, gallery.width + 0.20, 0.14, gallery.depth + 0.09, concrete);
    for (const [offset, title, tint] of [[-1.72, '今日演出', '#7b5045'], [0, '聽潮戲樓', wall], [1.72, '候場小院', '#536559']] as const) {
      sign(gallery.x + offset, y + 1.09, gallery.z + gallery.depth / 2 - 0.125, 1.38, title, tint);
      light(gallery.x + offset, y + 1.47, gallery.z + gallery.depth / 2 - 0.094, 1.05, 0.025, 0.04, '#d1ae78');
    }

    const seamX = quay.x + quay.width / 2 - 0.34;
    for (const z of [44.2, 47.2, 50.2, 53.2]) {
      block(seamX, y + 0.002, z, 0.22, 0.023, 2.55, '#8d8573');
      light(seamX, y + 0.025, z, 0.054, 0.018, 1.01, '#c6a878');
    }
  }

  function buildOperaGarden() {
    const { rearCourt, rearPassage, garden, pool, rearRail, gardenRail, gardenBench, backstage } = operaHouse;
    const y = rearCourt.y;
    for (const deck of [rearCourt, rearPassage, garden]) {
      terrace(deck.x, deck.z, deck.width, deck.depth, deck.y, deck.seed);
    }

    const rim = 0.18;
    block(pool.x, y, pool.z, pool.width, 0.19, pool.depth, '#657870');
    for (const side of [-1, 1]) {
      block(pool.x + side * (pool.width - rim) / 2, y + 0.16, pool.z, rim, 0.15, pool.depth, stone);
      block(pool.x, y + 0.16, pool.z + side * (pool.depth - rim) / 2, pool.width - rim * 2, 0.15, rim, stone);
    }
    const stillWater = new T.Mesh(
      new T.PlaneGeometry(pool.width - rim * 2, pool.depth - rim * 2),
      new T.MeshPhysicalMaterial({ color: '#365f61', metalness: 0.46, roughness: 0.16, clearcoat: 0.8, clearcoatRoughness: 0.14 }),
    );
    stillWater.position.set(pool.x, y + 0.217, pool.z);
    stillWater.rotation.x = -Math.PI / 2;
    stillWater.receiveShadow = true;
    scene.add(stillWater);
    for (const side of [-1, 1]) {
      light(pool.x + side * 1.47, y + 0.221, pool.z + pool.depth / 2 - 0.22, 0.82, 0.022, 0.032, '#b3cfac');
    }

    for (const rail of [rearRail, gardenRail]) {
      block(rail.x, y + 0.72, rail.z, rail.width, 0.09, rail.depth, '#4f6261');
      const gaps = Math.round(rail.width / 1.25);
      for (let post = 0; post <= gaps; post++) {
        block(rail.x + (post / gaps - 0.5) * (rail.width - 0.19), y, rail.z, 0.11, 0.77, 0.13, '#68716a');
      }
    }
    block(gardenBench.x, y, gardenBench.z, gardenBench.width, 0.37, gardenBench.depth, stone);
    block(gardenBench.x, y + 0.37, gardenBench.z, gardenBench.width, 0.09, 0.57, '#9b7959');
    for (const side of [-1, 1]) {
      block(gardenBench.x + side * 1.51, y, gardenBench.z - 0.42, 0.16, 2.69, 0.14, '#555c54');
    }
    block(gardenBench.x, y + 2.65, gardenBench.z, 3.84, 0.16, 2.30, '#bab8a7');
    for (let slat = -8; slat <= 8; slat++) {
      block(gardenBench.x + slat * 0.188, y + 0.5, gardenBench.z - 0.48, 0.064, 2.14, 0.06, '#8c765f');
    }
    light(gardenBench.x, y + 2.64, gardenBench.z + 0.12, 2.31, 0.028, 0.046, '#eace9b');
    nightLamp(gardenBench.x, y + 2.50, gardenBench.z + 0.12, '#f6c388', 17, 6.1);

    block(backstage.x, y + 2.51, backstage.z - 0.11, 2.63, 0.12, 0.23, '#a9a699');
    light(backstage.x, y + 2.54, backstage.z - 0.235, 1.66, 0.048, 0.045, '#ddc38d');
    return slidingOperaDoors({ ...backstage, z: backstage.z - 0.085 }, 2.51,
      new T.MeshStandardMaterial({ color: '#68716b', roughness: 0.52 }));
  }

  terrace(-17.7, 5.8, 14.3, 22.4, 3.1, 9);
  terrace(-23.3, -4.6, 7.2, 11.8, 6.2, 49);
  terrace(-20.6, -14.4, 13.3, 9.1, 7.1, 18);
  terrace(16.7, 5, 9.2, 17.8, 3.1, 32);
  terrace(18.1, 15.2, 13.3, 4.2, 1.6, 84, true);
  terrace(22.5, -3.7, 9.5, 11.4, 4.55, 76, true);
  terrace(22.1, -14.0, 10.1, 9.4, 6.9, 96, true);
  block(0, -5.2, 1.0, 28, 3.85, 37.0, '#48534c');
  block(0, -4.8, 28.2, 18, 3.45, 23.6, '#56635a');
  terrace(-16.0, 22.1, 11.2, 10.3, 1.6, 63);
  terrace(-15.43, 30.75, 17.35, 12.1, 0.91, 55);
  terrace(16.1, 22.47, 10.8, 10.84, 1.6, 32);
  terrace(14.66, 31.11, 16.19, 13.57, 0.91, 76);
  terrace(25.42, 27.2, 6.5, 14.1, 2.45, 79, true);
  for (const side of [-1, 1]) {
    for (let z = 18.17; z < 24.05; z += 0.42) {
      const edge = canalHalfWidth(z);
      block(side * (edge + 0.69), -0.72, z, 1.44, 1.57, 0.46, '#849083');
      block(side * (edge + 1.38), -1.34, z, 1.25, 2.20, 0.46, '#66796d');
    }
    block(side * 6.72, -1.38, 31.76, 0.35, 2.22, 15.61, '#77847a');
    for (let z = 25.0; z < 38; z += 0.84) {
      block(side * 6.84, 0.83, z, 0.48, 0.21, 0.80, stone);
      block(side * 6.58, -0.58, z, 0.13, 1.32, 0.07, '#586b64');
    }
  }

  shop(-21.5, 3.1, 9.65, 5.3, 4.4, lacquer, 'teahouse', '北岸茶館');
  shop(-20.7, 3.1, 1.8, 5.6, 4.5, '#477267', 'market', '百川市集');
  shop(-15.3, 3.1, -1.96, 4.3, 3.4, lacquer, 'market');
  shop(-25.15, 6.2, -6.5, 3.05, 3.85, '#477267', 'workshop');
  shop(-15.5, 3.1, 14.85, 3.8, 2.55, '#477267', 'station');
  shop(18.1, 3.1, 4.69, 4.9, 4.2, '#477267', 'market');
  shop(22.9, 4.55, -4.7, 5.05, 4.8, lacquer, 'station', '山門');
  shop(18.8, 3.1, -1.25, 3.75, 2.72, '#836249', 'workshop');
  eaves(-19.1, 5.9, -8.28, 6.1, 3.39);
  block(-19.1, 3.1, -8.28, 4.85, 2.8, 2.8, glass);
  for (const [x, z, y, w, d, n, style] of [
    [-21.5, -14.2, 7.1, 5.6, 4.75, 8, 2],
    [22.7, -15.2, 6.9, 5.3, 4.4, 5, 2],
  ]) tower(x, z, y, w, d, n, style, x * 3);

  stairs(-16.55, -4.6, 3.1, 6.2, 3.25, 1.8, 'west', 9);
  stairs(-24.65, -8.70, 6.2, 7.1, 1.66, 1.75, 'north', 4);
  stairs(20.67, 4.25, 3.1, 4.55, 2.45, 1.25, 'north', 7);
  stairs(26.04, -5.86, 4.55, 6.9, 3.6, 1.55, 'north', 8);
  stairs(16.05, 15.45, 1.6, 3.1, 2.04, 1.55, 'north', 6);
  stairs(-20.5, 18.77, 1.6, 3.1, 2.08, 2.05, 'north', 6);
  stairs(-17.35, 29.3, 0.91, 1.6, 2.20, 1.58, 'north', 4);
  stairs(17.3, 29.6, 0.91, 1.6, 2.12, 1.74, 'north', 4);
  stairs(20.0, 29.25, 0.91, 2.45, 2.24, 1.44, 'east', 6);

  shop(-19.2, 1.6, 23.5, 4.0, 3.58, '#725d43', 'workshop', '修造');
  shop(-17.86, 0.91, 33.16, 4.8, 4.15, '#914f3f', 'teahouse', '南渠茶室');
  shop(-10.15, 0.91, 29.34, 4.04, 3.51, '#497269', 'station', '渡口');
  shop(13.9, 0.91, 32.32, 4.32, 4.30, '#4f7165', 'station', '南渠公所');
  shop(18.94, 0.91, 35.42, 4.13, 2.96, '#855641', 'market', '小食');
  // The lock house has a lower public arcade and an upper control room.
  block(12.94, 1.6, 21.48, 4.13, 0.23, 4.17, stone);
  for (const x of [11.30, 12.35, 13.50, 14.58]) {
    block(x, 1.83, 22.94, 0.28, 2.20, 0.30, lacquer);
    block(x, 1.83, 22.94, 0.41, 0.25, 0.38, stone);
  }
  block(12.94, 1.83, 20.50, 4.0, 2.20, 2.65, '#756a54');
  block(12.94, 3.97, 21.48, 4.32, 0.40, 4.36, dark);
  eaves(12.94, 4.38, 21.20, 4.86, 4.51);
  block(12.94, 5.29, 20.65, 2.62, 1.23, 2.36, '#3c6260');
  for (const x of [12.04, 12.64, 13.24, 13.84]) {
    block(x, 5.32, 21.85, 0.085, 1.15, 0.09, porcelain);
  }
  eaves(12.94, 6.50, 20.65, 3.07, 2.74);
  sign(12.94, 3.77, 23.23, 2.19, '南渠閘房', '#4a635c');

  for (const [x, z, shade] of [
    [-13.62, 31.2, lacquer], [-15.7, 26.02, '#9b7951'], [10.13, 28.6, '#638a74'],
    [14.89, 36.00, '#725e51'], [-11.47, 35.38, '#8a6647'],
  ] as const) {
    block(x, 0.91, z, 1.16, 0.64, 0.52, '#86694a');
    for (const edge of [-1, 1]) block(x + edge * 0.49, 1.37, z, 0.10, 1.01, 0.11, dark);
    for (let stripe = 0; stripe < 7; stripe++) {
      block(x + (stripe - 3) * 0.185, 2.32, z, 0.185, 0.085, 0.97,
        stripe % 3 ? shade : '#c9ad76');
    }
  }
  tower(-21.5, 29.0, 0.91, 3.1, 3.3, 4, 2, 61);
  const setGaugeSignal = gaugeTower(...gaugeSite);
  practiceCourt(...practiceSite);
  teaCounter(...teaStand);
  const striker = rainbellSquare();
  plantRainbellGarden();
  const updateFrontDoor = buildOperaHouse();
  const updateBackstageDoor = buildOperaGarden();
  buildOperaStreetFurniture();
  for (const [x, z, size] of [[-22.9, 35.4, 2.35], [10.32, 34.3, 2.0], [20.6, 37.0, 2.1], [24.5, 22.5, 2.65]]) {
    const y = x > 22 ? 2.45 : 0.91;
    block(x, y, z, size * 0.94, 0.23, size * 0.91, '#949b79');
    tree(x, y + 0.23, z, size, z * 7, x > 22 ? 'pine' : 'willow');
  }

  // A low stone crossing connects the downstream locks and the quay.
  block(0, 0.60, 26.9, 14.35, 0.31, 2.13, porcelain);
  for (const side of [-1, 1]) {
    for (const x of [4.62, 6.26]) {
      block(side * x, -1.25, 26.9, 0.96, 1.88, 1.56, stone);
      block(side * x, 0.38, 26.9, 1.14, 0.20, 1.77, '#9b9981');
      block(side * x, -0.22, 26.9, 1.03, 0.086, 1.67, '#728576');
    }
    const z = 26.9 + side * 0.94;
    block(0, 0.66, z, 14.31, 0.11, 0.15, '#8b987d');
    block(0, 1.17, z, 14.20, 0.12, 0.115, '#768f7d');
    block(0, 1.59, z, 14.28, 0.12, 0.16, '#506e63');
    for (let i = -6; i <= 6; i++) {
      const post = i * 1.07;
      block(post, 0.91, z, 0.155, 0.68, 0.15, stone);
      if (i % 3 === 0) {
        block(post, 1.68, z, 0.25, 0.105, 0.165, '#9b9f81');
        block(post, 1.26, z + side * 0.071, 0.11, 0.16, 0.035, copper);
      }
    }
  }
  for (const x of [-7.15, 7.15]) {
    for (const z of [25.8, 28.2, 31.0, 34.2, 37.3]) {
      block(x, 0.91, z, 0.23, 0.24, 0.23, stone);
      block(x, 1.14, z, 0.10, 1.06, 0.10, dark);
      lantern(x, 2.17, z, 'signal');
    }
  }
  for (const side of [-1, 1]) {
    block(side * 7.10, -2.20, 38.2, 1.1, 5.5, 1.86, '#7f806a');
    block(side * 7.10, 3.10, 38.2, 1.67, 0.38, 2.28, '#998c72');
    block(side * 7.10, 3.48, 38.2, 1.83, 0.22, 2.42, dark);
    for (let row = 0; row < 8; row++) {
      // The built east quay replaces the wooded bank downstream of the crossing.
      if (side > 0 || row < 5) continue;
      const z = 40.35 + row * 3.5;
      const bank = canalHalfWidth(z);
      for (let i = 0; i < 5; i++) {
        const seed = side * 49 + row * 11 + i * 78;
        const x = bank + i * 2.48 + random(seed);
        const height = 1.05 + i * 0.37 + random(seed + 1) * 1.51;
        block(side * x, -2.2, z + random(seed + 2) * 2.39, 2.1 + random(seed + 3), height,
          2.7 + random(seed + 4) * 1.3, ['#52655a', '#606f5f', '#576b56'][i % 3]);
        if (i > 1 && row < 6 && random(seed - 1) > 0.39) {
          tree(side * x, height - 2.18, z + 0.84, 1.2 + random(seed + 7) * 1.08, seed, i === 2 ? 'willow' : 'pine');
        }
      }
    }
  }
  block(0, 2.80, 38.2, 15.29, 0.29, 1.06, '#87866e');
  block(0, 3.09, 38.2, 15.72, 0.16, 1.19, dark);
  for (let panel = -4; panel <= 4; panel++) {
    block(panel * 1.31, 3.25, 38.2, 0.10, 0.22, 0.89, stone);
  }

  // The downstream crossing and the dam gallery form a loop on the market levels.
  block(-0.22, 2.76, 12.02, 26.18, 0.34, 1.96, stone);
  for (const post of [-8.1, 7.7]) block(post, -0.15, 12.02, 0.58, 2.93, 0.99, stone);
  for (const edge of [-1, 1]) {
    block(-0.22, 3.96, 12.02 + edge * 0.88, 26.10, 0.10, 0.08, dark);
    for (let i = 0; i < 20; i++) {
      block(-12.58 + i * 1.30, 3.1, 12.02 + edge * 0.88, 0.08, 0.88, 0.08, dark);
    }
  }
  for (const [x, z, y, shade] of [
    [-18.7, 6.7, 3.1, lacquer], [-17.0, 9.45, 3.1, '#658178'],
    [-20.3, 13.0, 3.1, '#b68d58'], [-22.7, 5.87, 3.1, '#b68d58'],
    [15.0, 3.4, 3.1, '#698574'], [22.6, -8.45, 4.55, lacquer],
  ] as const) {
    block(x, y + 1.66, z, 1.38, 0.11, 0.98, shade);
    block(x, y + 0.66, z + 0.16, 1.28, 0.15, 0.45, '#aa946b');
    for (const side of [-1, 1]) block(x + side * 0.61, y, z, 0.083, 1.66, 0.08, dark);
  }
  for (const [x, y, z] of [[-22.75, 6.2, -2.6], [24.26, 6.9, -11.30]]) {
    block(x, y + 1.72, z, 3.94, 0.18, 1.3, dark);
    block(x, y + 1.90, z, 3.75, 0.16, 1.15, '#566e63');
    light(x, y + 1.69, z + 0.57, 3.75, 0.055, 0.09, '#bcce8f');
    for (const side of [-1, 1]) block(x + side * 1.78, y, z + 0.44, 0.13, 1.78, 0.15, dark);
  }
  for (const [x, z, y, size, kind] of [
    [-14, 11.0, 3.1, 2.2, 'ginkgo'], [-23.5, 15.1, 3.1, 1.9], [-15.05, 4.45, 3.1, 1.7],
    [16, 9.7, 3.1, 2.65], [18, 15.2, 1.6, 2.7, 'goldenrain'], [21, 15.7, 1.6, 2.3],
    [20.7, 1.25, 4.55, 2.05, 'pine'], [24.6, -1.85, 4.55, 1.96, 'pine'],
    [25.3, -10.4, 6.9, 3.0, 'pine'], [20.9, -11.2, 6.9, 2.52, 'pine'], [24.5, -17.8, 6.9, 2.1, 'pine'],
  ] as const) {
    block(x, y + 0.01, z, size * 0.82, 0.2, size * 0.82, '#838b68');
    tree(x, y + 0.2, z, size, x * z, kind);
  }
  for (const side of [-1, 1]) {
    for (let i = 0; i < 5; i++) {
      const z = 11.8 - i * 2.3;
      const x = side * 12.92;
      block(x, 3.1, z, 0.13, 1.1, 0.13, dark);
      block(x, 4.22, z, 0.42, 0.12, 0.42, dark);
      light(x, 4.0, z, 0.26, 0.22, 0.26, '#efca8e');
    }
  }

  // The spillway drops through the center of the dam. Its access gallery sits downstream.
  const crest = spillwayCrest;
  for (const side of [-1, 1]) {
    block(side * 11.9, 0, -9.9, 6.5, crest + 0.61, 5.6, '#9b987e');
    for (const [y, trim, setback] of [[0.24, 3.0, 0.72], [6.82, 0.62, 0.48], [12.23, 0.51, 0.32], [18.9, 0.42, 0.22]]) {
      block(side * 11.9, y, -9.74, 6.52 + setback, trim, 5.94 + setback, '#777e6d');
    }
    block(side * 11.9, crest + 0.46, -9.9, 6.8, 0.36, 5.9, porcelain);
    eaves(side * 11.9, crest + 0.85, -9.9, 6.72, 5.88);
    block(side * 11.9, crest - 3.99, -6.28, 6.12, 0.57, 0.3, dark);
    light(side * 11.9, crest - 3.83, -6.10, 5.66, 0.13, 0.08, '#9ac9ad');
    nightLamp(side * 11.9, crest - 4.05, -4.9, '#8af0e3', 48, 12);
    for (let rib = 0; rib < 4; rib++) {
      block(side * (9.2 + rib * 1.46), 0.3, -6.91, 0.37, crest + 0.03, 1.10, porcelain);
    }
    block(side * 11.6, crest - 2.28, -6.75, 5.5, 0.33, 0.89, lacquer);
    block(side * 11.6, crest - 1.72, -6.75, 5.5, 0.96, 0.82, dark);
    for (let i = 0; i < 9; i++) {
      block(side * (9.34 + i * 0.55), crest - 1.63, -6.29, 0.33, 0.69, 0.12, glass);
    }
    block(side * 8.22, 0, -8.65, 0.57, crest + 0.26, 2.1, dark);
    block(side * 8.22, crest - 0.12, -7.77, 0.68, 0.46, 0.54, copper);
    block(side * 28.9, 6.10, -13.0, 28.1, crest - 5.67, 5.35, '#858b78');
    block(side * 28.9, crest + 0.35, -13.0, 28.3, 0.37, 5.60, porcelain);
    block(side * 28.9, crest + 0.72, -10.62, 28.2, 0.47, 0.24, stone);
    block(side * 13.8, 0, -13.1, 3.7, crest + 0.84, 4.4, stone);
    shop(side * 20.2, crest + 0.72, -13.17, 3.7, 2.72, '#557469', 'station', '水務');
    block(side * 28.9, 11.86, -9.92, 28.0, 0.39, 0.85, '#536f66');
    block(side * 28.9, 12.08, -9.48, 28.0, 0.055, 0.08, '#b7be97');
    for (let seam = 0; seam < 11; seam++) {
      const x = side * (15.96 + seam * 2.51);
      block(x, 6.1, -10.25, 0.59, crest - 5.45, 1.13, '#a6a390');
      block(x, 6.1, -9.81, 0.94, 4.25, 1.55, stone);
      block(x, crest - 2.1, -9.82, 0.82, 0.62, 0.87, '#a2a28c');
      light(x, 12.10, -10.25, 0.26, 0.11, 0.09, '#c1d79e');
    }
    for (const outlet of [20.7, 32.7]) {
      const x = side * outlet;
      block(x, 6.17, -9.96, 2.03, 3.03, 0.85, '#4b5d56');
      for (let grate = -3; grate <= 3; grate++) {
        block(x + grate * 0.24, 6.22, -9.49, 0.082, 2.72, 0.12, '#879285');
      }
    }
  }
  block(0, -1.8, -13.3, 26, crest + 1.63, 7.0, '#627473');
  for (let i = 0; i < reservoirProfile.length - 1; i++) {
    const [front, a] = reservoirProfile[i];
    const [back, b] = reservoirProfile[i + 1];
    const mid = (front + back) / 2;
    const width = Math.max(a, b) * 2;
    const depth = front - back;
    block(0, 8.0, mid, width, crest - 8.38, depth + 0.03, '#526d62');
    if (i < 2) continue;
    const segments = Math.ceil(depth / 2.6);
    for (let j = 0; j < segments; j++) {
      const t = (j + 0.5) / segments;
      const z = front - t * depth;
      const x = a + (b - a) * t;
      for (const side of [-1, 1]) {
        block(side * (x + 0.79), 16.2, z, 1.85, crest - 15.64, depth / segments + 0.09, '#7d8b73');
        block(side * (x + 0.82), crest + 0.55, z, 1.61, 0.17, depth / segments + 0.03, stone);
        if (j % 2 === 0) {
          block(side * (x + 2.3), 15.2, z, 2.0, crest - 14.72, depth / segments + 0.11, garden);
          tree(side * (x + 2.3), crest + 0.48, z, 1.84, z * side, j % 4 === 0 ? 'willow' : 'pine');
        }
      }
    }
  }
  const [mouthZ, mouthX, , mouthWidth] = feederProfile[0];
  for (const [left, right] of [[-19.7, mouthX - mouthWidth], [mouthX + mouthWidth, 19.7]]) {
    block((left + right) / 2, 16.1, mouthZ, right - left, crest - 15.47, 0.95, '#768971');
    block((left + right) / 2, crest + 0.63, mouthZ, right - left, 0.28, 1.23, stone);
  }

  // The feeder drops through wooded retaining banks before reaching the reservoir.
  for (let i = 0; i < feederProfile.length - 1; i++) {
    const [front, ax, ay, aw] = feederProfile[i];
    const [back, bx, by, bw] = feederProfile[i + 1];
    const length = front - back;
    const steps = Math.ceil(length / 1.3);
    for (let step = 0; step < steps; step++) {
      const t = (step + 0.5) / steps;
      const z = front - t * length;
      const x = ax + (bx - ax) * t;
      const y = ay + (by - ay) * t;
      const w = aw + (bw - aw) * t;
      block(x, 12.1, z, w * 2 + 0.3, y - 12.33, length / steps + 0.05, '#576f62');
      for (const side of [-1, 1]) {
        const seed = z * 31 + side * 57;
        block(x + side * (w + 0.65), 12.3, z, 1.62, y - 11.72, length / steps + 0.06, '#829078');
        const bank = x + side * (w + 2.15);
        const rise = random(seed) * 0.81;
        block(bank, 11.8, z, 2.65, y - 11.08 + rise, length / steps + 0.11, '#637a62');
        block(bank, y + 0.72 + rise, z, 2.67, 0.15, length / steps + 0.08, garden);
        if ((step + i) % 3 === 1) tree(bank, y + 0.87 + rise, z, 1.7 + random(seed + 1), seed);
      }
    }
  }
  block(-2.8, crest + 2.33, -70.15, 8.6, 0.39, 1.74, stone);
  for (const z of [-70.90, -69.4]) {
    block(-2.8, crest + 3.51, z, 8.61, 0.09, 0.12, lacquer);
    for (let i = 0; i < 10; i++) {
      block(-6.85 + i * 0.90, crest + 2.72, z, 0.095, 0.79, 0.095, dark);
    }
  }
  for (const x of [-6.17, 0.71]) block(x, crest - 0.10, -70.15, 0.67, 2.43, 1.63, stone);
  terrace(-11.6, -68.6, 8.8, 4.7, crest + 2.72, 139);
  terrace(3.95, -68.6, 5.0, 4.7, crest + 2.72, 147);
  shop(-12.26, crest + 2.72, -68.91, 3.67, 2.65, '#557469', 'workshop', '引水站');
  block(0, crest + 0.14, -10.36, 16.68, 0.29, 1.77, porcelain);
  block(0, crest + 0.43, -10.65, 17.02, 0.16, 0.18, lacquer);
  // The gantry silhouettes the spillway while keeping its approach open.
  for (const side of [-1, 1]) {
    block(side * 5.7, crest + 0.41, -11.8, 0.88, 6.9, 0.91, dark);
    block(side * 5.7, crest + 0.81, -11.31, 0.46, 6.04, 0.15, porcelain);
    light(side * 5.7, crest + 1.51, -11.22, 0.15, 4.56, 0.055, '#abdfb9');
  }
  block(0, crest + 6.89, -11.8, 13.52, 0.51, 1.29, dark);
  block(0, crest + 7.24, -11.8, 14.08, 0.24, 1.65, porcelain);
  block(0, crest + 7.47, -11.8, 9.09, 0.19, 1.14, lacquer);
  block(0, 4.91, 2.5, 29.8, 0.72, 2.42, porcelain);
  block(0, 4.6, 2.5, 30.6, 0.39, 2.84, '#556766');
  for (const side of [-1, 1]) {
    const z = 2.5 + side * 1.09;
    block(0, 6.38, z, 29.7, 0.09, 0.16, lacquer);
    for (let i = -15; i <= 15; i++) block(i * 0.94, 5.62, z, 0.06, 0.78, 0.08, dark);
    block(side * 11.9, -0.2, 2.5, 1.31, 5.08, 1.83, stone);
    for (let step = 0; step < 7; step++) {
      block(side * (15.04 + step * 0.41), 3.1, 2.5, 0.42, 2.53 - step * 0.35, 2.42, stone);
    }
  }

  // The same streets, building kits, and materials continue around the reservoir.
  for (const [x, z, width, depth, top] of [
    [-43, -6, 32, 26, 6.1], [46, -7, 29, 25, 5.9],
    [-49.5, -30, 26, 22, 19.8], [48.3, -30.5, 24, 21, 21.7],
    [-47, -49.9, 23, 18, 22.3], [43.9, -50.1, 22, 18.2, 23.9],
    [-39.3, -65.1, 24, 12.1, 24.6], [37.1, -65.3, 24, 12.1, 25.9],
    [-22.3, -77.1, 30, 16.6, 23.7], [19.5, -76.4, 31, 15.2, 23.25],
    [-28.2, -93.0, 27, 13.4, 25.4], [15.6, -91.3, 26, 12.6, 24.7],
  ]) {
    terrace(x, z, width, depth, top, x * 13 + z * 71);
    const columns = Math.floor(width / 6.6);
    const rows = Math.max(1, Math.floor(depth / 7.8));
    const columnWidth = width / columns;
    const rowDepth = depth / rows;
    for (let row = 0; row < rows; row++) {
      const street = z + depth / 2 - (row + 0.12) * rowDepth;
      for (let col = 0; col < columns; col++) {
        const seed = x + z * 19 + row * 61 + col * 7;
        const px = x - width / 2 + (col + 0.54) * columnWidth;
        const pz = z + depth / 2 - (row + 0.60) * rowDepth;
        const w = 3.2 + random(seed + 2) * 1.85;
        const d = 2.9 + random(seed + 3) * 1.65;
        if (random(seed + 15) < 0.18) {
          block(px, top, pz, 2.39, 0.23, 2.31, '#8b9477');
          tree(px, top + 0.23, pz, 2.35, seed);
        } else if (random(seed + 18) < 0.48 || row === 0) {
          const style = (row + col) % 3;
          shop(px, top, pz, w, d, [lacquer, '#477267', '#806b4b'][style],
            (['market', 'station', 'workshop'] as const)[style]);
        } else {
          const floors = 5 + Math.floor(random(seed) * 7);
          tower(px, pz, top, w, d, floors, (row + col) % 3, seed);
        }
        block(px + 2.2, top, street, 0.12, 1.25, 0.12, dark);
        light(px + 2.2, top + 1.2, street, 0.24, 0.14, 0.24, '#eac78b');
      }
    }
  }
  // Elevated streets carry the dam into the adjoining districts.
  for (const side of [-1, 1]) {
    block(side * 36.2, 10.1, -8.2, 22.4, 0.5, 2.58, stone);
    block(side * 36.2, 11.5, -7.02, 22.4, 0.12, 0.10, dark);
    for (let i = 0; i < 13; i++) {
      block(side * (25.6 + i * 1.73), 10.6, -7.02, 0.085, 0.9, 0.085, dark);
    }
    for (const x of [32.5, 39.2, 45.8]) {
      block(side * x, 5.75, -8.2, 0.68, 4.4, 0.86, stone);
    }
    for (let i = 0; i < 5; i++) {
      const x = side * (28 + i * 4.8);
      const z = 8.5 + 0.63 * i;
      terrace(x, z, 4.8, 4.15, 2.1, i * 33 + side, true);
      tree(x, 2.1, z, 1.19 + random(x) * 0.75, x * 31);
    }
  }
  const masonry = batch(blocks, districtMaterial(T));
  masonry.castShadow = masonry.receiveShadow = true;
  batch(lights, new T.MeshBasicMaterial());
  foliage.mount(scene);
  mountCityReeds(T, scene, [
    ...rainbellProps.reedBeds,
    { x: -6.47, y: 0.085, z: 29.90, width: 0.67, depth: 1.7, height: 1.44, seed: 338 },
    { x: 6.43, y: 0.085, z: 32.73, width: 0.75, depth: 2.0, height: 1.69, seed: 634 },
    { x: -6.55, y: 0.085, z: 35.15, width: 0.79, depth: 1.49, height: 1.59, seed: 412 },
  ]);

  const foothills = new T.InstancedMesh(
    new T.IcosahedronGeometry(1, 1),
    new T.MeshStandardMaterial({ color: '#496b65', roughness: 1, flatShading: true }),
    20,
  );
  for (let i = 0; i < foothills.count; i++) {
    const seed = i * 7;
    pose.position.set(-137 + i * 14.2, -7 + random(seed) * 6, -89 - random(seed + 2) * 24);
    pose.scale.set(14 + random(seed + 1) * 15, 22 + random(seed + 3) * 16, 19);
    pose.rotation.y = random(seed + 6) * Math.PI;
    pose.updateMatrix();
    foothills.setMatrixAt(i, pose.matrix);
  }
  scene.add(foothills);
  const valley = new T.Mesh(
    new T.PlaneGeometry(580, 620),
    new T.MeshStandardMaterial({ color: '#405950', roughness: 1 }),
  );
  valley.rotation.x = -Math.PI / 2;
  valley.position.set(0, -5.23, -68);
  valley.receiveShadow = true;
  scene.add(valley);
  return {
    setGaugeSignal,
    updateOperaDoors(front: number, backstage: number) {
      updateFrontDoor(front);
      updateBackstageDoor(backstage);
    },
    setNightLight(night: number) {
      for (const { lamp, intensity } of nightLamps) {
        lamp.visible = night > 0;
        lamp.intensity = intensity * night;
      }
    },
    updateRainbell() {
      striker.rotation.x = rainbellSwing(time.value);
    },
  };
}

function districtMaterial(T: typeof Three) {
  const material = new T.MeshStandardMaterial({ roughness: 0.89 });
  material.onBeforeCompile = (shader) => {
    shader.vertexShader = shader.vertexShader
      .replace('#include <common>', '#include <common>\nvarying vec3 districtPosition;')
      .replace('#include <begin_vertex>', `
        #include <begin_vertex>
        districtPosition = (modelMatrix * instanceMatrix * vec4(position, 1.0)).xyz;
      `);
    shader.fragmentShader = shader.fragmentShader
      .replace('#include <common>', `
        #include <common>
        varying vec3 districtPosition;
        float grain(vec3 p) {
          vec3 n = fract(p * .3183099 + .17) * 17.0;
          return fract(n.x * n.y * n.z * (n.x + n.y + n.z));
        }
      `)
      .replace('#include <color_fragment>', `
        #include <color_fragment>
        vec3 p = districtPosition;
        float smallGrain = grain(floor(p * 15.0));
        float stones = grain(floor(p * vec3(1.9, 3.1, 1.9)));
        float wear = .94 + .09 * stones + .06 * smallGrain;
        diffuseColor.rgb *= wear;
      `);
  };
  return material;
}
