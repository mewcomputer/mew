import type { SpriteRole } from './agent-sprites.ts';
import type { CharacterPose, CityPosition } from './city-characters.ts';
import { operaHouse, rainbellWorld } from './rainbell-world.ts';
import { createTacticsMap, type MovementPoint, type TacticsTile } from './tactics-movement.ts';
import { createTacticsWalk } from './tactics-walk.ts';

type OperaGuest = {
  id: string;
  outfit: SpriteRole;
  entrance: 'public' | 'backstage';
  from: readonly [number, number];
  gathersAt: readonly [number, number];
  arrives: number;
  enters: number;
  leaves: number;
  buysTicket?: true;
};

export const operaCast: readonly OperaGuest[] = [
  { id: 'mei-yun', outfit: 'Merchant', entrance: 'public', from: [16.5, 61.4], gathersAt: [22.3, 65.9], arrives: 0, enters: 23, leaves: 58 },
  { id: 'du-ying', outfit: 'Write', entrance: 'public', from: [17.9, 60.6], gathersAt: [23.6, 65.8], arrives: 0.8, enters: 24.5, leaves: 59.6 },
  { id: 'qiu-han', outfit: 'Browse', entrance: 'public', from: [35.8, 61.1], gathersAt: [27.5, 67.7], arrives: 2.0, enters: 26, leaves: 61.1, buysTicket: true },
  { id: 'lin-xia', outfit: 'Ferrykeeper', entrance: 'public', from: [34.8, 60.1], gathersAt: [28.7, 67.6], arrives: 3.7, enters: 27.5, leaves: 62.5, buysTicket: true },
  { id: 'he-ling', outfit: 'Research', entrance: 'public', from: [18.3, 62.6], gathersAt: [24.8, 69.5], arrives: 5.1, enters: 29, leaves: 64 },
  { id: 'xie-jin', outfit: 'Warden', entrance: 'public', from: [38.3, 62.7], gathersAt: [32.1, 65.6], arrives: 5.8, enters: 30.5, leaves: 65.5, buysTicket: true },
  { id: 'tang-lu', outfit: 'Write', entrance: 'public', from: [37.15, 61.15], gathersAt: [33.25, 65.7], arrives: 7.4, enters: 32, leaves: 67 },
  { id: 'lu-xue', outfit: 'Performer', entrance: 'backstage', from: [35.7, 40.7], gathersAt: [44.7, 46.8], arrives: 0, enters: 11.5, leaves: 65 },
  { id: 'han-yue', outfit: 'Performer', entrance: 'backstage', from: [37.1, 39.4], gathersAt: [45.9, 47.8], arrives: 1.0, enters: 12.8, leaves: 67 },
  { id: 'shen-qiu', outfit: 'Performer', entrance: 'backstage', from: [38.4, 41.0], gathersAt: [47.1, 47.9], arrives: 1.8, enters: 13.9, leaves: 69 },
];

const showDuration = 94;
const ticketClerkOpens = 8.6;
const saleDuration = 1.45;
const queueAdvance = 0.70;
const queueSpacing = 1.08;

function rehearsedWalk(points: readonly MovementPoint[], speed: number) {
  let walk = createTacticsWalk(points, speed);
  let elapsed = 0;
  return (seconds: number) => {
    if (seconds < elapsed) {
      walk = createTacticsWalk(points, speed);
      elapsed = 0;
    }
    const pose = walk.advance(seconds - elapsed);
    elapsed = seconds;
    return pose;
  };
}

