import { requiredElement } from './dom.ts';
import { mountSandbox } from './sandbox.ts';

const root = requiredElement(document, '#jianghu', HTMLElement);
const note = requiredElement(root, '.fallback-note', HTMLElement);

let dispose: (() => void) | undefined;
let closed = false;
window.addEventListener('pagehide', (event) => {
  if (event.persisted) return;
  closed = true;
  dispose?.();
});

import('three')
  .then((three) => {
    if (!closed) dispose = mountSandbox(three, root);
  })
  .catch((error) => {
    note.textContent = 'WebGL is unavailable. Showing an illustration of the district.';
    console.warn('Jianghu graphics initialization', error);
  });
