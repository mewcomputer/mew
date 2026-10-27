const assert = require('node:assert/strict');
const { test } = require('node:test');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { fork, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { packageApp, validateAgentBrowserVersion } = require('./package.cjs');

async function fixture(t) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'mew-electron-package-input-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const sourceDirectory = path.join(root, 'apps/mew-electron');
  const runtime = path.join(sourceDirectory, 'node_modules/electron');
  const output = path.join(root, 'QA.app');
  const mewBinary = path.join(root, 'mew');
  const agentBrowser = path.join(root, 'agent-browser');
  await fs.mkdir(path.join(runtime, 'dist/Electron.app/Contents/MacOS'), { recursive: true });
  await fs.writeFile(path.join(sourceDirectory, 'package.json'), JSON.stringify({ name: '@mew/electron', main: 'main.cjs', devDependencies: { electron: '45.0.0' } }));
  await fs.writeFile(path.join(runtime, 'package.json'), JSON.stringify({ version: '45.0.0' }));
  await fs.writeFile(path.join(runtime, 'dist/version'), '45.0.0');
  for (const name of ['main', 'preload', 'policy', 'browser', 'support']) await fs.writeFile(path.join(sourceDirectory, `${name}.cjs`), '');
  // These fixtures exercise input rejection, never execute or sign this header.
  for (const filename of [mewBinary, agentBrowser, path.join(runtime, 'dist/Electron.app/Contents/MacOS/Electron')]) {
    await fs.writeFile(filename, Buffer.from('cffaedfe', 'hex'), { mode: 0o755 });
  }
  return { repoRoot: root, sourceDirectory, output, mewBinary, agentBrowser, version: '0.2.0', runtime };
}

test('agent-browser must report a stable version at least 0.34', () => {
  for (const value of ['agent-browser 0.34.0', 'agent-browser 0.35.2\n', 'agent-browser 1.0.0']) assert.match(validateAgentBrowserVersion(value), /^\d+\.\d+\.\d+$/);
  for (const value of ['', 'another-tool 9.0.0', 'agent-browser 0.33.9', 'agent-browser 0.34.0-beta']) assert.throws(() => validateAgentBrowserVersion(value));
});

test('invalid profiles, versions and bundle suffixes fail without producing output', async (t) => {
  const options = await fixture(t);
  await assert.rejects(packageApp({ ...options, profile: 'other' }), /profile/);
  await assert.rejects(packageApp({ ...options, version: 'not-a-version' }), /version/);
  await assert.rejects(packageApp({ ...options, output: path.join(options.repoRoot, 'folder') }), /\.app/);
  await assert.rejects(fs.access(options.output));
});

test('a custom destination and its contents are never overwritten', async (t) => {
  const options = await fixture(t);
  await fs.mkdir(options.output);
  await fs.writeFile(path.join(options.output, 'keep'), 'user artifact');
  await assert.rejects(packageApp(options), /exists/);
  assert.equal(await fs.readFile(path.join(options.output, 'keep'), 'utf8'), 'user artifact');
});

test('runtime must match the exact pinned Electron package and distribution', async (t) => {
  const options = await fixture(t);
  await fs.writeFile(path.join(options.runtime, 'dist/version'), '44.0.0');
  await assert.rejects(packageApp(options), /Electron 45\.0\.0/);
  await assert.rejects(fs.access(options.output));
});

test('missing source modules and daemon binaries are rejected before copying', async (t) => {
  const options = await fixture(t);
  await fs.rm(path.join(options.sourceDirectory, 'support.cjs'));
  await assert.rejects(packageApp(options), /support\.cjs/);
  await fs.writeFile(path.join(options.sourceDirectory, 'support.cjs'), '');
  await fs.rm(options.mewBinary);
  await assert.rejects(packageApp(options), /mew/);
  await assert.rejects(fs.access(options.output));
});

test('non-executable daemon binaries and script browser wrappers cannot be bundled', async (t) => {
  const options = await fixture(t);
  await fs.chmod(options.mewBinary, 0o644);
  await assert.rejects(packageApp(options), /executable/);
  await fs.chmod(options.mewBinary, 0o755);
  await fs.writeFile(options.agentBrowser, '#!/usr/bin/env node\n');
  await assert.rejects(packageApp(options), /standalone.*Mach-O/);
  await assert.rejects(fs.access(options.output));
});

