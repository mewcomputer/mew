import type * as Three from 'three';
import { createDistrictView } from './district-view.ts';
import type { createAgentTraffic } from './agent-traffic.ts';
import type { District } from './district.ts';
import {
  createAgentPainter,
  spriteCanvasSize,
  spriteGround,
  spriteView,
  type SpriteArtwork,
  type SpriteRole,
} from './agent-sprites.ts';
import { waterLayer } from './water-material.ts';

interface MapSceneOptions {
  canvas: HTMLCanvasElement;
  cast: readonly { id: string; role: SpriteRole }[];
  artwork: SpriteArtwork;
  district: District;
  traffic: ReturnType<typeof createAgentTraffic>;
  unavailable: (unavailable: boolean) => void;
  appearance?: (id: string) => { working?: boolean; muted?: boolean } | undefined;
  onProject?: (id: string, x: number, y: number, visible: boolean) => void;
  animateWater?: boolean;
  initialZoom?: number;
}

interface ActorSprite {
  sprite: Three.Sprite;
  texture: Three.CanvasTexture;
  shadow: Three.Mesh<Three.CircleGeometry, Three.MeshBasicMaterial>;
  painter: ReturnType<typeof createAgentPainter>;
}

export function mountMapScene(
  T: typeof Three,
  {
    canvas,
    cast,
    artwork,
    district,
    traffic,
    unavailable,
    appearance,
    onProject,
    animateWater = false,
    initialZoom = 1,
  }: MapSceneOptions,
) {
  const view = createDistrictView(T, canvas, district, {
    exposure: 1.13,
    orbit: {
      target: [0.2, 1.3, 0.3], yaw: Math.PI / 4, pitch: 0.57, zoom: initialZoom,
      width: 15.8, radius: 27, turn: Math.PI / 12, zoomFactor: 1.18,
      minPitch: 0.44, maxPitch: 1.13, minZoom: 0.7, maxZoom: 2.5, snapReset: true,
    },
  });
  const { scene, motion, water, waterScene, request } = view;
  const sun = new T.DirectionalLight('#ffe9cb', 2.4);
  sun.position.set(-8, 15, 7);
  scene.add(sun, new T.HemisphereLight('#e0eade', '#374e48', 1.35));
  for (const landmark of Object.values(view.landmarks)) {
    landmark.ring.visible = landmark.marker.visible = false;
  }
  waterScene.setLighting(false, sun);

  const actorSprites = new Map<string, ActorSprite>();
  const shadowGeometry = new T.CircleGeometry(0.19, 18).rotateX(-Math.PI / 2);
  const shadowMaterial = new T.MeshBasicMaterial({
    color: '#1e312b',
    transparent: true,
    opacity: 0.34,
    depthWrite: false,
  });
  for (const agent of cast) {
    const painter = createAgentPainter(agent.role, artwork);
    const texture = new T.CanvasTexture(painter.canvas);
    texture.magFilter = texture.minFilter = T.NearestFilter;
    texture.generateMipmaps = false;
    texture.colorSpace = T.SRGBColorSpace;
    const sprite = new T.Sprite(
      new T.SpriteMaterial({ map: texture, alphaTest: 0.48, toneMapped: false }),
    );
    sprite.center.set(0.5, 1 - spriteGround);
    sprite.scale.set(spriteCanvasSize * 0.031, spriteCanvasSize * 0.031, 1);
    sprite.layers.set(waterLayer);
    const shadow = new T.Mesh(shadowGeometry, shadowMaterial);
    shadow.layers.set(waterLayer);
    scene.add(sprite, shadow);
    actorSprites.set(agent.id, { sprite, texture, shadow, painter });
  }
  const marker = new T.Mesh(
    new T.BoxGeometry(0.08, 0.08, 0.08),
    new T.MeshBasicMaterial({ color: '#b2ebcc' }),
  );
  marker.visible = false;
  marker.layers.set(waterLayer);
  scene.add(marker);
  const projected = new T.Vector3();
  let saved: ReturnType<typeof view.cameraState> | null = null;
  let handoff: { from: Three.Vector3; to: Three.Vector3; start: number } | null = null;
  let hydraulicEnd = 0;

  function beforeRender(now: number) {
    const flowing = view.waterTime < hydraulicEnd || (animateWater && !motion.matches);
    view.advanceWater(now, flowing);
    if (flowing) request(25);
    if (handoff) {
      const t = Math.min(1, (now - handoff.start) / 620);
      marker.position.lerpVectors(handoff.from, handoff.to, t);
      marker.position.y += Math.sin(Math.PI * t) * 0.28;
      marker.rotation.y = t * 4;
      marker.visible = t < 1;
      if (t < 1) request();
      else handoff = null;
    }
    for (const [id, { sprite, texture, shadow, painter }] of actorSprites) {
      const actor = traffic.get(id);
      if (!actor) continue;
      const look = appearance?.(id);
      sprite.position.set(...actor.position);
      shadow.position.copy(sprite.position);
      shadow.position.y += 0.018;
      const working = look?.working;
      const mode = motion.matches ? 'idle' : actor.path.length ? 'walk' : working ? 'work' : 'idle';
      const ticks =
        mode === 'walk' ? Math.floor(actor.walked * 30) : mode === 'work' ? Math.floor(now * 0.048) : 0;
      if (painter.paint(mode, spriteView(actor.direction, view.yaw), ticks)) texture.needsUpdate = true;
      if (mode === 'work') request(110);
      sprite.material.color.set(look?.muted ? '#64746d' : '#ffffff');
    }
  }

  function projectLabels() {
    if (!onProject) return;
    for (const [id, { sprite }] of actorSprites) {
      projected.copy(sprite.position);
      projected.y += 1.19;
      const { x, y, width, height } = view.project(projected);
      onProject(id, x, y, x >= 22 && x <= width - 22 && y >= 25 && y <= height - 35);
    }
  }

  view.start({ beforeRender, afterRender: onProject && projectLabels, onContextChange: unavailable });
  return {
    control: view.control,
    update: request,
    signal: view.signal,
    dispose: view.dispose,
    handoff(from: string, to: string) {
      if (motion.matches) return;
      const sender = traffic.get(from);
      const recipient = traffic.get(to);
      if (!sender || !recipient) return;
      const start = new T.Vector3(...sender.position);
      const end = new T.Vector3(...recipient.position);
      start.y += 0.43;
      end.y += 0.43;
      handoff = { from: start, to: end, start: performance.now() };
      request();
    },
    setGate(closed: boolean) {
      if (water.gateClosed === closed) return;
      water.setGate(closed);
      waterScene.setGate(closed);
      hydraulicEnd = view.waterTime + 12;
      view.resetClock();
      view.invalidate();
    },
    focus(id: string) {
      const actor = traffic.get(id);
      if (!actor) return;
      saved ??= view.cameraState();
      view.move(new T.Vector3(...actor.position), 1.75);
    },
    back() {
      if (!saved) return;
      const previous = saved;
      saved = null;
      view.move(previous.target, previous.zoom);
    },
  };
}
