import { fileURLToPath } from 'node:url';
import type { BuildOptions } from 'esbuild';

export const project = new URL('../', import.meta.url);
export const root = fileURLToPath(project);

export const pages = {
  player: { html: 'index.html', css: ['player.css'], entry: 'player-main', sprites: true },
  '--map': {
    html: 'agents.html',
    css: ['styles.css', 'agent-map.css', 'tokens.css'],
    entry: 'map-main',
    sprites: true,
  },
  '--workshop': {
    html: 'workbench.html',
    css: ['styles.css', 'tokens.css'],
    entry: 'main',
    sprites: false,
  },
  '--city': { html: 'city.html', css: [], entry: 'city-main', sprites: true },
  '--mew': { html: 'mew.html', css: ['mew.css', 'mew-palette.css', 'mew-chrome.css'], entry: 'garden-main', sprites: true },
  '--garden': { html: 'garden.html', css: [], entry: 'garden-main', sprites: true },
} as const;

export function isPreviewMode(value: string): value is keyof typeof pages {
  return Object.hasOwn(pages, value);
}

export const browserBuild = {
  absWorkingDir: root,
  bundle: true,
  platform: 'browser',
  target: 'es2022',
  legalComments: 'eof',
} satisfies BuildOptions;
