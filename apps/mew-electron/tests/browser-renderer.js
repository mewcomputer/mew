const query = (request) => new Promise((resolve, reject) => window.mewQuery({ request: JSON.stringify(request), onSuccess: (value) => resolve(JSON.parse(value)), onFailure: (_code, message) => reject(new Error(message)) }));
const ready = new Map();
window.mewListen((event) => {
  if (event.type !== 'browser') return;
  if (event.detail.kind === 'error') window.reportTest({ error: event.detail.message });
  if (event.detail.kind === 'ready') ready.set(event.detail.tabId, event.detail);
  if (event.detail.kind === 'closed' && ready.get(event.detail.tabId)?.pageInstance === event.detail.pageInstance) ready.delete(event.detail.tabId);
});

async function waitFor(predicate) {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error('Timed out waiting for guest state');
}

async function rejects(request) {
  try { await query(request); return false; }
  catch { return true; }
}

function viewFor(permit, url) {
  const view = document.createElement('webview');
  view.setAttribute('partition', permit.partition);
  view.setAttribute('src', url);
  view.style.cssText = 'display:flex;width:400px;height:400px';
  return view;
}

async function attach(owner, permit, url) {
  const view = viewFor(permit, url);
  const loaded = new Promise((resolve, reject) => {
    const timer = setTimeout(() => finish(new Error('A valid browser attachment permit did not attach.')), 5000);
    const failed = (event) => { if (event.errorCode !== -3) finish(new Error(event.errorDescription)); };
    const finish = (error) => {
      clearTimeout(timer);
      view.removeEventListener('did-fail-load', failed);
      if (error) reject(error); else resolve();
    };
    view.addEventListener('dom-ready', () => {
      query({ type: 'browser_bind', ...owner, pageInstance: permit.pageInstance, guestId: view.getWebContentsId() }).then(() => finish(), finish);
    }, { once: true });
    view.addEventListener('did-fail-load', failed);
  });
  document.body.appendChild(view);
  await loaded;
  await waitFor(() => ready.get(owner.tabId)?.pageInstance === permit.pageInstance);
  return view;
}

async function rejectedAttachment(permit, url) {
  const before = await window.attachmentState();
  const view = viewFor(permit, url);
  document.body.appendChild(view);
  let state;
  await waitFor(async () => {
    state = await window.attachmentState();
    return state.attempts.length > before.attempts.length;
  });
  let guestId;
  try { guestId = view.getWebContentsId(); } catch { guestId = null; }
  view.remove();
  return state.attempts.at(-1).prevented === true && state.attached === before.attached && !guestId;
}

async function attachWithoutBinding(permit, url) {
  const view = viewFor(permit, url);
  const loaded = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Browser guest did not attach.')), 5000);
    view.addEventListener('dom-ready', () => { clearTimeout(timer); resolve(); }, { once: true });
    view.addEventListener('did-fail-load', (event) => {
      if (event.errorCode !== -3) { clearTimeout(timer); reject(new Error(event.errorDescription)); }
    }, { once: true });
  });
  document.body.appendChild(view);
  await loaded;
  return view;
}

