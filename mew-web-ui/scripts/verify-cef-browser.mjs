// Verify the embedded CEF browser of a running desktop app through its
// Chrome DevTools Protocol endpoint. Start the app first (for example
// `pnpm desktop:dev` or `target/debug/mew-desktop`), then run
// `pnpm desktop:verify:cef`. Exits non-zero when any check fails.
//
// Uses a minimal raw CDP client: CEF answers page-level domains reliably,
// while higher-level clients can stall on browser-level handshake details.

import { mkdir, writeFile } from "node:fs/promises";

const port = process.env.MEW_CEF_DEBUG_PORT ?? "9223";
const endpoint = `http://127.0.0.1:${port}`;
const outDir = process.env.MEW_CEF_VERIFY_OUT ?? "/tmp/mew-cef-verify";

const results = [];
const check = (name, ok, detail = "") => {
  results.push({ name, ok, detail });
  console.log(`${ok ? "PASS" : "FAIL"} ${name}${detail ? ` — ${detail}` : ""}`);
  return ok;
};

const version = await fetch(`${endpoint}/json/version`).catch(() => null);
if (!version?.ok) {
  console.error(
    `no CEF DevTools endpoint at ${endpoint}; start the desktop app first (CEF must initialize)`,
  );
  process.exit(1);
}
check("devtools endpoint", true, (await version.json()).Browser);

const targets = await (await fetch(`${endpoint}/json/list`)).json();
const pageTarget = targets.find((target) => target.type === "page");
if (!check("page target exists", Boolean(pageTarget), pageTarget?.id)) {
  process.exit(1);
}

const cdp = await connectCdp(pageTarget.webSocketDebuggerUrl);

await cdp.send("Page.enable");
await cdp.send("Runtime.enable");
await cdp.send("Page.navigate", { url: "https://example.com" });
const loaded = await cdp.waitFor("Page.loadEventFired", 15000);
check("navigation", Boolean(loaded), pageTarget.webSocketDebuggerUrl);

const title = await cdp.evaluate("document.title");
check("document title", title === "Example Domain", String(title));

const evaluated = await cdp.evaluate(
  "JSON.stringify({ sum: 1 + 1, ua: navigator.userAgent })",
);
const { sum, ua } = JSON.parse(evaluated);
check("javascript evaluation", sum === 2 && ua.includes("Chrome"), ua);

await cdp.send("Emulation.setDeviceMetricsOverride", {
  width: 960,
  height: 600,
  deviceScaleFactor: 1,
  mobile: false,
});
await cdp.evaluate(`document.body.innerHTML =
  '<h1 style="font:48px sans-serif;color:#0a7">mew CEF render check</h1>' +
  '<p>dom mutation from the verification script</p>'`);
const mutated = await cdp.evaluate("document.querySelector('h1').textContent");
check("dom mutation", mutated === "mew CEF render check", mutated);

const shot = await cdp.send("Page.captureScreenshot", { format: "png" });
await mkdir(outDir, { recursive: true });
const screenshotPath = `${outDir}/render.png`;
await writeFile(screenshotPath, Buffer.from(shot.data, "base64"));
check("compositor screenshot", shot.data.length > 1000, screenshotPath);

await cdp.send("Page.navigate", { url: "https://example.com" });
await cdp.waitFor("Page.loadEventFired", 15000).catch(() => undefined);
cdp.close();

const failed = results.filter((result) => !result.ok);
await writeFile(`${outDir}/results.json`, JSON.stringify(results, null, 2));
console.log(
  failed.length === 0
    ? `all ${results.length} checks passed`
    : `${failed.length} of ${results.length} checks failed`,
);
process.exit(failed.length === 0 ? 0 : 1);

async function connectCdp(wsUrl) {
  const ws = new WebSocket(wsUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error(`CDP websocket failed: ${wsUrl}`));
  });

  let nextId = 1;
  const pending = new Map();
  const eventWaiters = new Map();
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) {
        reject(new Error(`${message.error.message} (${message.error.code})`));
      } else {
        resolve(message.result ?? {});
      }
    } else if (message.method && eventWaiters.has(message.method)) {
      const waiters = eventWaiters.get(message.method);
      eventWaiters.delete(message.method);
      for (const resolve of waiters) resolve(message.params);
    }
  };

  return {
    send(method, params = {}) {
      const id = nextId++;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        ws.send(JSON.stringify({ id, method, params }));
        setTimeout(() => {
          if (pending.delete(id)) reject(new Error(`${method}: timed out`));
        }, 15000);
      });
    },
    waitFor(method, timeoutMs) {
      return new Promise((resolve, reject) => {
        const waiters = eventWaiters.get(method) ?? [];
        waiters.push(resolve);
        eventWaiters.set(method, waiters);
        setTimeout(() => reject(new Error(`${method}: timed out`)), timeoutMs);
      });
    },
    async evaluate(expression) {
      const result = await this.send("Runtime.evaluate", {
        expression,
        returnByValue: true,
      });
      if (result.exceptionDetails) {
        throw new Error(`evaluate failed: ${JSON.stringify(result.exceptionDetails)}`);
      }
      return result.result?.value;
    },
    close() {
      ws.close();
    },
  };
}
