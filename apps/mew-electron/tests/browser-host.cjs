const { app, BrowserWindow, session, ipcMain } = require('electron');
const assert = require('node:assert/strict');
const { execFile } = require('node:child_process');
const http = require('node:http');
const { createHash } = require('node:crypto');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { parseRequest } = require('../policy.cjs');

const root = require('node:fs').mkdtempSync(path.join(os.tmpdir(), 'mew-electron-test-'));
app.setPath('userData', root);
app.commandLine.appendSwitch('remote-debugging-port', '0');
app.commandLine.appendSwitch('remote-debugging-address', '127.0.0.1');
app.enableSandbox();
const timer = setTimeout(() => { console.error('Guest integration test timed out.'); void finish(1); }, 40000);
const servers = [];
const sockets = new Set();
const commands = new Set();
const cliSessions = new Set();
const counters = { allowedHttp: 0, allowedWs: 0, controlHttp: 0, controlWs: 0 };
let window;
let browsers;
let finishing = false;
const inputTest = process.argv.includes('input');

function listen(handler) {
  const server = http.createServer(handler);
  servers.push(server);
  server.on('connection', (socket) => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  return new Promise((resolve) => server.listen(0, '127.0.0.1', () => resolve(server)));
}

function websocket(server, count) {
  server.on('upgrade', (request, socket) => {
    counters[count]++;
    const accept = createHash('sha1').update(request.headers['sec-websocket-key'] + '258EAFA5-E914-47DA-95CA-C5AB0DC85B11').digest('base64');
    socket.on('error', () => {});
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    socket.on('data', () => socket.end(Buffer.from([0x88, 0x00])));
  });
}

function cli(args, input) {
  return new Promise((resolve, reject) => {
    const command = execFile(process.env.MEW_BROWSER_TEST_CLI, args, {
      timeout: 8000, killSignal: 'SIGKILL', maxBuffer: 8 * 1024 * 1024,
      env: { ...process.env, AGENT_BROWSER_IDLE_TIMEOUT_MS: '10000' },
    }, (error, stdout, stderr) => {
      commands.delete(command);
      if (error) reject(new Error(`agent-browser failed: ${stderr || error.message}`));
      else resolve(stdout.trim());
    });
    commands.add(command);
    if (input) command.stdin.end(input);
  });
}

function focusStatus(url, options = {}) {
  return new Promise((resolve, reject) => {
    const request = http.request(url, { method: 'POST', ...options }, (response) => {
      response.resume();
      response.once('end', () => resolve(response.statusCode));
    });
    request.setTimeout(3000, () => request.destroy(new Error('Browser focus request timed out.')));
    request.once('error', reject);
    request.end();
  });
}

async function closeCliSessions() {
  const failures = [];
  for (const name of [...cliSessions]) {
    try {
      await cli(['--session', name, 'close']);
      cliSessions.delete(name);
      console.log(`MEW_TEST_CLI_CLOSED ${name}`);
    } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, 'Failed to close test CLI sessions.');
}

async function agentTest(pages, endpoint) {
  const before = await fetch(`${endpoint}/json/list`).then((response) => response.json());
  const values = ['first baseline', 'second baseline'];
  let focusGuardsTested = false;
  assert.equal(pages.length, 2);
  async function focusFromChat(ready, index) {
    window.focus();
    await window.webContents.executeJavaScript("(() => { let chat = document.querySelector('#test-chat'); if (!chat) { chat = document.createElement('textarea'); chat.id = 'test-chat'; chat.value = 'chat unchanged'; chat.style.cssText='position:fixed;bottom:0;right:0'; document.body.appendChild(chat); } chat.focus(); })()", true);
    assert.equal(await window.webContents.executeJavaScript("document.activeElement.id"), 'test-chat');
    if (!focusGuardsTested) {
      const rejected = [
        [new URL('/unissued-capability', ready.focusUrl).href, {}, 404],
        [ready.focusUrl, { method: 'GET' }, 405],
        [ready.focusUrl, { headers: { Origin: 'https://attacker.invalid' } }, 403],
        [ready.focusUrl, { headers: { Host: 'attacker.invalid' } }, 403],
        [pages[1 - index].focusUrl, {}, 409],
      ];
      for (const [url, options, status] of rejected) {
        assert.equal(await focusStatus(url, options), status);
        assert.equal(await window.webContents.executeJavaScript("document.activeElement.id"), 'test-chat', 'denied focus requests must leave chat focused');
        assert.deepEqual(await notesValues(), values, 'denied focus requests must leave guest inputs unchanged');
      }
      focusGuardsTested = true;
    }
    assert.equal(await focusStatus(ready.focusUrl), 204);
    const focused = await Promise.all(['first', 'second'].map((tabId) => browsers.pages.get(browsers.key({ sessionId: 'session', tabId })).contents.executeJavaScript('document.hasFocus()')));
    assert.deepEqual(focused, index === 0 ? [true, false] : [false, true]);
  }
  async function notesValues() {
    return Promise.all(['first', 'second'].map((tabId) => browsers.pages.get(browsers.key({ sessionId: 'session', tabId })).contents.executeJavaScript("document.querySelector('#notes').value")));
  }
  try {
    for (const [index, tabId] of ['first', 'second'].entries()) {
      const ready = pages[index];
      const page = browsers.pages.get(browsers.key({ sessionId: 'session', tabId }));
      assert.equal(ready.pageInstance, page.pageInstance);
      assert.equal(ready.cdpEndpoint, endpoint);
      const focusUrl = new URL(ready.focusUrl);
      assert.equal(focusUrl.protocol, 'http:');
      assert.equal(focusUrl.hostname, '127.0.0.1');
      assert.notEqual(focusUrl.port, '');
      assert.ok(before.some((target) => target.id === ready.targetId && ['page', 'webview'].includes(target.type)));
      const name = `${process.env.MEW_BROWSER_TEST_SESSION_PREFIX}-${tabId}`;
      cliSessions.add(name);
      console.log(`MEW_TEST_CLI_SESSION ${name}`);
      const base = ['--session', name, '--cdp', endpoint];
      for (const pin of ['--no-pin-tab', '--pin-tab']) {
        const reply = JSON.parse(await cli([...base, pin, 'tab', ready.targetId, '--json']));
        assert.equal(reply.warning, undefined);
        assert.equal(reply.success, true);
        assert.equal(reply.data.targetId, ready.targetId);
      }
      await window.webContents.executeJavaScript(`document.querySelectorAll('webview').forEach((view, index) => view.style.display=index===${index}?'flex':'none')`);
      for (const tab of ['first', 'second']) {
        const current = browsers.pages.get(browsers.key({ sessionId: 'session', tabId: tab }));
        browsers.update({ sessionId: 'session', tabId: tab, url: current.url, visible: tab === tabId, interactive: tab === tabId, rect: { x: 0, y: 0, width: 400, height: 400 } });
      }
      const snapshot = JSON.parse(await cli([...base, '--pin-tab', 'snapshot', '--json']));
      assert.equal(snapshot.success, true);
      assert.ok(snapshot.data.snapshot.includes(`${tabId} unique`));
      assert.ok(!snapshot.data.snapshot.includes(`${tabId === 'first' ? 'second' : 'first'} unique`));
      await focusFromChat(ready, index);
      const text = tabId === 'first' ? 'agent first café 猫' : 'agent second naïve 雨';
      const replies = JSON.parse(await cli([...base, '--pin-tab', '--json', 'batch'], JSON.stringify([['fill', '#notes', text]])));
      assert.equal(replies.length, 1);
      assert.equal(replies[0].success, true);
      values[index] = text;
      assert.deepEqual(await notesValues(), values);
      await page.contents.executeJavaScript("document.querySelector('#notes').setSelectionRange(0, 5)");
      await focusFromChat(ready, index);
      const press = JSON.parse(await cli([...base, '--pin-tab', '--json', 'batch'], JSON.stringify([['press', 'Backspace']])));
      assert.equal(press.length, 1);
      assert.equal(press[0].success, true);
      values[index] = text.slice(5);
      assert.deepEqual(await notesValues(), values, 'press must preserve the selected guest input selection');
      assert.equal(await window.webContents.executeJavaScript("document.querySelector('#test-chat').value"), 'chat unchanged');
    }
    const after = await fetch(`${endpoint}/json/list`).then((response) => response.json());
    assert.deepEqual(after.map((target) => target.id).sort(), before.map((target) => target.id).sort(), 'binding must not create extra browser targets');
    return true;
  } finally { await closeCliSessions(); }
}

async function run() {
  const { BrowserHost } = require('../browser.cjs');
  await app.whenReady();
  const appSource = await fs.readFile(path.join(__dirname, 'browser-renderer.js'), 'utf8');
  const guestServer = await listen((request, response) => {
    response.writeHead(200, { 'Content-Type': 'text/html', 'X-Frame-Options': 'DENY' });
    if (request.url === '/probe') { counters.allowedHttp++; response.end('allowed'); return; }
    const stage = new URL(request.url, 'http://fixture').searchParams.get('stage') === 'latest' ? 'latest' : 'initial';
    response.end(`<p id="stage">${stage}</p><h1 id="identity">same page</h1><input id="notes" aria-label="notes"><script>document.title = JSON.stringify({node:typeof require,bridge:typeof mewQuery});</script>`);
  });
  websocket(guestServer, 'allowedWs');
  const controlServer = await listen((_request, response) => {
    counters.controlHttp++;
    response.writeHead(200, { 'Content-Type': 'text/plain' });
    response.end('host control reached');
  });
  websocket(controlServer, 'controlWs');
  const appServer = await listen((request, response) => {
    response.writeHead(200, { 'Content-Type': request.url === '/test.js' ? 'text/javascript' : 'text/html' });
    response.end(request.url === '/test.js' ? appSource : '<script src="/test.js" defer></script>');
  });
  let port;
  let debugPath;
  for (let attempt = 0; attempt < 100; attempt++) {
    try {
      const lines = (await fs.readFile(path.join(root, 'DevToolsActivePort'), 'utf8')).trim().split('\n');
      port = Number(lines[0]); debugPath = lines[1]; break;
    } catch { await new Promise((resolve) => setTimeout(resolve, 20)); }
  }
  if (!port || !debugPath) throw new Error('Missing DevTools endpoint.');
  const origin = `http://127.0.0.1:${appServer.address().port}`;
  const guestOrigin = `http://127.0.0.1:${guestServer.address().port}`;
  const controlOrigin = `http://127.0.0.1:${controlServer.address().port}`;
  const endpoint = `http://127.0.0.1:${port}`;
  const blocked = [];
  const guestSession = session.fromPartition('test-browser');
  guestSession.webRequest.onErrorOccurred((details) => blocked.push({ url: details.url, error: details.error }));
  const preload = path.join(root, 'preload.cjs');
  const productionPreload = await fs.readFile(path.join(__dirname, '../preload.cjs'), 'utf8');
  await fs.writeFile(preload, `${productionPreload}\ncontextBridge.exposeInMainWorld('runAgentTest', (pages) => ipcRenderer.invoke('test:agent', pages));\ncontextBridge.exposeInMainWorld('focusStatus', (url, options) => ipcRenderer.invoke('test:focus-status', url, options));\ncontextBridge.exposeInMainWorld('attachmentState', () => ipcRenderer.invoke('test:attachments'));\ncontextBridge.exposeInMainWorld('reportTest', (result) => ipcRenderer.send('test:result', result));`);
  window = new BrowserWindow({ show: inputTest, width: 1000, height: 700, webPreferences: { preload, sandbox: true, contextIsolation: true, nodeIntegration: false, webviewTag: true } });
  browsers = new BrowserHost(window.webContents, guestSession, endpoint, new Set([String(port), String(controlServer.address().port), String(appServer.address().port)]));
  await browsers.listen();
  const attachAttempts = [];
  window.webContents.on('will-attach-webview', (event, _preferences, params) => attachAttempts.push({ partition: params.partition, prevented: event.defaultPrevented }));
  const trusted = (event) => event.sender === window.webContents && event.senderFrame === window.webContents.mainFrame && event.senderFrame.url.startsWith(origin + '/');
  const cdpControlUrl = `${endpoint}/json/list`;
  const cdpWebsocketUrl = `ws://127.0.0.1:${port}${debugPath}`;
  ipcMain.handle('mew:query', (event, input) => {
    if (!trusted(event)) throw new Error('Untrusted sender');
    const request = parseRequest(input);
    if (request.type === 'bootstrap') return {
      inputTest, guestUrl: `${guestOrigin}/guest`, controlUrl: `${controlOrigin}/control`, controlWebsocketUrl: controlOrigin.replace('http:', 'ws:') + '/socket',
      allowedUrl: `${guestOrigin}/probe`, allowedWebsocketUrl: guestOrigin.replace('http:', 'ws:') + '/socket', cdpControlUrl, cdpWebsocketUrl,
    };
    if (request.type === 'browser_update') return browsers.update(request);
    if (request.type === 'browser_bind') return browsers.bind(request);
    if (request.type === 'browser_close') return browsers.close(request);
    throw new Error('Unsupported test request');
  });
  ipcMain.handle('test:attachments', (event) => {
    if (!trusted(event)) throw new Error('Untrusted sender');
    return { attached: browsers.attached.size, attempts: attachAttempts };
  });
  ipcMain.handle('test:agent', (event, pages) => {
    if (!trusted(event)) throw new Error('Untrusted sender');
    return agentTest(pages, endpoint);
  });
  ipcMain.handle('test:focus-status', (event, url, options) => {
    if (!trusted(event)) throw new Error('Untrusted sender');
    return focusStatus(url, options);
  });
  ipcMain.on('test:result', (event, result) => {
    if (!trusted(event)) { void finish(1); return; }
    if (result.error) { console.error(result.error); void finish(1); return; }
    const blockedCdp = [cdpControlUrl, cdpWebsocketUrl].every((url) => blocked.some((request) => request.url === url && request.error === 'net::ERR_BLOCKED_BY_CLIENT'));
    const blockedFocus = [result.focusUrl, result.focusUrl.replace('http:', 'ws:')].every((url) => blocked.some((request) => request.url === url && request.error === 'net::ERR_BLOCKED_BY_CLIENT'));
    delete result.focusUrl;
    console.log(`MEW_TEST_RESULT ${JSON.stringify({ ...result, ...counters, blockedCdp, blockedFocus })}`);
    void finish(0);
  });
  await window.loadURL(origin);
}

async function finish(code) {
  if (finishing) return;
  finishing = true;
  clearTimeout(timer);
  for (const command of commands) command.kill('SIGKILL');
  try { await closeCliSessions(); } catch (error) { console.error(error); code = 1; }
  try { await browsers?.shutdown(); } catch (error) { console.error(error); code = 1; }
  window?.destroy();
  for (const socket of sockets) socket.destroy();
  await Promise.all(servers.map((server) => new Promise((resolve) => server.close(resolve))));
  app.exit(code);
}
run().catch((error) => { console.error(error); void finish(1); });
app.on('quit', () => require('node:fs').rmSync(root, { recursive: true, force: true }));
