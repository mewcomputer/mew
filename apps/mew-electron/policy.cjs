function object(value, keys) {
  return value && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).every((key) => keys.includes(key));
}

function identity(value) {
  return typeof value === 'string' && value.trim().length > 0 && Buffer.byteLength(value) <= 256;
}

function navigationUrl(value) {
  if (typeof value !== 'string' || value.length > 8192 || /\s/.test(value)) return false;
  try {
    const url = new URL(value);
    return value === 'about:blank' || (['http:', 'https:'].includes(url.protocol) && !url.username && !url.password);
  } catch { return false; }
}

function parseRequest(value) {
  if (typeof value !== 'string' || Buffer.byteLength(value) > 16384) throw new Error('Invalid host request.');
  const request = JSON.parse(value);
  let valid = false;
  switch (request?.type) {
    case 'bootstrap':
      valid = object(request, ['type']);
      break;
    case 'set_theme':
      valid = object(request, ['type', 'themeMode', 'lightTheme', 'darkTheme'])
        && ['system', 'light', 'dark'].includes(request.themeMode)
        && identity(request.lightTheme) && identity(request.darkTheme);
      break;
    case 'browser_close':
      valid = object(request, ['type', 'sessionId', 'tabId']) && identity(request.sessionId) && identity(request.tabId);
      break;
    case 'browser_bind':
      valid = object(request, ['type', 'sessionId', 'tabId', 'pageInstance', 'guestId'])
        && identity(request.sessionId) && identity(request.tabId) && identity(request.pageInstance)
        && Number.isSafeInteger(request.guestId) && request.guestId > 0;
      break;
    case 'browser_update': {
      const rect = request.rect;
      valid = object(request, ['type', 'sessionId', 'tabId', 'url', 'rect', 'visible', 'interactive'])
        && identity(request.sessionId) && identity(request.tabId) && navigationUrl(request.url)
        && typeof request.visible === 'boolean' && (request.interactive === undefined || typeof request.interactive === 'boolean')
        && object(rect, ['x', 'y', 'width', 'height']) && ['x', 'y', 'width', 'height'].every((key) => Number.isFinite(rect[key]))
        && rect.width >= 0 && rect.height >= 0 && (!request.visible || (rect.width > 0 && rect.height > 0));
      break;
    }
  }
  if (!valid) throw new Error('Invalid host request.');
  return request;
}

function isAppUrl(value, appUrl) {
  try {
    const url = new URL(value);
    const app = new URL(appUrl);
    return url.protocol === app.protocol && url.host === app.host && !url.username && !url.password;
  } catch { return false; }
}

function endpointPort(url) {
  return url.port || (['https:', 'wss:'].includes(url.protocol) ? '443' : '80');
}

function browserProtection(appUrl, daemonUrl, debugUrl, relayUrl) {
  const daemon = new URL(daemonUrl);
  const daemonPort = endpointPort(daemon);
  const ports = new Set([endpointPort(new URL(appUrl)), daemonPort, endpointPort(new URL(debugUrl))]);
  if (relayUrl) ports.add(endpointPort(new URL(relayUrl)));
  const unavailable = ['80', '443'].includes(daemonPort)
    ? `The browser is unavailable while the daemon uses port ${daemonPort}. Connect to a daemon on another port to use browser tabs.`
    : null;
  return { ports, unavailable };
}

function guestRequestAllowed(value, protectedPorts) {
  try {
    const url = new URL(value);
    const port = endpointPort(url);
    if (url.username || url.password || (['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol) && protectedPorts.has(port))) return false;
    return ['http:', 'https:', 'ws:', 'wss:', 'data:', 'blob:'].includes(url.protocol) || value === 'about:blank';
  } catch { return false; }
}

function debugEndpoint(contents) {
  const [value, path] = contents.trim().split('\n');
  const port = Number(value);
  if (!Number.isInteger(port) || port < 1 || port > 65535 || !path?.startsWith('/devtools/browser/')) {
    throw new Error('Invalid Chromium debugging endpoint.');
  }
  return `http://127.0.0.1:${port}`;
}

module.exports = { parseRequest, isAppUrl, guestRequestAllowed, browserProtection, navigationUrl, debugEndpoint };
