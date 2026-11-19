import type * as Three from 'three';
import type { SpriteArtwork, SpriteMode, SpriteRole } from './agent-sprites.ts';
import { citySpritePixelSize, createAgentPainter, spriteCanvasSize, spriteGround, spriteView } from './agent-sprites.ts';
import {
  createCityEncounters, gaugeSite, practiceSite, teaStand,
} from './city-encounters.ts';
import { operaHouse, rainbellBridge, rainbellGarden, rainbellSite, rainbellSteps } from './rainbell-world.ts';
import { createOperaShow, operaCast } from './opera-show.ts';
import { createStreetLife } from './street-life.ts';

export type CityPosition = [x: number, y: number, z: number];
export type CityDirection = [x: number, z: number];

export type CharacterPose = {
  position: CityPosition;
  direction: CityDirection;
  mode: SpriteMode;
  ticks: number;
};

export type CharacterSampler = (time: number) => CharacterPose;

type CharacterTrack = {
  path: readonly [CityPosition, ...CityPosition[]];
  direction?: CityDirection;
  speed?: number;
  activity?: 'work';
  phase?: number;
};

export type CityCharacter = CharacterTrack & {
  id: string;
  outfit: SpriteRole;
  market?: boolean;
  street?: boolean;
  bellAttention?: number;
};

const { deck: gardenDeck, steps: gardenSteps, pathX: gardenPathX, lookoutZ } = rainbellGarden;
const ticketClerk = 'fang-ning';

export const baichuanCast: CityCharacter[] = [
  {
    id: 'lin-qiao', outfit: 'Merchant', market: true, speed: 1.05, phase: 0.5,
    path: [[-11.55, 3.1, 11.42], [-11.55, 3.1, 8.58], [-15.20, 3.1, 8.58]],
  },
  {
    id: 'he-yan', outfit: 'Merchant', market: true, speed: 1.14, phase: 0.05,
    path: [[-16.1, 3.1, 6.2], [-14.0, 3.1, 7.72], [-11.40, 3.1, 7.72]],
  },
  { id: 'da-sen', outfit: 'Warden', market: true, path: [[-13.6, 3.1, 8.8], [-13.6, 3.1, 10.22]], speed: 1.03, phase: 1.8 },
  {
    id: 'lu-ning', outfit: 'Ferrykeeper', street: true, speed: 0.94, phase: 0.87,
    path: [[-19.78, 3.1, 15.56], [-19.13, 3.1, 14.34], [-19.13, 3.1, 12.20]],
  },
  { id: 'fan-rui', outfit: 'Warden', path: [[4.33, 5.63, 2.65], [10.1, 5.63, 2.65]], speed: 1.24, phase: 0.2 },
  { id: 'wen-yu', outfit: 'Ferrykeeper', path: [[-2.84, 5.63, 2.62], [-9.7, 5.63, 2.62]], speed: 1.03, phase: 1.0 },
  { id: 'shen-tan', outfit: 'Merchant', market: true, path: [[13.8, 3.1, 5.9], [13.8, 3.1, 8.1]], speed: 0.79, phase: 1.5 },
  { id: 'qi-mei', outfit: 'Warden', market: true, path: [[17.05, 3.1, 8.36], [15, 3.1, 8.36]], speed: 1.10, phase: 0.8 },
  { id: 'bao-ren', outfit: 'Merchant', path: [[-13.8, 0.91, 34]], direction: [0, 1], activity: 'work', phase: 2.7 },
  { id: 'su-lian', outfit: 'Ferrykeeper', path: [[-8.35, 0.91, 31.65], [-8.35, 0.91, 33.8]], speed: 1.14, phase: 0.0 },
  { id: 'gao-wu', outfit: 'Warden', path: [[-1.4, 0.91, 26.82], [-5.7, 0.91, 26.82]], speed: 1.14, phase: 0.5 },
  { id: 'yan-hong', outfit: 'Ferrykeeper', path: [[3.15, 0.91, 26.81], [6.05, 0.91, 26.81]], speed: 0.93, phase: 1.15 },
  { id: 'xu-chun', outfit: 'Ferrykeeper', path: [[gaugeSite[0] - 1.6, gaugeSite[1], gaugeSite[2] + 2.88]] },
  {
    id: 'mo-qin', outfit: 'Merchant', street: true, speed: 0.96, phase: 1.1,
    path: [[23.18, 2.45, 32.9], [23.18, 2.45, 31.24], [25.5, 2.45, 31.24]],
  },
  { id: 'ye-xia', outfit: 'Disciple', path: [[24.12, 2.65, 28.11]] },
  { id: 'ning-fei', outfit: 'Disciple', path: [[25.94, 2.65, 29.08]] },
  {
    id: 'yu-lin', outfit: 'Warden', market: true, bellAttention: 0.95, speed: 1.34, phase: 0.6,
    path: [
      [10.65, rainbellBridge.y, 37.6],
      [10.65, rainbellBridge.y, rainbellBridge.z],
      [rainbellSteps.x, rainbellBridge.y, rainbellBridge.z],
      [rainbellSteps.x, rainbellBridge.y, rainbellSteps.z - rainbellSteps.run],
      [rainbellSteps.x, rainbellSite[1], rainbellSteps.z],
      [rainbellSteps.x, rainbellSite[1], 46.85],
      [-22.30, rainbellSite[1], 46.85],
    ],
  },
  {
    id: 'song-hui', outfit: 'Merchant', market: true, bellAttention: 1.55, speed: 0.94, phase: 3.2,
    path: [[-24.2, rainbellSite[1], 46.0], [-9.48, rainbellSite[1], 46.0], [-9.48, rainbellSite[1], 51.47]],
  },
  { id: 'pei-shu', outfit: 'Ferrykeeper', path: [[-21.9, rainbellSite[1], 52.55]] },
  {
    id: ticketClerk, outfit: 'Write', direction: [0, 1],
    path: [[operaHouse.ticketBooth.x, operaHouse.quay.y + 0.12, operaHouse.ticketBooth.z + 0.05]],
  },
  {
    id: 'qin-zhu', outfit: 'Merchant', bellAttention: 2.2, path: [[-12.4, rainbellSite[1], 51.28]],
    direction: [0, -1], activity: 'work', phase: 1,
  },
  {
    id: 'zhao-lan', outfit: 'Disciple', speed: 1.1, phase: 1.3,
    path: [
      [-24.3, rainbellSite[1], 50.02], [-21.4, rainbellSite[1], 50.02],
      [-21.4, rainbellSite[1], gardenSteps.z], [gardenSteps.x, rainbellSite[1], gardenSteps.z],
      [gardenSteps.x - gardenSteps.run, gardenDeck.y, gardenSteps.z],
      [gardenPathX, gardenDeck.y, gardenSteps.z], [gardenPathX, gardenDeck.y, lookoutZ],
    ],
  },
];

