const assert = require('node:assert/strict');
const { test } = require('node:test');
const { mkdtemp, mkdir, writeFile, symlink, rm, readdir, readlink, realpath } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const path = require('node:path');
const http = require('node:http');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { assetPath, isFixtureUrl, parseRequest, startAssetServer } = require('./main.cjs');
const run = promisify(execFile);

test('static paths resolve files without decoding traversal or path separators', () => {
  const root = path.resolve('/tmp/composition-assets');
  assert.equal(assetPath(root, '/'), path.join(root, 'index.html'));
  assert.equal(assetPath(root, '/assets/main.js?v=1'), path.join(root, 'assets/main.js'));
  assert.equal(assetPath(root, '/some%20file.css'), path.join(root, 'some file.css'));
  for (const url of ['/../outside', '/%2e%2e/outside', '/assets/%2fetc', '/assets/%5coutside', '/%00', '/%ff', '//outside', 'https://outside/']) {
    assert.equal(assetPath(root, url), null, url);
  }
});

test('the guest can load only the fixture on the exact local origin', () => {
  const origin = 'http://127.0.0.1:54321';
  for (const url of [`${origin}/fixture.html`, `${origin}/fixture.html?q=1#input`]) {
    assert.equal(isFixtureUrl(url, origin), true);
  }
  for (const url of [`${origin}/`, 'http://127.0.0.1:54322/fixture.html', 'http://localhost:54321/fixture.html', 'https://127.0.0.1:54321/fixture.html', 'file:///fixture.html', 'about:blank', 'not a URL']) {
    assert.equal(isFixtureUrl(url, origin), false, url);
  }
});

test('host requests accept only the two explicit commands', () => {
  assert.equal(parseRequest('{"type":"bootstrap"}'), 'bootstrap');
  assert.equal(parseRequest('{"type":"metrics"}'), 'metrics');
  for (const request of ['null', '[]', '{}', '{"type":"browser_update"}', '{"type":"metrics","extra":true}', '{']) {
    assert.throws(() => parseRequest(request));
  }
  assert.throws(() => parseRequest({ type: 'bootstrap' }));
});

test('the actual HTTP server serves local assets with frame protection and blocks escapes', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'mew-composition-test-'));
  const root = path.join(directory, 'assets');
  await mkdir(root);
  await writeFile(path.join(root, 'index.html'), '<main>composition</main>');
  await writeFile(path.join(root, 'fixture.html'), '<input aria-label="Guest input">');
  await writeFile(path.join(root, 'main.js'), 'console.log("fixture");');
  await writeFile(path.join(directory, 'outside.txt'), 'private');
  await symlink(path.join(directory, 'outside.txt'), path.join(root, 'escape.txt'));
  const { server, origin } = await startAssetServer(root);
  t.after(async () => {
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });

  const index = await fetch(origin);
  assert.equal(index.status, 200);
  assert.match(index.headers.get('content-type'), /^text\/html/);
  assert.equal(await index.text(), '<main>composition</main>');
  const fixture = await fetch(`${origin}/fixture.html`);
  assert.match(fixture.headers.get('content-security-policy'), /frame-ancestors 'none'/);
  assert.equal(fixture.headers.get('x-frame-options'), 'DENY');
  assert.equal((await fetch(`${origin}/main.js`)).headers.get('content-type'), 'text/javascript; charset=utf-8');
  assert.equal((await fetch(`${origin}/fixture.html`, { method: 'HEAD' })).status, 200);
  assert.equal((await fetch(`${origin}/fixture.html`, { method: 'POST' })).status, 405);
  assert.equal((await fetch(`${origin}/missing`)).status, 404);
  assert.equal((await fetch(`${origin}/escape.txt`)).status, 403);
  assert.equal((await fetch(`${origin}/%2e%2e%2foutside.txt`)).status, 400);
  const untrustedHostStatus = await new Promise((resolve, reject) => {
    http.get(origin, { headers: { Host: 'untrusted.example' } }, (response) => {
      response.resume();
      resolve(response.statusCode);
    }).on('error', reject);
  });
  assert.equal(untrustedHostStatus, 403);
});

test('a real packaged Electron runtime keeps relative framework links inside its signed bundle', { skip: process.platform !== 'darwin' }, async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'mew-composition-package-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const assets = path.join(directory, 'assets');
  await mkdir(assets);
  await writeFile(path.join(assets, 'index.html'), '<main>composition</main>');
  await writeFile(path.join(assets, 'fixture.html'), '<input aria-label="Guest input">');
  const output = path.join(directory, 'Composition.app');
  await run(process.execPath, [path.join(__dirname, 'main.cjs'), '--package', output], { env: { ...process.env, MEW_COMPOSITION_ASSETS: assets } });

  const runtime = path.resolve(require('electron'), '../../..');
  const packagedRoot = await realpath(output);
  let links = 0;
  async function checkLinks(relativeDirectory) {
    for (const entry of await readdir(path.join(runtime, relativeDirectory), { withFileTypes: true })) {
      const relative = path.join(relativeDirectory, entry.name);
      if (entry.isDirectory()) await checkLinks(relative);
      if (!entry.isSymbolicLink()) continue;
      const sourceTarget = await readlink(path.join(runtime, relative));
      const copiedTarget = await readlink(path.join(output, relative));
      assert.equal(path.isAbsolute(copiedTarget), false, relative);
      assert.equal(copiedTarget, sourceTarget, relative);
      assert((await realpath(path.join(output, relative))).startsWith(`${packagedRoot}${path.sep}`), relative);
      links++;
    }
  }
  await checkLinks('Contents/Frameworks');
  assert(links > 0, 'the actual runtime should include framework symlinks');
  await run('/usr/bin/codesign', ['--verify', '--deep', '--strict', output]);
});
