import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const dist = new URL('../dist/', import.meta.url);
const readPage = (path) => readFile(new URL(path, dist), 'utf8');

test('the built homepage serves the garden and leads readers to an existing docs page', async () => {
  const home = await readPage('index.html');
  assert.ok(home.includes('<h1>Mew is an agent harness designed for your work.</h1>'));
  assert.equal([...home.matchAll(/<section[^>]+data-focus=/g)].length, 7);

  for (const [attribute, expected] of [['data-garden-model', 'glTF'], ['data-sprite-sheet', '\u0089PNG']]) {
    const match = home.match(new RegExp(`${attribute}="([^"]+)"`));
    assert.ok(match, attribute);
    const asset = await readFile(new URL(match[1].replace(/^\//, ''), dist));
    assert.equal(asset.toString('latin1', 0, 4), expected, attribute);
  }

  const docsLink = home.match(/href="(\/docs\/[^"]+)"[^>]*>Get started/);
  assert.ok(docsLink, 'the primary action points to local installation docs');
  const docs = await readPage(docsLink[1].slice(1) + 'index.html');
  assert.ok(/<h1[^>]*>[^<]*Install/.test(docs));
});

test('the homepage exposes a locally built Open Graph share image', async () => {
  const home = await readPage('index.html');
  const image = home.match(/property="og:image" content="(https:\/\/mew\.computer\/[^"]+)"/);
  assert.ok(image, 'an absolute share image URL on the production origin');
  const path = new URL(image[1]).pathname.slice(1);
  const jpeg = await readFile(new URL(path, dist));
  assert.equal(jpeg.readUInt16BE(0), 0xffd8);
  assert.ok(home.includes('property="og:image:width" content="1200"'));
  assert.ok(home.includes('property="og:image:height" content="630"'));
});
