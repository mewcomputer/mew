import type * as Three from 'three';

type Plant = {
  x: number; y: number; z: number; size: number; seed: number;
};

const leafOutline = [[0, 0, -1], [1, -0.16, -0.1], [0, 0, 1.12], [-1, -0.16, -0.1]];
const fanOutline = [
  [0, 0, -1.10], [0.56, -0.05, -0.12], [1.20, -0.03, 0.44], [1.04, 0.08, 0.87],
  [0.43, 0.14, 1.12], [0, 0.09, 0.82], [-0.43, 0.14, 1.12], [-1.04, 0.08, 0.87],
  [-1.20, -0.03, 0.44], [-0.56, -0.05, -0.12],
];
const parasolOutline = [
  [0, 0, -0.9], [0.47, -0.06, -0.50], [1.05, -0.06, -0.17], [0.57, 0.04, 0.04],
  [1.17, 0.02, 0.62], [0.38, 0.09, 0.52], [0, 0.13, 1.16],
  [-0.38, 0.09, 0.52], [-1.17, 0.02, 0.62], [-0.57, 0.04, 0.04], [-1.05, -0.06, -0.17], [-0.47, -0.06, -0.50],
];
const leafTints = {
  broadleaf: ['#668850', '#507749', '#769759', '#73944e', '#465f3b', '#839c55'],
  maple: ['#a35034', '#bd683b', '#c6813c', '#aa572f', '#d2934e', '#bd7339'],
  pine: ['#426c52', '#52765a', '#668365'],
  willow: ['#829954', '#689057', '#9aa65a'],
  ginkgo: ['#d8ad47', '#b98c34', '#e2bc56', '#c29a37', '#d9a943', '#d0b252'],
  goldenrain: ['#729756', '#668748', '#89a761', '#658d55', '#95a85f', '#7c984a'],
  wutong: ['#81a354', '#547942', '#6a954b', '#789c4c', '#8ca953', '#638942'],
  snowball: ['#66864b', '#4d703c', '#6c8d48', '#577344', '#7d9556', '#5c8349'],
};
type TreeKind = keyof typeof leafTints;
type ShrubKind = 'mound' | 'flowering';

const treeParts = {
  broadleaf: { limbs: 7, crowns: 6 },
  maple: { limbs: 7, crowns: 6 },
  willow: { limbs: 7, crowns: 16 },
  pine: { limbs: 8, crowns: 7 },
  ginkgo: { limbs: 7, crowns: 6 },
  goldenrain: { limbs: 7, crowns: 6 },
  wutong: { limbs: 12, crowns: 11 },
  snowball: { limbs: 7, crowns: 6 },
};
const blossomsPerShrub = 9;
const culmsPerClump = 6;
const flowerClustersPerTree = 5;
const blossomsPerCluster = 7;
const snowballsPerBranch = 4;

function random(seed: number) {
  const value = Math.sin(seed * 127.1 + 81.7) * 43758.5453;
  return value - Math.floor(value);
}

