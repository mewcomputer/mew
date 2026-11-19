import { sampleCityTime } from './city-lighting.ts';

const transitionSeconds = 0.65;
const dayMinutes = 930;
const twilightStart = 0.16;
const twilightEnd = 0.88;

type Twilight = 'dusk' | 'dawn';

function smoothstep(t: number) {
  const x = Math.min(1, Math.max(0, t));
  return x * x * (3 - 2 * x);
}

function solarMinutes(progress: number, twilight: Twilight) {
  const start = twilight === 'dusk' ? 1050 : 450;
  const end = twilight === 'dusk' ? 1170 : 330;
  if (progress < twilightStart) return dayMinutes + (start - dayMinutes) * smoothstep(progress / twilightStart);
  if (progress < twilightEnd) {
    const fraction = (progress - twilightStart) / (twilightEnd - twilightStart);
    return start + (end - start) * smoothstep(fraction);
  }
  return end;
}

export function createGardenDaylight(initiallyNight: boolean) {
  let requestedNight = initiallyNight;
  let progress = Number(initiallyNight);
  let twilight: Twilight = 'dusk';
  let lanternsNeeded = initiallyNight;
  let lighting = sampleCityTime(initiallyNight ? 0 : dayMinutes);

  return {
    get nightRequested() { return requestedNight; },
    setNight(night: boolean) {
      if (requestedNight === night) return;
      if (progress === 0 || progress === 1) twilight = night ? 'dusk' : 'dawn';
      requestedNight = night;
    },
    advance(seconds: number, instant = false) {
      const target = Number(requestedNight);
      const step = seconds / transitionSeconds;
      const next = instant ? target : requestedNight
        ? Math.min(target, progress + step)
        : Math.max(target, progress - step);
      if (next !== progress) {
        progress = next;
        lighting = sampleCityTime(solarMinutes(progress, twilight));
      }
      if (requestedNight && lighting.night >= 0.20) lanternsNeeded = true;
      if (!requestedNight && lighting.night <= 0.80) lanternsNeeded = false;
      const lanternLevel = smoothstep((lighting.night - 0.05) / 0.90);
      return { lighting, twilight, lanternsNeeded, lanternLevel };
    },
  };
}
