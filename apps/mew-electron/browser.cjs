const { randomUUID } = require('node:crypto');
const http = require('node:http');
const { guestRequestAllowed, canonicalUrl } = require('./policy.cjs');

// Electron rejects a loadURL superseded by the next navigation with ERR_ABORTED.
// The new load is the one the user asked for, so this is not a browser error.
function isAbort(error) {
  return error?.code === 'ERR_ABORTED' || error?.errno === -3;
}

// DOM guests share browser storage, but never the privileged renderer's Session.
class BrowserHost {
  constructor(parent, guestSession, endpoint, protectedPorts, unavailable) {
    this.parent = parent;
    this.session = guestSession;
    this.endpoint = endpoint;
    this.ports = protectedPorts;
    this.unavailable = unavailable;
    this.pages = new Map();
    this.attached = new Map();
    guestSession.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
    guestSession.setPermissionCheckHandler(() => false);
    guestSession.on('will-download', (event) => event.preventDefault());
    guestSession.webRequest.onBeforeRequest((details, callback) => callback({ cancel: !guestRequestAllowed(details.url, this.ports) }));
    parent.on('will-attach-webview', (event, preferences, params) => {
      const page = this.pages.values().find((entry) => entry.partition === params.partition);
      // `params.src` is Electron's serialization of the requested URL, so the
      // permit has to have stored the same form or every typed domain fails.
      if (!page || page.consumed || !Number.isSafeInteger(params.instanceId) || params.instanceId <= 0
        || this.pages.values().some((entry) => entry.consumed && entry.viewInstanceId === params.instanceId)
        || params.src !== page.initialUrl || !guestRequestAllowed(params.src, this.ports)) {
        event.preventDefault();
        return;
      }
      page.consumed = true;
      page.viewInstanceId = params.instanceId;
      delete preferences.preload;
      delete preferences.preloadURL;
      delete preferences.additionalArguments;
      Object.assign(preferences, {
        session: guestSession, sandbox: true, contextIsolation: true,
        nodeIntegration: false, nodeIntegrationInWorker: false, nodeIntegrationInSubFrames: false,
        webviewTag: false, webSecurity: true, allowRunningInsecureContent: false,
      });
      delete params.allowpopups;
    });
    parent.on('did-attach-webview', (_event, contents) => {
      // Electron 45 copies params.instanceId onto the guest before this event.
      // Reject an unknown identity rather than accepting another tab's guest.
      const page = this.pages.values().find((entry) => entry.consumed
        && entry.viewInstanceId === contents.viewInstanceId && !entry.attachedContents);
      if (!page) { contents.close(); return; }
      page.attachedContents = contents;
      this.attached.set(contents.id, contents);
      contents.setWindowOpenHandler(() => ({ action: 'deny' }));
      const limit = (event) => { if (!guestRequestAllowed(event.url, this.ports)) event.preventDefault(); };
      contents.on('will-navigate', limit);
      contents.on('will-frame-navigate', limit);
      contents.on('will-redirect', limit);
      contents.once('destroyed', () => {
        this.attached.delete(contents.id);
        for (const [key, page] of this.pages) {
          if (page.attachedContents !== contents) continue;
          this.pages.delete(key);
          this.emit(page, 'closed');
        }
      });
    });
  }

  async listen() {
    this.server = http.createServer((request, response) => {
      const finish = (status, message = '') => {
        response.writeHead(status, { 'Connection': 'close', 'Content-Type': 'text/plain' });
        response.end(message);
      };
      request.resume();
      if (request.headers.host !== new URL(this.focusOrigin).host || request.headers.origin !== undefined) return finish(403);
      if (request.method !== 'POST') return finish(405);
      if (request.headers['transfer-encoding'] || Number(request.headers['content-length'] ?? 0) !== 0) return finish(400);
      const page = this.pages.values().find((entry) => entry.focusPath === request.url);
      if (!page) return finish(404);
      void this.focus(page).then(() => finish(204), (error) => finish(409, error.message));
    });
    await new Promise((resolve, reject) => {
      this.server.once('error', reject);
      this.server.listen(0, '127.0.0.1', resolve);
    });
    this.focusOrigin = `http://127.0.0.1:${this.server.address().port}`;
    this.ports.add(String(this.server.address().port));
    this.server.unref();
  }

