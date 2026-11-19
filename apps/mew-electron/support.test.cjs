const assert = require('node:assert/strict');
const { mkdtemp, readFile, rm, writeFile } = require('node:fs/promises');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { test } = require('node:test');
const { startSupport } = require('./support.cjs');

const bootstrap = {
  daemonUrl: 'ws://127.0.0.1:25566',
  websocketUrl: 'ws://127.0.0.1:34568/relay-fixture',
  themeMode: 'system',
  lightTheme: 'codex-light',
  darkTheme: 'codex-dark',
  themes: [{ id: 'codex-dark', mode: 'dark', css: '--background: #181818;' }],
};
const ready = { type: 'ready', appUrl: 'http://127.0.0.1:34567/', bootstrap };
const deadlines = { startupTimeoutMs: 2_000, requestTimeoutMs: 1_000, shutdownTimeoutMs: 100 };

async function fixture(t, body) {
  const directory = await mkdtemp(join(tmpdir(), 'mew-electron-support-'));
  const binary = join(directory, 'support-fixture');
  const pidFile = join(directory, 'pid');
  const marker = join(directory, 'eof');
  const instances = [];
  await writeFile(binary, `#!${process.execPath}
const fs = require('node:fs');
const readline = require('node:readline');
fs.writeFileSync(${JSON.stringify(pidFile)}, String(process.pid));
const ready = ${JSON.stringify(ready)};
const eofMarker = ${JSON.stringify(marker)};
const send = (value) => process.stdout.write(JSON.stringify(value) + '\\n');
process.stdout.on('error', () => process.exit(0));
${body}
`, { mode: 0o755 });
  t.after(async () => {
    for (const instance of instances) await instance.close();
    await rm(directory, { recursive: true, force: true });
  });
  return {
    binary,
    marker,
    async start(options = {}) {
      const instance = await startSupport(binary, { ...deadlines, ...options });
      instances.push(instance);
      return instance;
    },
    async assertStopped() {
      const pid = Number(await readFile(pidFile, 'utf8'));
      assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' });
    },
  };
}

test('starts the actual support subprocess with inherited cwd/environment and split JSONL', async (t) => {
  const f = await fixture(t, `
const line = JSON.stringify(ready) + '\\n';
process.stdout.write(line.slice(0, 25));
setImmediate(() => process.stdout.write(line.slice(25)));
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id } = JSON.parse(line);
  const response = JSON.stringify({ id, result: { argv: process.argv.slice(2), cwd: process.cwd(), path: process.env.PATH } });
  process.stdout.write(response.slice(0, 5));
  setImmediate(() => process.stdout.write(response.slice(5) + '\\n'));
});`);
  const support = await f.start();
  assert.equal(support.appUrl, ready.appUrl);
  assert.deepEqual(support.bootstrap, bootstrap);
  assert.deepEqual(await support.request({ type: 'bootstrap' }), {
    argv: ['desktop-host'], cwd: process.cwd(), path: process.env.PATH,
  });
  await support.close();
  await f.assertStopped();
});

test('routes out-of-order request IDs and ignores stale or duplicate responses', async (t) => {
  const f = await fixture(t, `
send(ready);
const requests = [];
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  requests.push(JSON.parse(line));
  if (requests.length === 2) {
    const [first, second] = requests;
    send({ id: 'unknown-stale-id', result: 'wrong' });
    send({ id: second.id, result: second.request.name });
    send({ id: second.id, result: 'duplicate' });
    send({ id: first.id, result: first.request.name });
  }
});`);
  const support = await f.start();
  assert.deepEqual(await Promise.all([
    support.request({ name: 'first' }), support.request({ name: 'second' }),
  ]), ['first', 'second']);
});

test('a correlated Rust error rejects only its request and leaves the process usable', async (t) => {
  const f = await fixture(t, `
send(ready);
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, request } = JSON.parse(line);
  send(request.type === 'set_theme' ? { id, error: 'unknown theme' } : { id, result: ready.bootstrap });
});`);
  const support = await f.start();
  await assert.rejects(support.request({ type: 'set_theme' }), /unknown theme/);
  assert.deepEqual(await support.request({ type: 'bootstrap' }), bootstrap);
});

test('request deadlines reject once and late replies cannot complete a later request', async (t) => {
  const f = await fixture(t, `
send(ready);
let delayed;
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, request } = JSON.parse(line);
  if (!delayed) { delayed = id; return; }
  send({ id: delayed, result: 'late' });
  send({ id, result: request.type });
});`);
  const support = await f.start({ requestTimeoutMs: 50 });
  await assert.rejects(support.request({ type: 'delayed' }), /timed out/i);
  assert.equal(await support.request({ type: 'bootstrap' }), 'bootstrap');
});

test('oversized UTF-8 and unserializable requests are rejected before writing', async (t) => {
  const f = await fixture(t, `
send(ready);
readline.createInterface({ input: process.stdin }).on('line', (line) => {
  const { id, request } = JSON.parse(line);
  send({ id, result: request.type });
});`);
  const support = await f.start();
  await assert.rejects(support.request({ type: 'set_theme', name: 'é'.repeat(9_000) }), /16 KiB/);
  const circular = {};
  circular.self = circular;
  await assert.rejects(support.request(circular), /serializ/i);
  assert.equal(await support.request({ type: 'bootstrap' }), 'bootstrap');
});