function leafCloud(T: typeof Three, shape: 'kite' | 'fan' | 'blade' | 'parasol' = 'kite') {
  const positions: number[] = [];
  const colors: number[] = [];
  const indices: number[] = [];
  const leaf = new T.Vector3();
  const rotation = new T.Euler();
  const outline = shape === 'fan' ? fanOutline : shape === 'parasol' ? parasolOutline : leafOutline;
  for (let i = 0; i < (shape === 'blade' ? 56 : 104); i++) {
    const seed = i * 19;
    const azimuth = random(seed + 1) * 2 * Math.PI;
    const above = 2 * random(seed + 2) - 1;
    const radius = 0.43 * Math.cbrt(random(seed + 3));
    const horizontal = radius * Math.sqrt(1 - above * above);
    const x = horizontal * Math.cos(azimuth);
    const y = radius * above;
    const z = horizontal * Math.sin(azimuth);
    const length = (0.10 + random(seed + 4) * 0.105) * (shape === 'blade' ? 1.65 : shape === 'parasol' ? 1.24 : 1);
    const width = length * (shape === 'blade' ? 0.18 : shape === 'parasol' ? 0.66 : 0.38 + random(seed + 5) * 0.17);
    const tint = 0.59 + random(seed + 6) * 0.41;
    rotation.set((random(seed + 7) - 0.5) * 1.7, azimuth, (random(seed + 8) - 0.5) * 1.8, 'YXZ');
    const at = positions.length / 3;
    for (const [u, v, w] of outline) {
      leaf.set(u * width, v * length, w * length).applyEuler(rotation);
      positions.push(x + leaf.x, y + leaf.y, z + leaf.z);
      colors.push(tint, tint, tint);
    }
    for (let corner = 2; corner < outline.length; corner++) indices.push(at, at + corner, at + corner - 1);
  }
  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  geometry.setIndex(indices);
  geometry.computeVertexNormals();
  return geometry;
}

function culmGeometry(T: typeof Three) {
  const profile = [new T.Vector2(0, 0), new T.Vector2(1, 0)];
  for (let node = 1; node < 6; node++) {
    const at = node / 6;
    const radius = 1 - at * 0.51;
    profile.push(new T.Vector2(radius, at - 0.013),
      new T.Vector2(radius * 1.23, at - 0.005),
      new T.Vector2(radius * 1.23, at + 0.005),
      new T.Vector2(radius, at + 0.012));
  }
  profile.push(new T.Vector2(0.37, 1), new T.Vector2(0, 1));
  const geometry = new T.LatheGeometry(profile, 7);
  const color = new T.Color();
  const positions = geometry.getAttribute('position');
  const colors: number[] = [];
  for (let i = 0; i < positions.count; i++) {
    const y = positions.getY(i);
    const section = Math.round(y * 6);
    const ring = section > 0 && section < 6 && Math.abs(y - section / 6) < 0.006;
    color.set(ring ? '#b0aa68' : y < 0.5 ? '#6b854e' : '#849553');
    colors.push(color.r, color.g, color.b);
  }
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  return geometry;
}

function blossomGeometry(T: typeof Three) {
  const positions: number[] = [];
  for (let petal = 0; petal < 5; petal++) {
    const angle = petal * Math.PI * 2 / 5;
    const left = angle - 0.46;
    const right = angle + 0.46;
    positions.push(
      0, 0, 0, Math.cos(left) * 0.79, 0.16, Math.sin(left) * 0.79, Math.cos(angle), 0.27, Math.sin(angle),
      0, 0, 0, Math.cos(angle), 0.27, Math.sin(angle), Math.cos(right) * 0.79, 0.16, Math.sin(right) * 0.79,
    );
  }
  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.computeVertexNormals();
  return geometry;
}

function snowballGeometry(T: typeof Three) {
  const floret = blossomGeometry(T);
  const petals = floret.getAttribute('position');
  const positions: number[] = [];
  const colors: number[] = [];
  const outward = new T.Vector3();
  const up = new T.Vector3(0, 1, 0);
  const point = new T.Vector3();
  const facing = new T.Quaternion();
  for (let bloom = 0; bloom < 30; bloom++) {
    const y = 1 - (bloom + 0.5) / 15;
    const angle = bloom * 2.39996;
    const ring = Math.sqrt(1 - y * y);
    outward.set(Math.cos(angle) * ring, y, Math.sin(angle) * ring);
    facing.setFromUnitVectors(up, outward);
    const radius = 0.73 + random(bloom + 127) * 0.1;
    const size = 0.26 + random(bloom + 93) * 0.10;
    const shade = 0.67 + random(bloom + 17) * 0.33;
    for (let i = 0; i < petals.count; i++) {
      point.fromBufferAttribute(petals, i).multiplyScalar(size).applyQuaternion(facing).addScaledVector(outward, radius);
      positions.push(point.x, point.y, point.z);
      colors.push(shade, shade, shade);
    }
  }
  floret.dispose();
  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  geometry.computeVertexNormals();
  return geometry;
}

