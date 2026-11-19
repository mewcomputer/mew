export const shopVariants = {
  market: { wall: '#685743', frame: '#70553f', glass: '#826f4b', lit: '#e1c282', height: 2.17 },
  teahouse: { wall: '#694b3d', frame: '#764e3d', glass: '#665845', lit: '#eed299', height: 2.38 },
  workshop: { wall: '#6a7468', frame: '#3e504c', glass: '#50685e', lit: '#a8c0a3', height: 1.99 },
  station: { wall: '#8f937b', frame: '#4b6760', glass: '#49665a', lit: '#aed2b2', height: 2.30 },
} as const;

export type ShopVariant = keyof typeof shopVariants;

export function shopFootprint({ width, depth }: { width: number; depth: number }) {
  return { width: width + 0.48, depth: depth + 0.47 };
}
