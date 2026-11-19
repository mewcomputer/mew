import type * as Three from 'three';
import type { Coordinates } from './world.ts';

const cameraControls = ['left', 'right', 'in', 'out', 'reset'] as const;
export type CameraControl = (typeof cameraControls)[number];

export function isCameraControl(name: string | undefined): name is CameraControl {
  return cameraControls.some((control) => control === name);
}

type Limits = readonly [number, number];
type OrbitOptions = {
  target: Readonly<Coordinates>;
  yaw: number;
  pitch: number;
  zoom?: number;
  width: number;
  height: number;
  radius: number;
  turn: number;
  zoomFactor: number;
  minPitch: number;
  maxPitch: number;
  minZoom: number;
  maxZoom: number;
  yawLimits?: Limits;
  pan?: { x: Limits; z: Limits };
  snapReset?: boolean;
  wheelZoom?: boolean;
  drag?: { threshold: number; yaw: number; pitch: number };
};

export type SceneViewOptions = {
  viewport?: HTMLElement;
  shadows?: boolean;
  exposure: number;
  alpha?: boolean;
  far?: number;
  maxPixelRatio?: number;
  orbit: OrbitOptions;
};

export type SceneHooks = {
  beforeRender?: (now: number, delta: number) => void;
  render?: (zoom: number) => void;
  afterRender?: () => void;
  onInvalidate?: () => void;
  onSuspend?: () => void;
  onDispose?: () => void;
  onContextChange?: (lost: boolean) => void;
  onHover?: (x: number, y: number) => void;
  onTap?: (x: number, y: number) => void;
  onDrag?: () => void;
  onLeave?: () => void;
};

type OrbitRotation = Pick<OrbitOptions, 'yaw' | 'pitch'>;

type CameraTransition = {
  from: Three.Vector3;
  to: Three.Vector3;
  fromZoom: number;
  toZoom: number;
  fromYaw: number;
  yawDelta: number;
  toYaw: number;
  fromPitch: number;
  toPitch: number;
  start: number;
};

