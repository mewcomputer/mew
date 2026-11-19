import type * as Three from 'three';
import type { WaterSimulation } from './fluid.ts';
import type { Coordinates, World } from './world.ts';
import { random } from './district.ts';
import { createWaterMaterial, waterLayer } from './water-material.ts';

export function createWaterScene(
  T: typeof Three, scene: Three.Scene, world: World, water: WaterSimulation,
  renderer: Three.WebGLRenderer,
) {
  const { origin, cell } = world;
  const {
    width: waterWidth,
    length: waterLength,
    count: waterCount,
    wet,
    channelStart,
    channelEnd,
    gateEdge,
    depth,
    bed,
    outlet,
    index: waterIndex,
    gateBlocks,
  } = water;
  const optics = createWaterMaterial(T, renderer);
  let waterTime = 0;
  const waterGeometry = new T.BufferGeometry();
  const maxVertices = waterCount * 20;
  const surfacePositions = new Float32Array(maxVertices * 3);
  const surfaceHydraulics = new Float32Array(maxVertices * 2);
  const surfaceUv = new Float32Array(maxVertices * 2);
  const surfaceIndices = new Uint32Array((maxVertices * 3) / 2);
  const surfacePositionAttribute = new T.BufferAttribute(surfacePositions, 3).setUsage(
    T.DynamicDrawUsage,
  );
  const surfaceUvAttribute = new T.BufferAttribute(surfaceUv, 2).setUsage(T.DynamicDrawUsage);
  const surfaceIndexAttribute = new T.BufferAttribute(surfaceIndices, 1).setUsage(T.DynamicDrawUsage);
  waterGeometry.setAttribute('position', surfacePositionAttribute);
  waterGeometry.setAttribute('uv', surfaceUvAttribute);
  const hydraulicAttribute = new T.BufferAttribute(surfaceHydraulics, 2).setUsage(T.DynamicDrawUsage);
  waterGeometry.setAttribute('hydraulics', hydraulicAttribute);
  waterGeometry.setIndex(surfaceIndexAttribute);
  const canal = new T.Mesh(waterGeometry, optics.material(false));
  canal.frustumCulled = false;
  canal.layers.set(waterLayer);
  scene.add(canal);
  const surface = new Float64Array(waterCount);
  let vertex = 0,
    surfaceTriangles = 0;
  function waterQuad(
    a: Coordinates, b: Coordinates, c: Coordinates, d: Coordinates, speed: number, localDepth: number,
  ) {
    const first = vertex;
    for (const [x, y, z] of [a, b, c, d]) {
      surfacePositions[vertex * 3] = x;
      surfacePositions[vertex * 3 + 1] = y;
      surfacePositions[vertex * 3 + 2] = z;
      surfaceUv[vertex * 2] = x + 0.5;
      surfaceUv[vertex * 2 + 1] = 1 - (z - origin[2]) / (waterLength * cell);
      surfaceHydraulics[vertex * 2] = speed;
      surfaceHydraulics[vertex * 2 + 1] = localDepth;
      vertex++;
    }
    surfaceIndices.set([first, first + 1, first + 2, first, first + 2, first + 3], surfaceTriangles);
    surfaceTriangles += 6;
  }
  function cornerHeight(x: number, z: number, dx: number, dz: number) {
    const base = surface[waterIndex(x, z)];
    let sum = base,
      count = 1;
    for (const [a, b] of [
      [dx, 0],
      [0, dz],
      [dx, dz],
    ]) {
      const nx = x + a,
        nz = z + b;
      if (nx < 0 || nx >= waterWidth || nz < 0 || nz >= waterLength) continue;
      if (b && gateBlocks(x, Math.min(z, nz))) continue;
      const n = waterIndex(nx, nz);
      if (depth[n] < wet || Math.abs(surface[n] - base) > 0.17) continue;
      sum += surface[n];
      count++;
    }
    return sum / count;
  }
  function buildWaterSurface() {
    vertex = 0;
    surfaceTriangles = 0;
    for (let i = 0; i < waterCount; i++) surface[i] = bed[i] + depth[i];
    for (let z = 0; z < waterLength; z++)
      for (let x = 0; x < waterWidth; x++) {
        const i = waterIndex(x, z);
        if (depth[i] < 0.004) continue;
        const wx = origin[0] + x * cell,
          wz = origin[2] + z * cell;
        const a: Coordinates = [wx, cornerHeight(x, z, -1, -1), wz];
        const b: Coordinates = [wx + cell, cornerHeight(x, z, 1, -1), wz];
        const c: Coordinates = [wx + cell, cornerHeight(x, z, 1, 1), wz + cell];
        const d: Coordinates = [wx, cornerHeight(x, z, -1, 1), wz + cell];
        const speed = water.speedAt(x, z);
        waterQuad(a, b, c, d, speed, depth[i]);
        for (const [dx, dz, p, q] of [
          [0, -1, a, b],
          [1, 0, b, c],
          [0, 1, c, d],
          [-1, 0, d, a],
        ] as const) {
          const nx = x + dx,
            nz = z + dz;
          if (nx < 0 || nx >= waterWidth || nz < 0 || nz >= waterLength) continue;
          const n = waterIndex(nx, nz);
          const lower = Math.max(bed[i], surface[n]);
          if (lower < Math.min(p[1], q[1]) - 0.045) {
            waterQuad(p, q, [q[0], lower, q[2]], [p[0], lower, p[2]], speed, depth[i]);
          }
        }
      }
    waterGeometry.setDrawRange(0, surfaceTriangles);
    for (const [attribute, count] of [
      [surfacePositionAttribute, vertex * 3],
      [surfaceUvAttribute, vertex * 2],
      [hydraulicAttribute, vertex * 2],
      [surfaceIndexAttribute, surfaceTriangles],
    ] as const) {
      attribute.clearUpdateRanges();
      if (count) attribute.addUpdateRange(0, count);
      attribute.needsUpdate = true;
    }
  }

  const fallMaterial = optics.material(true);
  const fallRows = 9;
  const streams = Array.from({ length: channelEnd - channelStart }, (_, column) => {
    const p = new Float32Array(fallRows * 6);
    const geometry = new T.BufferGeometry();
    geometry.setAttribute('position', new T.BufferAttribute(p, 3).setUsage(T.DynamicDrawUsage));
    const uv = [],
      indices = [];
    for (let row = 0; row < fallRows; row++) {
      uv.push(column * cell, row / (fallRows - 1), (column + 1) * cell, row / (fallRows - 1));
      const i = row * 2;
      if (row) indices.push(i - 2, i - 1, i, i - 1, i + 1, i);
    }
    geometry.setAttribute('uv', new T.Float32BufferAttribute(uv, 2));
    geometry.setIndex(indices);
    const mesh = new T.Mesh(geometry, fallMaterial);
    mesh.frustumCulled = false;
    mesh.layers.set(waterLayer);
    scene.add(mesh);
    return { mesh, p, history: new Float64Array(44) };
  });
  const sluice = new T.Group();
  const slab = new T.Mesh(
    new T.BoxGeometry(0.965, 0.94, 0.09),
    new T.MeshStandardMaterial({ color: '#31554d', metalness: 0.24, roughness: 0.68 }),
  );
  slab.position.y = 0.47;
  slab.castShadow = slab.receiveShadow = true;
  sluice.add(slab);
  const bandMat = new T.MeshStandardMaterial({ color: '#b08460', metalness: 0.28, roughness: 0.52 });
  const bandShape = new T.BoxGeometry(0.043, 0.91, 0.025);
  for (let i = -3; i <= 3; i++) {
    const band = new T.Mesh(bandShape, bandMat);
    band.position.set(i * 0.121, 0.46, 0.051);
    sluice.add(band);
  }
  sluice.position.set(0, 1.13, origin[2] + (gateEdge + 1) * cell);
  scene.add(sluice);

  let historyHead = 0;
  function advanceSpillway() {
    historyHead = (historyHead + 1) % streams[0].history.length;
    streams.forEach((stream, column) => {
      stream.history[historyHead] = outlet[column];
      let active = false;
      const height = surface[waterIndex(channelStart + column, waterLength - 1)];
      const center = origin[0] + (channelStart + column + 0.5) * cell;
      for (let row = 0; row < fallRows; row++) {
        const t = Math.sqrt(row / (fallRows - 1)) * Math.sqrt((2 * Math.max(0.7, height + 2.57)) / 9.81);
        const lag = Math.min(stream.history.length - 1, Math.round(t * 45));
        const discharge =
          stream.history[(historyHead + stream.history.length - lag) % stream.history.length];
        const opening = Math.min(1, Math.sqrt(discharge / 0.055));
        if (opening > 0.04) active = true;
        const half = opening * cell * 0.492 * (1 - (0.13 * row) / (fallRows - 1));
        const z = origin[2] + waterLength * cell + t * 0.72;
        const y = height - 4.905 * t * t;
        stream.p.set([center - half, y, z, center + half, y, z], row * 6);
      }
      stream.mesh.visible = active;
      stream.mesh.geometry.attributes.position.needsUpdate = true;
    });
  }
  const splashCount = 150;
  const sprayCanvas = document.createElement('canvas');
  sprayCanvas.width = sprayCanvas.height = 32;
  const sprayContext = sprayCanvas.getContext('2d');
  if (!sprayContext) throw new Error('Unable to draw spillway spray.');
  const mist = sprayContext.createRadialGradient(16, 16, 0, 16, 16, 16);
  mist.addColorStop(0, 'rgb(255 255 255 / .95)');
  mist.addColorStop(0.43, 'rgb(255 255 255 / .68)');
  mist.addColorStop(1, 'rgb(255 255 255 / 0)');
  sprayContext.fillStyle = mist;
  sprayContext.fillRect(0, 0, 32, 32);
  const splashGeometry = new T.BufferGeometry();
  const splashPositions = new Float32Array(splashCount * 3);
  const splashAttribute = new T.BufferAttribute(splashPositions, 3).setUsage(T.DynamicDrawUsage);
  splashGeometry.setAttribute('position', splashAttribute);
  const splash = new T.Points(
    splashGeometry,
    new T.PointsMaterial({
      map: new T.CanvasTexture(sprayCanvas),
      color: '#eef4eb',
      size: 0.11,
      transparent: true,
      opacity: 0.62,
      depthWrite: false,
    }),
  );
  splash.frustumCulled = false;
  splash.layers.set(waterLayer);
  splash.renderOrder = 2;
  scene.add(splash);
  const shadowCanvas = document.createElement('canvas');
  shadowCanvas.width = 64;
  shadowCanvas.height = 64;
  const shadow = shadowCanvas.getContext('2d');
  if (!shadow) throw new Error('Unable to draw the district shadow.');
  const shade = shadow.createRadialGradient(32, 32, 3, 32, 32, 32);
  shade.addColorStop(0, '#081a14c4');
  shade.addColorStop(1, '#081a1400');
  shadow.fillStyle = shade;
  shadow.fillRect(0, 0, 64, 64);
  const shadowPlane = new T.Mesh(
    new T.PlaneGeometry(16, 15),
    new T.MeshBasicMaterial({
      map: new T.CanvasTexture(shadowCanvas),
      transparent: true,
      depthWrite: false,
    }),
  );
  shadowPlane.rotateX(-Math.PI / 2);
  shadowPlane.position.set(0.1, -2.68, 0.5);
  scene.add(shadowPlane);
  function updateSpray() {
    for (let i = 0; i < splashCount; i++) {
      const f = (waterTime * (0.37 + random(i, 2, 7) * 0.3) + random(i, 4, 7)) % 1;
      const stream = streams[i % streams.length];
      const impact = stream.history[(historyHead + stream.history.length - 33) % stream.history.length];
      const strength = Math.min(1, impact / 0.044);
      const x = origin[0] + (channelStart + (i % streams.length) + 0.5) * cell;
      splashPositions[i * 3] = x + (random(i, 5, 7) - 0.5) * (0.16 + f * 0.41);
      const flight = f * 0.5;
      const lift = 2.45 + random(i, 6, 7) * 1.44;
      splashPositions[i * 3 + 1] =
        strength > 0.035 ? -2.55 + Math.max(0, lift * flight - 4.905 * flight * flight) * strength : -40;
      splashPositions[i * 3 + 2] = 6.02 + f * strength * (0.3 + random(i, 7, 7) * 0.92);
    }
    splashAttribute.needsUpdate = true;
  }

  function setGate(closed: boolean) {
    sluice.position.y = closed ? -0.39 : 1.13;
  }

  function update(time: number, simulationTime: number) {
    waterTime = time;
    optics.setTime(time);
    buildWaterSurface();
    while (spillTime + 1 / 45 <= simulationTime + 1e-9) {
      advanceSpillway();
      spillTime += 1 / 45;
    }
    updateSpray();
  }

  function prime() {
    buildWaterSurface();
    for (let i = 0; i < streams[0].history.length; i++) advanceSpillway();
    updateSpray();
  }

  let spillTime = 0;
  return {
    sluice,
    dispose: optics.dispose,
    setGate,
    update,
    prime,
    rebuildSurface: buildWaterSurface,
    capture: (camera: Three.OrthographicCamera) => optics.capture(scene, camera),
    invalidate: optics.invalidate,
    setLighting: optics.setLighting,
  };
}
