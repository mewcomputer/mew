import { createCharacterTrack, createCitySpriteArt } from './city-characters.ts';
import { loadAgentArtwork } from './agent-sprites.ts';
import { createCharacterBillboards } from './character-billboard.ts';
import { cityLighting, mountCityLighting, type CityTime } from './city-lighting.ts';
import { createReflectiveWater } from './city-water.ts';
import { createGardenDaylight } from './garden-daylight.ts';
import { createGardenLanterns, mountLanternRack } from './garden-lanterns.ts';
import { createGardenRenderQuality } from './garden-render-quality.ts';
import { gardenWaterGeometry } from './garden-waterfall.ts';
import { mountGardenWind } from './garden-wind.ts';
import { gardenOutfits, gardenOverview, mountImportedGarden } from './imported-garden.ts';
import { createLanternRound } from './lantern-round.ts';
import { mewCompactQuery, mewCompactTiltShift, mewOverview, mewTiltShift, mountMewLabels, mountMewStory } from './mew-story.ts';
import { createSceneView } from './scene-view.ts';
import { createTiltShift } from './tilt-shift.ts';

const story = document.querySelector<HTMLElement>('[data-mew-story]');
const viewport = document.querySelector<HTMLElement>('[data-garden-stage]');
const canvas = viewport?.querySelector('canvas');
const status = viewport?.querySelector<HTMLParagraphElement>('[data-status]');
const toolbar = document.querySelector<HTMLElement>('[data-controls]');
const sheet = viewport?.dataset.spriteSheet;
const modelUrl = viewport?.dataset.gardenModel;
if (!viewport || !canvas || !status || !toolbar || !sheet || !modelUrl) throw new Error('Missing Mew garden elements.');

const lifetime = new AbortController();
window.addEventListener('pagehide', (event) => {
  if (!event.persisted) lifetime.abort();
}, { signal: lifetime.signal });

