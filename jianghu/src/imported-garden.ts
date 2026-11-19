import type * as Three from 'three';
import { createCityFoliage } from './city-foliage.ts';
import type { CityCharacter, CityDirection, CityPosition } from './city-characters.ts';
import { mountGardenUnderstory } from './garden-planting.ts';

const modelScale = 1 / 16;
export const gardenOverview = { target: [0, 1.4, 0] as CityPosition, yaw: 0.76, pitch: 0.60, zoom: 0.86 };

const landmarks = {
  hall: { piece: 'hall', yaw: 0.45, pitch: 0.51, zoom: 1.85 },
  pavilion: { piece: 'pavilion_inner', yaw: 1.08, pitch: 0.54, zoom: 2.05 },
  gallery: { piece: 'gallery', yaw: 0.27, pitch: 0.52, zoom: 1.75 },
  wall: { piece: 'moon_gate_wall', yaw: -0.58, pitch: 0.55, zoom: 1.75 },
  gate: { piece: 'south_gate', yaw: -0.48, pitch: 0.50, zoom: 1.95 },
  tower: { piece: 'tower', yaw: 1.21, pitch: 0.57, zoom: 1.7 },
} as const;

type WalkPoint = readonly [row: number, column: number];
const walks = [
  ['daemon', 'Merchant', [[10, 6], [8, 6], [8, 5], [7, 5], [7, 2], [5, 2]]],
  ['agent', 'Research', [[13, 9], [13, 8], [11, 8], [11, 7], [9, 7]]],
  ['provider', 'Write', [[2, 5], [2, 7], [4, 7], [4, 5]]],
  ['tui', 'Code', [[0, 7], [2, 7], [2, 12]]],
  ['web', 'Browse', [[7, 15], [8, 15], [8, 9], [11, 9]]],
  ['desktop', 'Warden', [[2, 14], [2, 13], [4, 13], [4, 14]]],
  ['mobile', 'Ferrykeeper', [[15, 7.5], [13, 7.5], [13, 10]]],
] as const satisfies readonly (readonly [string, CityCharacter['outfit'], readonly [WalkPoint, ...WalkPoint[]]])[];

export const gardenOutfits = walks.map(([, outfit]) => outfit);
const bridgeNames = ['bridge_north', 'bridge_south', 'bridge_west', 'bridge_east'] as const;

const treeReplacements = [
  ['tree_redwood_0_4', 'pine', 2.35],
  ['tree_bamboo_2_1', 'bamboo', 2.5],
  ['tree_ginkgo_2_9', 'ginkgo', 2.15],
  ['tree_ginkgo_4_0', 'snowball', 1.85],
  ['tree_pine_10_5', 'wutong', 3.7],
  ['tree_ginkgo_11_14', 'maple', 2.1],
  ['tree_willow_13_5', 'willow', 2.3],
  ['tree_bamboo_13_11', 'bamboo', 2.25],
  ['tree_redwood_14_12', 'pine', 2.2],
] as const;

