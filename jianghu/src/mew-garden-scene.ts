import type * as Three from 'three';
import { createGardenWaterfall } from './garden-waterfall.ts';
import { createCityFoliage } from './city-foliage.ts';
import { createHippedRoofGeometry } from './city-roof.ts';
import { gardenBuildings, gardenPlanting, mewGarden } from './mew-garden.ts';
import { pavingJoint, surfaceGrid } from './surface-grid.ts';

type Block = [number, number, number, number, number, number, string];
const paving = ['#ae9e84', '#c1b196', '#b3a78e', '#cfc0a3', '#bba98f'];
const cliff = ['#4b575b', '#535e60', '#596366', '#616763', '#455358'];
const wood = '#824a36';
const plaster = '#c6bca5';
const jade = '#536a60';
const brass = '#b88d56';

export function mountMewGarden(T: typeof Three, scene: Three.Scene) {
  const structures: Block[] = [];
  const lamps: Block[] = [];
  const fixtures: Three.PointLight[] = [];
  const flora = createCityFoliage(T);
  const levels = new Map(mewGarden.surfaces.map(({ id, y }) => [id, y]));
  const y = levels.get('court')!;
  const grassPositions: number[] = [];
  const grassColors: number[] = [];
  const grassTint = new T.Color();

  function block(x: number, y: number, z: number, w: number, h: number, d: number, color: string) {
    structures.push([x, y + h / 2, z, w, h, d, color]);
  }

  function nightLight(x: number, y: number, z: number, power: number, range: number) {
    const light = new T.PointLight('#ffc483', power, range);
    light.position.set(x, y, z);
    fixtures.push(light);
    scene.add(light);
  }

  const roofTiles = new T.MeshStandardMaterial({ vertexColors: true, color: '#a1b0b9', roughness: 0.77 });
  function roof(x: number, base: number, z: number, w: number, d: number, rise: number) {
    const tiles = new T.Mesh(createHippedRoofGeometry(T, w, d, rise), roofTiles);
    tiles.position.set(x, base, z);
    tiles.castShadow = tiles.receiveShadow = true;
    scene.add(tiles);
    block(x, base + rise - 0.035, z, w * 0.192, 0.14, 0.13, jade);
    for (const side of [-1, 1]) {
      block(x + side * w * 0.097, base + rise + 0.09, z, 0.135, 0.12, 0.16, brass);
      const ribs = Math.floor(w / 0.21);
      for (let rib = 0; rib < ribs; rib++) {
        const along = (rib + 0.5) / ribs - 0.5;
        const lip = rise * 0.21 * Math.abs(2 * along) ** 5;
        block(x + along * w, base + lip - 0.05, z + side * (d / 2 - 0.035), 0.042, 0.12, 0.14, '#687972');
      }
    }
  }

  function lantern(x: number, level: number, z: number) {
    block(x, level, z, 0.34, 0.08, 0.34, jade);
    lamps.push([x, level + 0.23, z, 0.24, 0.31, 0.24, '#eec78f']);
    block(x, level + 0.38, z, 0.36, 0.085, 0.36, jade);
  }

  function verge(x: number, y: number, z: number, width: number, depth: number, seed: number) {
    for (let blade = 0; blade < 27; blade++) {
      const hash = Math.sin(seed * 53.53 + blade * 27.41) * 43758.5453;
      const u = hash - Math.floor(hash);
      const v = (u * 317.19) % 1;
      const px = x + (u - 0.5) * width * 0.9;
      const pz = z + (v - 0.5) * depth * 0.86;
      const height = 0.07 + ((u * 89.7) % 1) * 0.19;
      const lean = (v - 0.4) * 0.12;
      grassPositions.push(px - 0.025, y, pz, px + 0.025, y, pz, px + lean, y + height, pz + 0.046);
      grassTint.set(u < 0.33 ? '#718552' : u < 0.77 ? '#6d8048' : '#879058');
      for (let corner = 0; corner < 3; corner++) grassColors.push(grassTint.r, grassTint.g, grassTint.b);
    }
  }

  for (const [index, deck] of mewGarden.surfaces.entries()) {
    const y = deck.y;
    const { columns, rows, cellWidth, cellDepth } = surfaceGrid(deck.width, deck.depth,
      'columns' in deck ? deck.columns : undefined, 'rows' in deck ? deck.rows : undefined);
    if (deck.id.endsWith('bridge')) {
      block(deck.x, y - 0.48, deck.z, deck.width, 0.25, deck.depth, '#657079');
      const across = deck.id === 'west-bridge' || deck.id === 'east-bridge';
      for (const side of [-1, 1]) {
        const x = deck.x + (across ? 0 : side * (deck.width / 2 - 0.11));
        const z = deck.z + (across ? side * (deck.depth / 2 - 0.11) : 0);
        block(x, y + 0.61, z, across ? deck.width : 0.09, 0.08, across ? 0.09 : deck.depth, jade);
        for (const along of [-0.93, 0, 0.93]) {
          block(x + (across ? along : 0), y, z + (across ? 0 : along), 0.11, 0.64, 0.11, jade);
        }
      }
    } else {
      const bottom = deck.id === 'tower-landing' ? levels.get('north')! : -4.65 - index * 0.09;
      block(deck.x, bottom, deck.z, deck.width, y - bottom - 0.22, deck.depth, '#4c585f');
      for (let base = y - 0.49, course = 0; base >= bottom; base -= 0.29, course++) {
        for (const side of [-1, 1]) {
          for (let col = 0; col < columns; col++) {
            const x = deck.x + (col + 0.5) * cellWidth - deck.width / 2;
            block(x - cellWidth * 0.23, base, deck.z + side * (deck.depth / 2 - 0.02), cellWidth * 0.52 - 0.024,
              0.273, 0.11, cliff[(col * 3 + course * 7 + index) % cliff.length]);
            block(x + cellWidth * 0.26, base, deck.z + side * (deck.depth / 2 - 0.02), cellWidth * 0.46 - 0.024,
              0.273, 0.085, cliff[(col * 3 + course * 7 + index + 2) % cliff.length]);
          }
          for (let row = 0; row < rows; row++) {
            const z = deck.z + (row + 0.5) * cellDepth - deck.depth / 2;
            block(deck.x + side * (deck.width / 2 - 0.02), base, z - cellDepth * 0.24, 0.11, 0.273, cellDepth * 0.50 - 0.024,
              cliff[(row + course * 7 + index) % cliff.length]);
            block(deck.x + side * (deck.width / 2 - 0.02), base, z + cellDepth * 0.26, 0.085, 0.273, cellDepth * 0.47 - 0.024,
              cliff[(row + course * 7 + index + 2) % cliff.length]);
          }
        }
      }
    }
    for (let row = 0; row < rows; row++) {
      for (let col = 0; col < columns; col++) {
        const x = deck.x + (col + 0.5) * cellWidth - deck.width / 2;
        const z = deck.z + (row + 0.5) * cellDepth - deck.depth / 2;
        const turf = index > 0 && index < 5 && (row === 0 || row === rows - 1 || (index > 2 && col === (index === 3 ? 0 : columns - 1)))
          && col % 5 !== 1;
        block(x, y - 0.22, z, cellWidth - pavingJoint, 0.22, cellDepth - pavingJoint,
          turf ? ['#778361', '#647e5c', '#6a7959'][(row + col + index) % 3] : paving[(col * 3 + row * 7 + index) % 5]);
        if (turf) verge(x, y, z, cellWidth, cellDepth, (index * 23 + row) * 31 + col);
      }
    }
  }

  for (const stair of mewGarden.stairs) {
    const lower = levels.get(stair.from)!;
    const rise = levels.get(stair.to)! - lower;
    const across = stair.direction === 'west';
    const tread = stair.run / stair.steps;
    const skirt = stair.from === 'east' && stair.to === 'court' || stair.from === 'south' && stair.to === 'court' ? 0.54 : 0.16;
    for (let step = 0; step < stair.steps; step++) {
      const distance = (step + 0.5) * tread;
      const x = stair.x - (across ? distance : 0);
      const z = stair.z - (across ? 0 : distance);
      const height = rise * (step + 1) / stair.steps;
      block(x, lower - skirt, z, across ? tread : stair.width, height + skirt, across ? stair.width : tread, plaster);
      block(x, lower + height - 0.035, z, across ? tread + 0.021 : stair.width + 0.03, 0.039,
        across ? stair.width + 0.03 : tread + 0.021, paving[3]);
      for (const side of [-1, 1]) {
        block(x + (across ? 0 : side * (stair.width / 2 + 0.09)), lower - skirt,
          z + (across ? side * (stair.width / 2 + 0.09) : 0), across ? tread : 0.18,
          height + skirt + 0.16, across ? 0.18 : tread, '#74827a');
      }
    }
  }

  const { hall, pavilion, gallery, tower, gate, wall } = gardenBuildings;
  block(hall.x, y, hall.z, hall.width, 0.22, hall.depth, plaster);
  block(hall.x, y + 0.22, hall.z, hall.width - 0.67, 2.86, hall.depth - 0.67, '#a59378');
  const hallFront = hall.z + hall.depth / 2;
  for (let bay = -3; bay <= 3; bay++) {
    const x = hall.x + bay * 0.91;
    block(x, y + 0.23, hallFront - 0.065, 0.14, 2.9, 0.16, wood);
    if (bay !== 3) {
      lamps.push([x + 0.452, y + 1.53, hallFront - 0.26, 0.67, 2.24, 0.1, bay ? '#dcb982' : '#f1cc91']);
      for (let rail = 0; rail < 4; rail++) {
        block(x + 0.452, y + 0.64 + rail * 0.49, hallFront - 0.186, 0.68, 0.045, 0.055, wood);
      }
    }
  }
  for (const side of [-1, 1]) {
    for (const z of [hall.z - 1.52, hall.z, hall.z + 1.51]) {
      lamps.push([hall.x + side * (hall.width / 2 - 0.275), y + 1.61, z, 0.14, 1.49, 1.06, '#d7ab73']);
      for (const slat of [-1, 0, 1]) {
        block(hall.x + side * (hall.width / 2 - 0.195), y + 0.75, z + slat * 0.26, 0.055, 1.64, 0.042, wood);
      }
    }
    lantern(hall.x + side * 2.4, y + 1.9, hallFront + 0.2);
    nightLight(hall.x + side * 2.1, y + 2.35, hallFront + 0.72, 23, 8.2);
  }
  roof(hall.x, y + 3.22, hall.z, 9.45, 6.6, 1.7);
  block(hall.x, y + 4.03, hall.z, 4.7, 1.14, 3.1, wood);
  lamps.push([hall.x, y + 4.87, hall.z + 1.59, 3.6, 0.39, 0.048, '#d5a560']);
  roof(hall.x, y + 5.17, hall.z, 5.5, 4.13, 1.04);

  block(pavilion.x, y, pavilion.z, pavilion.width, 0.18, pavilion.depth, plaster);
  for (const side of [-1, 1]) {
    for (const edge of [-1, 1]) {
      const x = pavilion.x + side * 1.4;
      const z = pavilion.z + edge * 1.4;
      block(x, y + 0.16, z, 0.195, 2.43, 0.195, wood);
      block(x, y + 0.16, z, 0.31, 0.19, 0.31, plaster);
    }
    block(pavilion.x, y + 0.52, pavilion.z + side * 1.38, 2.2, 0.12, 0.31, '#9f7c50');
  }
  roof(pavilion.x, y + 2.67, pavilion.z, 4.81, 4.81, 1.08);
  lantern(pavilion.x, y + 1.68, pavilion.z);
  nightLight(pavilion.x, y + 2.21, pavilion.z, 20, 6.1);

  block(gallery.x, y, gallery.z, gallery.width, 0.24, gallery.depth, '#8e8c7c');
  block(gallery.x - 0.93, y + 0.22, gallery.z, 0.13, 2.12, gallery.depth, plaster);
  for (let index = 0; index <= 8; index++) {
    const z = gallery.z + (index / 8 - 0.5) * (gallery.depth - 0.3);
    block(gallery.x + 0.88, y + 0.18, z, 0.12, 2.15, 0.16, wood);
    if (index % 4 === 2) {
      lantern(gallery.x + 0.67, y + 1.43, z);
      nightLight(gallery.x + 0.67, y + 1.8, z, 13, 5.2);
    }
  }
  roof(gallery.x, y + 2.33, gallery.z, 3.06, 12.35, 0.89);

  const gateY = levels.get('south')!;
  for (const side of [-1, 1]) {
    block(gate.x + side * 2.2, gateY, gate.z, 0.55, 2.92, 0.55, wood);
    block(gate.x + side * 2.2, gateY, gate.z, 0.78, 0.32, 0.74, plaster);
    lantern(gate.x + side * 1.92, gateY + 1.78, gate.z);
  }
  block(gate.x, gateY + 2.53, gate.z, gate.width - 0.46, 0.31, 0.36, wood);
  roof(gate.x, gateY + 2.93, gate.z, gate.width, 1.72, 0.74);
  nightLight(gate.x, gateY + 2.25, gate.z, 18, 6.7);

  const wallShape = new T.Shape([
    new T.Vector2(-wall.width / 2, 0), new T.Vector2(-1.025, 0),
    new T.Vector2(-1.025, 0.48),
  ]);
  wallShape.absarc(0, 1.505, 1.45, Math.PI * 1.25, -Math.PI * 0.25, true);
  wallShape.lineTo(1.025, 0);
  wallShape.lineTo(wall.width / 2, 0);
  wallShape.lineTo(wall.width / 2, 3.05);
  wallShape.lineTo(-wall.width / 2, 3.05);
  wallShape.closePath();
  const gardenWall = new T.Mesh(new T.ExtrudeGeometry(wallShape, { depth: 0.42, bevelEnabled: false, curveSegments: 22 }),
    new T.MeshStandardMaterial({ color: plaster, roughness: 0.91 }));
  gardenWall.position.set(wall.x, y, wall.z - 0.21);
  gardenWall.castShadow = gardenWall.receiveShadow = true;
  scene.add(gardenWall);
  const moonFrame = new T.Mesh(
    new T.RingGeometry(1.45, 1.58, 40, 1, -Math.PI / 4, Math.PI * 1.5),
    new T.MeshStandardMaterial({ color: '#887a61', roughness: 0.82, side: T.DoubleSide }),
  );
  moonFrame.position.set(wall.x, y + 1.505, wall.z + 0.225);
  moonFrame.receiveShadow = true;
  scene.add(moonFrame);
  block(wall.x, y + 3.04, wall.z, wall.width + 0.14, 0.13, 0.58, jade);
  roof(wall.x, y + 3.14, wall.z, wall.width + 0.48, 1.02, 0.30);
  nightLight(wall.x - 1.83, y + 1.74, wall.z + 0.66, 18, 6.3);

  const towerY = levels.get(tower.surface)!;
  block(tower.x, towerY, tower.z, tower.width, 0.44, tower.depth, plaster);
  for (let tier = 0; tier < 5; tier++) {
    const width = 2.72 - tier * 0.32;
    const base = towerY + 0.44 + tier * 1.37;
    block(tower.x, base, tower.z, width, 1.12, width, '#baa486');
    for (const side of [-1, 1]) {
      lamps.push([tower.x, base + 0.62, tower.z + side * (width / 2 + 0.018), width * 0.61, 0.55, 0.045, '#e8b875']);
      lamps.push([tower.x + side * (width / 2 + 0.018), base + 0.62, tower.z, 0.045, 0.55, width * 0.61, '#e8b875']);
      for (let slat = -2; slat <= 2; slat++) {
        block(tower.x + slat * width * 0.12, base + 0.35, tower.z + side * (width / 2 + 0.05), 0.046, 0.55, 0.055, wood);
        block(tower.x + side * (width / 2 + 0.05), base + 0.35, tower.z + slat * width * 0.12, 0.055, 0.55, 0.046, wood);
      }
      block(tower.x, base + 0.13, tower.z + side * (width / 2 + 0.11), width + 0.27, 0.08, 0.09, jade);
      block(tower.x + side * (width / 2 + 0.11), base + 0.13, tower.z, 0.09, 0.08, width + 0.27, jade);
    }
    roof(tower.x, base + 1.13, tower.z, width + 0.92, width + 0.92, tier === 4 ? 0.74 : 0.38);
  }
  lamps.push([tower.x, towerY + 7.95, tower.z, 0.16, 0.22, 0.16, '#9ad2b8']);
  nightLight(tower.x, towerY + 1.65, tower.z + 2.4, 23, 7.1);

  for (const [index, plant] of gardenPlanting.entries()) {
    const y = levels.get(plant.surface)!;
    block(plant.x, y, plant.z, plant.width, 0.32, plant.depth, '#697460');
    block(plant.x, y + 0.31, plant.z, plant.width - 0.14, 0.07, plant.depth - 0.14, '#75865a');
    const rise = plant.surface === 'north' && plant.x < 0 ? 1.38 : plant.surface === 'east' ? 0.28 : 0;
    if (rise) {
      for (let layer = 0; layer < 3; layer++) {
        const width = plant.width * (0.88 - layer * 0.14);
        const depth = plant.depth * (0.89 - layer * 0.15);
        const top = y + 0.37 + (layer + 1) * rise / 3;
        block(plant.x - layer * 0.075, top - rise / 3, plant.z, width, rise / 3, depth,
          ['#707a65', '#7b8869', '#798961'][layer]);
        verge(plant.x - layer * 0.075, top, plant.z, width, depth, 215 + index * 37 + layer);
      }
    }
    flora.tree(plant.x - 0.1, y + 0.38 + rise, plant.z, plant.size, 28 + index * 39, plant.kind);
    flora.shrub(plant.x + 0.36, y + 0.38, plant.z + 0.45, 0.86, 195 + index * 31, 'mound');
    verge(plant.x, y + 0.38, plant.z, plant.width * 0.89, plant.depth * 0.85, 644 + index * 47);
  }
  for (const [x, z, surface] of [[-7, 6.85, 'court'], [6.85, -6.95, 'court'], [-11.6, 5.6, 'west'],
    [12.4, 5.75, 'east'], [-4.5, 12.9, 'south'], [5, 12.9, 'south']] as const) {
    const base = levels.get(surface)!;
    lantern(x, base + 0.2, z);
    nightLight(x, base + 0.69, z, 6, 3.4);
  }
  for (const [x, z, width, height, depth] of [
    [-14.62, 12.74, 1.72, 3.23, 3.38], [-8.67, 15.43, 3.6, 1.58, 1.01],
    [12.84, 15.29, 2.53, 2.83, 1.13], [15.32, 8.21, 1.3, 2.24, 3.84],
    [15.75, -13.08, 1.35, 3.14, 2.13],
  ]) {
    block(x, -4.42, z, width, height, depth, '#515c61');
    block(x - 0.21, -4.42 + height, z - 0.15, width * 0.69, 0.22, depth * 0.71, '#697266');
  }

  const unit = new T.BoxGeometry();
  function batch(items: Block[], material: Three.Material) {
    const mesh = new T.InstancedMesh(unit, material, items.length);
    const placement = new T.Object3D();
    const tint = new T.Color();
    for (const [i, [x, y, z, w, h, d, color]] of items.entries()) {
      placement.position.set(x, y, z);
      placement.scale.set(w, h, d);
      placement.updateMatrix();
      mesh.setMatrixAt(i, placement.matrix);
      mesh.setColorAt(i, tint.set(color));
    }
    scene.add(mesh);
    return mesh;
  }

  const masonry = batch(structures, new T.MeshStandardMaterial({ roughness: 0.84 }));
  masonry.castShadow = masonry.receiveShadow = true;
  const glazing = new T.MeshBasicMaterial();
  batch(lamps, glazing);
  flora.mount(scene);
  const blades = new T.BufferGeometry();
  blades.setAttribute('position', new T.Float32BufferAttribute(grassPositions, 3));
  blades.setAttribute('color', new T.Float32BufferAttribute(grassColors, 3));
  blades.computeVertexNormals();
  const grass = new T.Mesh(blades, new T.MeshStandardMaterial({ vertexColors: true, side: T.DoubleSide, roughness: 0.96 }));
  grass.receiveShadow = true;
  scene.add(grass);

  const ripples = new Uint8Array(32 * 32 * 4);
  for (let row = 0; row < 32; row++) {
    for (let col = 0; col < 32; col++) {
      const offset = (row * 32 + col) * 4;
      ripples[offset] = 128 + 25 * Math.sin(row * Math.PI / 8 + col * Math.PI / 16);
      ripples[offset + 1] = 128 + 32 * Math.cos(row * Math.PI / 4 - col * Math.PI / 8);
      ripples[offset + 2] = 245;
      ripples[offset + 3] = 255;
    }
  }
  const normalMap = new T.DataTexture(ripples, 32, 32, T.RGBAFormat);
  normalMap.wrapS = normalMap.wrapT = T.RepeatWrapping;
  normalMap.minFilter = normalMap.magFilter = T.LinearFilter;
  normalMap.repeat.set(13, 13);
  normalMap.needsUpdate = true;
  const water = new T.Mesh(
    new T.PlaneGeometry(21, 21),
    new T.MeshPhysicalMaterial({ color: '#355c5b', metalness: 0.38, roughness: 0.27, normalMap, clearcoat: 0.5 }),
  );
  water.rotation.x = -Math.PI / 2;
  water.position.y = 0.06;
  water.receiveShadow = true;
  scene.add(water);

  const fall = createGardenWaterfall(T, 1.31, 4.48);
  fall.position.set(-8.67, -1.03, 15.34);
  scene.add(fall);
  return {
    setNight(night: boolean) {
      for (const lamp of fixtures) lamp.visible = night;
      glazing.color.setScalar(night ? 1.45 : 0.72);
      water.material.color.set(night ? '#234746' : '#648989');
      fall.material.uniforms.tint.value.set(night ? '#416f68' : '#a1c3bb');
    },
    update(seconds: number) {
      normalMap.offset.set(seconds * 0.027, -seconds * 0.019);
      fall.material.uniforms.time.value = seconds;
    },
  };
}
