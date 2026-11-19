import type { CityDirection } from './city-characters.ts';
import type { MovementPoint } from './tactics-movement.ts';

export function createTacticsWalk(points: readonly MovementPoint[], speed = 4) {
  const first = points[0];
  if (!first) throw new Error('A walk requires a starting point.');

  const state = { ...first, direction: [0, 1] as CityDirection, ticks: 0, done: false };
  let leg = 1;
  let elapsed = 0;
  let distance = 0;

  return {
    advance(seconds: number) {
      while (leg < points.length) {
        const from = points[leg - 1];
        const to = points[leg];
        const dx = to.x - from.x;
        const dy = to.y - from.y;
        const dz = to.z - from.z;
        const ground = Math.hypot(dx, dz);
        const length = Math.hypot(ground, dy);
        const duration = Math.max(ground / speed, Math.abs(dy) / 2);
        if (ground > 1e-6) {
          state.direction[0] = dx / ground;
          state.direction[1] = dz / ground;
        }

        const step = Math.min(duration - elapsed, seconds);
        elapsed += step;
        seconds -= step;
        const progress = duration ? elapsed / duration : 1;
        state.x = from.x + dx * progress;
        state.y = from.y + dy * progress;
        state.z = from.z + dz * progress;
        state.ticks = Math.floor((distance + length * progress) * 23);
        if (elapsed < duration) return state;

        distance += length;
        elapsed = 0;
        leg++;
      }
      state.done = true;
      return state;
    },
  };
}
