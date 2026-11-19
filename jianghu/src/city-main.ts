import type * as Three from 'three';
import type { SpriteArtwork } from './agent-sprites.ts';
import { loadAgentArtwork } from './agent-sprites.ts';
import { createRainbellAudio } from './city-audio.ts';
import { baichuanCast, mountCityCharacters } from './city-characters.ts';
import { rainbellStrikeBetween } from './city-encounters.ts';
import { cityLighting, defaultCityTime, mountCityLighting, sampleCityTime } from './city-lighting.ts';
import { buildCity } from './city-scene.ts';
import { buildCityWater } from './city-water.ts';
import { operaCast } from './opera-show.ts';
import { createRainbellPlaytest } from './rainbell-playtest.ts';
import { rainbellSite, rainbellWorld } from './rainbell-world.ts';
import { createSceneView, type CameraControl } from './scene-view.ts';
import { createTiltShift } from './tilt-shift.ts';

const orbitRadius = 145;

const viewport = document.querySelector('main');
const canvas = viewport?.querySelector('canvas');
const status = viewport?.querySelector<HTMLParagraphElement>('[data-graphics-status]');
const controls = viewport?.querySelector<HTMLElement>('.movement-tools');
const testButton = controls?.querySelector('button');
const newStart = controls?.querySelector<HTMLButtonElement>('[data-new-start]');
const timeSlider = controls?.querySelector<HTMLInputElement>('.time-control input');
const timeLabel = controls?.querySelector<HTMLOutputElement>('[data-city-time]');
const movementStatus = controls?.querySelector<HTMLOutputElement>('[data-movement-status]');
const spriteSheet = viewport?.dataset.spriteSheet;
if (!viewport || !canvas || !status || !spriteSheet || !controls || !testButton || !newStart ||
    !timeSlider || !timeLabel || !movementStatus) {
  throw new Error('Baichuan requires its viewport, movement controls and sprite sheet.');
}
const lifetime = new AbortController();
const { signal } = lifetime;
const listenerOptions = { signal };

Promise.all([
  import('three'),
  loadAgentArtwork(spriteSheet, [...new Set([...baichuanCast, ...operaCast].map(({ outfit }) => outfit))]),
])
  .then(([T, artwork]) => {
    if (!signal.aborted) mount(T, artwork, { viewport, canvas, status, controls, testButton, newStart, timeSlider, timeLabel, movementStatus });
  })
  .catch((error) => {
    status.textContent = 'Baichuan could not load its graphics.';
    console.warn('Baichuan graphics initialization', error);
  });

window.addEventListener('pagehide', (event) => {
  if (!event.persisted) lifetime.abort();
}, listenerOptions);

