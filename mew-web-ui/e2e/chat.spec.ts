import { test, expect, type Page } from "@playwright/test";
import { spawn, type ChildProcess } from "child_process";
import { mkdtempSync, existsSync, rmSync, readFileSync } from "fs";
import { join } from "path";
import { tmpdir } from "os";

// ---------------------------------------------------------------------------
// Test fixture: start mew daemon + mew-web bridge, then load the UI.
// ---------------------------------------------------------------------------

let daemonProc: ChildProcess | null = null;
let bridgeProc: ChildProcess | null = null;
let tmpDir: string;

// Use a high port unlikely to conflict with other dev servers.
const BRIDGE_PORT = "19847";
let DAEMON_SOCKET = "";

// Find the mew + mew-web binaries in target/debug/.
function findBinary(name: string): string {
  const candidates = [
    join(process.cwd(), "..", "target", "debug", name),
    join(process.cwd(), "..", "..", "target", "debug", name),
  ];
  for (const p of candidates) {
    if (existsSync(p)) return p;
  }
  throw new Error(`binary not found: ${name} (looked in target/debug/)`);
}

test.beforeAll(async () => {
  tmpDir = mkdtempSync(join(tmpdir(), "mew-e2e-"));
  const socketPath = join(tmpDir, "mew.sock");
  DAEMON_SOCKET = socketPath;

  const mewBin = findBinary("mew");
  const webBin = findBinary("mew-web");

  // Start daemon with fake provider.
  daemonProc = spawn(
    mewBin,
    ["daemon", "--fake-provider", "--socket", socketPath],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  daemonProc.stdout?.on("data", (d) => process.stdout.write(`[daemon] ${d}`));
  daemonProc.stderr?.on("data", (d) => process.stderr.write(`[daemon] ${d}`));

  // Wait for the socket to appear.
  for (let i = 0; i < 50; i++) {
    if (existsSync(socketPath)) break;
    await new Promise((r) => setTimeout(r, 100));
  }
  if (!existsSync(socketPath)) throw new Error("daemon socket never appeared");

  // Start the bridge.
  bridgeProc = spawn(
    webBin,
    ["--port", `127.0.0.1:${BRIDGE_PORT}`, "--daemon-socket", socketPath, "--spawn", "false"],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  bridgeProc.stdout?.on("data", (d) => process.stdout.write(`[bridge] ${d}`));
  bridgeProc.stderr?.on("data", (d) => process.stderr.write(`[bridge] ${d}`));

  // Wait for the bridge port to be ready.
  for (let i = 0; i < 50; i++) {
    try {
      const resp = await fetch(`http://127.0.0.1:${BRIDGE_PORT}/`);
      if (resp.ok) break;
    } catch {
      // not ready yet
    }
    await new Promise((r) => setTimeout(r, 100));
  }
});

test.afterAll(() => {
  daemonProc?.kill("SIGTERM");
  bridgeProc?.kill("SIGTERM");
  if (tmpDir) rmSync(tmpDir, { recursive: true, force: true });
});

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

test("page loads and shows the mew UI", async ({ page }) => {
  await page.goto("/");
  // The TopBar should contain "mew" or a session button.
  await expect(page.locator("header")).toBeVisible();
});

test("can send a prompt and see streaming text", async ({ page }) => {
  await page.goto("/");

  // Wait for connection — the status dot text should show "connected".
  await expect(page.locator("text=connected")).toBeVisible({ timeout: 10_000 });

  // Find the textarea and type a prompt.
  const input = page.locator("textarea").first();
  await input.fill("hello");

  // Submit with Ctrl+Enter (Playwright doesn't have Meta on all platforms).
  await input.press("Control+Enter");

  // Wait for the assistant response to appear. The fake provider streams
  // "hello from fake provider" — we should see some of that text.
  await expect(page.locator("text=hello from fake")).toBeVisible({
    timeout: 10_000,
  });
});

test("session list drawer opens and shows sessions", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("text=connected")).toBeVisible({ timeout: 10_000 });

  // Click the session button in the TopBar (the button with session info).
  const sessionBtn = page.locator("header button").first();
  await sessionBtn.click();

  // The drawer should appear with the "Sessions" heading.
  await expect(page.getByRole("heading", { name: "Sessions" })).toBeVisible({ timeout: 5_000 });

  // There should be at least one session in the list.
  await expect(page.locator("text=sess_")).toBeVisible({ timeout: 5_000 });
});
