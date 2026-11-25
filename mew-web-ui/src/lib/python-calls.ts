/** Static reading of a Python cell, for the collapsed row.
 *
 *  Nested call events only exist after the cell has run, so the collapsed line
 *  would say nothing while it is in flight. The source names the tools it is
 *  about to use, and their first argument, which is enough to describe it. */
export interface PythonCall {
  name: string;
  target: string;
}

const CALL_PATTERN = /tools\.([a-z_]+)\s*\(/g;
const STRING_PATTERN = /(['"])(?:\\.|(?!\1).)*\1/g;
const TARGET_KEYS = /["'](path|file_path|file|pattern|command|url)["']\s*:\s*["']([^"']*)["']/;
const FIRST_QUOTED = /["']([^"']+)["']/;

/** Ranges of string literals, so `"tools.read()"` in a message is not a call. */
function stringRanges(source: string): [number, number][] {
  const ranges: [number, number][] = [];
  for (const match of source.matchAll(STRING_PATTERN)) {
    if (match.index !== undefined) ranges.push([match.index, match.index + match[0].length]);
  }
  return ranges;
}

/** The first argument of a call, favouring a path-like key. */
function firstArgument(rest: string): string {
  const argument = rest.split("\n")[0] ?? "";
  const keyed = argument.match(TARGET_KEYS);
  if (keyed?.[2]) return keyed[2];
  return argument.match(FIRST_QUOTED)?.[1] ?? "";
}

export function parsePythonCalls(source: string): PythonCall[] {
  const calls: PythonCall[] = [];
  const ranges = stringRanges(source);
  for (const match of source.matchAll(CALL_PATTERN)) {
    const name = match[1];
    const index = match.index ?? 0;
    if (!name) continue;
    if (ranges.some(([start, end]) => index >= start && index < end)) continue;
    const target = firstArgument(source.slice(index + match[0].length));
    if (!calls.some((call) => call.name === name && call.target === target)) {
      calls.push({ name, target });
    }
  }
  return calls;
}

/** `grep "desktop-electron", read AGENTS.md` - what the cell will touch. */
export function describePythonCalls(calls: PythonCall[]): string {
  return calls
    .slice(0, 3)
    .map((call) => (call.target ? `${call.name} ${call.target}` : call.name))
    .join(", ");
}
