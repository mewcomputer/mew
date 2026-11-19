import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import test from 'node:test';

const dist = new URL('../dist/', import.meta.url);
const installation = 'docs/getting-started/installation';

test('the themed docs use one Starlight menu and preserve article navigation', async () => {
  const page = await readFile(new URL(`${installation}/index.html`, dist), 'utf8');
  assert.match(page, /<a[^>]*class="[^"]*mew-wordmark[^"]*"[^>]*href="\/"/);
  assert.match(page, /<h1[^>]*id="_top"[^>]*>Installation<\/h1>/);
  assert.equal((page.match(/id="starlight__sidebar"/g) ?? []).length, 1);
  assert.match(page, /<a[^>]*href="\/docs\/getting-started\/installation\/"[^>]*aria-current="page"/);
  const sidebar = page.split('id="starlight__sidebar"')[1].split('</nav>')[0];
  assert.equal((sidebar.match(/href="\/docs\/getting-started\/installation\/"/g) ?? []).length, 1);
  assert.match(page, /<a href="\/docs\/getting-started\/quick-start\/" rel="next"/);
  assert.match(page, /site-search/);
  assert.match(page, /class="expressive-code"/);
});

test('the docs and homepage share the wordmark and header button styles', async () => {
  const [home, docs] = await Promise.all(['index.html', `${installation}/index.html`]
    .map((path) => readFile(new URL(path, dist), 'utf8')));
  for (const page of [home, docs]) {
    const header = page.match(/<header\b[^>]*>[\s\S]*?<\/header>/)?.[0];
    assert.ok(header);
    assert.match(header, /class="[^"]*mew-wordmark[^"]*"/);
    assert.match(header, /<a[^>]*class="[^"]*mew-button[^"]*"[^>]*>Get started<\/a>/);
  }
  assert.match(docs, /<a[^>]*href="\/docs\/getting-started\/quick-start\/"[^>]*>Get started<\/a>/);
});

test('the installation guide remains available as raw Markdown', async () => {
  const markdown = await readFile(new URL(`${installation}.md`, dist), 'utf8');
  assert.match(markdown, /## macOS and Linux/);
  assert.match(markdown, /```sh\s+brew tap mewcomputer\/tap\nbrew install mewcomputer\/tap\/mew/);
  assert.match(markdown, /brew upgrade mewcomputer\/tap\/mew/);
});

test('the docs index leads readers to each client', async () => {
  const [home, overview] = await Promise.all(['index.html', 'docs/index.html']
    .map((path) => readFile(new URL(path, dist), 'utf8')));
  assert.match(home, /<a href="\/docs\/">Docs<\/a>/);
  assert.match(overview, /<h1[^>]*id="_top"[^>]*>Mew Docs<\/h1>/);
  for (const slug of ['getting-started/installation', 'using-mew/desktop-app', 'using-mew/web-ui', 'using-mew/ios-app']) {
    assert.ok(overview.includes(`href="/docs/${slug}/"`), slug);
    const page = await readFile(new URL(`docs/${slug}/index.html`, dist), 'utf8');
    assert.match(page, /aria-current="page"/);
  }
});

test('the public docs render and link to existing pages and headings', async () => {
  const source = new URL('../../docs/', import.meta.url);
  const paths = (await readdir(source, { recursive: true })).filter((path) => path.endsWith('.md'));
  const pages = new Map(await Promise.all(paths.map(async (path) => {
    const route = '/docs/' + (path === 'index.md' ? '' : path.replace(/(?:\/index)?\.md$/, '/'));
    const [page, markdown] = await Promise.all([route + 'index.html', route.slice(0, -1) + '.md']
      .map((url) => readFile(new URL(url.slice(1), dist), 'utf8')));
    assert.ok(markdown.trim(), `${route} has a raw Markdown version`);
    const fragments = new Set(Array.from(page.matchAll(/\bid="([^"]+)"/g), ([, id]) => id));
    return [route, { page, fragments }];
  })));

  const sidebar = pages.get('/docs/').page.split('id="starlight__sidebar"')[1].split('</nav>')[0];
  for (const [route, { page }] of pages) {
    assert.ok(sidebar.includes(`href="${route}"`), `${route} is listed in the docs sidebar`);
    for (const [, href] of page.matchAll(/<a\b[^>]*\bhref="([^"]+)"/g)) {
      const url = new URL(href, `https://mew.computer${route}`);
      if (url.origin !== 'https://mew.computer' || !/^\/docs(?:\/|$)/.test(url.pathname)) continue;
      const destination = url.pathname.endsWith('/') ? url.pathname : url.pathname + '/';
      const target = pages.get(destination);
      assert.ok(target, `${route} links to missing page ${destination}`);
      if (url.hash) {
        assert.ok(target.fragments.has(decodeURIComponent(url.hash.slice(1))), `${route} links to missing heading ${href}`);
      }
    }
  }
});