async function run() {
  document.body.style.cssText = 'display:flex;margin:0';
  const settings = await query({ type: 'bootstrap' });
  const { guestUrl } = settings;
  const crossedOwners = ['cross-first', 'cross-second'].map((tabId) => ({ sessionId: 'session', tabId }));
  const crossedPermits = await Promise.all(crossedOwners.map((owner) => query({
    type: 'browser_update', ...owner, url: guestUrl, visible: true, interactive: true,
    rect: { x: 0, y: 0, width: 400, height: 400 },
  })));
  const crossedViews = await Promise.all(crossedPermits.map((permit) => attachWithoutBinding(permit, guestUrl)));
  for (const [index, owner] of crossedOwners.entries()) {
    if (!await rejects({ type: 'browser_bind', ...owner, pageInstance: crossedPermits[index].pageInstance, guestId: crossedViews[1 - index].getWebContentsId() })) {
      throw new Error('A guest was bound to another tab before either guest was bound.');
    }
  }
  for (const [index, owner] of crossedOwners.entries()) {
    await query({ type: 'browser_bind', ...owner, pageInstance: crossedPermits[index].pageInstance, guestId: crossedViews[index].getWebContentsId() });
    await query({ type: 'browser_close', ...owner });
    crossedViews[index].remove();
  }
  // A typed domain serializes with a trailing slash, so the guest's src never
  // matches the request verbatim.
  const rootOwner = { sessionId: 'session', tabId: 'root-url' };
  const rootUrl = new URL(guestUrl).origin;
  const rootPermit = await query({ type: 'browser_update', ...rootOwner, url: rootUrl, visible: true, rect: { x: 0, y: 0, width: 400, height: 400 } });
  const rootView = await attachWithoutBinding(rootPermit, rootUrl);
  const rootUrlAttached = await rootView.executeJavaScript('location.pathname') === '/';
  await query({ type: 'browser_close', ...rootOwner });
  rootView.remove();
  const views = [];
  const permits = [];
  for (const tabId of ['first', 'second']) {
    const owner = { sessionId: 'session', tabId };
    const permit = await query({ type: 'browser_update', ...owner, url: guestUrl, visible: true, interactive: true, rect: { x: 0, y: 0, width: 400, height: 400 } });
    permits.push(permit);
    views.push(await attach(owner, permit, guestUrl));
  }
  const initialPages = ['first', 'second'].map((tabId) => ready.get(tabId));
  const focusUrl = initialPages[0].focusUrl;
  if (typeof focusUrl !== 'string' || focusUrl === initialPages[1].focusUrl) throw new Error('Browser guests require distinct focus capabilities.');
  let focusEndpointGuards = await window.focusStatus(focusUrl, { method: 'GET' }) === 405
    && await window.focusStatus(focusUrl, { headers: { Origin: 'https://attacker.invalid' } }) === 403
    && await window.focusStatus(focusUrl, { headers: { Host: 'attacker.invalid' } }) === 403
    && await window.focusStatus(new URL('/unissued-capability', focusUrl).href) === 404;
  const firstUpdate = { type: 'browser_update', sessionId: 'session', tabId: 'first', url: guestUrl, rect: { x: 0, y: 0, width: 400, height: 400 } };
  await query({ ...firstUpdate, visible: true, interactive: false });
  focusEndpointGuards = focusEndpointGuards && await window.focusStatus(focusUrl) === 409;
  await query({ ...firstUpdate, visible: false, interactive: false });
  focusEndpointGuards = focusEndpointGuards && await window.focusStatus(focusUrl) === 409;
  await query({ ...firstUpdate, visible: true, interactive: true });
  const [first, second] = views;
  const isolated = JSON.parse(first.getTitle());
  await first.executeJavaScript("document.querySelector('#notes').value='keep me'; document.cookie='shared=yes'; localStorage.setItem('shared','yes')");
  first.style.visibility = 'hidden';
  first.style.visibility = 'visible';
  const retained = await first.executeJavaScript("document.querySelector('#notes').value === 'keep me'");
  const sharedStorage = await second.executeJavaScript("document.cookie.includes('shared=yes') && localStorage.getItem('shared') === 'yes'");
  await first.executeJavaScript("document.querySelector('#identity').textContent='first unique'; document.querySelector('#notes').value='first baseline'");
  await second.executeJavaScript("document.querySelector('#identity').textContent='second unique'; document.querySelector('#notes').value='second baseline'");
  const ownerChecks = await rejects({ type: 'browser_bind', sessionId: 'other-session', tabId: 'first', pageInstance: permits[0].pageInstance, guestId: first.getWebContentsId() })
    && await rejects({ type: 'browser_bind', sessionId: 'session', tabId: 'first', pageInstance: 'stale-generation', guestId: first.getWebContentsId() })
    && await rejects({ type: 'browser_bind', sessionId: 'session', tabId: 'first', pageInstance: permits[0].pageInstance, guestId: second.getWebContentsId() });
  const permitReuseBlocked = await rejectedAttachment(permits[0], guestUrl)
    && await rejectedAttachment({ partition: 'unissued-attachment-permit' }, guestUrl);
  const controlBlocked = await first.executeJavaScript(`(async () => {
    const settings = ${JSON.stringify({ ...settings, focusUrl })};
    const fetchAllowed = (url) => fetch(url, { mode: 'no-cors', cache: 'no-store' }).then(() => true, () => false);
    const websocketAllowed = (url) => new Promise((resolve) => {
      const socket = new WebSocket(url);
      const timer = setTimeout(() => { socket.close(); resolve(false); }, 2000);
      const finish = (allowed) => { clearTimeout(timer); socket.close(); resolve(allowed); };
      socket.onopen = () => finish(true);
      socket.onerror = () => finish(false);
    });
    const results = await Promise.all([
      fetchAllowed(settings.allowedUrl), websocketAllowed(settings.allowedWebsocketUrl),
      fetchAllowed(settings.controlUrl), websocketAllowed(settings.controlWebsocketUrl),
      fetchAllowed(settings.cdpControlUrl), websocketAllowed(settings.cdpWebsocketUrl),
      fetch(settings.focusUrl, { method: 'POST', mode: 'no-cors' }).then(() => true, () => false),
      websocketAllowed(settings.focusUrl.replace('http:', 'ws:')),
    ]);
    return JSON.stringify(results) === JSON.stringify([true, true, false, false, false, false, false, false]);
  })()`);
  const raceOwner = { sessionId: 'session', tabId: 'navigation-race' };
  const initialUrl = `${guestUrl}?stage=initial`;
  const latestUrl = `${guestUrl}?stage=latest`;
  const racePermit = await query({ type: 'browser_update', ...raceOwner, url: initialUrl, visible: true, rect: { x: 0, y: 0, width: 400, height: 400 } });
  const updatedPermit = await query({ type: 'browser_update', ...raceOwner, url: latestUrl, visible: true, rect: { x: 0, y: 0, width: 400, height: 400 } });
  const raceView = await attach(raceOwner, racePermit, initialUrl);
  await waitFor(async () => {
    try { return raceView.getURL() === latestUrl && await raceView.executeJavaScript("document.querySelector('#stage')?.textContent") === 'latest'; }
    catch { return false; }
  });
  const pendingNavigationKeptPermit = racePermit.partition === updatedPermit.partition && racePermit.pageInstance === updatedPermit.pageInstance;
  await raceView.executeJavaScript("document.querySelector('#notes').value='retain navigation state'; location.hash='retained'");
  const retainedUrl = `${latestUrl}#retained`;
  await waitFor(() => raceView.getURL() === retainedUrl);
  let staleNavigation = false;
  const trackNavigation = (event) => { if (event.url !== retainedUrl) staleNavigation = true; };
  raceView.addEventListener('did-start-navigation', trackNavigation);
  await query({ type: 'browser_update', ...raceOwner, url: latestUrl, visible: false, rect: { x: 0, y: 0, width: 0, height: 0 } });
  const retainedState = await raceView.executeJavaScript("new Promise((resolve) => setTimeout(() => resolve({ url: location.href, notes: document.querySelector('#notes')?.value }), 100))");
  raceView.removeEventListener('did-start-navigation', trackNavigation);
  const hiddenCleanupKeptNavigation = !staleNavigation && retainedState.url === retainedUrl && retainedState.notes === 'retain navigation state';
  await query({ type: 'browser_close', ...raceOwner });
  raceView.remove();
  await waitFor(async () => (await window.attachmentState()).attached === 2);
  const agentPinned = settings.inputTest ? await window.runAgentTest(initialPages) : undefined;
  const oldGuestId = first.getWebContentsId();
  await query({ type: 'browser_close', sessionId: 'session', tabId: 'first' });
  first.remove();
  let staleFocusRevoked = await window.focusStatus(focusUrl) === 404;
  const freshPermit = await query({ type: 'browser_update', sessionId: 'session', tabId: 'first', url: guestUrl, visible: true, rect: { x: 0, y: 0, width: 400, height: 400 } });
  const staleRejected = await rejects({ type: 'browser_bind', sessionId: 'session', tabId: 'first', pageInstance: permits[0].pageInstance, guestId: oldGuestId });
  const reopened = await attach({ sessionId: 'session', tabId: 'first' }, freshPermit, guestUrl);
  staleFocusRevoked = staleFocusRevoked && ready.get('first').focusUrl !== focusUrl && await window.focusStatus(focusUrl) === 404;
  const generationChanged = staleRejected && freshPermit.pageInstance !== permits[0].pageInstance && ready.get('first').targetId !== initialPages[0].targetId
    && await rejects({ type: 'browser_bind', sessionId: 'session', tabId: 'first', pageInstance: permits[0].pageInstance, guestId: reopened.getWebContentsId() });
  for (const tabId of ['first', 'second']) await query({ type: 'browser_close', sessionId: 'session', tabId });
  reopened.remove();
  second.remove();
  window.reportTest({ guests: initialPages.length, distinctTargets: initialPages[0].targetId !== initialPages[1].targetId,
    retained, isolated: isolated.node === 'undefined' && isolated.bridge === 'undefined', controlBlocked, sharedStorage,
    agentPinned, ownerChecks, permitReuseBlocked, generationChanged, pendingNavigationKeptPermit, hiddenCleanupKeptNavigation,
    focusEndpointGuards, staleFocusRevoked, rootUrlAttached, focusUrl });
}
run().catch((error) => window.reportTest({ error: error.stack }));
