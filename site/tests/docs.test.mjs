import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
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
