const fs = require('node:fs/promises');
const { constants } = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const modules = ['main.cjs', 'preload.cjs', 'policy.cjs', 'browser.cjs', 'support.cjs'];
const bundleId = 'com.mew.electron';
const executableName = 'mew-electron';

function validateAgentBrowserVersion(output) {
  const match = /^agent-browser\s+v?(\d+)\.(\d+)\.(\d+)\s*$/.exec(output.trim());
  if (!match || (Number(match[1]) === 0 && Number(match[2]) < 34)) throw new Error('agent-browser 0.34.0 or newer is required.');
  return match.slice(1).join('.');
}

async function exists(filename) {
  try { await fs.lstat(filename); return true; } catch (error) { if (error.code === 'ENOENT') return false; throw error; }
}

async function executable(filename) {
  try {
    await fs.access(filename, constants.X_OK);
    if (!(await fs.stat(filename)).isFile()) throw new Error('not a file');
  } catch { throw new Error(`Required binary is missing or not executable: ${filename}`); }
}

async function machO(filename) {
  const file = await fs.open(filename, 'r');
  try {
    const header = Buffer.alloc(4);
    await file.read(header, 0, 4, 0);
    return ['feedface', 'cefaedfe', 'feedfacf', 'cffaedfe', 'cafebabe', 'bebafeca', 'cafebabf', 'bfbafeca'].includes(header.toString('hex'));
  } finally { await file.close(); }
}

async function browserBinary(explicit) {
  let filename = explicit;
  if (!filename) {
    for (const directory of (process.env.PATH || '').split(path.delimiter)) {
      const candidate = path.join(directory, 'agent-browser');
      if (await exists(candidate)) { filename = candidate; break; }
    }
  }
  if (!filename) throw new Error('agent-browser 0.34.0 or newer must be installed before packaging.');
  filename = await fs.realpath(filename);
  await executable(filename);
  if (!await machO(filename)) {
    filename = path.join(path.dirname(filename), `agent-browser-darwin-${process.arch === 'arm64' ? 'arm64' : 'x64'}`);
    if (!await exists(filename) || !await machO(filename)) throw new Error('agent-browser must provide a standalone macOS Mach-O binary.');
    await executable(filename);
  }
  return filename;
}

async function requireOwnedInactiveBundle(output) {
  const stat = await fs.lstat(output);
  if (stat.isSymbolicLink() || !stat.isDirectory()) throw new Error(`Default bundle must be an owned directory, not a symlink: ${output}`);
  let owned = false;
  try {
    const plist = JSON.parse(execFileSync('/usr/bin/plutil', ['-convert', 'json', '-o', '-', path.join(output, 'Contents/Info.plist')], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }));
    const app = JSON.parse(await fs.readFile(path.join(output, 'Contents/Resources/app/package.json'), 'utf8'));
    owned = plist.CFBundleIdentifier === bundleId && [executableName, 'Electron'].includes(plist.CFBundleExecutable)
      && app.name === '@mew/electron' && app.main === 'main.cjs';
  } catch {}
  if (!owned) throw new Error(`Default destination is not a managed mew Electron bundle: ${output}`);
  const prefixes = [output, await fs.realpath(output)].map((directory) => path.join(directory, 'Contents') + path.sep);
  const processes = execFileSync('/bin/ps', ['-axo', 'pid=,command='], { encoding: 'utf8' });
  if (processes.split('\n').some((line) => {
    const match = /^\s*(\d+)\s+(.*)$/.exec(line);
    return match && Number(match[1]) !== process.pid && prefixes.some((prefix) => match[2].includes(prefix));
  })) throw new Error(`Default bundle is running or in use; close it before rebuilding: ${output}`);
}

