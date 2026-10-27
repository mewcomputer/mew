const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const { parseRequest, isAppUrl, debugEndpoint } = require('./policy.cjs');
const { BrowserHost } = require('./browser.cjs');
const { startSupport } = require('./support.cjs');

async function start() {
  const { app, BrowserWindow, ipcMain, Menu, session, dialog, nativeTheme, protocol, net } = require('electron');
  app.setName('mew Electron');
  app.enableSandbox();
  protocol.registerSchemesAsPrivileged([{ scheme: 'mew', privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true } }]);
  if (process.env.MEW_ELECTRON_USER_DATA) app.setPath('userData', path.resolve(process.env.MEW_ELECTRON_USER_DATA));
  if (!app.requestSingleInstanceLock()) { app.quit(); return; }
  fs.mkdirSync(app.getPath('userData'), { recursive: true });
  const endpointFile = path.join(app.getPath('userData'), 'DevToolsActivePort');
  fs.rmSync(endpointFile, { force: true });
  app.commandLine.appendSwitch('remote-debugging-port', '0');
  app.commandLine.appendSwitch('remote-debugging-address', '127.0.0.1');

  let window;
  let browsers;
  let support;
  let stopping = false;
  app.on('before-quit', (event) => {
    if (stopping) return;
    event.preventDefault();
    stopping = true;
    browsers?.shutdown();
    void Promise.resolve(support?.close()).finally(() => app.quit());
  });
  app.on('window-all-closed', () => app.quit());
  app.on('second-instance', () => { window?.show(); window?.focus(); });

  const bundledBinary = path.join(path.dirname(process.execPath), 'mew');
  const binary = process.env.MEW_DESKTOP_DAEMON_BINARY
    || (app.isPackaged ? bundledBinary : path.resolve(__dirname, '../../target/debug/mew'));
  support = await startSupport(binary, { onExit: (error) => {
    if (stopping) return;
    dialog.showErrorBox('mew desktop connection stopped', error.message);
    app.quit();
  } });
  await app.whenReady();
  let endpoint;
  for (let attempt = 0; attempt < 200; attempt++) {
    try { endpoint = debugEndpoint(await fsp.readFile(endpointFile, 'utf8')); break; }
    catch { await new Promise((resolve) => setTimeout(resolve, 25)); }
  }
  if (!endpoint) throw new Error('Chromium did not publish its local browser endpoint.');

  const appUrl = 'mew://app/';
  const daemonUrl = support.bootstrap.daemonUrl;
  const protectedPorts = new Set([support.appUrl, daemonUrl, endpoint].map((value) => {
    const url = new URL(value);
    return url.port || (['https:', 'wss:'].includes(url.protocol) ? '443' : '80');
  }));
  const hostSession = session.fromPartition('persist:mew-ui');
  hostSession.protocol.handle('mew', (request) => {
    if (!isAppUrl(request.url, appUrl) || !['GET', 'HEAD'].includes(request.method)) return new Response(null, { status: 403 });
    const source = new URL(request.url);
    const asset = new URL(support.appUrl);
    asset.pathname = source.pathname;
    asset.search = source.search;
    return net.fetch(asset.href, { method: request.method });
  });
  hostSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  hostSession.setPermissionCheckHandler(() => false);
  hostSession.on('will-download', (event) => event.preventDefault());
  hostSession.webRequest.onBeforeRequest((details, callback) => {
    const url = new URL(details.url);
    const allowed = isAppUrl(details.url, appUrl) || url.href === new URL(daemonUrl).href || ['data:', 'blob:', 'devtools:'].includes(url.protocol);
    callback({ cancel: !allowed });
  });
  window = new BrowserWindow({
    title: 'mew', width: 1440, height: 960, minWidth: 760, minHeight: 540,
    titleBarStyle: 'hidden', trafficLightPosition: { x: 14, y: 18 },
    backgroundColor: nativeTheme.shouldUseDarkColors ? '#181818' : '#ffffff',
    webPreferences: {
      preload: path.join(__dirname, 'preload.cjs'), session: hostSession,
      sandbox: true, contextIsolation: true, nodeIntegration: false, webviewTag: true,
    },
  });
  const contents = window.webContents;
  browsers = new BrowserHost(contents, session.fromPartition('persist:mew-browser'), endpoint, protectedPorts);
  await browsers.listen();
  contents.setWindowOpenHandler(() => ({ action: 'deny' }));
  for (const name of ['will-navigate', 'will-frame-navigate', 'will-redirect']) {
    contents.on(name, (event) => { if (!isAppUrl(event.url, appUrl)) event.preventDefault(); });
  }
  contents.on('did-start-navigation', (_event, _url, inPlace, mainFrame) => {
    if (mainFrame && !inPlace) browsers.dispose();
  });
  contents.on('render-process-gone', (_event, details) => {
    if (stopping) return;
    browsers.dispose();
    dialog.showErrorBox('mew interface stopped', `The interface stopped (${details.reason}). Restart mew to reconnect to your chats.`);
    app.quit();
  });
  ipcMain.handle('mew:query', async (event, value) => {
    if (event.sender !== contents || event.senderFrame !== contents.mainFrame || !isAppUrl(event.senderFrame.url, appUrl)) {
      throw new Error('Untrusted host request.');
    }
    const request = parseRequest(value);
    switch (request.type) {
      case 'bootstrap': return { ...await support.request(request), browserMode: 'electron' };
      case 'set_theme': return support.request(request);
      case 'browser_update': return browsers.update(request);
      case 'browser_bind': return browsers.bind(request);
      case 'browser_close': return browsers.close(request);
    }
  });
  const menuAction = (action) => () => {
    if (!contents.isDestroyed()) contents.send('mew:event', { type: 'menu', detail: action });
  };
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    { role: 'appMenu' },
    { label: 'File', submenu: [
      { label: 'New chat', accelerator: 'CmdOrCtrl+N', click: menuAction('new-chat') },
      { label: 'New tab', accelerator: 'CmdOrCtrl+T', click: menuAction('new-tab') },
      { type: 'separator' }, { role: 'close' },
    ] },
    { role: 'editMenu' },
    { label: 'View', submenu: [
      { label: 'Search chats', accelerator: 'CmdOrCtrl+K', click: menuAction('search') },
      { label: 'Toggle sidebar', accelerator: 'CmdOrCtrl+B', click: menuAction('toggle-sidebar') },
      { label: 'Toggle workbench', accelerator: 'CmdOrCtrl+Shift+B', click: menuAction('toggle-workbench') },
      { type: 'separator' }, { role: 'togglefullscreen' },
    ] },
    { role: 'windowMenu' },
  ]));
  await window.loadURL(appUrl);
  console.log(JSON.stringify({ host: 'electron', packaged: app.isPackaged, pid: process.pid, appUrl, daemonUrl }));
}

start().catch((error) => {
  console.error(error.message);
  const { app, dialog } = require('electron');
  if (app.isReady()) dialog.showErrorBox('Could not start mew', error.message);
  app.quit();
});