export function mountImportedGarden(T: typeof Three, scene: Three.Scene, model: Three.Group) {
  model.name = 'mew-garden';
  model.scale.setScalar(modelScale);
  model.updateMatrixWorld(true);

  function piece(name: string) {
    const node = model.children.find((child) => child.name === name);
    if (!node) throw new Error(`Missing garden piece: ${name}`);
    return node;
  }

  const terrain = piece('island_terrain');
  const pool = terrain.children.find((node): node is Three.Mesh =>
    node instanceof T.Mesh && !Array.isArray(node.material) && node.material.name === 'water');
  if (!pool) throw new Error('Missing garden pool mesh.');
  const ground = terrain.children.filter((node) =>
    node instanceof T.Mesh && !Array.isArray(node.material) && node.material.name.startsWith('ground_'));
  // The first mesh in each bridge is the deck. Its two siblings are the raised rails.
  const decks = bridgeNames.map((name) => piece(name).children[0]);
  const walkingMeshes = [...ground, ...decks];
  const ray = new T.Raycaster(new T.Vector3(), new T.Vector3(0, -1, 0), 0, 16);

  function point(row: number, column: number): CityPosition {
    const x = (column - 7.5) * 2;
    const z = (row - 7.5) * 2;
    ray.ray.origin.set(x, 12, z);
    const hit = ray.intersectObjects(walkingMeshes, false)[0];
    if (!hit) throw new Error(`Garden walk leaves the ground at row ${row}, column ${column}.`);
    return [x, hit.point.y, z];
  }

  function walkPath([start, ...waypoints]: readonly [WalkPoint, ...WalkPoint[]]) {
    let from: WalkPoint = start;
    const path: [CityPosition, ...CityPosition[]] = [point(start[0], start[1])];
    for (const to of waypoints) {
      const count = Math.ceil(Math.hypot(to[0] - from[0], to[1] - from[1]) * 8);
      for (let step = 1; step <= count; step++) {
        path.push(point(
          T.MathUtils.lerp(from[0], to[0], step / count),
          T.MathUtils.lerp(from[1], to[1], step / count),
        ));
      }
      from = to;
    }
    return path;
  }

  const cast = walks.map(([id, outfit, path], index): CityCharacter => ({
    id, outfit, path: walkPath(path), speed: 1.18 + index * 0.12, phase: index * 0.83,
  }));

  const rack = point(11.06, 9.025);
  const roundLocations = [
    ['provider', {
      connector: [[10.84, 8.815], [10.2, 8.815], [10.2, 6], [8, 6], [8, 5], [4, 5]],
      hookX: -0.42, departure: 0.85,
    }],
    ['mobile', {
      connector: [[10.84, 9.235], [10.58, 9.235], [10.58, 8], [11, 8], [13, 8], [13, 10]],
      hookX: 0.42, departure: 2.35,
    }],
  ] as const;
  const rounds = new Map<string, {
    path: CityCharacter['path']; hook: CityPosition; rackDirection: CityDirection;
    departure: number; joinIndex: number; speed: number;
  }>(roundLocations.map(([id, { connector, hookX, departure }]) => {
    const approach = walkPath(connector);
    const walker = cast.find((actor) => actor.id === id)!;
    const path: typeof walker.path = [approach[0], ...approach.slice(1), ...walker.path.slice(0, -1).reverse()];
    const hook: CityPosition = [rack[0] + hookX, rack[1] + 0.83, rack[2] - 0.13];
    const rackDirection: CityDirection = [0, 1];
    return [id, { path, hook, rackDirection, departure, joinIndex: approach.length - 1, speed: walker.speed ?? 1 }];
  }));

  const stops = {
    garden: gardenOverview,
    ...Object.fromEntries(Object.entries(landmarks).map(([id, { piece: name, ...rotation }]) => {
      const bounds = new T.Box3().setFromObject(piece(name));
      const center = bounds.getCenter(new T.Vector3());
      return [id, { target: center.toArray(), ...rotation }];
    })) as Record<keyof typeof landmarks, typeof gardenOverview>,
  };

  const emissive = new Map<Three.MeshStandardMaterial, number>();
  model.traverse((node) => {
    if (!(node instanceof T.Mesh)) return;
    const materials = Array.isArray(node.material) ? node.material : [node.material];
    node.castShadow = materials.every(({ transparent }) => !transparent);
    node.receiveShadow = true;
    for (const material of materials) {
      if (material instanceof T.MeshStandardMaterial && material.emissive.getHex() !== 0) {
        emissive.set(material, material.emissiveIntensity);
      }
    }
  });

  const lamps = ([
    ['hall', -31, 58, 50, 17], ['hall', 31, 58, 50, 17],
    ['pavilion_inner', 0, 19, 5, 15], ['pavilion_east', 1, 19, 0, 13],
    ['gallery', 16, 32, -56, 12], ['gallery', 16, 32, 54, 12],
    ['moon_gate_wall', 0, 23, 13, 12], ['south_gate', 0, 36, 0, 15],
    ['tower', -16, 23, 30, 18],
  ] as const).map(([name, x, y, z, power]) => {
    const lamp = new T.PointLight('#ffc483', power, 6);
    lamp.name = 'garden-lamp-' + name;
    lamp.position.set(x, y, z);
    piece(name).add(lamp);
    return { lamp, power };
  });
  const foliage = createCityFoliage(T);
  const plantingSite = new T.Vector3();
  for (const [index, [name, kind, size]] of treeReplacements.entries()) {
    const original = piece(name);
    const { x, y, z } = original.getWorldPosition(plantingSite);
    const seed = 34 + index * 41;
    if (kind === 'bamboo') foliage.bamboo(x, y, z, size, seed);
    else foliage.tree(x, y, z, size, seed, kind);
    original.visible = false;
  }
  // The karst props also include miniature trees. Keep their stone at garden scale.
  for (const peak of model.children.filter(({ name }) => name.startsWith('karst_peak_'))) {
    peak.scale.y *= 0.43;
    for (const child of peak.children) {
      if (!(child instanceof T.Mesh)) child.visible = false;
    }
  }
  mountGardenUnderstory(T, scene, foliage, ground, pool, cast);
  foliage.mount(scene);
  scene.add(model);

  return {
    cast,
    rack,
    rounds,
    stops,
    pool,
    setLampLevel(level: number) {
      for (const { lamp, power } of lamps) {
        lamp.visible = level > 0;
        lamp.intensity = power * level;
      }
      for (const [material, strength] of emissive) material.emissiveIntensity = strength * (0.18 + 0.82 * level);
    },
  };
}
