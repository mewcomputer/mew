import type * as Three from 'three';
import { citySpritePixelSize, spriteCanvasSize, spriteGround } from './agent-sprites.ts';

export function createGardenLanterns(T: typeof Three) {
  const frame = new T.MeshStandardMaterial({ color: '#775438', roughness: 0.7 });
  const shade = new T.CylinderGeometry(0.114, 0.103, 0.194, 6);
  const rim = new T.CylinderGeometry(0.122, 0.122, 0.028, 6);
  const rib = new T.CylinderGeometry(0.009, 0.009, 0.198, 4);
  const handle = new T.TorusGeometry(0.047, 0.009, 5, 10);
  const pin = new T.CylinderGeometry(0.020, 0.008, 0.068, 6);
  const handHeight = (spriteGround - 0.5) * spriteCanvasSize;

  return (carrier: Three.Object3D, phase: number) => {
    const root = new T.Group();
    root.name = 'carried-lantern';
    root.visible = false;
    const ring = new T.Mesh(handle, frame);
    root.add(ring);

    const body = new T.Group();
    root.add(body);
    function piece(geometry: Three.BufferGeometry, material: Three.Material, x: number, y: number, z: number) {
      const mesh = new T.Mesh(geometry, material);
      mesh.position.set(x, y, z);
      body.add(mesh);
    }

    const paper = new T.MeshStandardMaterial({
      color: '#b78a53', roughness: 0.93, emissive: '#ffbc72', emissiveIntensity: 0,
    });
    piece(shade, paper, 0, -0.15, 0);
    piece(rim, frame, 0, -0.056, 0);
    piece(rim, frame, 0, -0.244, 0);
    piece(pin, frame, 0, -0.285, 0);
    for (let i = 0; i < 6; i++) {
      const angle = i * Math.PI / 3;
      piece(rib, frame, Math.sin(angle) * 0.107, -0.15, Math.cos(angle) * 0.107);
    }

    const light = new T.PointLight('#ffbc72', 2.7, 3.25, 1.35);
    light.name = 'lantern-light';
    light.position.y = -0.15;
    body.add(light);

    return {
      root,
      store(hook: readonly [number, number, number]) {
        root.visible = true;
        root.position.set(...hook);
        body.rotation.set(0, 0, 0);
        paper.emissiveIntensity = 0;
        light.intensity = 0;
      },
      shiftToHand(hook: readonly [number, number, number], reach: number) {
        root.position.set(
          T.MathUtils.lerp(hook[0], root.position.x, reach),
          T.MathUtils.lerp(hook[1], root.position.y, reach),
          T.MathUtils.lerp(hook[2], root.position.z, reach),
        );
        light.intensity *= reach;
        paper.emissiveIntensity *= reach;
      },
      update(grip: readonly [number, number] | null, time: number, walking = false, lightLevel = 1) {
        root.visible = grip !== null;
        if (!grip) return;
        root.position.set(grip[0] * citySpritePixelSize, (handHeight - grip[1]) * citySpritePixelSize, 0.09);
        carrier.localToWorld(root.position);
        body.rotation.z = (walking ? 0.18 : 0.047) * Math.sin(time * 4.6 + phase);
        body.rotation.x = 0.065 * Math.sin(time * 3.7 + phase);
        light.intensity = lightLevel * (2.7 + 0.15 * Math.sin(time * 8.4 + phase) + 0.07 * Math.sin(time * 17.1));
        paper.emissiveIntensity = lightLevel * 1.75;
      },
    };
  };
}


export function mountLanternRack(T: typeof Three, scene: Three.Scene, at: readonly [number, number, number]) {
  const rack = new T.Group();
  rack.name = 'lantern-rack';
  rack.position.set(...at);
  const wood = new T.MeshStandardMaterial({ color: '#655041', roughness: 0.83 });
  const metal = new T.MeshStandardMaterial({ color: '#ab885b', metalness: 0.48, roughness: 0.52 });
  const stone = new T.MeshStandardMaterial({ color: '#928b79', roughness: 0.89 });
  function part(w: number, h: number, d: number, x: number, y: number, z: number, material: Three.Material) {
    const mesh = new T.Mesh(new T.BoxGeometry(w, h, d), material);
    mesh.position.set(x, y, z);
    mesh.castShadow = mesh.receiveShadow = true;
    rack.add(mesh);
  }
  for (const side of [-1, 1]) {
    part(0.26, 0.13, 0.27, side * 0.68, 0.065, 0, stone);
    part(0.12, 1.04, 0.14, side * 0.68, 0.61, 0, wood);
    part(0.19, 0.065, 0.18, side * 0.68, 1.14, 0, metal);
    part(0.045, 0.062, 0.18, side * 0.42, 0.87, -0.061, metal);
  }
  part(1.59, 0.12, 0.18, 0, 0.93, 0, wood);
  part(1.55, 0.045, 0.25, 0, 1.025, 0, metal);
  scene.add(rack);
}
