import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import * as T from 'three';
import { GLTFLoader } from 'three/addons/loaders/GLTFLoader.js';
import { createCharacterTrack } from '../src/city-characters.ts';
import { mountImportedGarden } from '../src/imported-garden.ts';
import { mountGardenWind } from '../src/garden-wind.ts';
import { gardenWaterGeometry } from '../src/garden-waterfall.ts';

async function loadGeometry() {
  const bytes = await readFile(new URL('../assets/mew-diorama.glb', import.meta.url));
  const loader = new GLTFLoader();
  // Route tests need the exported meshes and transforms. The browser loads the embedded textures.
  loader.register(() => ({ name: 'GeometryTest', loadTexture: () => Promise.resolve(new T.Texture()) }));
  return (await loader.parseAsync(Uint8Array.from(bytes).buffer, '')).scene;
}

test('the imported garden keeps its building nodes and gets its light from Jianghu', async () => {
  const model = await loadGeometry();
  const scene = new T.Scene();
  const authoredNodes = [...model.children];
  const imported = mountImportedGarden(T, scene, model);
  for (const name of ['hall', 'pavilion_inner', 'pavilion_east', 'gallery', 'moon_gate_wall', 'south_gate', 'tower']) {
    const piece = authoredNodes.find((node) => node.name === name);
    assert.ok(piece && piece.children.length > 0, name);
    assert.equal(piece.parent, model);
  }
  for (const node of authoredNodes) node.traverse((object) => {
    if (object instanceof T.Light) assert.ok(object.name.startsWith('garden-lamp'));
    assert.ok(!(object instanceof T.Camera));
  });
  const lamps = scene.getObjectsByProperty('type', 'PointLight');
  assert.ok(lamps.length > 0);
  imported.setLampLevel(0);
  assert.ok(lamps.every(({ visible }) => !visible));
  imported.setLampLevel(1);
  assert.ok(lamps.every(({ visible }) => visible));
  const brightness = lamps.map((lamp) => (lamp as T.PointLight).intensity);
  imported.setLampLevel(0.5);
  assert.deepEqual(lamps.map((lamp) => (lamp as T.PointLight).intensity), brightness.map((power) => power * 0.5));
  assert.ok(imported.stops.tower.target[1] > imported.stops.hall.target[1]);
});

test('Jianghu routes ride the four imported bridge decks between the moat and raised court', async () => {
  const garden = mountImportedGarden(T, new T.Scene(), await loadGeometry());
  assert.equal(garden.cast.length, 7);
  const crossings = [[-1, -9], [1, 9], [-9, -1], [9, 1]];
  for (const [x, z] of crossings) {
    const point = garden.cast.flatMap((actor) => actor.path).find(([px, , pz]) => px === x && pz === z);
    assert.ok(point && point[1] > 1.45 && point[1] < 1.8, `bridge deck at ${x}, ${z}`);
  }
  for (const actor of garden.cast) {
    assert.ok(actor.path.length > 2);
    assert.ok(actor.path.every((point) => point.every(Number.isFinite) && point[1] >= 0.5));
    const sample = createCharacterTrack(actor);
    assert.ok(Array.from({ length: 12 }, (_, tick) => sample(tick / 2).mode).includes('walk'));
  }
});

test('the animated spillway starts at the imported pool lip and falls past the cliff', async () => {
  const model = await loadGeometry();
  const garden = mountImportedGarden(T, new T.Scene(), model);
  const { pool, spillway, runoff } = gardenWaterGeometry(T, garden.pool);
  const points = spillway.getAttribute('position');
  const topMiddle = 16;
  pool.computeBoundingBox();
  const lip = pool.boundingBox!.max;
  assert.equal(points.getX(topMiddle), -10);
  assert.equal(points.getY(topMiddle), lip.y);
  assert.ok(lip.z - points.getZ(topMiddle) < 0.1 && lip.z >= points.getZ(topMiddle));
  assert.ok(points.getY(points.count - 1) < -4);
  assert.ok(points.getZ(points.count - 1) > lip.z);
  const bottomMiddle = points.count - 17;
  assert.deepEqual(runoff, [
    points.getX(bottomMiddle), points.getY(bottomMiddle), points.getZ(bottomMiddle),
  ]);
  assert.equal(garden.pool.visible, false);
});

