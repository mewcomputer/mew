import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { createCityFoliage } from '../src/city-foliage.ts';

test('city trees batch forked limbs and leaf crowns while keeping their planted positions', () => {
  const scene = new T.Scene();
  const city = createCityFoliage(T);
  city.tree(-3, 1.5, 7, 3, 32);
  city.tree(4, 2, 10, 3, 44, 'maple');
  city.mount(scene);
  const leaves = scene.children.find((mesh) => mesh.name === 'tree-canopies');
  const wood = scene.children.find((mesh) => mesh.name === 'tree-limbs');
  assert.ok(leaves instanceof T.InstancedMesh && wood instanceof T.InstancedMesh);
  assert.ok(leaves.count > 2 && wood.count > 2);
  assert.ok(leaves.geometry.getAttribute('position').count > 200);
  leaves.computeBoundingBox();
  wood.computeBoundingBox();
  assert.ok(leaves.boundingBox && wood.boundingBox);
  assert.ok(leaves.boundingBox.max.y > 4.5);
  const trunk = new T.Matrix4();
  wood.getMatrixAt(0, trunk);
  const roots = new T.Vector3(0, -0.5, 0).applyMatrix4(trunk);
  assert.ok(roots.distanceTo(new T.Vector3(-3, 1.5, 7)) < 0.001);
  assert.ok(wood.boundingBox.max.x > 4.5, 'limbs reach out from the trunk');

  const green = new T.Color();
  const maple = new T.Color();
  leaves.getColorAt(0, green);
  leaves.getColorAt(leaves.count - 1, maple);
  assert.ok(green.g > green.r);
  assert.ok(maple.r > maple.g);
});

test('pines have a tall layered crown and willows spread low with hanging sprays', () => {
  function canopy(kind: 'pine' | 'willow') {
    const scene = new T.Scene();
    const city = createCityFoliage(T);
    city.tree(0, 0, 0, 3, 13, kind);
    city.mount(scene);
    const leaves = scene.getObjectByName('tree-canopies');
    assert.ok(leaves instanceof T.InstancedMesh);
    leaves.computeBoundingBox();
    assert.ok(leaves.boundingBox);
    return leaves.boundingBox;
  }

  const pine = canopy('pine');
  const willow = canopy('willow');
  const pineSize = pine.getSize(new T.Vector3());
  const willowSize = willow.getSize(new T.Vector3());
  assert.ok(pine.max.y >= 3 && pineSize.x < willowSize.x, 'pine grows upward while the willow spreads');
  assert.ok(willow.min.y < pine.min.y - 0.3, 'willow sprays hang below the pine canopy');
  assert.ok(willow.min.y > 0, 'the tree canopy stays above the bank');
});

test('shrub batches work without trees and only flowering shrubs bloom', () => {
  function garden() {
    const scene = new T.Scene();
    const city = createCityFoliage(T);
    city.shrub(0, 1, 0, 1.8, 13);
    city.shrub(5, 1, 0, 1.8, 19, 'flowering');
    city.mount(scene);
    const leaves = scene.getObjectByName('shrub-canopies');
    const flowers = scene.getObjectByName('shrub-blossoms');
    assert.ok(leaves instanceof T.InstancedMesh && flowers instanceof T.InstancedMesh);
    leaves.computeBoundingBox();
    flowers.computeBoundingBox();
    assert.ok(leaves.boundingBox && flowers.boundingBox);
    assert.ok(leaves.boundingBox.min.y >= 1 && leaves.boundingBox.max.y < 2, 'shrubs stay low to the planted surface');
    assert.ok(flowers.boundingBox.min.x > 4, 'flowers only grow at the flowering shrub');
    return { leaves, flowers };
  }

  const first = garden();
  const second = garden();
  assert.deepEqual(first.leaves.instanceMatrix.array, second.leaves.instanceMatrix.array);
  assert.deepEqual(first.flowers.instanceMatrix.array, second.flowers.instanceMatrix.array);
});

