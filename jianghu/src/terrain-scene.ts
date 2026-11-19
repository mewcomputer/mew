import type * as Three from 'three';
import { meshChunk, type Chunk, type World } from './world.ts';

export function createTerrainScene(
  T: typeof Three, scene: Three.Scene, world: World, materials: readonly string[],
) {
  const palette = materials.map((tint) => new T.Color(tint));
  const material = new T.MeshStandardMaterial({ vertexColors: true, roughness: 0.95 });
  const chunks = new Map<number, Three.Mesh<Three.BufferGeometry, Three.MeshStandardMaterial>>();
  const { chunkColumns, chunkRows } = world;
  let meshes: Three.Mesh<Three.BufferGeometry, Three.MeshStandardMaterial>[] = [];

  function rebuild(affected: readonly Chunk[]) {
    for (const [cx, cz] of affected) {
      const key = cx + cz * chunkColumns;
      const previous = chunks.get(key);
      if (previous) {
        scene.remove(previous);
        previous.geometry.dispose();
        chunks.delete(key);
      }
      const { positions, normals, colors, indices } = meshChunk(world, cx, cz, palette);
      if (!positions.length) continue;
      const geometry = new T.BufferGeometry();
      geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
      geometry.setAttribute('normal', new T.Float32BufferAttribute(normals, 3));
      geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
      geometry.setIndex(indices);
      const mesh = new T.Mesh(geometry, material);
      mesh.castShadow = mesh.receiveShadow = true;
      chunks.set(key, mesh);
      scene.add(mesh);
    }
    meshes = [...chunks.values()];
  }

  const initial: Chunk[] = [];
  for (let cx = 0; cx < chunkColumns; cx++) {
    for (let cz = 0; cz < chunkRows; cz++) initial.push([cx, cz]);
  }
  rebuild(initial);
  return {
    rebuild,
    get meshes() {
      return meshes;
    },
  };
}