export function createOrbitCamera(T: typeof Three, camera: Three.OrthographicCamera, orbit: OrbitOptions) {
  const target = new T.Vector3(...orbit.target);
  const initialZoom = orbit.zoom ?? 1;
  let yaw = orbit.yaw;
  let pitch = orbit.pitch;
  let zoom = initialZoom;
  let transition: CameraTransition | null = null;
  let shiftX = 0;
  let shiftY = 0;

  function setYaw(value: number) {
    yaw = orbit.yawLimits ? T.MathUtils.clamp(value, ...orbit.yawLimits) : value;
  }

  function zoomBy(factor: number) {
    transition = null;
    zoom = T.MathUtils.clamp(zoom * factor, orbit.minZoom, orbit.maxZoom);
  }

  function setView(nextTarget: Readonly<Coordinates>, nextYaw: number, nextPitch: number, nextZoom: number) {
    transition = null;
    target.set(...nextTarget);
    setYaw(nextYaw);
    pitch = nextPitch;
    zoom = nextZoom;
  }

  function control(name: CameraControl) {
    transition = null;
    switch (name) {
      case 'left': setYaw(yaw - orbit.turn); break;
      case 'right': setYaw(yaw + orbit.turn); break;
      case 'in': zoomBy(orbit.zoomFactor); break;
      case 'out': zoomBy(1 / orbit.zoomFactor); break;
      case 'reset': {
        const quadrant = orbit.snapReset ? Math.round((yaw - orbit.yaw) / (Math.PI / 2)) : 0;
        setView(orbit.target, orbit.yaw + quadrant * Math.PI / 2, orbit.pitch, initialZoom);
        break;
      }
    }
  }

  function update(width: number, height: number, now: number, reducedMotion: boolean) {
    if (transition) {
      const t = reducedMotion ? 1 : T.MathUtils.clamp((now - transition.start) / 340, 0, 1);
      const eased = 1 - (1 - t) ** 3;
      target.lerpVectors(transition.from, transition.to, eased);
      zoom = T.MathUtils.lerp(transition.fromZoom, transition.toZoom, eased);
      yaw = t === 1 ? transition.toYaw : transition.fromYaw + transition.yawDelta * eased;
      pitch = T.MathUtils.lerp(transition.fromPitch, transition.toPitch, eased);
      if (t === 1) transition = null;
    }
    const ratio = width / height;
    const field = Math.max(orbit.height, orbit.width / ratio) * 0.5 / zoom;
    const widthRadius = field * ratio;
    Object.assign(camera, {
      left: -widthRadius - 2 * widthRadius * shiftX, right: widthRadius - 2 * widthRadius * shiftX,
      top: field - 2 * field * shiftY, bottom: -field - 2 * field * shiftY,
    });
    camera.updateProjectionMatrix();
    const distance = Math.cos(pitch) * orbit.radius;
    camera.position.set(
      target.x + Math.sin(yaw) * distance,
      target.y + Math.sin(pitch) * orbit.radius,
      target.z + Math.cos(yaw) * distance,
    );
    camera.lookAt(target);
    camera.updateMatrixWorld();
  }

  function pan(right: number, forward: number) {
    if (!orbit.pan) return false;
    transition = null;
    const x = T.MathUtils.clamp(target.x + right * Math.cos(yaw) - forward * Math.sin(yaw), ...orbit.pan.x);
    const z = T.MathUtils.clamp(target.z - right * Math.sin(yaw) - forward * Math.cos(yaw), ...orbit.pan.z);
    const moved = x !== target.x || z !== target.z;
    target.x = x;
    target.z = z;
    return moved;
  }

  return {
    control, pan, setView, update, zoomBy,
    setFrame(x: number, y: number) {
      shiftX = x;
      shiftY = y;
    },
    get yaw() { return yaw; },
    get zoom() { return zoom; },
    get targetZ() { return target.z; },
    get transitioning() { return transition !== null; },
    cancelTransition() {
      transition = null;
    },
    drag(dx: number, dy: number, height: number, panning: boolean) {
      if (panning && orbit.pan && height) {
        const scale = (camera.top - camera.bottom) / height;
        pan(-dx * scale, dy * scale / Math.max(0.45, Math.sin(pitch)));
      } else {
        setYaw(yaw - dx * (orbit.drag?.yaw ?? 0.009));
        pitch = T.MathUtils.clamp(pitch + dy * (orbit.drag?.pitch ?? 0.006), orbit.minPitch, orbit.maxPitch);
      }
    },
    snapshot() {
      return { target: target.clone(), zoom, yaw, pitch };
    },
    move(to: Three.Vector3, toZoom: number, now: number, instant: boolean, rotation?: OrbitRotation) {
      const requestedYaw = rotation?.yaw ?? yaw;
      const toYaw = orbit.yawLimits ? T.MathUtils.clamp(requestedYaw, ...orbit.yawLimits) : requestedYaw;
      const toPitch = rotation ? T.MathUtils.clamp(rotation.pitch, orbit.minPitch, orbit.maxPitch) : pitch;
      if (instant) {
        transition = null;
        target.copy(to);
        zoom = toZoom;
        yaw = toYaw;
        pitch = toPitch;
      } else {
        const delta = toYaw - yaw;
        // An unrestricted camera can cross the yaw seam. A bounded camera has to stay inside its arc.
        const yawDelta = orbit.yawLimits || Math.abs(delta) <= Math.PI
          ? delta : T.MathUtils.euclideanModulo(delta + Math.PI, 2 * Math.PI) - Math.PI;
        transition = {
          from: target.clone(), to, fromZoom: zoom, toZoom, fromYaw: yaw, yawDelta, toYaw,
          fromPitch: pitch, toPitch, start: now,
        };
      }
    },
  };
}