export function createCityFoliage(T: typeof Three) {
  const trees: (Plant & { kind: TreeKind })[] = [];
  const shrubs: (Plant & { kind: ShrubKind })[] = [];
  const bambooClumps: Plant[] = [];

  return {
    tree(x: number, y: number, z: number, size: number, seed: number, kind: TreeKind = 'broadleaf') {
      trees.push({ x, y, z, size, seed, kind });
    },
    shrub(x: number, y: number, z: number, size: number, seed: number, kind: ShrubKind = 'mound') {
      shrubs.push({ x, y, z, size, seed, kind });
    },
    bamboo(x: number, y: number, z: number, size: number, seed: number) {
      bambooClumps.push({ x, y, z, size, seed });
    },
    mount(scene: Three.Scene) {
      if (!trees.length && !shrubs.length && !bambooClumps.length) return;
      const pose = new T.Object3D();
      const branch = new T.Vector3();
      const up = new T.Vector3(0, 1, 0);
      const color = new T.Color();
      let treeLimbs = 0;
      let treeCrowns = 0;
      let ginkgoCount = 0;
      let goldenrainCount = 0;
      let snowballCount = 0;
      let wutongCount = 0;
      for (const { kind } of trees) {
        if (kind === 'wutong') {
          wutongCount++;
          continue;
        }
        treeLimbs += treeParts[kind].limbs;
        if (kind === 'ginkgo') ginkgoCount++;
        else treeCrowns += treeParts[kind].crowns;
        if (kind === 'goldenrain') goldenrainCount++;
        if (kind === 'snowball') snowballCount++;
      }
      const leaves = treeCrowns || shrubs.length ? leafCloud(T) : null;
      const leafMaterial = new T.MeshStandardMaterial({ side: T.DoubleSide, vertexColors: true, roughness: 0.9 });

      function batch(name: string, geometry: Three.BufferGeometry, material: Three.Material, capacity: number) {
        const mesh = new T.InstancedMesh(geometry, material, capacity);
        mesh.count = 0;
        mesh.name = name;
        mesh.castShadow = mesh.receiveShadow = true;
        scene.add(mesh);
        return mesh;
      }

      function limb(mesh: Three.InstancedMesh, ax: number, ay: number, az: number, bx: number, by: number, bz: number, radius: number) {
        branch.set(bx - ax, by - ay, bz - az);
        pose.scale.set(radius, branch.length(), radius);
        pose.quaternion.setFromUnitVectors(up, branch.normalize());
        pose.position.set((ax + bx) / 2, (ay + by) / 2, (az + bz) / 2);
        pose.updateMatrix();
        mesh.setMatrixAt(mesh.count++, pose.matrix);
      }

      function crown(mesh: Three.InstancedMesh, x: number, y: number, z: number, width: number, height: number, yaw: number, tint: string) {
        pose.position.set(x, y, z);
        pose.scale.set(width, height, width * 0.89);
        pose.rotation.set(0, yaw, 0);
        pose.updateMatrix();
        mesh.setColorAt(mesh.count, color.set(tint));
        mesh.setMatrixAt(mesh.count++, pose.matrix);
      }

      if (trees.length) {
        const wood = treeLimbs ? batch('tree-limbs', new T.CylinderGeometry(0.59, 1, 1, 7),
          new T.MeshStandardMaterial({ color: '#795e45', roughness: 0.92 }), treeLimbs) : null;
        const wutongBark = wutongCount ? batch('wutong-limbs', new T.CylinderGeometry(0.55, 1, 1, 8),
          new T.MeshStandardMaterial({ color: '#677953', roughness: 0.82 }),
          wutongCount * treeParts.wutong.limbs) : null;
        const parasols = wutongCount ? batch('wutong-canopies', leafCloud(T, 'parasol'), leafMaterial,
          wutongCount * treeParts.wutong.crowns) : null;
        const snowballs = snowballCount ? batch('snowball-blooms', snowballGeometry(T), leafMaterial,
          snowballCount * 5 * snowballsPerBranch) : null;
        const foliage = treeCrowns && leaves ? batch('tree-canopies', leaves, leafMaterial, treeCrowns) : null;
        const fans = ginkgoCount ?
          batch('ginkgo-canopies', leafCloud(T, 'fan'), leafMaterial, ginkgoCount * treeParts.ginkgo.crowns) : null;
        const panicles = goldenrainCount ? batch('goldenrain-panicles', blossomGeometry(T),
          new T.MeshStandardMaterial({ side: T.DoubleSide, roughness: 0.85 }),
          goldenrainCount * flowerClustersPerTree * blossomsPerCluster) : null;
        for (const { x, y, z, size, seed, kind } of trees) {
          const canopy = kind === 'ginkgo' ? fans! : foliage!;
          const tints = leafTints[kind];
          const angle = random(seed) * 2 * Math.PI;
          const forkX = x + Math.cos(angle) * size * 0.09;
          const forkZ = z + Math.sin(angle) * size * 0.09;
          if (kind === 'wutong' && wutongBark && parasols) {
            const forkY = y + size * 0.63;
            limb(wutongBark, x, y, z, forkX, forkY, forkZ, size * 0.083);
            limb(wutongBark, forkX, forkY, forkZ, x, y + size * 1.01, z, size * 0.046);
            crown(parasols, x, y + size * 1.06, z, size * 1.12, size * 0.46, angle, tints[0]);
            for (let i = 0; i < 5; i++) {
              const heading = angle + i * Math.PI * 0.4 + random(seed + i * 29) * 0.43;
              const dx = Math.cos(heading) * size;
              const dz = Math.sin(heading) * size;
              const reach = 0.56 + random(seed + i * 17 + 12) * 0.14;
              const bx = x + dx * 0.34;
              const bz = z + dz * 0.34;
              const by = y + size * (0.80 + random(seed + i * 11) * 0.12);
              limb(wutongBark, forkX, forkY, forkZ, bx, by, bz, size * 0.039);
              limb(wutongBark, bx, by, bz, x + dx * reach, by + size * 0.042, z + dz * reach, size * 0.023);
              crown(parasols, x + dx * reach, by + size * 0.1, z + dz * reach,
                size * 0.94, size * 0.32, heading, tints[i + 1]);
              crown(parasols, bx, by + size * 0.15, bz, size * 1.02, size * 0.39, heading, tints[i + 1]);
            }
            continue;
          }
          if (!wood) continue;
          const forkY = y + size * (kind === 'snowball' ? 0.28 : 0.48);
          const height = size * ((kind === 'ginkgo' ? 0.97 : 0.84) + random(seed + 4) * 0.17);
          limb(wood, x, y, z, forkX, forkY, forkZ, size * 0.064);

          if (kind === 'pine') {
            limb(wood, forkX, forkY, forkZ, x, y + size * 1.14, z, size * 0.041);
            crown(canopy, x, y + size * 1.07, z, size * 0.38, size * 0.29, angle, tints[2]);
            for (let tier = 0; tier < 3; tier++) {
              const by = y + size * (0.56 + tier * 0.19);
              const branchY = by - size * 0.075;
              const alongTrunk = (branchY - forkY) / (size * 0.66);
              for (const side of [-1, 1]) {
                const heading = angle + tier * 1.34 + side * Math.PI / 2;
                const reach = size * (0.35 - tier * 0.078);
                const bx = x + Math.cos(heading) * reach;
                const bz = z + Math.sin(heading) * reach;
                limb(wood, forkX + (x - forkX) * alongTrunk, branchY,
                  forkZ + (z - forkZ) * alongTrunk, bx, by, bz, size * 0.035);
                crown(canopy, bx, by + size * 0.04, bz, size * (0.77 - tier * 0.14),
                  size * 0.23, heading, tints[tier]);
              }
            }
            continue;
          }

          const willow = kind === 'willow';
          const ginkgo = kind === 'ginkgo';
          const goldenrain = kind === 'goldenrain';
          const snowball = kind === 'snowball';
          limb(wood, forkX, forkY, forkZ, x, y + height, z, size * 0.041);
          crown(canopy, x, y + height, z, size * (ginkgo ? 0.58 : 0.79), size * 0.54, angle, tints[0]);

          for (let i = 0; i < 5; i++) {
            const heading = angle + i * 2.4;
            const spread = (willow ? 0.45 : ginkgo ? 0.16 : goldenrain || snowball ? 0.39 : 0.29)
              + random(seed + i * 7 + 1) * 0.18;
            const reach = size * spread;
            const bx = forkX + Math.cos(heading) * reach;
            const bz = forkZ + Math.sin(heading) * reach;
            const by = y + size * ((ginkgo ? 0.68 : 0.55) + random(seed + i * 9 + 2) * 0.31);
            const width = size * ((ginkgo ? 0.47 : goldenrain ? 0.72 : 0.61) + random(seed + i * 13 + 3) * 0.18);
            limb(wood, forkX, forkY - size * 0.065, forkZ, bx, by, bz, size * 0.033);
            crown(canopy, bx, by + size * 0.07, bz, width, width * (ginkgo ? 1.02 : willow ? 0.56 : 0.68),
              heading, tints[1 + i % (tints.length - 1)]);
            if (goldenrain && panicles) {
              const bloomY = by + width * 0.25;
              for (let bloom = 0; bloom < blossomsPerCluster; bloom++) {
                const rotation = heading + bloom * 2.4;
                const spray = size * (0.16 - bloom * 0.019);
                const radius = size * (0.048 - bloom * 0.0045);
                crown(panicles, bx + Math.cos(rotation) * spray, bloomY + bloom * size * 0.022,
                  bz + Math.sin(rotation) * spray, radius, radius * 0.84, rotation, bloom % 3 ? '#e5c65f' : '#f2df8b');
              }
            }
            if (snowball && snowballs) {
              for (let bloom = 0; bloom < snowballsPerBranch; bloom++) {
                const variation = seed + i * 59 + bloom * 13;
                const scatter = heading + (random(variation + 1) - 0.5) * 2.8;
                const offset = width * (0.12 + random(variation + 2) * 0.34);
                const ball = size * (0.081 + random(variation + 3) * 0.060);
                const tint = ['#f1f4da', '#ffffef', '#d4e29e'][Math.floor(random(variation + 5) * 3)];
                crown(snowballs, bx + Math.cos(scatter) * offset,
                  by + width * (0.26 + random(variation + 4) * 0.29),
                  bz + Math.sin(scatter) * offset, ball, ball * 0.94, scatter, tint);
              }
            }
            if (!willow) continue;
            for (const spray of [-1, 1]) {
              const tip = heading + spray * 0.61;
              const drop = size * (0.21 + random(seed + i * 27 + spray) * 0.08);
              const sprayY = Math.max(y + size * 0.35, by - drop);
              crown(canopy, bx + Math.cos(tip) * size * 0.15, sprayY, bz + Math.sin(tip) * size * 0.15,
                size * 0.27, size * 0.64, tip, tints[1 + (i + Number(spray > 0)) % 2]);
            }
          }
        }
      }

      if (shrubs.length && leaves) {
        const foliage = batch('shrub-canopies', leaves, leafMaterial, shrubs.length * 5);
        const blossomCount = shrubs.reduce((count, { kind }) => count + (kind === 'flowering' ? blossomsPerShrub : 0), 0);
        const flowers = blossomCount ? batch('shrub-blossoms', blossomGeometry(T),
          new T.MeshStandardMaterial({ side: T.DoubleSide, roughness: 0.85 }), blossomCount) : null;
        for (const { x, y, z, size, seed, kind } of shrubs) {
          const angle = random(seed) * 2 * Math.PI;
          const flowering = kind === 'flowering';
          crown(foliage, x, y + size * 0.24, z, size * 0.72, size * 0.35, angle, flowering ? '#70854d' : '#657e4d');
          for (let lobe = 0; lobe < 4; lobe++) {
            const heading = angle + lobe * 2.4;
            const reach = size * (0.19 + random(seed + lobe) * 0.09);
            crown(foliage, x + Math.cos(heading) * reach, y + size * (flowering ? 0.19 : 0.16),
              z + Math.sin(heading) * reach, size * (flowering ? 0.43 : 0.54), size * 0.28,
              heading, ['#4a6e49', '#769154', '#647e4b', '#59784e'][lobe]);
          }
          if (!flowering || !flowers) continue;
          for (let bloom = 0; bloom < blossomsPerShrub; bloom++) {
            const spin = angle + bloom * 2.4;
            const reach = size * (0.09 + random(seed + bloom * 17) * 0.24);
            const scale = size * (0.042 + random(seed + bloom * 13) * 0.035);
            pose.position.set(x + Math.cos(spin) * reach,
              y + size * (0.35 - reach / size * 0.23 + random(seed + bloom * 19) * 0.035),
              z + Math.sin(spin) * reach);
            pose.rotation.set(Math.sin(spin) * 0.52, spin, Math.cos(spin) * 0.52);
            pose.scale.setScalar(scale);
            pose.updateMatrix();
            flowers.setColorAt(flowers.count, color.set(['#dfad9f', '#eef0d0', '#aa636a'][bloom % 3]));
            flowers.setMatrixAt(flowers.count++, pose.matrix);
          }
        }
      }

      if (bambooClumps.length) {
        const culms = batch('bamboo-culms', culmGeometry(T),
          new T.MeshStandardMaterial({ vertexColors: true, roughness: 0.86 }),
          bambooClumps.length * culmsPerClump);
        const sprays = batch('bamboo-leaves', leafCloud(T, 'blade'), leafMaterial,
          bambooClumps.length * culmsPerClump * 3);
        for (const { x, y, z, size, seed } of bambooClumps) {
          for (let stem = 0; stem < culmsPerClump; stem++) {
            const heading = stem * 2.4 + random(seed) * 2 * Math.PI;
            const dx = Math.cos(heading);
            const dz = Math.sin(heading);
            const reach = size * (0.05 + 0.131 * random(seed + stem * 21));
            const height = size * (0.80 + 0.19 * random(seed + stem * 31));
            const bend = size * (0.045 + 0.08 * random(seed + stem * 37));
            const bx = x + dx * reach;
            const bz = z + dz * reach;
            branch.set(dx * bend, height, dz * bend);
            pose.position.set(bx, y, bz);
            pose.scale.set(size * 0.016, branch.length(), size * 0.016);
            pose.quaternion.setFromUnitVectors(up, branch.normalize());
            pose.updateMatrix();
            culms.setMatrixAt(culms.count++, pose.matrix);
            for (let tier = 0; tier < 3; tier++) {
              const t = 0.59 + tier * 0.15;
              const pitch = heading + (tier % 2 ? -0.84 : 0.84);
              const spread = size * (0.13 - tier * 0.032);
              crown(sprays, bx + dx * bend * t + Math.cos(pitch) * spread, y + height * t,
                bz + dz * bend * t + Math.sin(pitch) * spread,
                size * (0.40 - tier * 0.058), size * 0.16,
                pitch, ['#5d7e4d', '#71924e', '#95a35d'][tier]);
            }
          }
        }
      }
    },
  };
}