test('malformed startup output and invalid ready messages clean up before rejecting', async (t) => {
  for (const body of [
    `process.stdout.write('private-payload-not-json\\n');`,
    `send({ ...ready, appUrl: 'https://untrusted.example/' });`,
    `send({ ...ready, bootstrap: null });`,
    `send({ ...ready, type: 'unexpected' });`,
  ]) {
    const f = await fixture(t, `${body}\nsetInterval(() => {}, 1000);`);
    await assert.rejects(f.start(), (error) => {
      assert.doesNotMatch(error.message, /private-payload/);
      return true;
    });
    await f.assertStopped();
  }
});

test('support binaries that omit the desktop relay must be updated', async (t) => {
  const f = await fixture(t, `delete ready.bootstrap.websocketUrl; send(ready); setInterval(() => {}, 1000);`);
  await assert.rejects(f.start(), /update.*support binary/i);
  await f.assertStopped();
});

test('startup timeout, stdout EOF and nonzero exit reject and reap the subprocess', async (t) => {
  for (const body of [
    `setInterval(() => {}, 1000);`,
    `process.stdout.end(); setInterval(() => {}, 1000);`,
    `process.exit(7);`,
  ]) {
    const f = await fixture(t, body);
    await assert.rejects(f.start({ startupTimeoutMs: 300 }), /desktop support/i);
    await f.assertStopped();
  }
});

test('spawn failure rejects startup without an unhandled error event', async () => {
  await assert.rejects(startSupport('/missing/mew-desktop-support-binary', deadlines), /desktop support/i);
});

test('startup failure waits for initialization to observe EOF before sending signals', async (t) => {
  const f = await fixture(t, `
process.on('SIGTERM', () => { fs.writeFileSync(eofMarker, 'signal'); process.exit(1); });
setTimeout(() => {
  send(ready);
  readline.createInterface({ input: process.stdin });
  process.stdin.on('end', () => fs.writeFileSync(eofMarker, 'eof'));
}, 100);`);
  await assert.rejects(f.start({ startupTimeoutMs: 50, shutdownTimeoutMs: 2_000 }), /startup timed out/i);
  assert.equal(await readFile(f.marker, 'utf8'), 'eof');
  await f.assertStopped();
});

test('malformed replies reject all pending requests, notify once, and terminate the process', async (t) => {
  const f = await fixture(t, `
send(ready);
let count = 0;
readline.createInterface({ input: process.stdin }).on('line', () => {
  if (++count === 2) process.stdout.write('sensitive-output-not-json\\n');
});`);
  const exits = [];
  const support = await f.start({ onExit: (error) => exits.push(error) });
  const results = await Promise.allSettled([
    support.request({ type: 'bootstrap' }), support.request({ type: 'bootstrap' }),
  ]);
  assert.ok(results.every((result) => result.status === 'rejected'));
  for (const result of results) assert.doesNotMatch(result.reason.message, /sensitive-output/);
  await support.close();
  assert.equal(exits.length, 1);
  await assert.rejects(support.request({ type: 'bootstrap' }), /closed|stopped|closing/i);
  await f.assertStopped();
});

test('unexpected nonzero exit or incomplete EOF rejects pending requests and notifies main', async (t) => {
  for (const finish of [
    `process.exit(9);`,
    `process.stdout.end('{"id":');`,
  ]) {
    const f = await fixture(t, `
send(ready);
readline.createInterface({ input: process.stdin }).on('line', () => { ${finish} });`);
    const exits = [];
    const support = await f.start({ onExit: (error) => exits.push(error) });
    await assert.rejects(support.request({ type: 'bootstrap' }), /desktop support/i);
    await support.close();
    assert.equal(exits.length, 1);
    await f.assertStopped();
  }
});

test('oversized JSONL output is bounded and terminates the support process', async (t) => {
  const f = await fixture(t, `
send(ready);
readline.createInterface({ input: process.stdin }).on('line', () => {
  process.stdout.write(Buffer.alloc(4 * 1024 * 1024 + 1, 'x'));
});`);
  const support = await f.start();
  await assert.rejects(support.request({ type: 'bootstrap' }), /response.*limit|response.*large/i);
  await support.close();
  await f.assertStopped();
});

test('close rejects pending work, delivers stdin EOF, waits for exit and stays idempotent', async (t) => {
  const f = await fixture(t, `
send(ready);
readline.createInterface({ input: process.stdin });
process.stdin.on('end', () => fs.writeFileSync(eofMarker, 'eof'));`);
  const exits = [];
  const support = await f.start({ onExit: (error) => exits.push(error) });
  const pending = assert.rejects(support.request({ type: 'bootstrap' }), /closing|closed/i);
  await Promise.all([support.close(), support.close(), pending]);
  assert.equal(await readFile(f.marker, 'utf8'), 'eof');
  assert.deepEqual(exits, []);
  await f.assertStopped();
});

test('shutdown escalates when a subprocess ignores EOF and SIGTERM', async (t) => {
  const f = await fixture(t, `
process.on('SIGTERM', () => {});
send(ready);
process.stdin.resume();
setInterval(() => {}, 1000);`);
  const support = await f.start({ shutdownTimeoutMs: 50 });
  await support.close();
  await f.assertStopped();
});