function mount(
  T: typeof Three, artwork: SpriteArtwork,
  { viewport, canvas, status, controls, testButton, newStart, timeSlider, timeLabel, movementStatus }: {
    viewport: HTMLElement; canvas: HTMLCanvasElement; status: HTMLParagraphElement;
    controls: HTMLElement; testButton: HTMLButtonElement; newStart: HTMLButtonElement;
    timeSlider: HTMLInputElement; timeLabel: HTMLOutputElement; movementStatus: HTMLOutputElement;
  },
) {
  const view = createSceneView(T, canvas, {
    viewport,
    alpha: false,
    far: 360,
    exposure: cityLighting.exposure,
    shadows: true,
    orbit: {
      target: [0, 11.6, 0], yaw: 0.39, pitch: 0.60, zoom: 0.90,
      width: 92, height: 104, radius: orbitRadius, turn: 0.12, zoomFactor: 1.15,
      minPitch: 0.17, maxPitch: 0.78, minZoom: 0.75, maxZoom: 4.2,
      yawLimits: [-1.4, 1.4], pan: { x: [-58, 58], z: [-70, 74] },
      wheelZoom: true, drag: { threshold: 4, yaw: 0.006, pitch: 0.004 },
    },
  });
  const { renderer, scene, camera, motion } = view;
  signal.addEventListener('abort', view.dispose, { once: true });
  const lighting = mountCityLighting(T, scene);
  const tiltShift = createTiltShift(T, renderer);
  const water = buildCityWater(T, scene, renderer);
  const city = buildCity(T, scene, water.time);
  const updateCharacters = mountCityCharacters(T, scene, artwork);
  const playtest = createRainbellPlaytest(T, scene, artwork);
  const bell = createRainbellAudio(view.signal);
  canvas.addEventListener('pointerup', bell.unlock, { signal: view.signal });

  let gaugeLit = false;
  let testing = false;
  let minute = defaultCityTime;
  let night = 0;
  let timeChange: { from: number; to: number; start: number } | null = null;
  let hoverPending = false;
  const hoverPoint = new T.Vector2();
  const pickHint = `Choose a walkable tile. ${rainbellWorld.movement} movement points per turn.`;
  function clearHover() {
    hoverPending = false;
    playtest.clearHover();
    view.request();
  }

  function resetStart() {
    hoverPending = false;
    playtest.clear();
    newStart.hidden = true;
    movementStatus.value = testing ? pickHint : '';
    view.request();
  }

  function showRange(count: number) {
    newStart.hidden = false;
    movementStatus.value = `${count} reachable ${count === 1 ? 'tile' : 'tiles'}. Hover to preview, click to walk.`;
  }

  function setPlaytest(active: boolean) {
    testing = active;
    playtest.root.visible = active;
    viewport.toggleAttribute('data-playtest', active);
    testButton.setAttribute('aria-pressed', String(active));
    movementStatus.hidden = !active;
    resetStart();
    if (active) view.setView([rainbellSite[0] - 3, rainbellSite[1] + 3.15, rainbellSite[2] - 3], 0.42, 0.65, 3.4);
  }
  testButton.addEventListener('click', () => setPlaytest(!testing), { signal: view.signal });
  timeSlider.setAttribute('aria-valuetext', timeLabel.value);
  timeSlider.addEventListener('input', () => {
    const to = timeSlider.valueAsNumber;
    const hours = String(Math.floor(to / 60)).padStart(2, '0');
    const minutes = String(to % 60).padStart(2, '0');
    timeLabel.value = `${hours}:${minutes}`;
    timeSlider.setAttribute('aria-valuetext', timeLabel.value);
    timeChange = { from: minute, to, start: performance.now() };
    view.request();
  }, { signal: view.signal });
  newStart.addEventListener('click', resetStart, { signal: view.signal });
  canvas.addEventListener('wheel', clearHover, { signal: view.signal, passive: true });
  const keys = new Map<string, CameraControl>([
    ['arrowleft', 'left'], ['arrowright', 'right'],
    ['+', 'in'], ['=', 'in'], ['-', 'out'], ['r', 'reset'],
  ]);
  const panKeys = new Map([
    ['w', [0, 1]], ['a', [-1, 0]], ['s', [0, -1]], ['d', [1, 0]],
  ]);
  const pressed = new Set<string>();
  function pan(right: number, forward: number) {
    clearHover();
    view.pan(right, forward);
  }
  document.addEventListener('keydown', (event) => {
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.target === timeSlider && event.key.startsWith('Arrow')) return;
    bell.unlock();
    if (event.key === 'Escape' && testing) {
      setPlaytest(false);
      return;
    }
    const key = event.key.toLowerCase();
    const direction = panKeys.get(key);
    if (direction) {
      event.preventDefault();
      pressed.add(key);
      if (motion.matches || !event.repeat) pan(direction[0] * 0.65, direction[1] * 0.65);
      return;
    }
    const control = keys.get(key);
    if (!control) return;
    event.preventDefault();
    clearHover();
    view.control(control);
  }, { signal: view.signal });
  document.addEventListener('keyup', (event) => pressed.delete(event.key.toLowerCase()), { signal: view.signal });
  window.addEventListener('blur', () => pressed.clear(), { signal: view.signal });

  status.hidden = true;
  canvas.hidden = false;
  controls.hidden = false;
  view.start({
    onHover(x, y) {
      if (!testing) return;
      hoverPoint.set(x, y);
      hoverPending = true;
      view.request();
    },
    onLeave: clearHover,
    onDrag: clearHover,
    onSuspend() {
      pressed.clear();
      hoverPending = false;
    },
    onTap(x, y) {
      if (!testing) return;
      const result = playtest.pick(camera, x, y, canvas.getBoundingClientRect());
      if (result === undefined) return;
      hoverPending = false;
      if (result === 'walking') movementStatus.value = 'Walking…';
      else showRange(result);
      view.request();
    },
    beforeRender(now, delta) {
      if (timeChange) {
        const progress = motion.matches ? 1 : T.MathUtils.clamp((now - timeChange.start) / 440, 0, 1);
        minute = T.MathUtils.lerp(timeChange.from, timeChange.to, 1 - (1 - progress) ** 3);
        if (progress === 1) timeChange = null;
        const time = sampleCityTime(minute);
        night = time.night;
        lighting.setTime(time);
        city.setNightLight(night);
        water.setTime(time, lighting.keyColor);
        view.refreshShadows();
      }
      if (pressed.size && delta) {
        const right = Number(pressed.has('d')) - Number(pressed.has('a'));
        const forward = Number(pressed.has('w')) - Number(pressed.has('s'));
        const length = Math.hypot(right, forward);
        if (length) pan(right * delta * 14 / length, forward * delta * 14 / length);
      }
      const previous = water.time.value;
      water.time.value += delta;
      const bellRung = rainbellStrikeBetween(previous, water.time.value);
      if (bellRung) bell.ring();
      city.updateRainbell();
      const { gaugeLit: nextGauge, frontDoor, rearDoor } = updateCharacters(water.time.value, view.yaw, bellRung, night);
      city.updateOperaDoors(frontDoor, rearDoor);
      if (nextGauge !== gaugeLit) {
        gaugeLit = nextGauge;
        city.setGaugeSignal(gaugeLit);
        water.invalidate();
      }
      playtest.root.visible = false;
      try {
        water.capture(camera);
      } finally {
        playtest.root.visible = testing;
      }
      if (testing) {
        const count = playtest.update(camera, delta, motion.matches);
        if (count !== undefined) showRange(count);
        if (hoverPending) {
          hoverPending = false;
          playtest.hover(camera, hoverPoint.x, hoverPoint.y, canvas.getBoundingClientRect());
        }
      }
      if (!motion.matches) view.request();
    },
    render(zoom) {
      if (tiltShift && !testing) tiltShift.render(scene, camera, zoom, orbitRadius);
      else renderer.render(scene, camera);
    },
    onContextChange(lost) {
      status.hidden = !lost;
      controls.hidden = lost;
      if (lost) status.textContent = 'Waiting for the graphics connection…';
    },
    onInvalidate: water.invalidate,
    onDispose() {
      lighting.dispose();
      water.dispose();
      tiltShift?.dispose();
    },
  });
}