export function createCharacterTrack({
  path, direction = [0, 1], speed = 1, activity, phase = 0,
}: CharacterTrack): CharacterSampler {
  if (path.length === 1) {
    const state: CharacterPose = { position: [...path[0]], direction: [...direction], mode: 'idle', ticks: 0 };
    return (time) => {
      const beat = (time + phase) % 4.6;
      state.mode = activity && beat >= 1.1 && beat < 3.16 ? activity : 'idle';
      state.ticks = Math.max(0, Math.floor((beat - 1.1) * 30));
      return state;
    };
  }

  const motion = createCharacterPath(path, speed, direction);
  const travel = motion.duration;
  return (time) => {
    const elapsed = (time + phase) % (2 * travel + 2);
    const returning = elapsed >= travel + 1;
    const sinceEnd = returning ? elapsed - travel - 1 : elapsed;
    const moving = sinceEnd >= 1;
    const walkingTime = Math.min(travel, Math.max(0, sinceEnd - 1));
    const at = returning ? travel - walkingTime : walkingTime;
    return motion.sample(at, returning ? -1 : 1, moving ? 'walk' : 'idle', walkingTime);
  };
}

export function createCharacterPath(
  path: CharacterTrack['path'], speed: number, direction: CityDirection = [0, 1],
) {
  const state: CharacterPose = { position: [...path[0]], direction: [...direction], mode: 'idle', ticks: 0 };
  const legs: {
    from: CityPosition; to: CityPosition; dx: number; dz: number; at: number; duration: number;
  }[] = [];
  let travel = 0;
  for (let i = 1; i < path.length; i++) {
    const from = path[i - 1];
    const to = path[i];
    const dx = to[0] - from[0];
    const dz = to[2] - from[2];
    const length = Math.hypot(dx, dz);
    const duration = Math.max(length / speed, Math.abs(to[1] - from[1]) / 2);
    const previous = legs[legs.length - 1];
    legs.push({ from, to, dx: length ? dx / length : previous?.dx ?? direction[0],
      dz: length ? dz / length : previous?.dz ?? direction[1], at: travel, duration });
    travel += duration;
  }

  return {
    duration: travel,
    timeAtPoint: (index: number) => legs[index]?.at ?? travel,
    sample(at: number, sign: number, mode: SpriteMode, ticksAt = at): CharacterPose {
      const leg = legs.find(({ at: start, duration }) => at < start + duration) ?? legs[legs.length - 1];
      const t = leg.duration ? (at - leg.at) / leg.duration : 0;
      for (let i = 0; i < 3; i++) state.position[i] = leg.from[i] + (leg.to[i] - leg.from[i]) * t;
      state.direction[0] = leg.dx * sign || 0;
      state.direction[1] = leg.dz * sign || 0;
      state.mode = mode;
      state.ticks = Math.floor(ticksAt * speed * 27);
      return state;
    },
  };
}

