import type * as Three from 'three';
import { brushes, createDistrict, type LandmarkId } from './district.ts';
import { createDistrictView } from './district-view.ts';
import { isCameraControl } from './scene-view.ts';
import { requiredElement } from './dom.ts';
import type { Coordinates } from './world.ts';
import { waterLayer } from './water-material.ts';

type BrushMode = 'orbit' | 'place' | 'remove';

function isBrush(value: string): value is keyof typeof brushes {
  return Object.hasOwn(brushes, value);
}

export function mountSandbox(T: typeof Three, root: HTMLElement) {
  const find = (selector: string) => requiredElement(root, selector, HTMLElement);
  const stage = find('.viewport');
  const canvas = requiredElement(root, 'canvas', HTMLCanvasElement);
  const fallback = find('.fallback');
  const note = find('.fallback-note');
  const tools = find('.tools');
  const cameraControls = find('.camera-controls');
  const environment = find('.environment');
  const waterButton = find('[data-water]');
  const duskButton = find('[data-dusk]');
  const gateButton = find('[data-gate]');
  const spillwayButton = find('[data-camera="spillway"]');
  const materialMenu = requiredElement(root, '[data-material]', HTMLSelectElement);
  const undoButton = requiredElement(root, '[data-undo]', HTMLButtonElement);
  const aimButton = find('[data-aim]');
  const crosshair = find('.crosshair');
  const message = find('.edit-status');
  const measurements = find('.measurements');
  const landmarkControls = find('.landmarks [role=group]');
  const landmarkTitle = find('.landmark-title');
  const landmarkCopy = find('.landmark-copy');
  const pins = [...stage.querySelectorAll<HTMLElement>('[data-landmark]')];
  const landmarkButtons = [...root.querySelectorAll<HTMLElement>('[data-landmark]')];
  const modeButtons = [...tools.querySelectorAll<HTMLElement>('[data-mode]')];

  const district = createDistrict();
  const { world } = district;
  const view = createDistrictView(T, canvas, district, {
    viewport: stage,
    exposure: 1.12,
    shadows: true,
    orbit: {
      target: [0.28, 1.18, 0.22], yaw: 0.7, pitch: 0.54,
      width: 16.6, radius: 28, turn: Math.PI / 9, zoomFactor: 1.2,
      minPitch: 0.17, maxPitch: 1.19, minZoom: 0.65, maxZoom: 2.7,
    },
  });
  const { scene, camera, terrain, architecture, landmarks, water, waterScene, motion, signal } = view;
  const onClick = (node: HTMLElement, fn: () => void) => node.addEventListener('click', fn, { signal });
  const isLandmark = (id: string | undefined): id is LandmarkId =>
    id !== undefined && Object.hasOwn(landmarks, id);
  const hemi = new T.HemisphereLight();
  const sun = new T.DirectionalLight();
  sun.castShadow = true;
  Object.assign(sun.shadow.camera, { left: -11, right: 11, top: 11, bottom: -11, far: 38 });
  sun.shadow.mapSize.set(1536, 1536);
  sun.shadow.normalBias = 0.036;
  sun.shadow.bias = -0.0003;
  const fill = new T.DirectionalLight('#8faec4', 0.16);
  fill.position.set(6, 8, -6);
  const lanterns = [
    [-3.6, 1.82, 2.23],
    [-2.85, 2.27, -2.89],
    [4.05, 1.84, 1.04],
    [0, 2.2, -1.19],
  ].map(([x, y, z]) => {
    const light = new T.PointLight('#ffbc72', 0, 5.8);
    light.position.set(x, y, z);
    return light;
  });
  scene.add(hemi, sun, fill, ...lanterns);

  const occluders = [architecture, waterScene.sluice];

  const ray = new T.Raycaster();
  const aim = new T.Vector2();
  const cursorSize = world.cell * 1.025;
  const cursorBox = new T.BoxGeometry(cursorSize, cursorSize, cursorSize);
  const edges = new T.EdgesGeometry(cursorBox);
  cursorBox.dispose();
  const wire = new T.LineSegments(edges, new T.LineBasicMaterial({ color: '#ffe6aa' }));
  wire.visible = false;
  wire.layers.set(waterLayer);
  scene.add(wire);

  const history: [voxel: Coordinates, material: number][] = [];
  let mode: BrushMode = 'orbit';
  let paused = motion.matches;
  let measuredAt = -0.4;

  function placePins() {
    for (const pin of pins) {
      const id = pin.dataset.landmark;
      if (!isLandmark(id)) continue;
      const { x, y, width, height } = view.project(landmarks[id].anchor);
      pin.hidden = x < 22 || x > width - 22 || y < 22 || y > height - 22;
      pin.style.left = x + 'px';
      pin.style.top = y + 'px';
    }
  }

  function measure() {
    const { upstream, discharge } = water.measure();
    measurements.textContent = `Upstream ${upstream.toFixed(2)} m · Spillway ${Math.round(discharge)} L/s`;
  }

  function selectLandmark(id: LandmarkId) {
    const { title, copy } = landmarks[id];
    landmarkTitle.textContent = title;
    landmarkCopy.textContent = copy;
    for (const button of landmarkButtons) {
      button.setAttribute('aria-pressed', String(button.dataset.landmark === id));
    }
    for (const [key, landmark] of Object.entries(landmarks)) {
      landmark.ring.visible = landmark.marker.visible = key === id;
    }
    view.invalidate();
  }
  for (const button of landmarkButtons) {
    onClick(button, () => {
      const id = button.dataset.landmark;
      if (isLandmark(id)) selectLandmark(id);
    });
  }

  function pickVoxel(clientX: number, clientY: number, placing: boolean): Coordinates | null {
    const bounds = canvas.getBoundingClientRect();
    aim.set(
      ((clientX - bounds.left) / bounds.width) * 2 - 1,
      1 - ((clientY - bounds.top) / bounds.height) * 2,
    );
    ray.setFromCamera(aim, camera);
    const hit = ray.intersectObjects(terrain.meshes, false)[0];
    if (!hit?.face) return null;
    const obstruction = ray.intersectObjects(occluders, true)[0];
    if (obstruction && obstruction.distance < hit.distance - 0.015) return null;
    const normal = hit.face.normal;
    const bias = placing ? 0.015 : -0.015;
    const { point } = hit;
    const { cell, origin: [ox, oy, oz] } = world;
    const voxel: Coordinates = [
      Math.floor((point.x + normal.x * bias - ox) / cell),
      Math.floor((point.y + normal.y * bias - oy) / cell),
      Math.floor((point.z + normal.z * bias - oz) / cell),
    ];
    if (!world.inside(...voxel) || Boolean(world.get(...voxel)) === placing) return null;
    return voxel;
  }
  function previewEdit(x: number, y: number) {
    const voxel = pickVoxel(x, y, mode === 'place');
    wire.visible = Boolean(voxel);
    if (voxel) {
      const { cell, origin: [ox, oy, oz] } = world;
      wire.position.set(
        ox + (voxel[0] + 0.5) * cell,
        oy + (voxel[1] + 0.5) * cell,
        oz + (voxel[2] + 0.5) * cell,
      );
    }
    view.request();
  }
  function updateVoxel(voxel: Coordinates, material: number) {
    terrain.rebuild(world.set(...voxel, material));
    waterScene.rebuildSurface();
    view.refreshShadows();
  }
  function edit(x: number, y: number) {
    if (mode === 'orbit') return;
    const voxel = pickVoxel(x, y, mode === 'place');
    if (!voxel) {
      message.textContent = 'Aim at an exposed terrain block.';
      return;
    }
    let material = 0;
    if (mode === 'place') {
      const brush = materialMenu.value;
      if (!isBrush(brush)) return;
      material = brushes[brush];
    }
    history.push([voxel, world.get(...voxel)]);
    updateVoxel(voxel, material);
    undoButton.disabled = false;
    message.textContent =
      mode === 'place'
        ? `${materialMenu.selectedOptions[0].textContent} block placed.`
        : 'Terrain block removed.';
    previewEdit(x, y);
  }
  for (const button of modeButtons) {
    onClick(button, () => {
      const next = button.dataset.mode;
      if (next !== 'orbit' && next !== 'place' && next !== 'remove') return;
      mode = next;
      for (const item of modeButtons) item.setAttribute('aria-pressed', String(item === button));
      canvas.classList.toggle('is-editing', mode !== 'orbit');
      crosshair.hidden = aimButton.hidden = mode === 'orbit';
      wire.visible = false;
      message.textContent =
        mode === 'orbit'
          ? 'Drag to orbit. Use Place or Remove to edit the terrain.'
          : mode === 'place'
            ? 'Select a block face to place your material.'
            : 'Select a terrain block to remove it.';
      view.request();
    });
  }
  onClick(undoButton, () => {
    const entry = history.pop();
    if (!entry) return;
    const [voxel, material] = entry;
    updateVoxel(voxel, material);
    wire.visible = false;
    undoButton.disabled = !history.length;
    message.textContent = 'Last terrain edit undone.';
  });
  onClick(aimButton, () => {
    const bounds = canvas.getBoundingClientRect();
    edit(bounds.left + bounds.width / 2, bounds.top + bounds.height / 2);
  });

  function setLighting() {
    const dusk = duskButton.getAttribute('aria-pressed') === 'true';
    sun.position.set(dusk ? -10 : -8, dusk ? 6 : 15, dusk ? -7 : 7);
    sun.color.set(dusk ? '#ffa575' : '#ffe9cb');
    sun.intensity = dusk ? 0.66 : 2.85;
    hemi.color.set(dusk ? '#96b5f2' : '#e0eade');
    hemi.groundColor.set(dusk ? '#253c5e' : '#374e48');
    hemi.intensity = dusk ? 0.67 : 1.35;
    fill.intensity = dusk ? 0.43 : 0.16;
    for (const lantern of lanterns) lantern.intensity = dusk ? 11 : 0;
    waterScene.setLighting(dusk, sun);
    view.refreshShadows();
  }
  onClick(duskButton, () => {
    duskButton.setAttribute('aria-pressed', String(duskButton.getAttribute('aria-pressed') !== 'true'));
    setLighting();
  });
  onClick(gateButton, () => {
    const closed = !water.gateClosed;
    water.setGate(closed);
    waterScene.setGate(closed);
    gateButton.textContent = closed ? 'Open sluice' : 'Close sluice';
    message.textContent = closed
      ? 'Sluice closed. Water gathers upstream.'
      : 'Sluice open. Water can flow through the gate.';
    view.refreshShadows();
  });
  function setMotion() {
    waterButton.textContent = paused ? 'Run water' : 'Pause water';
    waterButton.toggleAttribute('data-paused', paused);
    view.resetClock();
    view.request();
  }
  onClick(waterButton, () => {
    paused = !paused;
    setMotion();
  });
  motion.addEventListener('change', (event) => {
    paused = event.matches;
    setMotion();
  }, { signal });

  for (const button of cameraControls.querySelectorAll<HTMLElement>('[data-camera]')) {
    onClick(button, () => {
      if (button === spillwayButton) view.setView([0, -0.42, 4.22], 0.42, 0.37, 1.9);
      else if (isCameraControl(button.dataset.camera)) view.control(button.dataset.camera);
      spillwayButton.setAttribute('aria-pressed', String(view.targetZ > 3));
      wire.visible = false;
    });
  }

  function showGraphics(available: boolean) {
    canvas.hidden =
      tools.hidden =
      cameraControls.hidden =
      environment.hidden =
      landmarkControls.hidden =
        !available;
    fallback.hidden = note.hidden = available;
    if (!available) {
      for (const pin of pins) pin.hidden = true;
    }
  }
  measure();
  selectLandmark('west');
  setLighting();
  setMotion();
  showGraphics(true);
  view.start({
    beforeRender(now) {
      view.advanceWater(now, !paused);
      if (paused) return;
      if (view.waterTime - measuredAt >= 0.32) {
        measuredAt = view.waterTime;
        measure();
      }
      view.request(25);
    },
    afterRender: placePins,
    onTap: edit,
    onHover(x, y) {
      if (mode !== 'orbit') previewEdit(x, y);
    },
    onDrag() {
      wire.visible = false;
    },
    onLeave() {
      if (!wire.visible) return;
      wire.visible = false;
      view.request();
    },
    onContextChange(lost) {
      if (lost) note.textContent = 'Graphics connection interrupted. Showing the district illustration.';
      showGraphics(!lost);
    },
  });
  return view.dispose;
}
