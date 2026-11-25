import { getClient } from "./client-ref";

const checks = new Map<string, Promise<boolean>>();

/** Ask the daemon for a one-byte preview; an error means the path is not openable. */
export function fileExists(sessionId: string | null, path: string): Promise<boolean> {
  if (!sessionId) return Promise.resolve(false);
  const key = `${sessionId}\u0000${path}`;
  const existing = checks.get(key);
  if (existing) return existing;

  const check = new Promise<boolean>((resolve) => {
    const client = getClient();
    if (!client) { resolve(false); return; }
    let done = false;
    const finish = (exists: boolean) => {
      if (done) return;
      done = true;
      window.clearTimeout(timer);
      client.off("file-preview", receive);
      resolve(exists);
    };
    const receive = (data: { session_id?: string; path: string; error?: string }) => {
      if (data.session_id !== sessionId || data.path !== path) return;
      finish(!data.error);
    };
    const timer = window.setTimeout(() => finish(false), 5000);
    client.on("file-preview", receive);
    try { client.readFilePreview(sessionId, path, 1); }
    catch { finish(false); }
  });

  checks.set(key, check);
  return check;
}
