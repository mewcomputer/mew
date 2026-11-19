import data from '../vendor/tactics/tactics-sprite-data.json' with { type: 'json' };
import {
  cameraStep,
  createAtlas,
  createPose,
  drawPose,
  FACING,
  findAnimation,
  handPoint,
  poseAt,
  STANCE_HALF_H,
  type SpritePose,
  viewOf,
} from '../vendor/tactics/tactics-sprite.mjs';
import { staffAtlas as heldWalkAtlas } from './staff-pose.ts';

export const spriteCanvasSize = 72;
export const citySpritePixelSize = 0.0375;
export const spriteGround = (spriteCanvasSize / 2 + STANCE_HALF_H) / spriteCanvasSize;

const atlas = createAtlas(data);
const idle = findAnimation(atlas, 'Idle');
const walk = findAnimation(atlas, 'Walk');
const talk = findAnimation(atlas, 'Talk Skill');
const guard = findAnimation(atlas, 'Defend');
const sip = findAnimation(atlas, 'Found Item');
const poses = { idle, walk, talk, guard, sip, watch: idle };

interface Costume {
  hair: [number, number, number, number, number];
  coat: [number, number, number];
  work: ReturnType<typeof findAnimation>;
  instrument?: number;
}
const roles = {
  Research: {
    hair: [0x243536, 0x314649, 0x516463, 0x7c8c80, 0xacb19c],
    coat: [0x263d3b, 0x477a70, 0x6ea995],
    work: findAnimation(atlas, 'Read Book'),
    instrument: 0xa4e1d2,
  },
  Code: {
    hair: [0x382925, 0x4c342b, 0x74483a, 0xa76c50, 0xe3ae78],
    coat: [0x3b3833, 0x785541, 0xb37854],
    work: findAnimation(atlas, 'Flip Ground Switch'),
    instrument: 0xecc282,
  },
  Browse: {
    hair: [0x283344, 0x344a61, 0x53687c, 0x789ca8, 0xb2cdcd],
    coat: [0x24364a, 0x435e79, 0x668ba4],
    work: findAnimation(atlas, 'Found Item'),
    instrument: 0x9acfdf,
  },
  Write: {
    hair: [0x302735, 0x47364c, 0x745569, 0xa97c88, 0xe3bbc0],
    coat: [0x39313e, 0x685570, 0xa187a7],
    work: findAnimation(atlas, 'Read Book 2'),
    instrument: 0xe6d5b6,
  },
  Merchant: {
    hair: [0x332827, 0x48322d, 0x6b4b3c, 0x967247, 0xbc995e],
    coat: [0x443039, 0x8f5641, 0xb77a56],
    work: findAnimation(atlas, 'Read Book'),
    instrument: 0xe2cba0,
  },
  Ferrykeeper: {
    hair: [0x2b3432, 0x39453b, 0x596453, 0x7d8360, 0xa3a175],
    coat: [0x2d4546, 0x587c76, 0x82a49a],
    work: findAnimation(atlas, 'Flip Ground Switch'),
    instrument: 0xcfa875,
  },
  Warden: {
    hair: [0x273040, 0x3b4855, 0x596d77, 0x7e9194, 0xa0b6ac],
    coat: [0x323a59, 0x506885, 0x7b94a0],
    work: findAnimation(atlas, 'Found Item'),
    instrument: 0x9acfdf,
  },
  Disciple: {
    hair: [0x32242b, 0x46303c, 0x694a58, 0x91717b, 0xaf9591],
    coat: [0x49313a, 0x9f5554, 0xcd8270],
    work: findAnimation(atlas, 'Swing Mid'),
  },
  Performer: {
    hair: [0x252331, 0x38283f, 0x573956, 0x8d597b, 0xc38798],
    coat: [0x45375a, 0x81608d, 0xc393aa],
    work: findAnimation(atlas, 'Found Item'),
    instrument: 0xedc592,
  },
} satisfies Record<string, Costume>;

export type SpriteRole = keyof typeof roles;
export type SpriteMode = keyof typeof poses | 'work';
export type SpriteArtwork = ReadonlyMap<SpriteRole, HTMLCanvasElement>;

