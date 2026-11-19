import { readFile, writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { build } from 'esbuild';
import { browserBuild, isPreviewMode, pages, project } from './pages.ts';

const output = process.argv[2];
const mode = process.argv[3] ?? 'player';

if (!output || !isPreviewMode(mode)) {
  console.error('Usage: npm run preview -- /path/to/jianghu.html [--map|--workshop|--city|--garden|--mew]');
  process.exitCode = 1;
} else {
  const selection = pages[mode];
  const usesGarden = selection.entry === 'garden-main';
  const standalone = mode === '--city' || usesGarden;
  const local = (path: string) => new URL(path, project);
  const [html, styles, poster, sheet, notice, garden, fonts, bundled] = await Promise.all([
    readFile(local(selection.html), 'utf8'),
    Promise.all(selection.css.map((path) => readFile(local(path), 'utf8'))),
    standalone ? null : readFile(local('assets/waterworks.webp')),
    selection.sprites ? readFile(local('vendor/tactics/a1.png')) : null,
    selection.sprites ? readFile(local('vendor/tactics/NOTICE.md'), 'utf8') : '',
    usesGarden ? readFile(local('assets/mew-diorama.glb')) : null,
    mode === '--mew' ? Promise.all([
      readFile(local('assets/sentient.woff2')),
      readFile(local('assets/instrument-sans.woff2')),
      readFile(local('assets/instrument-sans-OFL.txt'), 'utf8'),
    ]) : null,
    build({
      ...browserBuild,
      entryPoints: [`src/${selection.entry}.ts`],
      format: 'iife',
      minify: true,
      write: false,
    }),
  ]);
  const entry = `<script type="module" src="./generated/${selection.entry}.js"></script>`;
  const script = (notice ? '/*\n' + notice + '\n*/\n' : '') + bundled.outputFiles[0].text;
  const inline = `<script>\n${script}</script>\n`;
  const page = sheet
    ? html.replace('./vendor/tactics/a1.png', 'data:image/png;base64,' + sheet.toString('base64'))
    : html;
  if (standalone) {
    let markup = garden
      ? page.replace('./assets/mew-diorama.glb', 'data:model/gltf-binary;base64,' + garden.toString('base64'))
      : page;
    if (fonts) {
      const [sentient, instrumentSans, fontLicense] = fonts;
      const css = styles.join('\n')
        .replace(/^@import "\.\/mew-(?:palette|chrome)\.css";\n/gm, '')
        .replace('./assets/sentient.woff2', 'data:font/woff2;base64,' + sentient.toString('base64'))
        .replace('./assets/instrument-sans.woff2', 'data:font/woff2;base64,' + instrumentSans.toString('base64'));
      markup = markup.replace('<link rel="stylesheet" href="./mew.css">',
        () => `<!--\n${fontLicense}-->\n<style>\n${css}</style>`);
    }
    await writeFile(resolve(output), markup.replace(entry, () => inline));
  } else {
    const markup = page
      .slice(page.indexOf('  <main'), page.indexOf(entry))
      .replace(/\s*<a class="world-workshop"[^>]*>[^<]*<\/a>/, '')
      .replace('./assets/waterworks.webp', 'data:image/webp;base64,' + poster?.toString('base64'));
    const css = styles.join('\n').replace(/^body \{[^\n]*\}\n/m, '');
    await writeFile(resolve(output), `<style>\n${css}</style>\n${markup}${inline}`);
  }
}