export function createCitySpriteArt(T: typeof Three, outfit: SpriteRole, artwork: SpriteArtwork) {
  const painter = createAgentPainter(outfit, artwork);
  const texture = new T.CanvasTexture(painter.canvas);
  texture.magFilter = texture.minFilter = T.NearestFilter;
  texture.generateMipmaps = false;
  texture.colorSpace = T.SRGBColorSpace;
  return {
    texture,
    hand: painter.hand,
    paint(mode: SpriteMode, direction: CityDirection, yaw: number, ticks: number, carrying = false) {
      if (painter.paint(mode, spriteView(direction, yaw), ticks, carrying)) texture.needsUpdate = true;
    },
  };
}

export function createCitySprite(T: typeof Three, outfit: SpriteRole, artwork: SpriteArtwork) {
  const { texture, paint } = createCitySpriteArt(T, outfit, artwork);
  const sprite = new T.Sprite(new T.SpriteMaterial({ map: texture, alphaTest: 0.48, toneMapped: false }));
  sprite.center.set(0.5, 1 - spriteGround);
  const scale = spriteCanvasSize * citySpritePixelSize;
  sprite.scale.set(scale, scale, 1);
  return { sprite, paint };
}

export function mountCityCharacters(T: typeof Three, scene: Three.Scene, artwork: SpriteArtwork) {
  const encounters = createCityEncounters(baichuanCast);
  const show = createOperaShow();
  const ticket = createOperaTicket(T);
  const counter = operaHouse.ticketBooth;
  const counterFront = counter.z + counter.depth / 2;
  scene.add(ticket);
  const speech = createSpeechMarker(T);
  const shadowGeometry = new T.CircleGeometry(0.24, 12).rotateX(-Math.PI / 2);
  const shadowMaterial = new T.MeshBasicMaterial({
    color: '#24322e', transparent: true, opacity: 0.25, depthWrite: false,
  });
  const characters = baichuanCast.map((definition) => {
    const { sprite, paint } = createCitySprite(T, definition.outfit, artwork);
    sprite.name = definition.id;
    const shadow = new T.Mesh(shadowGeometry, shadowMaterial);
    scene.add(sprite, shadow);
    const marker = definition.market ? speech.clone() : null;
    if (marker) {
      marker.name = 'market-conversation-' + definition.id;
      scene.add(marker);
    }
    return {
      id: definition.id, market: definition.market, street: definition.street, bellAttention: definition.bellAttention,
      paint, sprite, shadow, marker, sample: createCharacterTrack(definition),
    };
  });
  const operaCharacters = operaCast.map(({ id, outfit }) => {
    const { sprite, paint } = createCitySprite(T, outfit, artwork);
    sprite.name = id;
    const shadow = new T.Mesh(shadowGeometry, shadowMaterial);
    scene.add(sprite, shadow);
    return { id, sprite, paint, shadow };
  });
  const streetLife = createStreetLife(characters.filter(({ market, street, bellAttention }) => market || street || bellAttention), {
    stops: [
      { mode: 'sip', near: [teaStand[0] + 1.07, teaStand[1], teaStand[2]], face: teaStand, radius: 0.62 },
      { mode: 'watch', near: [practiceSite[0], practiceSite[1], practiceSite[2] + 2.94], face: practiceSite, radius: 0.98 },
    ],
  });

  const frame = { gaugeLit: false, frontDoor: 0, rearDoor: 0 };
  return (time: number, yaw: number, bellRung = false, night = 0) => {
    const { actors, gaugeLit, courtActive } = encounters(time);
    const street = streetLife(time, courtActive, bellRung ? rainbellSite : undefined);
    const opera = show(time);
    for (const { id, paint, sprite, shadow, marker, sample } of characters) {
      const { position, direction, mode, ticks } = actors.get(id) ?? street.get(id) ?? sample(time);
      sprite.position.set(position[0], position[1] + 0.026, position[2]);
      sprite.material.color.setRGB(1 - 0.52 * night, 1 - 0.46 * night, 1 - 0.32 * night);
      shadow.position.set(position[0], position[1] + 0.012, position[2]);
      if (id === ticketClerk && opera.ticketBuyer) paint('work', direction, yaw, Math.floor(opera.ticketProgress * 30));
      else paint(mode, direction, yaw, ticks);
      if (marker) {
        marker.visible = mode === 'talk';
        if (marker.visible) marker.position.set(position[0], position[1] + 1.72, position[2]);
      }
    }
    const handoff = (opera.ticketProgress - 0.49) / 0.39;
    ticket.visible = handoff >= 0 && handoff <= 1;
    if (ticket.visible) ticket.position.set(counter.x + 0.16, operaHouse.quay.y + 1.1, counterFront - 0.17 + handoff * 0.63);
    for (const { id, sprite, paint, shadow } of operaCharacters) {
      const { position, direction, mode, ticks, visible } = opera.actors.get(id)!;
      sprite.visible = shadow.visible = visible;
      if (!visible) continue;
      sprite.position.set(position[0], position[1] + 0.026, position[2]);
      sprite.material.color.setRGB(1 - 0.52 * night, 1 - 0.46 * night, 1 - 0.32 * night);
      shadow.position.set(position[0], position[1] + 0.012, position[2]);
      paint(mode, direction, yaw, ticks);
    }
    frame.gaugeLit = gaugeLit;
    frame.frontDoor = opera.frontDoor;
    frame.rearDoor = opera.rearDoor;
    return frame;
  };
}