export function spriteFacing([x, z]: readonly [number, number]) {
  return x > 0 ? FACING.SE : x < 0 ? FACING.NW : z > 0 ? FACING.SW : FACING.NE;
}

export function spriteView(direction: readonly [number, number], yaw: number) {
  return viewOf(spriteFacing(direction), cameraStep(Math.sin(yaw), Math.cos(yaw)));
}

export function spritePose(role: SpriteRole, mode: SpriteMode, view: number, ticks: number, out?: SpritePose) {
  const animation = mode === 'work' ? roles[role].work : poses[mode] ?? idle;
  return poseAt(atlas, animation, view, ticks, out, mode === 'work' || mode === 'walk');
}

function tintCostume(image: HTMLImageElement, role: Costume) {
  const sheet = document.createElement('canvas');
  sheet.width = image.naturalWidth;
  sheet.height = image.naturalHeight;
  const context = sheet.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error('Unable to prepare sprite costume');
  context.drawImage(image, 0, 0);
  const pixels = context.getImageData(0, 0, sheet.width, sheet.height);
  const { hair, coat } = role;
  // Palette substitutions preserve the licensed sheet's layer masks and hand anchors.
  const colors = new Map([
    [0x252028, 0x202a2d],
    [0x281626, hair[0]],
    [0x5c2259, hair[1]],
    [0x89326d, hair[2]],
    [0xa34d95, hair[2]],
    [0xae4b9b, hair[3]],
    [0xc972ad, hair[3]],
    [0xda7bc4, hair[4]],
    [0xf5adde, hair[4]],
    [0x263d33, coat[0]],
    [0x2c5b46, coat[1]],
    [0x237253, coat[2]],
    [0x888988, 0x928980],
    [0xb8b9b8, 0xd0c3ad],
    [0xe0e1e0, 0xf2e5c9],
  ]);
  for (let i = 0; i < pixels.data.length; i += 4) {
    if (!pixels.data[i + 3]) continue;
    const value = (pixels.data[i] << 16) | (pixels.data[i + 1] << 8) | pixels.data[i + 2];
    const tint = colors.get(value);
    if (tint === undefined) continue;
    pixels.data[i] = (tint >> 16) & 255;
    pixels.data[i + 1] = (tint >> 8) & 255;
    pixels.data[i + 2] = tint & 255;
  }
  context.putImageData(pixels, 0, 0);
  return sheet;
}

export async function loadAgentArtwork(
  url: string | undefined,
  outfits: readonly SpriteRole[] = ['Research', 'Code', 'Browse', 'Write'],
): Promise<SpriteArtwork> {
  if (!url) throw new Error('Missing sprite sheet URL');
  const image = new Image();
  image.src = url;
  await image.decode();
  return new Map(outfits.map((name) => [name, tintCostume(image, roles[name])]));
}