Promise.all([
  import('three'),
  import('three/addons/loaders/GLTFLoader.js'),
  loadAgentArtwork(sheet, gardenOutfits),
  fetch(modelUrl, { signal: lifetime.signal }).then((response) => {
    if (!response.ok) throw new Error(`Garden model request failed (${response.status}).`);
    return response.arrayBuffer();
  }),
]).then(async ([T, { GLTFLoader }, artwork, modelBytes]) => {
  const { scene: model } = await new GLTFLoader().parseAsync(modelBytes, '');
  if (lifetime.signal.aborted) return;
  const compact = story && matchMedia(`${mewCompactQuery}, (pointer: coarse)`).matches;
  const quality = compact ? createGardenRenderQuality() : null;
  const view = createSceneView(T, canvas, {
    viewport, alpha: false, exposure: cityLighting.exposure, far: 150, shadows: true,
    maxPixelRatio: quality?.current.pixelRatio,
    orbit: {
      ...(story ? mewOverview : gardenOverview), width: story ? 53.75 : 43, height: 35, radius: 65,
      turn: Math.PI / 4, zoomFactor: 1.2, minPitch: 0.37, maxPitch: 0.8, minZoom: 0.8, maxZoom: 2.5,
      drag: { threshold: 4, yaw: 0.006, pitch: 0.004 }, wheelZoom: !story,
    },
  });
  lifetime.signal.addEventListener('abort', view.dispose, { once: true });
  const { scene, renderer, camera, motion, signal } = view;
  const garden = mountImportedGarden(T, scene, model);
  const wind = mountGardenWind(T, scene);
  const light = mountCityLighting(T, scene);
  const { pool, spillway, level, runoff } = gardenWaterGeometry(T, garden.pool);
  const water = createReflectiveWater(T, scene, renderer, level);
  water.channel(pool);
  water.fallingSheet(spillway, true);
  water.sprayAt(runoff, 4.2);
  const tiltShift = createTiltShift(T, renderer, compact ? mewCompactTiltShift : story ? mewTiltShift : undefined);
  const billboard = createCharacterBillboards(T);
  const carryLantern = createGardenLanterns(T);
  mountLanternRack(T, scene, garden.rack);
  const characters = garden.cast.map((definition, index) => {
    const { texture, paint, hand } = createCitySpriteArt(T, definition.outfit, artwork);
    const sprite = billboard(texture);
    sprite.name = definition.id;
    scene.add(sprite);
    const route = garden.rounds.get(definition.id);
    const round = route ? createLanternRound(route.path, route.joinIndex, route.speed,
      route.rackDirection, route.departure) : null;
    const lantern = route ? carryLantern(sprite, index * 1.7) : null;
    if (lantern && route) {
      lantern.store(route.hook);
      scene.add(lantern.root);
    }
    return { sprite, paint, hand, lantern, route, round, sample: round ? null : createCharacterTrack(definition) };
  });

  const nightButton = toolbar.querySelector<HTMLButtonElement>('[data-night]');
  const sky = createGardenDaylight(true);
  const background = new T.Color();
  const dayBackground = new T.Color('#eee9dc');
  const nightBackground = new T.Color('#131c1c');
  const twilightColors = { dusk: new T.Color('#9f8495'), dawn: new T.Color('#cfab99') };
  scene.background = background;
  scene.fog = null;
  let lastLighting: CityTime | undefined;
  let skyFrame: number | undefined;
  let time = 0;
  let reflectedLanternFrame = -1;

  nightButton?.addEventListener('click', () => {
    sky.setNight(!sky.nightRequested);
    skyFrame = performance.now();
    nightButton.setAttribute('aria-pressed', String(sky.nightRequested));
    if (story) {
      const night = sky.nightRequested;
      document.documentElement.dataset.theme = night ? 'dark' : 'light';
      nightButton.setAttribute('aria-label', night ? 'Switch to day' : 'Switch to night');
      nightButton.querySelector('path')?.setAttribute('d', night
        ? 'M12 3v2M12 19v2M3 12h2M19 12h2M5.6 5.6l1.4 1.4M17 17l1.4 1.4M5.6 18.4 7 17M17 7l1.4-1.4M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0z'
        : 'M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z');
    }
    view.request();
  }, { signal });

  const stops = [...toolbar.querySelectorAll<HTMLButtonElement>('[data-focus]')];
  for (const button of stops) {
    button.addEventListener('click', () => {
      const name = button.dataset.focus;
      if (!name || !Object.hasOwn(garden.stops, name)) return;
      const stop = garden.stops[name as keyof typeof garden.stops];
      for (const choice of stops) choice.setAttribute('aria-pressed', String(choice === button));
      view.move(new T.Vector3(...stop.target), stop.zoom, stop);
    }, { signal });
  }
  for (const direction of ['left', 'right'] as const) {
    toolbar.querySelector(`[data-turn="${direction}"]`)?.addEventListener('click', () => view.control(direction), { signal });
  }

  if (story) mountMewStory(story, view, garden.stops, toolbar);
  canvas.hidden = false;
  toolbar.hidden = false;
  status.hidden = true;
  view.start({
    ...(story && mountMewLabels(T, view, characters)),
    beforeRender(now, delta) {
      const nextQuality = quality?.recordFrame(delta * 1000);
      if (nextQuality) {
        const pixelRatio = Math.min(devicePixelRatio || 1, nextQuality.pixelRatio);
        if (pixelRatio !== renderer.getPixelRatio()) renderer.setPixelRatio(pixelRatio);
      }
      time += delta;
      const skyDelta = Math.max(0, now - (skyFrame ?? now)) / 1000;
      skyFrame = now;
      const { lighting, twilight, lanternsNeeded, lanternLevel } = sky.advance(skyDelta, motion.matches);
      if (lighting !== lastLighting) {
        lastLighting = lighting;
        light.setTime(lighting);
        water.setTime(lighting, light.keyColor);
        background.copy(dayBackground).lerp(nightBackground, lighting.night)
          .lerp(twilightColors[twilight], lighting.twilight * 0.38);
        garden.setLampLevel(lanternLevel);
        renderer.shadowMap.needsUpdate = true;
      }
      water.time.value = time;
      wind.update(time, motion.matches);
      for (const { sprite, paint, hand, lantern, route, round, sample } of characters) {
        const patrol = round?.(time, lanternsNeeded);
        const { position, mode, direction, ticks } = patrol?.pose ?? sample!(time);
        sprite.position.set(position[0], position[1] + 0.025, position[2]);
        sprite.quaternion.copy(camera.quaternion);
        const carrying = patrol?.carrying ?? false;
        paint(mode, direction, view.yaw, ticks, carrying);
        if (lantern && route) {
          if (patrol?.carrying) {
            lantern.update(hand(), time, mode === 'walk', lanternLevel);
            if (patrol.reach < 1) lantern.shiftToHand(route.hook, patrol.reach);
          } else {
            lantern.store(route.hook);
          }
        }
      }
      const lanternFrame = Math.floor(time * 4);
      if (lanternLevel > 0 && lanternFrame !== reflectedLanternFrame) {
        reflectedLanternFrame = lanternFrame;
        water.invalidate();
      }
      if (!motion.matches) view.request(34);
    },
    render(zoom) {
      water.capture(camera, quality?.current.waterCaptureSize);
      if (tiltShift) tiltShift.render(scene, camera, zoom, 65);
      else renderer.render(scene, camera);
    },
    onInvalidate: water.invalidate,
    onSuspend() {
      skyFrame = undefined;
      quality?.resetTiming();
    },
    onContextChange(lost) {
      status.hidden = !lost;
      status.textContent = 'Waiting for the graphics connection…';
    },
    onDispose() {
      water.dispose();
      light.dispose();
      tiltShift?.dispose();
    },
  });
}).catch((error) => {
  if (lifetime.signal.aborted) return;
  lifetime.abort();
  status.hidden = false;
  status.textContent = 'The garden could not start.';
  console.warn('Mew garden initialization', error);
});