test('default output refuses foreign bundles and symlinks without modifying their data', async (t) => {
  const options = await fixture(t);
  const output = path.join(options.repoRoot, 'target/debug/bundle/macos/mew-electron.app');
  await fs.mkdir(output, { recursive: true });
  await fs.writeFile(path.join(output, 'keep'), 'foreign data');
  await assert.rejects(packageApp({ ...options, profile: 'debug', output: undefined }), /owned|managed/);
  assert.equal(await fs.readFile(path.join(output, 'keep'), 'utf8'), 'foreign data');
  const foreign = path.join(options.repoRoot, 'foreign.app');
  await fs.rename(output, foreign);
  await fs.symlink(foreign, output);
  await assert.rejects(packageApp({ ...options, profile: 'debug', output: undefined }), /symlink|owned|managed/);
  assert.equal(await fs.readFile(path.join(foreign, 'keep'), 'utf8'), 'foreign data');
});

test('a process using an owned default bundle prevents replacement', { skip: process.platform !== 'darwin' }, async (t) => {
  const options = await fixture(t);
  const output = path.join(options.repoRoot, 'target/debug/bundle/macos/mew-electron.app');
  const app = path.join(output, 'Contents/Resources/app');
  await fs.mkdir(app, { recursive: true });
  await fs.writeFile(path.join(app, 'package.json'), JSON.stringify({ name: '@mew/electron', main: 'main.cjs' }));
  await fs.writeFile(path.join(output, 'Contents/Info.plist'), '<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.mew.electron</string><key>CFBundleExecutable</key><string>mew-electron</string></dict></plist>');
  const script = path.join(app, 'hold.cjs');
  await fs.writeFile(script, "process.send('ready'); setInterval(() => {}, 1000);\n");
  const child = fork(script, { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
  t.after(async () => { child.kill(); await once(child, 'exit'); });
  await once(child, 'message');
  await assert.rejects(packageApp({ ...options, profile: 'debug', output: undefined }), /running|in use/);
  assert.equal((await fs.readFile(script, 'utf8')).includes('setInterval'), true);
});

const nativePackaging = process.platform === 'darwin' && process.env.MEW_ELECTRON_NATIVE_PACKAGE_TESTS === '1';

test('a real bundle uses a packaged executable and repeated default builds replace only their owned output', { skip: !nativePackaging }, async (t) => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'mew-electron-package-native-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const options = { repoRoot: root, sourceDirectory: __dirname, mewBinary: path.resolve(__dirname, '../../target/debug/mew'), profile: 'debug', version: '0.2.0' };
  const first = await packageApp(options);
  const plist = JSON.parse(execFileSync('/usr/bin/plutil', ['-convert', 'json', '-o', '-', path.join(first.appPath, 'Contents/Info.plist')], { encoding: 'utf8' }));
  assert.equal(plist.CFBundleExecutable, 'mew-electron');
  assert.equal(path.basename(first.executable), plist.CFBundleExecutable);
  // Electron 45 App::IsPackaged() checks FILE_EXE basename != "electron" on macOS.
  assert.notEqual(path.basename(first.executable).toLowerCase(), 'electron');
  await assert.rejects(fs.access(path.join(first.appPath, 'Contents/MacOS/Electron')));
  await fs.writeFile(path.join(first.appPath, 'old-build'), 'replace this generated bundle');
  const second = await packageApp({ ...options, version: '0.2.1' });
  assert.equal(second.appPath, first.appPath);
  assert.equal(JSON.parse(await fs.readFile(path.join(second.appPath, 'Contents/Resources/app/package.json'), 'utf8')).version, '0.2.1');
  await assert.rejects(fs.access(path.join(second.appPath, 'old-build')));
  execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', second.appPath], { stdio: 'pipe' });
  assert.deepEqual(await fs.readdir(path.dirname(second.appPath)), ['mew-electron.app']);
});
