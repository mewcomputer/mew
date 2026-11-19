import type { CharacterPose, CityCharacter, CityDirection, CityPosition } from './city-characters.ts';
import { rainbellSite } from './rainbell-world.ts';

export const gaugeSite: CityPosition = [18.5, 1.6, 24.21];
export const teaStand: CityPosition = [-20.2, 3.1, 14.32];
export const practiceSite: CityPosition = [25.0, 2.45, 28.3];
export const rainbellPeriod = 18.4;
export const rainbellSignalStart = 5.4;
export const rainbellSignalEnd = 8.0;

export function rainbellSignal(time: number) {
  const beat = time % rainbellPeriod;
  return beat >= rainbellSignalStart && beat < rainbellSignalEnd;
}

export function rainbellStrikeBetween(previous: number, time: number) {
  const cycle = Math.floor(previous / rainbellPeriod);
  const strike = cycle * rainbellPeriod + rainbellSignalStart;
  const next = previous < strike ? strike : (cycle + 1) * rainbellPeriod + rainbellSignalStart;
  return time >= next;
}

export function rainbellSwing(time: number) {
  const strike = Math.floor(time / rainbellPeriod) * rainbellPeriod + rainbellSignalStart;
  const elapsed = time - strike;
  if (elapsed < -0.72 || elapsed >= 1.3) return 0;
  if (elapsed < -0.3) return -0.19 * Math.sin((elapsed + 0.72) / 0.42 * Math.PI / 2);
  if (elapsed < 0) return -0.19 + 0.325 * ((elapsed + 0.3) / 0.3) ** 2;
  return 0.135 * Math.cos(elapsed * 8.5) * (1 - elapsed / 1.3) ** 3;
}

export function createCityEncounters(cast: readonly Pick<CityCharacter, 'id' | 'path'>[]) {
  const observed = ['ye-xia', 'ning-fei', 'xu-chun', 'pei-shu'];
  const homes = new Map<string, CityPosition>();
  const actors = new Map<string, CharacterPose>();
  for (const { id, path } of cast) {
    if (!observed.includes(id)) continue;
    homes.set(id, path[0]);
    actors.set(id, { position: [...path[0]], direction: [0, 1], mode: 'idle', ticks: 0 });
  }
  const encounter = { actors, gaugeLit: false, courtActive: false };
  const yeForm: CityPosition = [24.10, 2.65, 28.58];
  const ningForm: CityPosition = [25.98, 2.65, 28.58];
  const gaugeControl: CityPosition = [gaugeSite[0], gaugeSite[1], gaugeSite[2] + 2.56];
  const bellControl: CityPosition = [rainbellSite[0] - 1.05, rainbellSite[1], rainbellSite[2] + 2.27];
  const courtVisit = [0.35, 1.26, 4.75, 5.62] as const;
  const gaugeVisit = [1.9, 4.3, 7.35, 10.4] as const;
  const bellVisit = [2.0, 4.85, 8.55, 11.4] as const;
  const north: CityDirection = [0, -1];
  const east: CityDirection = [1, 0];
  const west: CityDirection = [-1, 0];

  function place(
    actor: CharacterPose, destination: CityPosition, direction: CityDirection,
    mode: CharacterPose['mode'] = 'idle', time = 0,
  ) {
    for (let axis = 0; axis < 3; axis++) actor.position[axis] = destination[axis];
    actor.direction[0] = direction[0];
    actor.direction[1] = direction[1];
    actor.mode = mode;
    actor.ticks = Math.floor(time * 30);
  }

  function walk(actor: CharacterPose, from: CityPosition, to: CityPosition, progress: number) {
    for (let axis = 0; axis < 3; axis++) {
      actor.position[axis] = from[axis] + (to[axis] - from[axis]) * progress;
    }
    const dx = to[0] - from[0];
    const dz = to[2] - from[2];
    const across = Math.abs(dx) > Math.abs(dz);
    actor.direction[0] = across ? Math.sign(dx) : 0;
    actor.direction[1] = across ? 0 : Math.sign(dz);
    actor.mode = 'walk';
    actor.ticks = Math.floor(progress * Math.hypot(dx, dz) * 27);
  }

  function stage(
    id: string, destination: CityPosition, facing: CityDirection, beat: number,
    [start, arrive, depart, end]: readonly [number, number, number, number],
    mode: CharacterPose['mode'], poseTime: number,
  ) {
    const home = homes.get(id);
    const actor = actors.get(id);
    if (!home || !actor) throw new Error(`Missing Baichuan encounter character ${id}.`);
    if (beat >= start && beat < arrive) {
      walk(actor, home, destination, (beat - start) / (arrive - start));
    } else if (beat >= arrive && beat < depart) {
      place(actor, destination, facing, mode, poseTime);
    } else if (beat >= depart && beat < end) {
      walk(actor, destination, home, (beat - depart) / (end - depart));
    } else {
      place(actor, home, facing);
    }
  }

  return (time: number) => {
    const drill = time % 8;
    encounter.courtActive = drill >= courtVisit[1] && drill < courtVisit[2];
    const yeLeads = drill >= 1.65 && drill < 2.5;
    const ningLeads = drill >= 2.78 && drill < 3.65;
    stage('ye-xia', yeForm, east, drill, courtVisit, yeLeads ? 'work' : 'guard', yeLeads ? drill - 1.65 : 0.3);
    stage('ning-fei', ningForm, west, drill, courtVisit, ningLeads ? 'work' : 'guard', ningLeads ? drill - 2.78 : 0.3);

    const check = time % 15.8;
    stage('xu-chun', gaugeControl, north, check, gaugeVisit,
      check < 5.85 ? 'work' : 'idle', Math.max(0, check - 4.3));
    encounter.gaugeLit = check >= 5.15 && check < 9.6;

    const bell = time % rainbellPeriod;
    stage('pei-shu', bellControl, east, bell, bellVisit,
      rainbellSignal(time) ? 'work' : 'idle', Math.max(0, bell - rainbellSignalStart));
    return encounter;
  };
}