  async focus(page) {
    const current = () => this.pages.get(this.key(page)) === page && page.visible && page.interactive
      && page.contents && !page.contents.isDestroyed();
    if (!current()) throw new Error('The browser page is no longer active.');
    const window = this.parent.getOwnerBrowserWindow();
    if (!window || window.isDestroyed()) throw new Error('The browser window is unavailable.');
    window.focus();
    // CDP keyboard input follows the embedder's focused widget. Transfer through
    // the outer document before focusing the guest's actual iframe, preserving
    // its active control and selection. Guest webContents.focus() is a no-op.
    const focused = await this.parent.executeJavaScript(`(() => {
      const view = document.querySelectorAll('webview').values().find((element) => element.getWebContentsId() === ${page.contents.id});
      const frame = view?.shadowRoot?.querySelector('iframe');
      if (!frame || !view.getClientRects().length || view.closest('[hidden], [inert]')) return false;
      const reset = document.createElement('textarea');
      reset.tabIndex = -1;
      reset.style.cssText = 'position:fixed;width:1px;height:1px;opacity:0;pointer-events:none';
      document.body.appendChild(reset);
      try {
        reset.focus({ preventScroll: true });
        frame.focus({ preventScroll: true });
        return document.activeElement === view;
      } finally { reset.remove(); }
    })()`, true);
    if (!focused) throw new Error('The browser page is no longer visible.');
    for (let attempt = 0; attempt < 25; attempt++) {
      if (!current()) throw new Error('The browser page is no longer active.');
      if (await page.contents.executeJavaScript('document.hasFocus()') && current()) return;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    throw new Error('The browser page could not receive keyboard focus.');
  }

  key(request) { return JSON.stringify([request.sessionId, request.tabId]); }

  emit(page, kind, extra = {}) {
    if (this.parent.isDestroyed()) return;
    const contents = page.contents;
    this.parent.send('mew:event', { type: 'browser', detail: {
      sessionId: page.sessionId, tabId: page.tabId, pageInstance: page.pageInstance,
      url: contents && !contents.isDestroyed() ? contents.getURL() || page.url : page.url,
      title: contents && !contents.isDestroyed() ? contents.getTitle() : '', kind, ...extra,
    } });
  }

  update(request) {
    if (this.unavailable && request.visible) throw new Error(this.unavailable);
    const key = this.key(request);
    const url = canonicalUrl(request.url);
    let page = this.pages.get(key);
    if (!page) {
      if (!request.visible) return {};
      if (!guestRequestAllowed(request.url, this.ports)) throw new Error('Browser navigation to a host control endpoint is blocked.');
      page = {
        sessionId: request.sessionId, tabId: request.tabId, url, initialUrl: url,
        focusPath: `/${randomUUID()}`,
        partition: `mew-attach-${randomUUID()}`, pageInstance: randomUUID(), consumed: false,
      };
      this.pages.set(key, page);
    } else if (request.visible && page.url !== url) {
      if (!guestRequestAllowed(request.url, this.ports)) throw new Error('Browser navigation to a host control endpoint is blocked.');
      page.url = url;
      if (page.contents && !page.contents.isDestroyed()) {
        void page.contents.loadURL(page.url).catch((error) => {
          if (this.pages.get(key) === page && !isAbort(error)) this.emit(page, 'error', { message: error.message });
        });
      }
    }
    page.visible = request.visible;
    page.interactive = request.interactive ?? request.visible;
    return { partition: page.partition, pageInstance: page.pageInstance };
  }

  async bind(request) {
    const key = this.key(request);
    const page = this.pages.get(key);
    const contents = this.attached.get(request.guestId);
    if (!page || page.pageInstance !== request.pageInstance || !page.consumed || !contents || contents.isDestroyed()
      || page.attachedContents !== contents || contents.hostWebContents !== this.parent || contents.session !== this.session) {
      throw new Error('The browser page is no longer available.');
    }
    if (page.contents) {
      if (page.contents !== contents) throw new Error('The browser page is already attached.');
      return {};
    }
    page.contents = contents;
    const desiredUrl = page.url;
    const current = () => this.pages.get(key) === page && !contents.isDestroyed();
    const changed = () => {
      if (!current()) return;
      page.url = contents.getURL() || page.url;
      this.emit(page, 'changed');
    };
    contents.on('did-navigate', changed);
    contents.on('did-navigate-in-page', changed);
    contents.on('page-title-updated', changed);
    contents.on('render-process-gone', (_event, details) => {
      if (current()) this.emit(page, 'error', { message: `Browser renderer stopped (${details.reason}). Close and reopen this tab to retry.` });
    });
    contents.on('did-fail-load', (_event, code, description, _url, mainFrame) => {
      if (current() && mainFrame && code !== -3) this.emit(page, 'error', { message: description });
    });
    try {
      if (contents.getURL() !== desiredUrl) await contents.loadURL(desiredUrl);
      contents.debugger.attach('1.3');
      const { targetInfo } = await contents.debugger.sendCommand('Target.getTargetInfo');
      const targets = await fetch(`${this.endpoint}/json/list`, { signal: AbortSignal.timeout(5000) }).then((response) => response.json());
      if (!targets.some((target) => target.id === targetInfo.targetId && ['page', 'webview'].includes(target.type))) {
        throw new Error('The browser did not expose an exact page target.');
      }
      if (current()) this.emit(page, 'ready', { cdpEndpoint: this.endpoint, targetId: targetInfo.targetId, focusUrl: `${this.focusOrigin}${page.focusPath}` });
    } catch (error) {
      if (current() && !isAbort(error)) this.emit(page, 'error', { message: error.message });
    } finally {
      if (!contents.isDestroyed() && contents.debugger.isAttached()) contents.debugger.detach();
    }
    return {};
  }

  close(request) {
    const key = this.key(request);
    const page = this.pages.get(key);
    if (!page) return {};
    this.pages.delete(key);
    this.emit(page, 'closed');
    if (page.attachedContents && !page.attachedContents.isDestroyed()) page.attachedContents.close();
    return {};
  }

  dispose() {
    for (const page of this.pages.values()) this.close(page);
    for (const contents of this.attached.values()) if (!contents.isDestroyed()) contents.close();
    this.attached.clear();
  }

  shutdown() {
    this.dispose();
    this.server?.close();
    this.server?.closeAllConnections();
  }
}

module.exports = { BrowserHost, isAbort };