test('garden trees use the shared foliage at the imported planting sites', async () => {
  const model = await loadGeometry();
  const scene = new T.Scene();
  mountImportedGarden(T, scene, model);
  const originals = model.children.filter(({ name }) => name.startsWith('tree_'));
  assert.ok(originals.length > 0);
  assert.ok(originals.every(({ visible }) => !visible));
  const trunks = scene.getObjectByName('tree-limbs');
  const bamboo = scene.getObjectByName('bamboo-culms');
  assert.ok(trunks instanceof T.InstancedMesh && trunks.count >= 35);
  assert.ok(bamboo instanceof T.InstancedMesh && bamboo.count === 12);
  const trunkAt = new T.Matrix4();
  trunks.getMatrixAt(0, trunkAt);
  const planted = new T.Vector3().setFromMatrixPosition(trunkAt);
  const firstTree = model.getObjectByName('tree_redwood_0_4')!.getWorldPosition(new T.Vector3());
  assert.ok(Math.hypot(planted.x - firstTree.x, planted.z - firstTree.z) < 0.2);
  assert.ok(planted.y >= firstTree.y);
  const peaks = model.children.filter(({ name }) => name.startsWith('karst_peak_'));
  assert.ok(peaks.every(({ scale }) => scale.y < 1));
  assert.ok(peaks.every(({ children }) => children.some((child) => child.children.length && !child.visible)));
});

test('terrace planting and waterside reeds keep the walking lanes clear', async () => {
  const scene = new T.Scene();
  const garden = mountImportedGarden(T, scene, await loadGeometry());
  const routes = garden.cast.flatMap(({ path }) => path);
  const clear = (x: number, z: number) => routes.every(([px, , pz]) => Math.hypot(x - px, z - pz) >= 0.40);
  const grass = scene.getObjectByName('terrace-grasses');
  const reeds = scene.getObjectByName('canal-reeds');
  const shrubs = scene.getObjectByName('shrub-canopies');
  assert.ok(grass instanceof T.Mesh);
  assert.ok(reeds instanceof T.Mesh && reeds.castShadow && reeds.material.depthWrite);
  assert.ok(shrubs instanceof T.InstancedMesh && shrubs.count >= 15);

  for (const mesh of [grass, reeds]) {
    const positions = mesh.geometry.getAttribute('position');
    assert.ok(positions.count > 100);
    for (let i = 0; i < positions.count; i++) {
      assert.ok(clear(positions.getX(i), positions.getZ(i)), mesh.name);
    }
  }
  const matrix = new T.Matrix4();
  for (let i = 0; i < shrubs.count; i++) {
    shrubs.getMatrixAt(i, matrix);
    const root = new T.Vector3().setFromMatrixPosition(matrix);
    assert.ok(clear(root.x, root.z));
  }

  const poolBounds = new T.Box3().setFromObject(garden.pool);
  reeds.geometry.computeBoundingBox();
  assert.ok(Math.abs(reeds.geometry.boundingBox!.min.y - poolBounds.max.y) < 0.035);
  assert.ok(reeds.geometry.boundingBox!.max.y - poolBounds.max.y < 1.2);
});

test('the imported garden plants a court wutong and smaller snowball trees around it', async () => {
  const scene = new T.Scene();
  mountImportedGarden(T, scene, await loadGeometry());
  const bark = scene.getObjectByName('wutong-limbs');
  const blooms = scene.getObjectByName('snowball-blooms');
  assert.ok(bark instanceof T.InstancedMesh && blooms instanceof T.InstancedMesh);
  assert.equal(blooms.count, 60);
  const transform = new T.Matrix4();
  bark.getMatrixAt(0, transform);
  assert.ok(new T.Vector3(0, -0.5, 0).applyMatrix4(transform).distanceTo(new T.Vector3(-5, 2, 5)) < 0.02);
  blooms.computeBoundingBox();
  assert.ok(blooms.boundingBox!.min.x < -10 && blooms.boundingBox!.max.x > 12);
});


