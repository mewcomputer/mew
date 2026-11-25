import { cn } from "../lib/utils";

export const SUBAGENT_IDENTICON_PALETTE = [
  "#3b82f6",
  "#10b981",
  "#f59e0b",
  "#8b5cf6",
  "#ec4899",
  "#06b6d4",
  "#f97316",
  "#6366f1",
] as const;

const FNV_OFFSET_BASIS = 0x811c9dc5;
const FNV_PRIME = 0x01000193;
const GRID_SIZE = 5;
const HALF_COLUMNS = 3;

export function hashSeedFnv1a(seed: string): number {
  let hash = FNV_OFFSET_BASIS;
  for (const byte of new TextEncoder().encode(seed)) {
    hash ^= byte;
    hash = Math.imul(hash, FNV_PRIME) >>> 0;
  }
  return hash >>> 0;
}

export function identiconColor(seed: string): string {
  const hash = hashSeedFnv1a(seed);
  return SUBAGENT_IDENTICON_PALETTE[hash & 0x7]!;
}

/** Build a 5x5 horizontally symmetric matrix from the deterministic FNV-1a hash. */
export function identiconCells(seed: string): boolean[][] {
  const hash = hashSeedFnv1a(seed);
  let pattern = (hash >>> 3) & 0x7fff;
  if (pattern === 0) {
    // Keep zero-bit hashes visibly non-empty at the center pixel.
    pattern = 1 << (2 * HALF_COLUMNS + 2);
  }
  const rows: boolean[][] = [];
  for (let row = 0; row < GRID_SIZE; row++) {
    const line: boolean[] = Array(GRID_SIZE).fill(false);
    for (let col = 0; col < HALF_COLUMNS; col++) {
      const bitIndex = row * HALF_COLUMNS + col;
      const filled = ((pattern >>> bitIndex) & 1) === 1;
      line[col] = filled;
      line[GRID_SIZE - 1 - col] = filled;
    }
    rows.push(line);
  }
  return rows;
}

export function SubagentIdenticon({
  seed,
  size = 16,
  className,
}: {
  seed: string;
  size?: number;
  className?: string;
}) {
  const color = identiconColor(seed);
  const cells = identiconCells(seed);

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 7 7"
      shapeRendering="crispEdges"
      aria-hidden="true"
      data-seed={seed}
      className={cn("shrink-0 rounded-sm", className)}
    >
      <rect width="7" height="7" fill={color} fillOpacity={0.16} />
      {cells.map((row, y) =>
        row.map((filled, x) =>
          filled ? (
            <rect
              key={`${x}-${y}`}
              x={x + 1}
              y={y + 1}
              width={1}
              height={1}
              fill={color}
            />
          ) : null,
        ),
      )}
    </svg>
  );
}
