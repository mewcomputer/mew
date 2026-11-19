import type { CityPosition } from './city-characters.ts';

export type JoineryPiece = { min: CityPosition; max: CityPosition };

export function gaugeCrown() {
  const halfBeam = 0.41;
  const notch = 0.434;
  const halfKey = 0.13;
  const mortise = 0.158;
  const height = 0.9;

  function beam(start: number, end: number, webBottom: number, webTop: number, across: boolean) {
    const box = (a: number, b: number, y0: number, y1: number, c: number, d: number): JoineryPiece => ({
      min: across ? [c, y0, a] : [a, y0, c],
      max: across ? [d, y1, b] : [b, y1, d],
    });
    return [
      box(start, -notch, 0, height, -halfBeam, halfBeam),
      box(notch, end, 0, height, -halfBeam, halfBeam),
      box(-notch, -mortise, webBottom, webTop, -halfBeam, halfBeam),
      box(mortise, notch, webBottom, webTop, -halfBeam, halfBeam),
      box(-mortise, mortise, webBottom, webTop, -halfBeam, -mortise),
      box(-mortise, mortise, webBottom, webTop, mortise, halfBeam),
    ];
  }

  return {
    alongX: beam(-1.95, 3.08, 0, 0.435, false),
    alongZ: beam(-2.10, 2.03, 0.465, height, true),
    key: { min: [-halfKey, -0.53, -halfKey], max: [halfKey, 1.46, halfKey] },
  } satisfies { alongX: JoineryPiece[]; alongZ: JoineryPiece[]; key: JoineryPiece };
}