export function createOperaShow() {
  const map = createTacticsMap(rainbellWorld);
  const { court, quay, garden, rearCourt, porch, ticketBooth } = operaHouse;
  const actors = new Map<string, CharacterPose & { visible: boolean }>();
  const frame = { actors, frontDoor: 0, rearDoor: 0, ticketBuyer: null as string | null, ticketProgress: 0 };

  function nearest(surface: string, x: number, z: number) {
    let result: TacticsTile | undefined;
    let distance = Infinity;
    for (const tile of map.tiles) {
      if (tile.surface !== surface) continue;
      const next = Math.hypot(tile.x - x, tile.z - z);
      if (next < distance) {
        result = tile;
        distance = next;
      }
    }
    if (!result) throw new Error(`Missing an opera route tile on ${surface}.`);
    return result;
  }

  function route(from: TacticsTile, to: TacticsTile) {
    const path = map.plan(from, 80).pathTo(to);
    if (!path) throw new Error(`Opera route from ${from.surface} to ${to.surface} is blocked.`);
    return path;
  }

  const counter = { x: ticketBooth.x, y: quay.y, z: ticketBooth.z + ticketBooth.depth / 2 + 0.57 };
  const serveInterval = saleDuration + queueAdvance;
  let queueLength = 0;
  function buyTicket(home: TacticsTile, meeting: TacticsTile, arrives: number) {
    const order = queueLength++;
    const serviceAt = ticketClerkOpens + order * serveInterval;
    const paidAt = serviceAt + saleDuration;
    const slot = { ...counter, z: counter.z + order * queueSpacing };
    const lane = nearest(quay.id, counter.x - 1.6, slot.z);
    const join = rehearsedWalk([...route(home, lane), slot], 2.65);
    const gathering = rehearsedWalk([counter, ...route(nearest(quay.id, counter.x - 1.6, counter.z), meeting)], 2.5);
    const waiting = { ...slot, direction: [0, -1] as CharacterPose['direction'], ticks: 0, done: true };
    return {
      serviceAt, paidAt,
      sample(beat: number) {
        if (beat >= paidAt) return gathering(beat - paidAt);
        const arriving = join(Math.max(0, beat - arrives));
        if (!arriving.done) return arriving;
        const elapsed = Math.max(0, beat - ticketClerkOpens);
        const served = Math.min(order, Math.floor(elapsed / serveInterval));
        const fraction = served < order ? Math.max(0, (elapsed - served * serveInterval - saleDuration) / queueAdvance) : 0;
        waiting.z = counter.z + (order - served - fraction) * queueSpacing;
        waiting.done = fraction === 0;
        waiting.ticks = Math.floor(fraction * queueSpacing * 23);
        return waiting;
      },
    };
  }

  const guests = operaCast.map((guest, index) => {
    const publicEntry = guest.entrance === 'public';
    const door = publicEntry ? operaHouse.entrance : operaHouse.backstage;
    const home = nearest(publicEntry ? quay.id : rearCourt.id, ...guest.from);
    const meeting = nearest(publicEntry ? court.id : garden.id, ...guest.gathersAt);
    const direction = publicEntry ? -1 : 1;
    const threshold = nearest(publicEntry ? porch.id : rearCourt.id, door.x, door.z - direction * 0.8);
    const arrival = route(home, meeting);
    const entry = [
      ...route(meeting, threshold),
      { x: door.x, y: door.y, z: door.z - direction * 0.27 },
      { x: door.x, y: door.y, z: door.z + direction * (publicEntry ? 2.09 : 1.52) },
    ];
    const exit = [...entry].reverse().concat([...arrival].reverse().slice(1));
    const position: CityPosition = [home.x, home.y, home.z];
    const actor: CharacterPose & { visible: boolean } = { position, direction: [0, direction], mode: 'idle', ticks: 0, visible: true };
    actors.set(guest.id, actor);
    return {
      guest, actor, door, publicEntry, index,
      ticket: guest.buysTicket ? buyTicket(home, meeting, guest.arrives) : null,
      approach: rehearsedWalk(arrival, 2.0), enter: rehearsedWalk(entry, 2.45), depart: rehearsedWalk(exit, 2.35),
    };
  });

  return (time: number) => {
    const beat = time % showDuration;
    frame.frontDoor = frame.rearDoor = 0;
    frame.ticketBuyer = null;
    frame.ticketProgress = 0;
    for (const { guest, actor, door, publicEntry, index, ticket, approach, enter, depart } of guests) {
      const entering = beat >= guest.enters && beat < guest.leaves;
      const leaving = beat >= guest.leaves;
      const walk = leaving ? depart(beat - guest.leaves)
        : entering ? enter(beat - guest.enters)
        : ticket ? ticket.sample(beat) : approach(Math.max(0, beat - guest.arrives));
      actor.visible = !entering || !walk.done;
      actor.position[0] = walk.x;
      actor.position[1] = walk.y;
      actor.position[2] = walk.z;
      const gathering = !entering && !leaving && walk.done && (!ticket || beat >= ticket.paidAt);
      const buying = ticket && walk.done && beat >= ticket.serviceAt && beat < ticket.paidAt;
      if (buying) {
        frame.ticketBuyer = guest.id;
        frame.ticketProgress = (beat - ticket.serviceAt) / saleDuration;
      }
      actor.mode = buying ? 'work' : gathering && publicEntry && index % 4 < 2 ? 'talk'
        : walk.done || beat < guest.arrives ? 'idle' : 'walk';
      actor.ticks = buying ? Math.floor((beat - ticket.serviceAt) * 22)
        : actor.mode === 'talk' ? Math.floor(beat * 12 + index * 7) : walk.ticks;
      actor.direction[0] = gathering ? (index % 2 ? -1 : 1) : walk.direction[0];
      actor.direction[1] = gathering ? 0 : walk.direction[1];
      if ((entering && actor.visible) || (leaving && !walk.done)) {
        const distance = Math.hypot(walk.x - door.x, walk.y - door.y, walk.z - door.z);
        const open = Math.max(0, Math.min(1, (1.45 - distance) / 0.95));
        if (publicEntry) frame.frontDoor = Math.max(frame.frontDoor, open);
        else frame.rearDoor = Math.max(frame.rearDoor, open);
      }
    }
    return frame;
  };
}
