import type * as Three from 'three';
import type { createSceneView } from './scene-view.ts';

type Stop = { target: readonly [number, number, number]; yaw: number; pitch: number; zoom: number };
type Chapter = { stop: Stop; center: number; side: 'left' | 'right'; hero?: boolean };

export const mewOverview = {
  target: [0, 1.875, 0], yaw: Math.PI / 4, pitch: Math.atan2(0.3, Math.SQRT1_2), zoom: 0.92,
} satisfies Stop;
export const mewCompactQuery = '(max-width: 860px)';
export const mewTiltShift = { wideAperture: 0.28, closeupZoom: 2.5, maxBlurRadius: 6.4, blurSamples: 8, multisample: false };

const clamp = (n: number) => Math.min(1, Math.max(0, n));

export function frameMewStory(first: Chapter, last: Chapter, center: number, compact: boolean, reduced: boolean) {
  const fraction = first === last ? 0 : clamp((center - first.center) / (last.center - first.center));
  const progress = reduced ? Number(fraction >= 0.5) : clamp((fraction - 0.25) / 0.5);
  const eased = progress * progress * (3 - 2 * progress);
  const between = (a: number, b: number) => a + (b - a) * eased;
  const a = first.stop;
  const b = last.stop;
  const angle = b.yaw - a.yaw;
  const yaw = a.yaw + Math.atan2(Math.sin(angle), Math.cos(angle)) * eased;
  const shiftX = (chapter: Chapter) => compact ? 0 : chapter.hero ? 0.14 : chapter.side === 'right' ? -0.22 : 0.22;
  const shiftY = (chapter: Chapter) => compact ? (chapter.hero ? -0.20 : -0.24) : chapter.hero ? -0.10 : 0;
  const zoom = (chapter: Chapter) => chapter.stop.zoom * (compact && chapter.hero ? 1.3 : 1);
  return {
    target: [between(a.target[0], b.target[0]), between(a.target[1], b.target[1]), between(a.target[2], b.target[2])] as const,
    yaw,
    pitch: between(a.pitch, b.pitch),
    zoom: Math.exp(between(Math.log(zoom(first)), Math.log(zoom(last)))) *
      (reduced ? 1 : 1 - 0.28 * Math.sin(Math.PI * eased)),
    shiftX: between(shiftX(first), shiftX(last)),
    shiftY: between(shiftY(first), shiftY(last)),
  };
}

