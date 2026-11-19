import type * as Three from 'three';
import type { CityCharacter } from './city-characters.ts';
import type { createCityFoliage } from './city-foliage.ts';
import { mountCityReeds } from './city-reeds.ts';

const terraceBeds = [
  [-13, -15], [-11, -15], [-9, -15], [-5, -15], [-3, -15],
  [-15, -13], [-11, -13], [-9, -13], [-13, -11], [-9, -11], [-5, -11],
  [-15, -9], [-11, -9], [-13, -7], [-15, -5], [-15, -1], [-15, 3], [-15, 7],
  [-13, 7], [-13, 9], [-15, 11], [-7, 11], [-5, 11], [-3, 11], [5, 11], [9, 11],
  [-5, 13], [-3, 13], [3, 13], [5, 13], [9, 13], [13, 13],
  [13, 11], [11, 9], [15, 9], [11, 5], [15, 5], [13, 3], [11, -1], [13, -3], [15, -5],
  [-5, 5],
] as const;

const borderPlanting = [
  [-11.55, -14.85, 1.23, 'mound'], [-9.3, -13.1, 1.05, 'mound'],
  [-14.85, -10.9, 1.38, 'mound'], [-11.1, -8.75, 1.14, 'flowering'],
  [-14.75, -3, 1.16, 'mound'], [-14.8, 5.1, 1.32, 'mound'], [-13.15, 8.6, 1.16, 'flowering'],
  [-5.6, 11.5, 1.28, 'mound'], [-3.55, 13.4, 1.68, 'snowball'], [4.7, 13.55, 1.32, 'mound'],
  [13.65, 11.5, 1.22, 'mound'], [11.45, 9.15, 1.26, 'flowering'], [14.7, 5.2, 1.96, 'snowball'],
  [11.35, -1.1, 1.12, 'mound'], [14.6, -5.15, 1.31, 'mound'],
] as const;

const reedBeds = [
  [-9.2, -8.55, 1.1, 0.54, 0.86], [-5.6, -8.64, 1.3, 0.57, 0.76], [4.35, -9.37, 1.6, 0.55, 0.91],
  [-8.63, -4.46, 0.62, 1.76, 0.96], [-9.42, 2.7, 0.53, 1.39, 0.80],
  [-12.68, 9.42, 1.3, 0.66, 0.89], [-11.53, 12.76, 0.57, 1.15, 1.04], [-8.45, 12.71, 0.55, 1.12, 0.75],
  [-3.7, 9.42, 1.5, 0.56, 0.79], [5.2, 8.59, 1.62, 0.57, 0.92],
  [8.56, 4.4, 0.58, 1.72, 1.05], [9.4, -4.34, 0.61, 1.55, 0.85],
] as const;

function random(seed: number) {
  const value = Math.sin(seed * 127.1 + 81.7) * 43758.5453;
  return value - Math.floor(value);
}

export function mountGardenUnderstory(
  T: typeof Three, scene: Three.Scene, foliage: ReturnType<typeof createCityFoliage>,
  ground: Three.Object3D[], pool: Three.Mesh, cast: CityCharacter[],
) {
  const lawn = ground.filter((part) => part instanceof T.Mesh && !Array.isArray(part.material) &&
    ['ground_8a9666', 'ground_8f9b6b'].includes(part.material.name));
  const ray = new T.Raycaster(new T.Vector3(), new T.Vector3(0, -1, 0), 0, 16);
  const route = cast.flatMap(({ path }) => path);
  function clear(x: number, z: number, radius: number) {
    const distance = (0.55 + radius) ** 2;
    return route.every(([px, , pz]) => (x - px) ** 2 + (z - pz) ** 2 >= distance);
  }
  function heightAt(x: number, z: number, surfaces: Three.Object3D[]) {
    ray.ray.origin.set(x, 12, z);
    return ray.intersectObjects(surfaces, false)[0]?.point.y;
  }

  for (const [i, [x, z, size, kind]] of borderPlanting.entries()) {
    if (!clear(x, z, size * (kind === 'snowball' ? 0.87 : 0.58))) continue;
    const y = heightAt(x, z, lawn);
    if (y === undefined) continue;
    const seed = 91 + 19 * i;
    if (kind === 'snowball') foliage.tree(x, y, z, size, seed, kind);
    else foliage.shrub(x, y, z, size, seed, kind);
  }

  const positions: number[] = [];
  const colors: number[] = [];
  const shades = ['#4a6538', '#607a40', '#74854b', '#929658', '#788c55'].map((color) => new T.Color(color));
  for (const [i, [centerX, centerZ]] of terraceBeds.entries()) {
    for (let clump = 0; clump < 19; clump++) {
      const seed = 91 * i + 421 * clump;
      const x = centerX + (random(seed) - 0.5) * 1.78;
      const z = centerZ + (random(seed + 1) - 0.5) * 1.78;
      if (!clear(x, z, 0.16)) continue;
      const y = heightAt(x, z, lawn);
      if (y === undefined) continue;
      const size = 0.10 + random(seed + 2) * 0.19;
      for (let blade = 0; blade < 4; blade++) {
        const angle = random(seed + 3) * Math.PI * 2 + blade * 2.4;
        const dx = Math.cos(angle);
        const dz = Math.sin(angle);
        const width = 0.017 + random(seed + blade + 4) * 0.012;
        const height = size * (0.47 + random(seed + blade + 8) * 0.53);
        const shade = shades[(i + clump * 3 + blade) % shades.length];
        positions.push(
          x - dz * width + dx * 0.025, y, z + dx * width + dz * 0.025,
          x + dz * width + dx * 0.025, y, z - dx * width + dz * 0.025,
          x + dx * height * 0.47, y + height, z + dz * height * 0.47,
        );
        colors.push(shade.r * 0.66, shade.g * 0.66, shade.b * 0.66,
          shade.r * 0.74, shade.g * 0.74, shade.b * 0.74, shade.r, shade.g, shade.b);
      }
    }
  }
  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  geometry.computeVertexNormals();
  const grass = new T.Mesh(geometry,
    new T.MeshStandardMaterial({ vertexColors: true, side: T.DoubleSide, roughness: 0.96 }));
  grass.name = 'terrace-grasses';
  grass.receiveShadow = true;
  scene.add(grass);

  const water = [pool];
  mountCityReeds(T, scene, reedBeds.flatMap(([x, z, width, depth, height], i) => {
    if (!clear(x, z, Math.hypot(width / 2 + 0.12, depth / 2 + 0.12))) return [];
    const y = heightAt(x, z, water);
    if (y === undefined) return [];
    for (const side of [-1, 1]) {
      for (const end of [-1, 1]) {
        if (heightAt(x + side * width / 2, z + end * depth / 2, water) === undefined) return [];
      }
    }
    return [{ x, z, y: y - 0.027, width, depth, height, seed: 81 + 97 * i }];
  }));
}