export function createSceneView(T: typeof Three, canvas: HTMLCanvasElement, options: SceneViewOptions) {
  const { viewport: container = canvas.parentElement, shadows = false, exposure, alpha = true, far = 100, orbit } = options;
  if (!container) throw new Error('The scene canvas requires a viewport.');
  const viewport = container;
  const renderer = new T.WebGLRenderer({ canvas, alpha, antialias: true });
  renderer.setPixelRatio(Math.min(devicePixelRatio || 1, options.maxPixelRatio ?? 1.5));
  renderer.toneMapping = T.ACESFilmicToneMapping;
  renderer.toneMappingExposure = exposure;
  renderer.shadowMap.enabled = shadows;
  renderer.shadowMap.type = T.PCFShadowMap;
  renderer.shadowMap.autoUpdate = false;
  renderer.shadowMap.needsUpdate = shadows;

  const scene = new T.Scene();
  const camera = new T.OrthographicCamera(-10, 10, 7, -7, 0.1, far);
  const rig = createOrbitCamera(T, camera, orbit);
  const abort = new AbortController();
  const { signal } = abort;
  const motion = matchMedia('(prefers-reduced-motion: reduce)');
  const projected = new T.Vector3();
  const screen = { x: 0, y: 0, width: 0, height: 0 };
  let frame = 0;
  let timer = 0;
  let due = Infinity;
  let visible = true;
  let lost = false;
  let lastFrame: number | null = null;
  let drag: { id: number; x: number; y: number; moved: boolean; pan: boolean } | null = null;
  let callbacks: SceneHooks | null = null;

  function runnable() {
    return callbacks && !signal.aborted && !document.hidden && visible && !lost;
  }

  function cancelDraw() {
    cancelAnimationFrame(frame);
    clearTimeout(timer);
    frame = timer = 0;
    due = Infinity;
    lastFrame = null;
    callbacks?.onSuspend?.();
  }

  function request(delay = 0) {
    if (!runnable() || frame) return;
    if (delay) {
      const at = performance.now() + delay;
      if (at >= due) return;
      clearTimeout(timer);
      due = at;
      timer = window.setTimeout(() => {
        timer = 0;
        due = Infinity;
        request();
      }, delay);
      return;
    }
    clearTimeout(timer);
    timer = 0;
    due = Infinity;
    frame = requestAnimationFrame(draw);
  }

  function invalidate() {
    callbacks?.onInvalidate?.();
    request();
  }

  function draw(now: number) {
    frame = 0;
    const hooks = callbacks;
    if (!hooks || !runnable() || !screen.width || !screen.height) return;
    if (rig.transitioning) hooks.onInvalidate?.();
    rig.update(screen.width, screen.height, now, motion.matches);
    if (rig.transitioning) request();

    const delta = lastFrame === null || motion.matches ? 0 : Math.min(80, Math.max(0, now - lastFrame)) / 1000;
    lastFrame = now;
    hooks.beforeRender?.(now, delta);
    if (signal.aborted) return;
    if (hooks.render) hooks.render(rig.zoom);
    else renderer.render(scene, camera);
    hooks.afterRender?.();
  }

  function resize() {
    const bounds = viewport.getBoundingClientRect();
    screen.width = Math.round(bounds.width);
    screen.height = Math.round(bounds.height);
    if (screen.width && screen.height) renderer.setSize(screen.width, screen.height, false);
    invalidate();
  }

  const resizeObserver = new ResizeObserver(resize);
  const visibilityObserver = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    if (visible) request();
    else cancelDraw();
  });

  function control(name: CameraControl) {
    rig.control(name);
    invalidate();
  }

  function start(hooks: SceneHooks) {
    if (callbacks || signal.aborted) return;
    callbacks = hooks;
    const listen = <K extends keyof HTMLElementEventMap>(
      name: K, fn: (event: HTMLElementEventMap[K]) => void,
    ) => canvas.addEventListener(name, fn, { signal });
    listen('pointerdown', (event) => {
      if (!event.isPrimary || (event.button !== 0 && event.button !== 2)) return;
      rig.cancelTransition();
      drag = { id: event.pointerId, x: event.clientX, y: event.clientY, moved: false, pan: event.shiftKey };
      canvas.setPointerCapture(event.pointerId);
    });
    listen('pointermove', (event) => {
      if (drag?.id !== event.pointerId) {
        if (event.pointerType !== 'touch') hooks.onHover?.(event.clientX, event.clientY);
        return;
      }
      const dx = event.clientX - drag.x;
      const dy = event.clientY - drag.y;
      if (!drag.moved && Math.hypot(dx, dy) < (orbit.drag?.threshold ?? 5)) return;
      drag.moved = true;
      drag.x = event.clientX;
      drag.y = event.clientY;
      rig.drag(dx, dy, screen.height, drag.pan);
      hooks.onDrag?.();
      invalidate();
    });
    listen('pointerup', (event) => {
      if (drag?.id !== event.pointerId) return;
      const { moved } = drag;
      drag = null;
      if (!moved && event.button === 0) hooks.onTap?.(event.clientX, event.clientY);
    });
    for (const name of ['pointercancel', 'lostpointercapture'] as const) {
      listen(name, (event) => {
        if (drag?.id === event.pointerId) drag = null;
      });
    }
    listen('pointerleave', () => hooks.onLeave?.());
    listen('contextmenu', (event) => event.preventDefault());
    if (orbit.snapReset) listen('dblclick', () => control('reset'));
    if (orbit.wheelZoom) listen('wheel', (event) => {
      rig.zoomBy(Math.exp(-event.deltaY * 0.0012));
      invalidate();
    });
    canvas.addEventListener('webglcontextlost', (event) => {
      event.preventDefault();
      lost = true;
      cancelDraw();
      hooks.onContextChange?.(true);
    }, { signal });
    canvas.addEventListener('webglcontextrestored', () => {
      lost = false;
      renderer.shadowMap.needsUpdate = shadows;
      hooks.onContextChange?.(false);
      resize();
    }, { signal });
    document.addEventListener('visibilitychange', () => {
      if (document.hidden) cancelDraw();
      else request();
    }, { signal });
    motion.addEventListener('change', () => {
      lastFrame = null;
      request();
    }, { signal });
    resizeObserver.observe(viewport);
    visibilityObserver.observe(viewport);
    resize();
  }

  function dispose() {
    if (signal.aborted) return;
    abort.abort();
    cancelDraw();
    resizeObserver.disconnect();
    visibilityObserver.disconnect();
    if (drag && canvas.hasPointerCapture(drag.id)) canvas.releasePointerCapture(drag.id);
    const cleanup = callbacks?.onDispose;
    callbacks = drag = null;
    rig.cancelTransition();
    try {
      cleanup?.();
    } finally {
      disposeScene(T, scene);
      renderer.dispose();
    }
  }

  return {
    scene, camera, renderer, motion, signal, start, dispose, request, invalidate, control,
    pan(right: number, forward: number) {
      if (rig.pan(right, forward)) invalidate();
    },
    get yaw() { return rig.yaw; },
    get targetZ() { return rig.targetZ; },
    cameraState: rig.snapshot,
    setView(nextTarget: Readonly<Coordinates>, yaw: number, pitch: number, zoom: number,
      framing?: readonly [number, number]) {
      rig.setView(nextTarget, yaw, pitch, zoom);
      if (framing) rig.setFrame(...framing);
      invalidate();
    },
    refreshShadows() {
      renderer.shadowMap.needsUpdate = true;
      invalidate();
    },
    move(to: Three.Vector3, zoom: number, rotation?: OrbitRotation) {
      rig.move(to, zoom, performance.now(), motion.matches || document.hidden, rotation);
      invalidate();
    },
    project(point: Three.Vector3) {
      projected.copy(point).project(camera);
      screen.x = ((projected.x + 1) * screen.width) / 2;
      screen.y = ((1 - projected.y) * screen.height) / 2;
      return screen;
    },
  };
}

function disposeScene(T: typeof Three, scene: Three.Scene) {
  const resources = new Set<{ dispose(): void }>();
  scene.traverse((object) => {
    if (object instanceof T.InstancedMesh) resources.add(object);
    if (object instanceof T.DirectionalLight || object instanceof T.PointLight || object instanceof T.SpotLight) {
      resources.add(object.shadow);
    }
    if (object instanceof T.Mesh || object instanceof T.Line || object instanceof T.Points) {
      resources.add(object.geometry);
    }
    if (!(object instanceof T.Mesh || object instanceof T.Line ||
        object instanceof T.Points || object instanceof T.Sprite)) return;
    for (const material of Array.isArray(object.material) ? object.material : [object.material]) {
      if (resources.has(material)) continue;
      resources.add(material);
      const values: unknown[] = Object.values(material);
      for (const value of values) {
        if (value instanceof T.Texture) resources.add(value);
      }
    }
  });
  for (const resource of resources) resource.dispose();
  scene.clear();
}
