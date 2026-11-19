import type * as Three from 'three';

function petalGeometry(T: typeof Three) {
  const petal = new T.BufferGeometry();
  petal.setAttribute('position', new T.Float32BufferAttribute([
    0, -0.68, 0, 0.44, -0.05, 0.11, 0.43, 0.48, 0.18, 0, 0.69, 0.25,
    -0.43, 0.48, 0.18, -0.44, -0.05, 0.11, 0, 0.15, 0,
  ], 3));
  petal.setIndex([0, 1, 6, 1, 2, 6, 2, 3, 6, 3, 4, 6, 4, 5, 6, 5, 0, 6]);
  petal.computeVertexNormals();
  return petal;
}

export function mountGardenWind(T: typeof Three, scene: Three.Scene) {
  const reeds = scene.getObjectByName('canal-reeds');
  const leaves = scene.getObjectByName('wutong-canopies');
  const blooms = scene.getObjectByName('snowball-blooms');
  if (!(reeds instanceof T.Mesh && leaves instanceof T.InstancedMesh && blooms instanceof T.InstancedMesh)) {
    throw new Error('Missing garden wind planting.');
  }
  const blades = reeds.geometry.getAttribute('position');
  if (!(blades instanceof T.BufferAttribute)) throw new Error('Missing reed vertices.');
  const reedRest = blades.array.slice();
  blades.setUsage(T.DynamicDrawUsage);
  reeds.geometry.computeBoundingBox();
  reeds.geometry.computeBoundingSphere();
  const reedRoot = reeds.geometry.boundingBox!.min.y + 0.02;
  reeds.geometry.boundingSphere!.radius += 0.15;
  const leafRest = leaves.instanceMatrix.array.slice();
  leaves.instanceMatrix.setUsage(T.DynamicDrawUsage);
  leaves.computeBoundingSphere();
  leaves.boundingSphere!.radius += 0.23;

  const petalCount = 8;
  const petals = new T.InstancedMesh(petalGeometry(T), new T.MeshStandardMaterial({
    color: '#fff3de', side: T.DoubleSide, roughness: 0.8,
    emissive: '#b0b397', emissiveIntensity: 0.13,
  }), petalCount);
  petals.name = 'snowball-petals';
  petals.instanceMatrix.setUsage(T.DynamicDrawUsage);
  petals.frustumCulled = false;
  petals.visible = false;
  scene.add(petals);

  const matrix = new T.Matrix4();
  const tilt = new T.Matrix4();
  const center = new T.Vector3();
  const pose = new T.Object3D();
  const drifts = Array.from({ length: petalCount }, (_, i) => {
    // Spread the sources across the three planted shrubs and over the canopy.
    blooms.getMatrixAt(Math.floor((i + 0.35) * blooms.count / petalCount), matrix);
    return {
      origin: new T.Vector3().setFromMatrixPosition(matrix),
      duration: 3.15 + (i % 4) * 0.34,
      interval: 9.9 + (i % 3) * 1.25,
      offset: i * 1.49,
      phase: i * 2.39996,
    };
  });
  let last = -1;

  return {
    update(time: number, reducedMotion: boolean) {
      const frame = reducedMotion ? -1 : time;
      if (frame === last) return;
      last = frame;
      if (reducedMotion) {
        blades.array.set(reedRest);
        leaves.instanceMatrix.array.set(leafRest);
        petals.visible = false;
      } else {
        const gust = Math.sin(time * 0.76) * 0.34 + Math.sin(time * 1.43) * 0.22;
        for (let i = 0; i < blades.count; i++) {
          const j = i * 3;
          const x = reedRest[j];
          const y = reedRest[j + 1];
          const z = reedRest[j + 2];
          const lift = Math.max(0, y - reedRoot) ** 2;
          const drift = gust + Math.sin(time * 2.05 + x * 0.47 + z * 0.32) * 0.68;
          blades.setXYZ(i, x + lift * drift * 0.115, y, z + lift * drift * 0.044);
        }
        for (let i = 0; i < leaves.count; i++) {
          matrix.fromArray(leafRest, i * 16);
          center.setFromMatrixPosition(matrix);
          const drift = gust * 0.72 + Math.sin(time * 1.62 + i * 0.68) * 0.49;
          matrix.setPosition(0, 0, 0);
          matrix.premultiply(tilt.makeRotationZ(drift * 0.071));
          matrix.setPosition(center.x + drift * 0.115, center.y + drift * 0.043, center.z - drift * 0.044);
          leaves.setMatrixAt(i, matrix);
        }

        let count = 0;
        for (const { origin, duration, interval, offset, phase } of drifts) {
          const age = (time + offset) % interval;
          if (age >= duration) continue;
          const flutter = Math.sin(age * 2.6 + phase) - Math.sin(phase);
          pose.position.set(origin.x + age * 0.22 + flutter * 0.08,
            origin.y - age * (0.23 + age * 0.038) + flutter * 0.029,
            origin.z - age * 0.10 - flutter * 0.06);
          pose.rotation.set(age * 3.7 + phase, age * 2.2, age * 2.9 + phase);
          const life = age / duration;
          pose.scale.setScalar(0.084 * Math.min(1, life * 18, (1 - life) * 5));
          pose.updateMatrix();
          petals.setMatrixAt(count++, pose.matrix);
        }
        petals.count = count;
        petals.visible = count > 0;
        petals.instanceMatrix.needsUpdate = true;
      }
      blades.needsUpdate = true;
      leaves.instanceMatrix.needsUpdate = true;
    },
  };
}