test('garden wind bends the reeds from fixed roots and drifts petals from the snowball canopy', async () => {
  const scene = new T.Scene();
  mountImportedGarden(T, scene, await loadGeometry());
  const wind = mountGardenWind(T, scene);
  const reeds = scene.getObjectByName('canal-reeds');
  const leaves = scene.getObjectByName('wutong-canopies');
  const petals = scene.getObjectByName('snowball-petals');
  const blossoms = scene.getObjectByName('snowball-blooms');
  assert.ok(reeds instanceof T.Mesh && leaves instanceof T.InstancedMesh);
  assert.ok(petals instanceof T.InstancedMesh && blossoms instanceof T.InstancedMesh);
  const blades = reeds.geometry.getAttribute('position');
  const original = blades.array.slice();
  const canopy = leaves.instanceMatrix.array.slice();
  reeds.geometry.computeBoundingBox();
  const root = reeds.geometry.boundingBox!.min.y;
  assert.equal(petals.visible, false);

  wind.update(1.2, false);
  assert.notDeepEqual(blades.array, original);
  assert.notDeepEqual(leaves.instanceMatrix.array, canopy);
  for (let i = 0; i < blades.count; i++) {
    assert.equal(blades.getY(i), original[i * 3 + 1]);
    const displacement = Math.hypot(blades.getX(i) - original[i * 3], blades.getZ(i) - original[i * 3 + 2]);
    assert.ok(displacement < 0.16);
    if (blades.getY(i) <= root + 0.02) assert.equal(displacement, 0);
  }
  assert.ok(petals.visible && petals.count > 0 && petals.count < 12);
  blossoms.computeBoundingBox();
  const pose = new T.Matrix4();
  petals.getMatrixAt(0, pose);
  const first = new T.Vector3().setFromMatrixPosition(pose);
  assert.ok(first.y <= blossoms.boundingBox!.max.y && first.y > 0);
  wind.update(1.35, false);
  petals.getMatrixAt(0, pose);
  assert.notDeepEqual(new T.Vector3().setFromMatrixPosition(pose), first);

  wind.update(1.35, true);
  assert.deepEqual(blades.array, original);
  assert.deepEqual(leaves.instanceMatrix.array, canopy);
  assert.equal(petals.visible, false);
});

test('all seven garden stops save their own orbit angles and zoom', async () => {
  const { stops } = mountImportedGarden(T, new T.Scene(), await loadGeometry());
  const views = Object.values(stops);
  assert.equal(views.length, 7);
  assert.equal(new Set(views.map(({ yaw, pitch }) => `${yaw}:${pitch}`)).size, views.length);
  for (const { target, yaw, pitch, zoom } of views) {
    assert.ok([...target, yaw, pitch, zoom].every(Number.isFinite));
  }
});

test('each lantern route connects its own rack hook to the regular garden walk', async () => {
  const {cast, rack, rounds} = mountImportedGarden(T, new T.Scene(), await loadGeometry());
  assert.equal(rounds.size, 2);
  const hooks = new Set<number>();
  for (const [id, {path, hook, joinIndex}] of rounds) {
    const original = cast.find((actor) => actor.id === id)!.path;
    assert.deepEqual(path.slice(joinIndex), [...original].reverse());
    const [x, y, z] = path[0];
    assert.ok(Math.hypot(x - hook[0], z - hook[2]) < 0.5);
    assert.ok(hook[1] > y && hook[1] < y + 1);
    assert.ok(path.every(([, height]) => height >= 0.5));
    hooks.add(hook[0]);
  }
  assert.equal(hooks.size, 2);
  assert.ok(rack[1] > 1.5);
});
