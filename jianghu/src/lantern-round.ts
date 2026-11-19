import { createCharacterPath, type CharacterPose, type CityDirection, type CityPosition } from './city-characters.ts';

type RoundPhase = 'roam' | 'fetch' | 'take' | 'round' | 'return' | 'hang';

const rackPause = 1.15;
const handoff = 0.52;
const transfer = 0.25;

export function createLanternRound(
  path: readonly [CityPosition, ...CityPosition[]], joinIndex: number, speed: number,
  rackDirection: CityDirection, departure: number,
) {
  const motion = createCharacterPath(path, speed);
  const join = motion.timeAtPoint(joinIndex);
  let progress = Math.min(departure, join);
  let previousTime = 0;
  let phase: RoundPhase = 'roam';
  let roamDirection = 1;
  let pause = 0;
  let carrying = false;
  let servedTonight = false;
  const result: { pose: CharacterPose; carrying: boolean; phase: RoundPhase; reach: number } =
    { pose: motion.sample(progress, 1, 'walk'), carrying, phase, reach: 0 };

  return (time: number, night: boolean) => {
    if (!night) {
      servedTonight = false;
      if (phase === 'fetch' || phase === 'take') {
        phase = carrying ? 'hang' : 'roam';
        pause = 0;
        roamDirection = 1;
      } else if (phase === 'round') {
        phase = 'return';
      }
    } else if (phase === 'roam' && !servedTonight) {
      phase = 'fetch';
    }

    let remaining = Math.max(0, time - previousTime);
    previousTime = time;
    while (remaining > 1e-8) {
      if (phase === 'take' || phase === 'hang') {
        const step = Math.min(remaining, rackPause - pause);
        pause += step;
        remaining -= step;
        if (pause >= handoff) carrying = phase === 'take';
        if (pause < rackPause) break;
        if (phase === 'take') {
          servedTonight = true;
          phase = 'round';
        } else {
          phase = 'roam';
          roamDirection = 1;
        }
        continue;
      }

      const towardRack = phase === 'fetch' || phase === 'return';
      const direction = phase === 'roam' ? roamDirection : towardRack ? -1 : 1;
      const end = direction > 0 ? motion.duration : phase === 'roam' ? join : 0;
      const distance = Math.abs(end - progress);
      if (remaining < distance) {
        progress += direction * remaining;
        break;
      }
      progress = end;
      remaining -= distance;
      if (phase === 'roam') {
        roamDirection *= -1;
        if (join === motion.duration) break;
      } else if (phase === 'round') {
        phase = 'return';
      } else {
        phase = phase === 'fetch' ? 'take' : 'hang';
        pause = 0;
      }
    }

    const atRack = phase === 'take' || phase === 'hang';
    const heading = phase === 'roam' ? roamDirection : phase === 'fetch' || phase === 'return' ? -1 : 1;
    const pose = motion.sample(progress, heading, atRack ? 'talk' : 'walk');
    if (atRack) {
      pose.direction[0] = rackDirection[0];
      pose.direction[1] = rackDirection[1];
      pose.ticks = Math.floor(pause * 30);
    }
    result.phase = phase;
    result.carrying = carrying;
    result.reach = !carrying ? 0
      : phase === 'take' ? Math.min(1, (pause - handoff) / transfer)
      : phase === 'hang' ? Math.min(1, (handoff - pause) / transfer)
      : 1;
    return result;
  };
}