test('ginkgo has an upright gold crown and goldenrain has a spread of raised flower clusters', () => {
  const scene = new T.Scene();
  const city = createCityFoliage(T);
  city.tree(0, 0, 0, 3, 42, 'ginkgo');
  city.tree(12, 0, 0, 3, 42, 'goldenrain');
  city.mount(scene);
  const fans = scene.getObjectByName('ginkgo-canopies');
  const canopy = scene.getObjectByName('tree-canopies');
  const panicles = scene.getObjectByName('goldenrain-panicles');
  assert.ok(fans instanceof T.InstancedMesh && canopy instanceof T.InstancedMesh && panicles instanceof T.InstancedMesh);
  fans.computeBoundingBox();
  canopy.computeBoundingBox();
  panicles.computeBoundingBox();
  assert.ok(fans.boundingBox && canopy.boundingBox && panicles.boundingBox);
  const ginkgoWidth = fans.boundingBox.max.x - fans.boundingBox.min.x;
  const rainWidth = canopy.boundingBox.max.x - canopy.boundingBox.min.x;
  assert.ok(rainWidth > ginkgoWidth + 0.5);
  assert.ok(fans.boundingBox.max.y > 3 && fans.boundingBox.min.y > 1.2);
  assert.ok(panicles.boundingBox.min.x > 9 && panicles.boundingBox.max.y > 2.8);

  const gold = new T.Color();
  fans.getColorAt(0, gold);
  assert.ok(gold.r > gold.g && gold.g > 2 * gold.b);
});

test('bamboo is a narrow rooted cluster with segmented culms and leafy upper sprays', () => {
  const scene = new T.Scene();
  const city = createCityFoliage(T);
  city.bamboo(4, 2, 7, 2.15, 33);
  city.mount(scene);
  const culms = scene.getObjectByName('bamboo-culms');
  const leaves = scene.getObjectByName('bamboo-leaves');
  assert.ok(culms instanceof T.InstancedMesh && leaves instanceof T.InstancedMesh);
  const transform = new T.Matrix4();
  const root = new T.Vector3();
  const tip = new T.Vector3();
  for (let cue = 0; cue < culms.count; cue++) {
    culms.getMatrixAt(cue, transform);
    root.set(0, 0, 0).applyMatrix4(transform);
    tip.set(0, 1, 0).applyMatrix4(transform);
    assert.ok(Math.hypot(root.x - 4, root.z - 7) < 0.42);
    assert.ok(Math.abs(root.y - 2) < 1e-5);
    assert.ok(tip.y > 3.5 && tip.y < 4.4);
  }

  const sectionHeights = new Set();
  const positions = culms.geometry.getAttribute('position');
  for (let i = 0; i < positions.count; i++) sectionHeights.add(positions.getY(i));
  assert.ok(sectionHeights.size >= 15);
  leaves.computeBoundingBox();
  assert.ok(leaves.boundingBox);
  assert.ok(leaves.boundingBox.min.y > 2.7 && leaves.boundingBox.max.y > 3.7);
});

test('wutong spreads a high parasol and snowball trees generate distinct repeatable flower clusters', () => {
  function planting(seed: number) {
    const scene = new T.Scene();
    const foliage = createCityFoliage(T);
    foliage.tree(0, 2, 0, 3.7, 35, 'wutong');
    foliage.tree(8, 1, 0, 1.9, seed, 'snowball');
    foliage.mount(scene);
    const crown = scene.getObjectByName('wutong-canopies');
    const blooms = scene.getObjectByName('snowball-blooms');
    assert.ok(crown instanceof T.InstancedMesh && blooms instanceof T.InstancedMesh);
    crown.computeBoundingBox();
    const span = crown.boundingBox!.getSize(new T.Vector3());
    assert.ok(crown.boundingBox!.min.y > 4.2 && span.x > span.y * 2);
    assert.ok(blooms.count >= 15 && blooms.geometry.getAttribute('position').count > 300);
    blooms.computeBoundingBox();
    assert.ok(blooms.boundingBox!.min.x > 6 && blooms.boundingBox!.max.y > 2.5);
    return blooms.instanceMatrix.array;
  }
  assert.deepEqual(planting(55), planting(55));
  assert.notDeepEqual(planting(55), planting(56));
});
