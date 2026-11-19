import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { gunzipSync } from 'node:zlib';

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

test('the initial HTML offers a lightweight garden still for desktop and phones', async () => {
  const home = await readPage('index.html');
  const picture = home.match(/<picture[^>]*data-garden-poster[^>]*>([\s\S]*?)<\/picture>/);
  assert.ok(picture, 'the still is available before JavaScript runs');
  const mobile = picture[1].match(/<source[^>]*media="\(max-width: 860px\)"[^>]*srcset="([^"]+)"/);
  const desktop = picture[1].match(/<img[^>]*src="([^"]+)"[^>]*fetchpriority="high"/);
  assert.ok(mobile && desktop, 'the browser can choose and prioritize one still');
  for (const [, url] of [mobile, desktop]) {
    const asset = await readFile(new URL(url.slice(1), dist));
    assert.equal(asset.toString('ascii', 8, 12), 'WEBP');
    assert.ok(asset.length < 100_000, 'each still stays below 100 KB');
  }
});

test('the compressed model downloads fewer bytes and decodes to the original garden', async () => {
  const home = await readPage('index.html');
  const original = home.match(/data-garden-model="([^"]+)"/);
  const packed = home.match(/data-garden-model-gzip="([^"]+)"/);
  assert.ok(original && packed);
  const readAsset = ([, url]) => readFile(new URL(new URL(url, 'https://mew.computer').pathname.slice(1), dist));
  const [glb, gzip] = await Promise.all([readAsset(original), readAsset(packed)]);
  assert.ok(gzip.length < glb.length / 4);
  assert.deepEqual(gunzipSync(gzip), glb);
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
