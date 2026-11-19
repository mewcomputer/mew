import { copyFile, mkdir } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { build, context } from 'esbuild';
import { browserBuild, pages, root } from './pages.ts';

const entryPoints = [...new Set(Object.values(pages).map(({ entry }) => `src/${entry}.ts`))];

if (process.argv.includes('--serve')) {
  const browser = await context({
    ...browserBuild,
    entryPoints,
    format: 'esm',
    splitting: true,
    sourcemap: true,
    outdir: 'generated',
    write: false,
  });
  try {
    const { hosts, port } = await browser.serve({
      servedir: root,
      host: '127.0.0.1',
      port: Number(process.env.JIANGHU_PORT ?? 4173),
    });
    await browser.watch();
    console.log(`Jianghu running at http://${hosts[0]}:${port}`);
    for (const signal of ['SIGINT', 'SIGTERM'] as const) {
      process.once(signal, () => void browser.dispose());
    }
  } catch (error) {
    await browser.dispose();
    throw error;
  }
} else {
  const assets = new Set<string>(Object.values(pages).flatMap(({ html, css }) => [html, ...css]));
  assets.add('assets/waterworks.webp');
  assets.add('assets/mew-diorama.glb');
  assets.add('assets/sentient.woff2');
  assets.add('assets/instrument-sans.woff2');
  assets.add('assets/instrument-sans-OFL.txt');
  assets.add('vendor/tactics/a1.png');
  assets.add('vendor/tactics/NOTICE.md');
  const dist = resolve(root, 'dist');

  await build({
    ...browserBuild, entryPoints, format: 'esm', splitting: true, minify: true, outdir: 'dist/generated',
  });
  await Promise.all([...assets].map(async (file) => {
    const destination = resolve(dist, file);
    await mkdir(dirname(destination), { recursive: true });
    await copyFile(resolve(root, file), destination);
  }));
  await copyFile(resolve(root, 'node_modules/three/LICENSE'), resolve(dist, 'three-LICENSE.txt'));
}
