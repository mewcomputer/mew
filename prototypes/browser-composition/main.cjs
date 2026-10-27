const http = require('node:http');
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const { randomUUID } = require('node:crypto');
const { execFileSync } = require('node:child_process');

function assetPath(root, requestUrl) {
  if (!requestUrl.startsWith('/') || requestUrl.startsWith('//')) return null;
  try {
    const segments = requestUrl.split('?')[0].split('/').slice(1).map(decodeURIComponent);
    if (segments.some((part) => part === '.' || part === '..' || /[/\\\0]/.test(part))) return null;
    return path.join(root, segments.join('/') || 'index.html');
  } catch {
    return null;
  }
}

function isFixtureUrl(value, origin) {
  try {
    const url = new URL(value);
    return url.origin === origin && url.pathname === '/fixture.html' && !url.username && !url.password;
  } catch {
    return false;
  }
}

function parseRequest(value) {
  if (typeof value !== 'string' || value.length > 1024) throw new Error('Invalid host request.');
  const request = JSON.parse(value);
  if (!request || Object.keys(request).length !== 1 || !['bootstrap', 'metrics'].includes(request.type)) {
    throw new Error('Unsupported host request.');
  }
  return request.type;
}

async function startAssetServer(directory) {
  const root = await fsp.realpath(directory);
  const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.woff2': 'font/woff2' };
  let origin;
  const server = http.createServer(async (request, response) => {
    const finish = (status) => { response.writeHead(status); response.end(); };
    if (request.headers.host !== new URL(origin).host) return finish(403);
    if (!['GET', 'HEAD'].includes(request.method)) return finish(405);
    const file = assetPath(root, request.url);
    if (!file) return finish(400);
    try {
      const resolved = await fsp.realpath(file);
      if (!resolved.startsWith(`${root}${path.sep}`)) return finish(403);
      const stat = await fsp.stat(resolved);
      if (!stat.isFile()) return finish(404);
      const type = types[path.extname(resolved)] ?? 'application/octet-stream';
      const fixture = resolved === path.join(root, 'fixture.html');
      response.writeHead(200, {
        'Content-Type': `${type}${type.startsWith('text/') || type === 'application/json' ? '; charset=utf-8' : ''}`,
        'Content-Length': stat.size,
        'Cache-Control': 'no-store',
        'X-Content-Type-Options': 'nosniff',
        'X-Frame-Options': 'DENY',
        'Content-Security-Policy': `default-src 'self'; script-src 'self'${fixture ? " 'unsafe-inline'" : ''}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`,
      });
      if (request.method === 'HEAD') return response.end();
      fs.createReadStream(resolved).on('error', () => response.destroy()).pipe(response);
    } catch (error) {
      finish(error.code === 'ENOENT' || error.code === 'ENOTDIR' ? 404 : 500);
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  origin = `http://127.0.0.1:${server.address().port}`;
  return { server, origin };
}

function assetsDirectory() {
  return path.resolve(process.env.MEW_COMPOSITION_ASSETS || (fs.existsSync(path.join(__dirname, 'assets')) ? path.join(__dirname, 'assets') : path.join(__dirname, '../../target/composition-ui')));
}

async function startElectron() {
  const { app, BrowserWindow, ipcMain, Menu, session } = require('electron');
  app.setName('Mew Composition Electron');
  app.enableSandbox();
  const userData = process.env.MEW_COMPOSITION_USER_DATA ? path.resolve(process.env.MEW_COMPOSITION_USER_DATA) : fs.mkdtempSync(path.join(os.tmpdir(), 'mew-composition-electron-'));
  fs.mkdirSync(userData, { recursive: true });
  app.setPath('userData', userData);
  const { server, origin } = await startAssetServer(assetsDirectory());
  await app.whenReady();

  const appUrl = (value) => value === `${origin}/` || value === `${origin}/index.html`;
  const secureSession = (ses, allowedNavigation) => {
    ses.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
    ses.setPermissionCheckHandler(() => false);
    ses.on('will-download', (event) => event.preventDefault());
    ses.webRequest.onBeforeRequest((details, callback) => {
      callback({ cancel: new URL(details.url).origin !== origin || (details.resourceType === 'mainFrame' && !allowedNavigation(details.url)) });
    });
  };
  const hostSession = session.fromPartition('mew-composition-host');
  const guestSession = session.fromPartition('mew-composition-guest');
  secureSession(hostSession, appUrl);
  secureSession(guestSession, (url) => isFixtureUrl(url, origin));
  const window = new BrowserWindow({
    title: 'Mew Composition · Electron', width: 1440, height: 960, minWidth: 1000, minHeight: 600,
    backgroundColor: '#181818',
    webPreferences: { preload: path.join(__dirname, 'preload.cjs'), session: hostSession, sandbox: true, contextIsolation: true, nodeIntegration: false, webviewTag: true },
  });
  const limitNavigation = (contents, allowed) => {
    contents.setWindowOpenHandler(() => ({ action: 'deny' }));
    for (const eventName of ['will-navigate', 'will-frame-navigate', 'will-redirect']) {
      contents.on(eventName, (event) => { if (!allowed(event.url)) event.preventDefault(); });
    }
  };
  limitNavigation(window.webContents, appUrl);
  window.webContents.on('will-attach-webview', (event, preferences, params) => {
    if (!isFixtureUrl(params.src, origin)) return event.preventDefault();
    delete preferences.preload;
    delete preferences.preloadURL;
    Object.assign(preferences, { session: guestSession, sandbox: true, contextIsolation: true, nodeIntegration: false, nodeIntegrationInWorker: false, nodeIntegrationInSubFrames: false, webviewTag: false, webSecurity: true });
    delete params.allowpopups;
  });
  window.webContents.on('did-attach-webview', (_event, contents) => limitNavigation(contents, (url) => isFixtureUrl(url, origin)));
  ipcMain.handle('mew-composition:query', (event, value) => {
    if (event.sender !== window.webContents || event.senderFrame !== window.webContents.mainFrame || !appUrl(event.senderFrame.url)) throw new Error('Untrusted host request.');
    const type = parseRequest(value);
    if (type === 'bootstrap') return { engine: 'electron-webview', fixtureUrl: `${origin}/fixture.html`, versions: { electron: process.versions.electron, chromium: process.versions.chrome, node: process.versions.node } };
    return { engine: 'electron-webview', sampledAt: Date.now(), processes: app.getAppMetrics() };
  });
  Menu.setApplicationMenu(Menu.buildFromTemplate([{ role: 'appMenu' }, { role: 'editMenu' }, { role: 'windowMenu' }]));
  app.on('window-all-closed', () => app.quit());
  app.on('quit', () => server.close());
  await window.loadURL(`${origin}/`);
  console.log(JSON.stringify({ engine: 'electron-webview', pid: process.pid, origin, userData }));
}

async function packageApp(destination) {
  if (process.platform !== 'darwin') throw new Error('The disposable app bundle is macOS-only.');
  const output = destination ? path.resolve(destination) : path.join(await fsp.mkdtemp(path.join(os.tmpdir(), 'mew-composition-package-')), 'Mew Composition Electron.app');
  if (!output.endsWith('.app')) throw new Error('Package destination must end in .app.');
  if (fs.existsSync(output)) throw new Error(`Package destination already exists: ${output}`);
  const assets = assetsDirectory();
  await Promise.all(['index.html', 'fixture.html'].map((name) => fsp.access(path.join(assets, name))));
  const executable = require('electron');
  const runtime = path.resolve(executable, '../../..');
  await fsp.cp(runtime, output, { recursive: true, errorOnExist: true, force: false, verbatimSymlinks: true });
  const appDirectory = path.join(output, 'Contents/Resources/app');
  await fsp.mkdir(appDirectory);
  await Promise.all(['main.cjs', 'preload.cjs', 'package.json'].map((name) => fsp.copyFile(path.join(__dirname, name), path.join(appDirectory, name))));
  await fsp.cp(assets, path.join(appDirectory, 'assets'), { recursive: true });
  await fsp.rm(path.join(output, 'Contents/Resources/default_app.asar'), { force: true });
  const bundleId = `org.mew.composition.electron.${randomUUID().replaceAll('-', '')}`;
  const plist = path.join(output, 'Contents/Info.plist');
  for (const [key, value] of Object.entries({ CFBundleIdentifier: bundleId, CFBundleName: 'Mew Composition Electron', CFBundleDisplayName: 'Mew Composition Electron' })) {
    execFileSync('/usr/bin/plutil', ['-replace', key, '-string', value, plist]);
  }
  execFileSync('/usr/bin/codesign', ['--force', '--deep', '--sign', '-', '--preserve-metadata=entitlements', output], { stdio: 'pipe' });
  execFileSync('/usr/bin/codesign', ['--verify', '--deep', '--strict', output], { stdio: 'pipe' });
  console.log(JSON.stringify({ appPath: output, bundleId, executable: path.join(output, 'Contents/MacOS/Electron') }));
}

module.exports = { assetPath, isFixtureUrl, parseRequest, startAssetServer };

if (require.main === module) {
  const operation = process.versions.electron ? startElectron() : process.argv[2] === '--package' ? packageApp(process.argv[3]) : Promise.reject(new Error('Use npm start or npm run package.'));
  operation.catch((error) => { console.error(error.message); process.exitCode = 1; if (process.versions.electron) require('electron').app.exit(1); });
}
