const { spawn } = require('node:child_process');
const { randomUUID } = require('node:crypto');

const MAX_REQUEST_BYTES = 16_384;
const MAX_RESPONSE_BYTES = 4 * 1024 * 1024;

function isBootstrap(value) {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  try {
    const daemon = new URL(value.daemonUrl);
    if (!['ws:', 'wss:'].includes(daemon.protocol)) return false;
    const relay = new URL(value.websocketUrl);
    if (relay.protocol !== 'ws:' || relay.hostname !== '127.0.0.1' || !relay.port
      || relay.pathname === '/' || relay.username || relay.password || relay.search || relay.hash) return false;
  } catch { return false; }
  return ['system', 'light', 'dark'].includes(value.themeMode)
    && typeof value.lightTheme === 'string'
    && typeof value.darkTheme === 'string'
    && Array.isArray(value.themes)
    && value.themes.every((theme) => theme && typeof theme.id === 'string'
      && ['light', 'dark'].includes(theme.mode) && typeof theme.css === 'string');
}

function isAppUrl(value) {
  try {
    const url = new URL(value);
    return url.protocol === 'http:' && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname)
      && url.port !== '' && url.username === '' && url.password === ''
      && url.pathname === '/' && url.search === '' && url.hash === '';
  } catch { return false; }
}

async function startSupport(binary, {
  onExit,
  startupTimeoutMs = 20_000,
  requestTimeoutMs = 10_000,
  shutdownTimeoutMs = 15_000,
} = {}) {
  const child = spawn(binary, ['desktop-host'], { stdio: ['pipe', 'pipe', 'inherit'] });
  const pending = new Map();
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let ready;
  let closing = false;
  let closed = false;
  let failure;
  let stopTimer;
  let killTimer;
  let chunks = [];
  let lineBytes = 0;
  let resolveReady;
  let rejectReady;
  const startup = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  let resolveClosed;
  const finished = new Promise((resolve) => { resolveClosed = resolve; });
  const startupTimer = setTimeout(() => fail(new Error('Desktop support startup timed out.')), startupTimeoutMs);

  function rejectPending(error) {
    for (const request of pending.values()) {
      clearTimeout(request.timer);
      request.reject(error);
    }
    pending.clear();
  }

  function stop(error) {
    if (closing || closed) return finished;
    closing = true;
    clearTimeout(startupTimer);
    rejectPending(error);
    // Rust observes EOF after supervisor startup and drops its owned daemon.
    // Give that bounded startup time to finish before escalating to signals.
    child.stdin.end();
    stopTimer = setTimeout(() => {
      child.kill('SIGTERM');
      killTimer = setTimeout(() => child.kill('SIGKILL'), Math.min(shutdownTimeoutMs, 1_000));
    }, shutdownTimeoutMs);
    return finished;
  }

  function fail(error) {
    if (failure || closing || closed) return;
    failure = error;
    rejectReady(error);
    stop(error);
    if (ready) onExit?.(error);
  }

  function receive(bytes) {
    let value;
    try { value = JSON.parse(decoder.decode(bytes)); }
    catch { fail(new Error('Desktop support returned an invalid JSONL response.')); return; }
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
      fail(new Error('Desktop support returned an invalid response.'));
      return;
    }
    if (!ready) {
      if (value.type === 'ready' && value.bootstrap && value.bootstrap.websocketUrl === undefined) {
        fail(new Error('Update the desktop support binary to use the authenticated WebSocket relay.'));
        return;
      }
      if (value.type !== 'ready' || !isAppUrl(value.appUrl) || !isBootstrap(value.bootstrap)) {
        fail(new Error('Desktop support returned invalid startup settings.'));
        return;
      }
      clearTimeout(startupTimer);
      ready = value;
      resolveReady();
      return;
    }
    const validId = typeof value.id === 'string' && value.id.length > 0
      || Number.isSafeInteger(value.id) && value.id >= 0;
    const hasResult = Object.hasOwn(value, 'result');
    const hasError = Object.hasOwn(value, 'error');
    if (!validId || hasResult === hasError || hasError && typeof value.error !== 'string') {
      fail(new Error('Desktop support returned an invalid request reply.'));
      return;
    }
    const request = pending.get(value.id);
    if (!request) return;
    pending.delete(value.id);
    clearTimeout(request.timer);
    if (hasError) request.reject(new Error(value.error));
    else request.resolve(value.result);
  }

  child.stdout.on('data', (chunk) => {
    if (closing) return;
    let offset = 0;
    while (offset < chunk.length) {
      const newline = chunk.indexOf(10, offset);
      const end = newline < 0 ? chunk.length : newline;
      const part = chunk.subarray(offset, end);
      lineBytes += part.length;
      if (lineBytes > MAX_RESPONSE_BYTES) {
        chunks = [];
        fail(new Error('Desktop support response exceeds the 4 MiB limit.'));
        return;
      }
      chunks.push(part);
      if (newline < 0) return;
      const line = chunks.length === 1 ? chunks[0] : Buffer.concat(chunks, lineBytes);
      chunks = [];
      lineBytes = 0;
      receive(line);
      if (closing) return;
      offset = newline + 1;
    }
  });
  child.stdout.on('end', () => fail(new Error('Desktop support closed its output unexpectedly.')));
  child.stdout.on('error', () => fail(new Error('Desktop support output failed.')));
  child.stdin.on('error', () => fail(new Error('Desktop support input failed.')));
  child.on('error', () => fail(new Error('Desktop support process could not be started.')));
  child.on('exit', (code, signal) => fail(new Error(
    `Desktop support process exited${signal ? ` (${signal})` : ` (code ${code})`}.`,
  )));
  child.on('close', () => {
    clearTimeout(startupTimer);
    clearTimeout(stopTimer);
    clearTimeout(killTimer);
    closed = true;
    resolveClosed();
  });

  function request(request) {
    if (closing || closed) return Promise.reject(new Error('Desktop support is closed.'));
    const id = randomUUID();
    let line;
    try { line = JSON.stringify({ id, request }); }
    catch { return Promise.reject(new Error('Desktop support request could not be serialized.')); }
    if (Buffer.byteLength(line) > MAX_REQUEST_BYTES) {
      return Promise.reject(new Error('Desktop support request exceeds 16 KiB.'));
    }
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error('Desktop support request timed out.'));
      }, requestTimeoutMs);
      pending.set(id, { resolve, reject, timer });
      child.stdin.write(`${line}\n`, (error) => {
        if (error) fail(new Error('Desktop support input failed.'));
      });
    });
  }

  try { await startup; }
  catch (error) { await stop(error); throw error; }
  return {
    appUrl: ready.appUrl,
    bootstrap: ready.bootstrap,
    request,
    close: () => stop(new Error('Desktop support is closing.')),
  };
}

module.exports = { startSupport };
