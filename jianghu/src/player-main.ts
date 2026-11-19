import { loadAgentArtwork } from './agent-sprites.ts';
import { createAgentTraffic } from './agent-traffic.ts';
import { createDistrict, districtCast } from './district.ts';
import { mountMapScene } from './map-scene.ts';
import type { CameraControl } from './scene-view.ts';
import { requiredElement } from './dom.ts';

const root = requiredElement(document, '#jianghu', HTMLElement);
const canvas = requiredElement(root, 'canvas', HTMLCanvasElement);
const poster = requiredElement(root, 'img', HTMLImageElement);

function showPoster(visible: boolean) {
  canvas.hidden = visible;
  poster.hidden = !visible;
}

let scene: ReturnType<typeof mountMapScene> | undefined;
let closed = false;
window.addEventListener('pagehide', (event) => {
  if (event.persisted) return;
  closed = true;
  scene?.dispose();
});

Promise.all([
  import('three'),
  loadAgentArtwork(root.dataset.spriteSheet),
])
  .then(([three, artwork]) => {
    if (closed) return;
    const district = createDistrict();
    const traffic = createAgentTraffic(district, districtCast);
    const mounted = mountMapScene(three, {
      canvas,
      cast: districtCast,
      artwork,
      district,
      traffic,
      unavailable: showPoster,
      animateWater: true,
      initialZoom: 0.91,
    });
    scene = mounted;
    showPoster(false);
    const keys = new Map<string, CameraControl>([
      ['q', 'left'],
      ['e', 'right'],
      ['+', 'in'],
      ['=', 'in'],
      ['-', 'out'],
      ['r', 'reset'],
    ]);
    document.addEventListener('keydown', (event) => {
      if (event.ctrlKey || event.metaKey || event.altKey) return;
      const control = keys.get(event.key.toLowerCase());
      if (control) {
        event.preventDefault();
        mounted.control(control);
      }
    }, { signal: mounted.signal });
  })
  .catch((error) => {
    showPoster(true);
    console.warn('Jianghu Waterworks initialization', error);
  });