function drawHeadgear(context: CanvasRenderingContext2D, role: SpriteRole, pose: SpritePose) {
  const { data } = context.getImageData(16, 4, 40, 38);
  const hair = roles[role].hair;
  let top = -1,
    left = 40,
    right = -1;
  for (let y = 0; y < 38 && (top < 0 || y < top + 7); y++) {
    for (let x = 0; x < 40; x++) {
      const i = (y * 40 + x) * 4;
      const color = (data[i] << 16) | (data[i + 1] << 8) | data[i + 2];
      if (!data[i + 3] || !hair.includes(color)) continue;
      if (top < 0) top = y;
      left = Math.min(left, x);
      right = Math.max(right, x);
    }
  }
  if (top < 0) return;
  const y = 4 + top,
    x = 16 + Math.floor((left + right) / 2);
  const side = pose.mirrored ? -1 : 1;
  const stripe = (dx: number, dy: number, w: number, h: number, color: string) => {
    context.fillStyle = color;
    context.fillRect(x + dx, y + dy, w, h);
  };
  switch (role) {
    case 'Merchant':
      stripe(-5, 2, 11, 3, '#63402e');
      stripe(-4, 1, 9, 1, '#d3a764');
      break;
    case 'Ferrykeeper':
      stripe(-9, 2, 19, 2, '#6e5836');
      stripe(-6, 0, 13, 2, '#a48651');
      stripe(-8, 2, 17, 1, '#c4a26a');
      break;
    case 'Warden':
      stripe(-6, 1, 13, 2, '#333c50');
      stripe(-1, 0, 3, 2, '#c6a66d');
      break;
    case 'Disciple':
      stripe(-5, 3, 10, 1, '#b95549');
      stripe(side * 5, 4, 2, 6, '#984438');
      break;
    case 'Performer':
      stripe(-6, 2, 13, 2, '#4d3a64');
      stripe(-2, 0, 5, 2, '#edc592');
      stripe(side * 5, 4, 2, 5, '#ca819b');
      break;
    case 'Browse':
      stripe(-8, 1, 17, 2, '#263845');
      stripe(-5, -1, 11, 2, '#7d9ba3');
      stripe(-8, 1, 17, 1, '#b6c7be');
      stripe(-3, -2, 6, 1, '#416776');
      break;
    case 'Code':
      stripe(-4, 3, 9, 2, '#502e2b');
      stripe(-3, 3, 7, 1, '#b76955');
      stripe(side * 4, 3, 2, 4, '#d39373');
      break;
    case 'Research':
      stripe(side * 5, 2, 2, 5, '#284447');
      stripe(side * 5, 2, 1, 3, '#9acdb5');
      break;
    case 'Write':
      stripe(-2, 1, 8, 1, '#ad8e65');
      stripe(side * 5, 0, 2, 2, '#e5cba1');
      break;
  }
}

export function createAgentPainter(role: SpriteRole, artwork: SpriteArtwork) {
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = spriteCanvasSize;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) throw new Error('Unable to draw character sprite');
  const costume = artwork.get(role);
  if (!costume) throw new Error(`Missing sprite costume for ${role}`);
  const pose = createPose();
  const grip: [number, number] = [0, 0];
  const center = spriteCanvasSize / 2;
  let previous: string | undefined;
  let held = false;

  return {
    canvas,
    hand() {
      return held ? handPoint(heldWalkAtlas, pose, 'last', grip) : null;
    },
    paint(mode: SpriteMode, view: number, ticks: number, carrying = false) {
      held = carrying;
      if (carrying) poseAt(heldWalkAtlas, walk, view, mode === 'walk' ? ticks : 0, pose, true);
      else spritePose(role, mode, view, ticks, pose);
      const steam = mode === 'sip' ? Math.floor(ticks / 7) % 3 : 0;
      const key = [mode, view, pose.shp, pose.moveX, pose.moveY, steam, carrying].join(':');
      if (key === previous) return false;
      previous = key;
      context.clearRect(0, 0, canvas.width, canvas.height);
      drawPose(context, costume, pose, center, center);
      drawHeadgear(context, role, pose);
      if (mode === 'sip' && handPoint(atlas, pose, 'last', grip)) {
        const x = Math.floor(center + grip[0]);
        const y = Math.floor(center + grip[1]);
        context.fillStyle = '#705243';
        context.fillRect(x - 2, y - 2, 5, 4);
        context.fillStyle = '#dfd2a8';
        context.fillRect(x - 1, y - 2, 3, 3);
        context.fillStyle = '#efe0c59e';
        context.fillRect(x - 1 + steam, y - 5 - steam, 1, 2);
      }
      if (mode === 'work' && (role === 'Disciple' || !pose.back) && handPoint(atlas, pose, 'last', grip)) {
        const x = Math.floor(center + grip[0]),
          y = Math.floor(center + grip[1]);
        const sign = pose.mirrored ? -1 : 1;
        if (role === 'Disciple') {
          context.fillStyle = '#60463c';
          context.fillRect(x - 1, y - 1, 3, 2);
          context.fillStyle = '#e2ded0';
          for (let i = 1; i <= 9; i++) {
            context.fillRect(x + sign * i, y - Math.floor(i * 0.6) - 1, 2, 1);
          }
          return true;
        }
        context.fillStyle = '#243b40';
        context.fillRect(x + sign * 2 - 1, y - 3, 3, 4);
        context.fillStyle = '#' + roles[role].instrument.toString(16);
        context.fillRect(x + sign * 2, y - 4, 1, 3);
      }
      return true;
    },
  };
}
