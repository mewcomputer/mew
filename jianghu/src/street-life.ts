const speakingDistance = 1.45;
const separationDistance = 1.9;
const speakingDistanceSquared = speakingDistance ** 2;
const separationDistanceSquared = separationDistance ** 2;
const floorTolerance = 0.25;
const turnLength = 1.05;

type Participant = { id: string; market?: boolean; bellAttention?: number; sample: CharacterSampler };

type StreetStop = {
  mode: 'sip' | 'watch';
  near: CityPosition;
  face: CityPosition;
  radius: number;
};

type Place = StreetStop & {
  occupant: StreetActor | null;
  nearby: Set<StreetActor>;
  reach: number;
  exit: number;
};

type Pause = { start: number; ends: number } & (
  { place: Place; speaker?: never } | { speaker: StreetActor; place?: never } |
  { place?: never; speaker?: never }
);

type StreetActor = Participant & {
  elapsed: number;
  availableAt: number;
  pause: Pause | null;
  base: CharacterPose;
  held: CharacterPose;
};

export function createStreetLife(
  participants: readonly Participant[],
  { random = Math.random, stops = [] }: { random?: () => number; stops?: readonly StreetStop[] } = {},
) {
  const actors = participants.map<StreetActor>(({ id, market, bellAttention, sample }) => {
    const base = sample(0);
    return {
      id, market, bellAttention, sample, elapsed: 0, availableAt: 0, pause: null, base,
      held: { position: base.position, direction: [0, 1], mode: 'idle', ticks: 0 },
    };
  });
  const places = stops.map<Place>((stop) => ({
    ...stop, occupant: null, nearby: new Set(),
    reach: stop.radius ** 2, exit: (stop.radius + 0.35) ** 2,
  }));
  const market = actors.filter(({ market }) => market);
  const pairs: { first: StreetActor; second: StreetActor; near: boolean }[] = [];
  for (let i = 0; i < market.length; i++) {
    for (let j = i + 1; j < market.length; j++) {
      pairs.push({ first: market[i], second: market[j], near: false });
    }
  }
  const views = new Map<string, CharacterPose>();
  let last = 0;

  function face(actor: StreetActor, dx: number, dz: number) {
    const direction = actor.held.direction;
    const across = Math.abs(dx) >= Math.abs(dz);
    direction[0] = across ? Math.sign(dx) || 1 : 0;
    direction[1] = across ? 0 : Math.sign(dz);
  }

  return (time: number, courtActive = false, toll?: CityPosition) => {
    for (const actor of actors) {
      const resumesAt = actor.pause?.ends ?? last;
      if (time >= resumesAt) {
        actor.elapsed += time - Math.max(last, resumesAt);
        if (actor.pause?.place) actor.pause.place.occupant = null;
        actor.pause = null;
      }
      actor.base = actor.sample(actor.elapsed);
      if (toll && actor.bellAttention && !actor.pause) {
        const [x, y, z] = actor.base.position;
        const dx = toll[0] - x;
        const dz = toll[2] - z;
        if (Math.abs(toll[1] - y) <= floorTolerance && dx * dx + dz * dz <= 8 ** 2) {
          actor.pause = { start: time, ends: time + actor.bellAttention };
          face(actor, dx, dz);
        }
      }
    }
    last = time;

    for (const pair of pairs) {
      const { first, second } = pair;
      const [x, y, z] = first.base.position;
      const [otherX, otherY, otherZ] = second.base.position;
      const dx = otherX - x;
      const dz = otherZ - z;
      const distanceSquared = dx * dx + dz * dz;
      if (Math.abs(y - otherY) > floorTolerance || distanceSquared > separationDistanceSquared) {
        pair.near = false;
        continue;
      }
      if (pair.near || distanceSquared > speakingDistanceSquared ||
          (first.base.mode !== 'walk' && second.base.mode !== 'walk')) continue;
      pair.near = true;

      if (first.pause || second.pause || Math.max(first.availableAt, second.availableAt) > time) continue;
      if (random() >= 0.64) continue;

      const ends = time + 2.4 + random() * 1.7;
      const speaker = first.base.mode === 'walk' ? first : second;
      first.pause = second.pause = { start: time, ends, speaker };
      first.availableAt = second.availableAt = ends + 7.5;
      face(first, dx, dz);
      face(second, -dx, -dz);
    }

    for (const place of places) {
      const { near, reach, exit, nearby, mode } = place;
      for (const actor of actors) {
        const [x, y, z] = actor.base.position;
        const distanceSquared = (x - near[0]) ** 2 + (z - near[2]) ** 2;
        if (Math.abs(y - near[1]) > floorTolerance || distanceSquared > exit) {
          nearby.delete(actor);
          continue;
        }
        if (nearby.has(actor) || distanceSquared > reach || actor.base.mode !== 'walk'
            || (mode === 'watch' && !courtActive)) continue;
        nearby.add(actor);
        if (place.occupant || actor.pause || actor.availableAt > time || random() >= 0.74) continue;

        const ends = time + 3 + random() * 1.35;
        place.occupant = actor;
        actor.pause = { start: time, ends, place };
        actor.availableAt = ends + 6;
        face(actor, place.face[0] - x, place.face[2] - z);
      }
    }

    for (const actor of actors) {
      if (!actor.pause) {
        views.set(actor.id, actor.base);
        continue;
      }
      const { held, pause } = actor;
      const sinceStart = time - pause.start;
      held.position = actor.base.position;
      if (pause.speaker) {
        const firstTurn = Math.floor(sinceStart / turnLength) % 2 === 0;
        held.mode = (pause.speaker === actor) === firstTurn ? 'talk' : 'idle';
        held.ticks = Math.floor((sinceStart % turnLength) * 30);
      } else {
        held.mode = pause.place?.mode ?? 'watch';
        held.ticks = Math.floor(sinceStart * 30);
      }
      views.set(actor.id, held);
    }
    return views;
  };
}
import type { CharacterPose, CharacterSampler, CityPosition } from './city-characters.ts';

