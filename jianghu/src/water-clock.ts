import type { WaterSimulation } from './fluid.ts';

export function createWaterClock(water: Pick<WaterSimulation, 'dt' | 'step'>) {
  let last: number | null = null;
  let elapsed = 0;
  let ticks = 0;

  return {
    get elapsed() {
      return elapsed;
    },
    get simulated() {
      return ticks * water.dt;
    },
    reset() {
      last = null;
    },
    advance(now: number, active: boolean) {
      const previous = last;
      last = active ? now : null;
      if (!active || previous === null || now <= previous) return false;

      elapsed += Math.min(80, now - previous) / 1000;
      while ((ticks + 1) * water.dt <= elapsed + 1e-9) {
        water.step();
        ticks++;
      }
      return true;
    },
  };
}
