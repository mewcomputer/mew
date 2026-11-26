const { test } = require('node:test');
const assert = require('node:assert/strict');
const { parseRequest, isAppUrl, hostPermissionAllowed, contextMenuTemplate, guestRequestAllowed, browserProtection, debugEndpoint, canonicalUrl } = require('./policy.cjs');

test('host requests reject malformed, oversized and ambiguous input', () => {
  for (const value of [null, '{}', 'null', '{', JSON.stringify({ type: 'bootstrap', extra: true }), ' '.repeat(16385)]) {
    assert.throws(() => parseRequest(value));
  }
  assert.deepEqual(parseRequest('{"type":"bootstrap"}'), { type: 'bootstrap' });
});

test('browser updates validate identities, navigation and finite bounds', () => {
  const request = { type: 'browser_update', sessionId: 'session', tabId: 'tab', url: 'https://example.com/', rect: { x: 0, y: 1, width: 400, height: 600 }, visible: true };
  assert.deepEqual(parseRequest(JSON.stringify(request)), request);
  for (const change of [{ sessionId: '' }, { tabId: 1 }, { url: 'file:///etc/passwd' }, { url: 'https://u:p@example.com' }, { visible: 'yes' }, { rect: { ...request.rect, width: -1 } }]) {
    assert.throws(() => parseRequest(JSON.stringify({ ...request, ...change })));
  }
  assert.throws(() => parseRequest(JSON.stringify({ ...request, rect: { ...request.rect, width: null } })));
  assert.deepEqual(parseRequest(JSON.stringify({ type: 'browser_bind', sessionId: 's', tabId: 't', pageInstance: 'p', guestId: 42 })), { type: 'browser_bind', sessionId: 's', tabId: 't', pageInstance: 'p', guestId: 42 });
  assert.throws(() => parseRequest('{"type":"browser_bind","sessionId":"s","tabId":"t","pageInstance":"p","guestId":0}'));
});

test('only the exact local app origin is privileged', () => {
  const appUrl = 'http://127.0.0.1:41001/';
  assert.equal(isAppUrl('http://127.0.0.1:41001/chat/test', appUrl), true);
  for (const value of ['http://localhost:41001/', 'http://127.0.0.1:41002/', 'http://user@127.0.0.1:41001/', 'file:///app.html', 'garbage']) {
    assert.equal(isAppUrl(value, appUrl), false);
  }
  assert.equal(isAppUrl('mew://app/session/test', 'mew://app/'), true);
  for (const value of ['mew://other/', 'file:///app.html', 'https://app/', 'mew://user@app/']) {
    assert.equal(isAppUrl(value, 'mew://app/'), false);
  }
});

test('host session allows only sanitized clipboard writes for the app origin', () => {
  for (const origin of ['mew://app/', 'mew://app/session/test', 'mew://app']) {
    assert.equal(hostPermissionAllowed('clipboard-sanitized-write', origin, 'mew://app/'), true);
  }
  for (const origin of ['mew://other/', 'https://example.com/', '', undefined]) {
    assert.equal(hostPermissionAllowed('clipboard-sanitized-write', origin, 'mew://app/'), false);
  }
  for (const permission of ['clipboard-read', 'notifications', 'geolocation', 'media']) {
    assert.equal(hostPermissionAllowed(permission, 'mew://app/', 'mew://app/'), false);
  }
});

test('context menu shows copy and edit actions only when applicable', () => {
  assert.deepEqual(contextMenuTemplate({ isEditable: false, selectionText: '' }), []);
  assert.deepEqual(contextMenuTemplate({
    isEditable: false,
    selectionText: 'hello',
    editFlags: { canCopy: true, canSelectAll: true },
  }), [
    { role: 'copy', enabled: true },
    { type: 'separator' },
    { role: 'selectAll', enabled: true },
  ]);
  assert.deepEqual(contextMenuTemplate({
    isEditable: true,
    selectionText: '',
    editFlags: { canCut: false, canCopy: false, canPaste: true, canSelectAll: true },
  }), [
    { role: 'cut', enabled: false },
    { role: 'copy', enabled: false },
    { role: 'paste', enabled: true },
    { type: 'separator' },
    { role: 'selectAll', enabled: true },
  ]);
});

test('guest traffic cannot reach host control ports through aliases or websockets', () => {
  const ports = new Set(['41001', '41002', '41003']);
  for (const host of ['127.0.0.1', 'localhost', '[::1]', '2130706433', 'untrusted.example']) {
    for (const protocol of ['http:', 'https:', 'ws:', 'wss:']) {
      assert.equal(guestRequestAllowed(`${protocol}//${host}:41002/path`, ports), false);
    }
  }
  for (const value of ['https://example.com/', 'http://127.0.0.1:42000/', 'wss://example.com/socket', 'data:image/png;base64,AA==', 'blob:https://example.com/123', 'about:blank']) {
    assert.equal(guestRequestAllowed(value, ports), true);
  }
  for (const value of ['file:///etc/passwd', 'javascript:alert(1)', 'devtools://devtools/', 'ftp://example.com/', 'garbage', 'https://user:password@example.com/']) {
    assert.equal(guestRequestAllowed(value, ports), false);
  }
  assert.equal(guestRequestAllowed('http://localhost/', new Set(['80'])), false);
  assert.equal(guestRequestAllowed('wss://localhost/', new Set(['443'])), false);
  assert.equal(guestRequestAllowed('data:text/plain,hello', new Set(['80', '443'])), true);
});

test('debugging endpoint accepts only an actual ephemeral port file', () => {
  assert.equal(debugEndpoint('43210\n/devtools/browser/abc\n'), 'http://127.0.0.1:43210');
  for (const value of ['', '0\n/devtools/browser/a', '65536\n/devtools/browser/a', '43210', 'abc\n/devtools/browser/a']) {
    assert.throws(() => debugEndpoint(value));
  }
});

test('daemon ports remain protected across aliases and standard ports disable browsing', () => {
  for (const daemon of ['wss://daemon.example/', 'ws://daemon.example/']) {
    const { ports, unavailable } = browserProtection('http://127.0.0.1:41001/', daemon, 'http://127.0.0.1:41003/');
    assert.match(unavailable, /browser.*unavailable/i);
    assert.equal(guestRequestAllowed(daemon, ports), false);
  }
  for (const daemon of ['ws://localhost.:41002/', 'ws://[::ffff:127.0.0.1]:41002/', 'ws://daemon.example:41002/']) {
    const { ports, unavailable } = browserProtection('http://127.0.0.1:41001/', daemon, 'http://127.0.0.1:41003/');
    assert.equal(unavailable, null);
    assert.equal(guestRequestAllowed('http://rebind.example:41002/', ports), false);
    assert.equal(guestRequestAllowed('ws://127.0.0.1:41002/', ports), false);
  }
});

test('the private relay port is protected alongside the underlying daemon', () => {
  const { ports, unavailable } = browserProtection(
    'http://127.0.0.1:41001/',
    'ws://daemon.example:41002/',
    'http://127.0.0.1:41003/',
    'ws://127.0.0.1:41004/private',
  );
  assert.equal(unavailable, null);
  assert.equal(guestRequestAllowed('ws://127.0.0.1:41004/', ports), false);
  assert.equal(guestRequestAllowed('ws://daemon.example:41002/', ports), false);
});

test('page urls use the serialization a webview src sends back', () => {
  assert.equal(canonicalUrl('https://example.com'), 'https://example.com/');
  assert.equal(canonicalUrl('https://example.com/a?b=1#c'), 'https://example.com/a?b=1#c');
  assert.equal(canonicalUrl('about:blank'), 'about:blank');
});
