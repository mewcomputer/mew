import type { ConnectionState } from "../stores/session";

/** Extract the model name from a "provider/model" string. */
export function shortModel(model: string): string {
  return model.split("/").pop() ?? model;
}

/** Format a token count with k/M suffixes. */
export function formatTokens(n: number): string {
  if (n < 1000) return String(n);
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(2)}M`;
}

/** Human-readable relative age from a unix timestamp (seconds). */
export function formatRelativeAge(timestamp: number): string {
  const diffMs = Date.now() - timestamp * 1000;
  const sec = Math.floor(diffMs / 1000);
  if (sec < 60) return "now";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}d`;
  return new Date(timestamp * 1000).toLocaleDateString();
}

const CONNECTION_DOT: Record<ConnectionState, string> = {
  connected: "bg-green-500",
  connecting: "bg-yellow-500",
  reconnecting: "bg-yellow-500",
  disconnected: "bg-red-500",
};

/** Tailwind class for a connection-state indicator dot. */
export function connectionDotClass(state: ConnectionState): string {
  return CONNECTION_DOT[state] ?? "bg-gray-500";
}
