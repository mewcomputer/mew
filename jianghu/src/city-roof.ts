import type * as Three from 'three';

type Point = [x: number, y: number, z: number];

const corners = [[-1, 1], [1, 1], [1, -1], [-1, -1]] as const;
const tiles = ['#354b45', '#405950', '#3a5149', '#435e53', '#394d45'];

export function createHippedRoofGeometry(T: typeof Three, width: number, depth: number, rise: number) {
  const positions: number[] = [];
  const colors: number[] = [];
  const color = new T.Color();

  function quad(a: Point, b: Point, c: Point, d: Point, tint: string) {
    positions.push(...a, ...b, ...c, ...a, ...c, ...d);
    color.set(tint);
    for (let i = 0; i < 6; i++) colors.push(color.r, color.g, color.b);
  }

  function point(side: number, u: number, t: number): Point {
    const [ax, az] = corners[side];
    const [bx, bz] = corners[(side + 1) % corners.length];
    const inset = 1 - t;
    const cornerLift = rise * 0.21 * Math.abs(2 * u - 1) ** 5 * inset ** 2;
    return [
      (ax + (bx - ax) * u) * (0.18 + 0.82 * inset) * width / 2,
      rise * (0.42 * t + 0.58 * t * t) + cornerLift,
      (az + (bz - az) * u) * (0.045 + 0.955 * inset) * depth / 2,
    ];
  }

  const courses = 7;
  for (let side = 0; side < corners.length; side++) {
    const columns = Math.max(8, Math.ceil((side % 2 ? depth : width) / 0.3));
    for (let tile = 0; tile < columns; tile++) {
      const left = tile / columns;
      const right = (tile + 1) / columns;
      const a = point(side, left, 0);
      const b = point(side, right, 0);
      quad([a[0], a[1] - 0.13, a[2]], [b[0], b[1] - 0.13, b[2]], b, a, '#648071');
      for (let course = 0; course < courses; course++) {
        const low = course / courses;
        const high = (course + 1) / courses;
        const tint = tiles[(tile * 3 + side + (course % 3 === 0 ? 1 : 0)) % tiles.length];
        quad(point(side, left, low), point(side, right, low),
          point(side, right, high), point(side, left, high), tint);
      }
    }
  }
  quad(point(0, 0, 1), point(1, 0, 1), point(2, 0, 1), point(3, 0, 1), '#627e70');

  const geometry = new T.BufferGeometry();
  geometry.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  geometry.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  geometry.computeVertexNormals();
  return geometry;
}

export function createOperaCanopyGeometry(T: typeof Three, width: number, depth: number, rise: number) {
  const positions: number[] = [];
  const colors: number[] = [];
  const color = new T.Color();
  const panels = ['#dad9cf', '#d2d4cc', '#deded6', '#cecfc7'];
  const across = 14;
  const along = 8;

  function point(column: number, row: number, underside = false): Point {
    const u = 2 * column / across - 1;
    const v = 2 * row / along - 1;
    const lift = 0.15 + 0.39 * u * u - 0.36 * u + 0.18 * v * (0.6 - 0.4 * u) + 0.12 * v * v;
    return [
      u * width * (0.5 - 0.07 * v * v) + 0.32 * v,
      lift * rise - (underside ? 0.30 + 0.14 * (1 - u * u) : 0),
      v * depth / 2 + (1 - 0.24 * v * v) * u * depth * 0.14,
    ];
  }

  function quad(a: Point, b: Point, c: Point, d: Point, tint: string) {
    positions.push(...a, ...b, ...c, ...a, ...c, ...d);
    color.set(tint);
    for (let i = 0; i < 6; i++) colors.push(color.r, color.g, color.b);
  }

  function edge(ax: number, az: number, bx: number, bz: number) {
    quad(point(bx, bz), point(ax, az), point(ax, az, true), point(bx, bz, true), '#c0c0b4');
  }

  for (let row = 0; row < along; row++) {
    for (let column = 0; column < across; column++) {
      quad(point(column, row), point(column, row + 1), point(column + 1, row + 1), point(column + 1, row),
        panels[(2 * column + row + Math.floor(column / 3)) % panels.length]);
      quad(point(column + 1, row, true), point(column + 1, row + 1, true), point(column, row + 1, true), point(column, row, true),
        '#8f8275');
    }
    edge(0, row, 0, row + 1);
    edge(across, row + 1, across, row);
  }
  for (let column = 0; column < across; column++) {
    edge(column + 1, 0, column, 0);
    edge(column, along, column + 1, along);
  }

  const shell = new T.BufferGeometry();
  shell.setAttribute('position', new T.Float32BufferAttribute(positions, 3));
  shell.setAttribute('color', new T.Float32BufferAttribute(colors, 3));
  shell.computeVertexNormals();
  return shell;
}