async function packageApp(options = {}) {
  const repoRoot = path.resolve(options.repoRoot || path.join(__dirname, '../..'));
  const source = path.resolve(options.sourceDirectory || __dirname);
  const profile = options.profile || process.env.MEW_DESKTOP_PROFILE || 'release';
  if (!['debug', 'release'].includes(profile)) throw new Error(`Unsupported desktop profile: ${profile}`);
  let version = options.version || process.env.MEW_VERSION;
  if (!version) version = execFileSync('cargo', ['pkgid', '--manifest-path', path.join(repoRoot, 'Cargo.toml'), '-p', 'mew'], { encoding: 'utf8' }).trim().split('#').at(-1);
  version = version.replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error(`Invalid desktop version: ${version}`);
  const output = path.resolve(options.output || path.join(repoRoot, 'target', profile, 'bundle/macos/mew-electron.app'));
  if (!output.endsWith('.app')) throw new Error('Package destination must end in .app.');
  const replacing = await exists(output);
  if (replacing) {
    if (options.output) throw new Error(`Package destination already exists: ${output}. Choose a fresh --output.`);
    await requireOwnedInactiveBundle(output);
  }
  const manifest = JSON.parse(await fs.readFile(path.join(source, 'package.json'), 'utf8'));
  const pinned = manifest.devDependencies.electron;
  const electron = path.join(source, 'node_modules/electron');
  const installed = JSON.parse(await fs.readFile(path.join(electron, 'package.json'), 'utf8'));
  const distribution = (await fs.readFile(path.join(electron, 'dist/version'), 'utf8')).trim();
  if (pinned !== '45.0.0' || installed.version !== pinned || distribution !== pinned) throw new Error('Packaging requires the pinned Electron 45.0.0 runtime; install the app dependencies again.');
  for (const name of modules) await fs.access(path.join(source, name)).catch(() => { throw new Error(`Required app source is missing: ${name}`); });
  const runtime = path.join(electron, 'dist/Electron.app');
  const mew = path.resolve(options.mewBinary || path.join(repoRoot, 'target', profile, 'mew'));
  await executable(mew);
  if (!await machO(mew)) throw new Error(`mew must be a macOS Mach-O binary: ${mew}`);
  const agentBrowser = await browserBinary(options.agentBrowser);
  await executable(path.join(runtime, 'Contents/MacOS/Electron'));
  if (process.platform !== 'darwin') throw new Error('Electron app packaging currently supports macOS only.');
  const agentBrowserVersion = validateAgentBrowserVersion(execFileSync(agentBrowser, ['--version'], { encoding: 'utf8', timeout: 10_000 }));
  await fs.mkdir(path.dirname(output), { recursive: true });
  const stage = await fs.mkdtemp(path.join(path.dirname(output), '.mew-electron-package-'));
  const staged = path.join(stage, path.basename(output));
  let preserveStage = false;
  try {
    await fs.cp(runtime, staged, { recursive: true, errorOnExist: true, force: false, verbatimSymlinks: true });
    await fs.rename(path.join(staged, 'Contents/MacOS/Electron'), path.join(staged, 'Contents/MacOS', executableName));
    const resources = path.join(staged, 'Contents/Resources');
    const app = path.join(resources, 'app');
    await fs.mkdir(app);
    for (const name of modules) await fs.copyFile(path.join(source, name), path.join(app, name));
    await fs.writeFile(path.join(app, 'package.json'), JSON.stringify({ name: manifest.name, version, main: manifest.main }, null, 2) + '\n');
    await fs.rm(path.join(resources, 'default_app.asar'), { force: true });
    for (const [input, name] of [[mew, 'mew'], [agentBrowser, 'agent-browser']]) {
      const binary = path.join(staged, 'Contents/MacOS', name);
      await fs.copyFile(input, binary);
      await fs.chmod(binary, 0o755);
    }
    for (const [input, name] of [['LICENSE', 'Electron-LICENSE'], ['LICENSES.chromium.html', 'Chromium-LICENSES.html']]) await fs.copyFile(path.join(electron, 'dist', input), path.join(resources, name));
    const plist = path.join(staged, 'Contents/Info.plist');
    for (const [key, value] of Object.entries({ CFBundleIdentifier: bundleId, CFBundleExecutable: executableName, CFBundleName: 'mew', CFBundleDisplayName: 'mew', CFBundleShortVersionString: version, CFBundleVersion: version })) {
      execFileSync('/usr/bin/plutil', ['-replace', key, '-string', value, plist]);
    }
    execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', '--preserve-metadata=entitlements', staged], { stdio: 'pipe' });
    execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', staged], { stdio: 'pipe' });
    if (replacing) {
      await requireOwnedInactiveBundle(output);
      const previous = path.join(stage, 'previous.app');
      await fs.rename(output, previous);
      try { await fs.rename(staged, output); } catch (error) {
        try { await fs.rename(previous, output); } catch (restoreError) {
          preserveStage = true;
          throw new AggregateError([error, restoreError], `Could not restore the previous bundle; it remains at ${previous}`);
        }
        throw error;
      }
    } else {
      if (await exists(output)) throw new Error(`Package destination appeared during build: ${output}`);
      await fs.rename(staged, output);
    }
    return { appPath: output, bundleId, executable: path.join(output, 'Contents/MacOS', executableName), version, agentBrowserVersion };
  } finally { if (!preserveStage) await fs.rm(stage, { recursive: true, force: true }); }
}

module.exports = { packageApp, validateAgentBrowserVersion };

if (require.main === module) {
  const args = process.argv.slice(2);
  const options = {};
  let error;
  for (let index = 0; index < args.length; index++) {
    if (!['--output', '--profile'].includes(args[index]) || !args[index + 1]) { error = new Error('Usage: node package.cjs [--output /path/to/fresh.app] [--profile debug|release]'); break; }
    options[args[index] === '--output' ? 'output' : 'profile'] = args[++index];
  }
  (error ? Promise.reject(error) : packageApp(options)).then((result) => console.log(JSON.stringify(result))).catch((error) => { console.error(error.message); process.exitCode = 1; });
}