function createOperaTicket(T: typeof Three) {
  const canvas = document.createElement('canvas');
  canvas.width = 24;
  canvas.height = 13;
  const paint = canvas.getContext('2d');
  if (!paint) throw new Error('Could not draw the opera ticket.');
  paint.fillStyle = '#62493d';
  paint.fillRect(1, 1, 22, 11);
  paint.fillStyle = '#f4dcae';
  paint.fillRect(2, 2, 20, 9);
  paint.fillStyle = '#827157';
  paint.fillRect(5, 4, 11, 1);
  paint.fillRect(5, 7, 8, 1);
  paint.fillRect(18, 3, 1, 7);
  const map = new T.CanvasTexture(canvas);
  map.magFilter = map.minFilter = T.NearestFilter;
  map.generateMipmaps = false;
  map.colorSpace = T.SRGBColorSpace;
  const sprite = new T.Sprite(new T.SpriteMaterial({ map, alphaTest: 0.42, toneMapped: false }));
  sprite.name = 'opera-ticket-handoff';
  sprite.scale.set(0.34, 0.184, 1);
  sprite.visible = false;
  return sprite;
}

function createSpeechMarker(T: typeof Three) {
  const canvas = document.createElement('canvas');
  canvas.width = 32;
  canvas.height = 22;
  const context = canvas.getContext('2d');
  if (!context) throw new Error('Could not draw Baichuan speech markers.');
  context.fillStyle = '#535449';
  context.beginPath();
  context.roundRect(1, 1, 30, 16, 5);
  context.fill();
  context.fillStyle = '#f2ddb3';
  context.beginPath();
  context.roundRect(2, 2, 28, 14, 4);
  context.fill();
  context.beginPath();
  context.moveTo(10, 14);
  context.lineTo(9, 21);
  context.lineTo(17, 14);
  context.fill();
  context.fillStyle = '#59554d';
  for (const x of [8, 15, 22]) context.fillRect(x, 8, 3, 2);
  const map = new T.CanvasTexture(canvas);
  map.magFilter = map.minFilter = T.NearestFilter;
  map.generateMipmaps = false;
  map.colorSpace = T.SRGBColorSpace;
  const marker = new T.Sprite(new T.SpriteMaterial({ map, alphaTest: 0.42, toneMapped: false }));
  marker.scale.set(0.75, 0.515, 1);
  marker.visible = false;
  return marker;
}