export function mountMewStory(
  main: HTMLElement, view: ReturnType<typeof createSceneView>,
  stops: Record<string, Stop>, toolbar: HTMLElement,
) {
  const compact = matchMedia(mewCompactQuery);
  const sections = [...main.querySelectorAll<HTMLElement>('section[data-focus]')]
    .flatMap<Chapter & { section: HTMLElement; panel: HTMLElement | null; top: number; height: number }>((section) => {
      const hero = section.dataset.focus === 'garden';
      const side = section.dataset.side === 'right' ? 'right' : 'left';
      const stop = hero ? mewOverview : stops[section.dataset.focus ?? ''];
      return stop ? [{
        section, panel: section.querySelector('.panel, .intro'), hero, side, stop, center: 0, top: 0, height: 0,
      }] : [];
    });
  if (!sections.length) return;
  const leftWash = document.querySelector<HTMLElement>('.wash.left');
  const rightWash = document.querySelector<HTMLElement>('.wash.right');
  const tilt = toolbar.querySelector<HTMLButtonElement>('[data-tilt]');
  let steeper = false;
  let baseYaw = mewOverview.yaw;
  let chapterPitch = mewOverview.pitch;
  let height = 0;

  function frame() {
    const scroll = window.scrollY;
    const center = scroll + height / 2;
    let index = 0;
    let left = 0;
    let right = 0;
    for (const [i, chapter] of sections.entries()) {
      if (chapter.center <= center) index = i;
      const panel = chapter.panel;
      if (!panel) continue;
      const top = chapter.top - scroll;
      let opacity = '';
      if (compact.matches && !chapter.hero) {
        const visibility = clamp((height * 0.6 - top) / (height * 0.22));
        opacity = String(view.motion.matches ? 1 : 0.04 + 0.96 * visibility ** 2);
      }
      if (panel.style.opacity !== opacity) panel.style.opacity = opacity;
      if (!chapter.height) continue;
      const visible = Math.max(0, Math.min(top + chapter.height, height) - Math.max(top, 0));
      const wash = clamp(visible / Math.min(height, chapter.height) * 1.25);
      if (chapter.side === 'right') right = Math.max(right, wash);
      else left = Math.max(left, wash);
    }
    const leftOpacity = String(compact.matches ? 1 : left);
    const rightOpacity = String(compact.matches ? 0 : right);
    if (leftWash && leftWash.style.opacity !== leftOpacity) leftWash.style.opacity = leftOpacity;
    if (rightWash && rightWash.style.opacity !== rightOpacity) rightWash.style.opacity = rightOpacity;

    const composed = frameMewStory(sections[index], sections[Math.min(index + 1, sections.length - 1)],
      center, compact.matches, view.motion.matches);
    const userYaw = view.yaw - baseYaw;
    baseYaw = composed.yaw;
    chapterPitch = composed.pitch;
    view.setView(composed.target, composed.yaw + userYaw, steeper ? 0.62 : composed.pitch, composed.zoom,
      [composed.shiftX, composed.shiftY]);
  }

  function measure() {
    height = window.innerHeight;
    const scroll = window.scrollY;
    for (const chapter of sections) {
      const rect = chapter.section.getBoundingClientRect();
      chapter.center = scroll + rect.top + rect.height / 2;
      const panel = chapter.panel?.getBoundingClientRect();
      if (panel) {
        chapter.top = scroll + panel.top;
        chapter.height = panel.height;
      }
    }
    frame();
  }

  const { signal } = view;
  const observer = new ResizeObserver(measure);
  observer.observe(main);
  for (const { section, panel } of sections) {
    observer.observe(section);
    if (panel) observer.observe(panel);
  }
  window.addEventListener('scroll', frame, { passive: true, signal });
  window.addEventListener('resize', measure, { signal });
  view.motion.addEventListener('change', frame, { signal });
  signal.addEventListener('abort', () => {
    observer.disconnect();
    for (const { panel } of sections) panel?.style.removeProperty('opacity');
    leftWash?.style.removeProperty('opacity');
    rightWash?.style.removeProperty('opacity');
  }, { once: true });
  tilt?.addEventListener('click', () => {
    steeper = !steeper;
    tilt.setAttribute('aria-pressed', String(steeper));
    const { target, yaw, zoom } = view.cameraState();
    view.move(target, zoom, { yaw, pitch: steeper ? 0.62 : chapterPitch });
  }, { signal });
  measure();
}


const characterNotes = new Map([
  ['daemon', 'owns the session and runs one turn at a time.'],
  ['agent', 'streams the model, runs tools in order, loops until done.'],
  ['provider', 'one trait, with openai, anthropic and responses adapters.'],
  ['tui', 'ratatui, attached to a loopback daemon.'],
  ['web', 'reaches the daemon through mew-web-bridge.'],
  ['desktop', 'gpui. finds a daemon on :25566, or starts its own.'],
  ['mobile', 'a uniffi core that speaks over iroh.'],
]);

export function mountMewLabels(
  T: typeof Three, view: ReturnType<typeof createSceneView>, characters: readonly { sprite: Three.Object3D }[],
) {
  const label = document.querySelector<HTMLElement>('#character-label');
  const name = label?.querySelector('.name');
  const line = label?.querySelector('.line');
  if (!label || !name || !line) return;
  const anchor = new T.Vector3();
  let selected: Three.Object3D | undefined;

  function close() {
    selected = undefined;
    label!.hidden = true;
  }

  window.addEventListener('keydown', (event) => {
    if (event.key === 'Escape') close();
  }, { signal: view.signal });
  view.signal.addEventListener('abort', close, { once: true });

  return {
    onTap(x: number, y: number) {
      let distance = Math.max(22, 18 * view.cameraState().zoom);
      selected = undefined;
      for (const { sprite } of characters) {
        const point = view.project(anchor.copy(sprite.position).addScaledVector(view.camera.up, 0.64));
        const error = Math.hypot(point.x - x, point.y - y);
        if (error < distance) {
          distance = error;
          selected = sprite;
        }
      }
      if (!selected) {
        close();
        return;
      }
      name.textContent = selected.name;
      line.textContent = characterNotes.get(selected.name) ?? '';
      label.hidden = false;
      view.request();
    },
    onDrag: close,
    afterRender() {
      if (!selected) return;
      const { x, y, width, height } = view.project(anchor.copy(selected.position).addScaledVector(view.camera.up, 1.55));
      label.hidden = x < 0 || x > width || y < 0 || y > height;
      label.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px) translate(-50%, -100%)`;
    },
  };
}
