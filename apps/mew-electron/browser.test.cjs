const { test } = require('node:test');
const assert = require('node:assert/strict');
const { spawn, execFile } = require('node:child_process');
const { randomUUID } = require('node:crypto');
const path = require('node:path');
const { promisify } = require('node:util');
const { isAbort } = require('./browser.cjs');

const execute = promisify(execFile);

test('a superseded navigation is not reported as a browser error', () => {
  const aborted = Object.assign(new Error("ERR_ABORTED (-3) loading 'https://www.google.com/'"), { code: 'ERR_ABORTED', errno: -3 });
  assert.equal(isAbort(aborted), true);
  assert.equal(isAbort(Object.assign(new Error("(-3) loading 'https://www.google.com/'"), { errno: -3 })), true);
  assert.equal(isAbort(new Error('ERR_CONNECTION_REFUSED')), false);
  assert.equal(isAbort(undefined), false);
});

async function runFixture(t, mode) {
  const cli = process.env.MEW_BROWSER_TEST_CLI || 'agent-browser';
  if (mode === 'input') {
    const { stdout: version } = await execute(cli, ['--version'], { timeout: 5000, killSignal: 'SIGKILL' });
    const match = version.match(/(\d+)\.(\d+)\.\d+/);
    assert.ok(match && (Number(match[1]) > 0 || Number(match[2]) >= 34), `agent-browser >=0.34 required: ${version}`);
  }
  const prefix = `mew-electron-test-${randomUUID()}`;
  const child = spawn(require('electron'), [path.join(__dirname, 'tests/browser-host.cjs'), mode], {
    env: { ...process.env, ELECTRON_RUN_AS_NODE: undefined, MEW_BROWSER_TEST_CLI: cli, MEW_BROWSER_TEST_SESSION_PREFIX: prefix },
    stdio: ['ignore', 'pipe', 'pipe'], signal: t.signal, killSignal: 'SIGKILL',
  });
  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk; });
  child.stderr.on('data', (chunk) => { stderr += chunk; });
  try {
    const code = await new Promise((resolve, reject) => {
      child.once('close', resolve);
      child.once('error', reject);
    });
    assert.equal(code, 0, `${stdout}\n${stderr}`);
    const result = stdout.split('\n').find((line) => line.startsWith('MEW_TEST_RESULT '));
    assert.ok(result, `${stdout}\n${stderr}`);
    return JSON.parse(result.slice(16));
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      await new Promise((resolve) => {
        child.once('close', resolve);
        child.kill('SIGKILL');
      });
    }
    const sessions = new Set();
    for (const line of stdout.split('\n')) {
      const [kind, name] = line.split(' ');
      if (![`${prefix}-first`, `${prefix}-second`].includes(name)) continue;
      if (kind === 'MEW_TEST_CLI_SESSION') sessions.add(name);
      if (kind === 'MEW_TEST_CLI_CLOSED') sessions.delete(name);
    }
    for (const session of sessions) {
      await execute(cli, ['--session', session, 'close'], {
        timeout: 5000, killSignal: 'SIGKILL', env: { ...process.env, AGENT_BROWSER_IDLE_TIMEOUT_MS: '10000' },
      });
    }
  }
}

const securityResult = {
  guests: 2, distinctTargets: true, retained: true, isolated: true,
  controlBlocked: true, sharedStorage: true, ownerChecks: true,
  permitReuseBlocked: true, generationChanged: true, pendingNavigationKeptPermit: true, hiddenCleanupKeptNavigation: true,
  rootUrlAttached: true, focusEndpointGuards: true, staleFocusRevoked: true,
  allowedHttp: 1, allowedWs: 1, controlHttp: 0, controlWs: 0, blockedCdp: true, blockedFocus: true,
};

test('real Electron guests isolate control endpoints, ownership and retained generations', { timeout: 30000 }, async (t) => {
  assert.deepEqual(await runFixture(t, 'security'), securityResult);
});

test('agent-browser pinned input modifies only its same-URL guest after chat steals focus', { timeout: 60000 }, async (t) => {
  assert.deepEqual(await runFixture(t, 'input'), { ...securityResult, agentPinned: true });
});
