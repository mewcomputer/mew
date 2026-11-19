export const pavingJoint = 0.035;

export function stairLanes(width: number) {
  return Math.ceil(width);
}

export function surfaceGrid(
  width: number, depth: number,
  columns = Math.max(1, Math.round(width / 1.15)),
  rows = Math.max(1, Math.round(depth / 1.15)),
) {
  return { columns, rows, cellWidth: width / columns, cellDepth: depth / rows };
}
