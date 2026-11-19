import type spriteData from './tactics-sprite-data.json';

type SpriteData = typeof spriteData;
type SpriteCell = SpriteData['cells'][number];
type SpriteSequence = SpriteData['sequences'][number] & { totalTicks: number };
type SequenceAnimation = SpriteData['animations'][number] & { idle?: false };
type IdleAnimation = {
  name: 'Idle';
  idle: true;
  loop: false;
  water: false;
  hitTick: null;
};
type SpriteAnimation = SequenceAnimation | IdleAnimation;

interface SpriteAtlas {
  data: SpriteData;
  cells: Record<string, SpriteCell>;
  animations: Map<string, SpriteAnimation>;
  sequences: SpriteSequence[];
  bodyTile: number[];
}

interface SpriteTile {
  id: string;
  sx: number;
  sy: number;
  w: number;
  h: number;
  x: number;
  y: number;
  flipX: boolean;
  order: number;
  body: boolean;
}

export interface SpritePose {
  tiles: SpriteTile[];
  count: number;
  shp: number;
  frame: number;
  done: boolean;
  back: boolean;
  mirrored: boolean;
  fb: number;
  du: number;
  moveX: number;
  moveY: number;
}

export const FACING: Readonly<{ NE: 7; SE: 1; SW: 3; NW: 5 }>;
export const STANCE_HALF_H: 20;
export function cameraStep(towardCameraX: number, towardCameraZ: number): number;
export function viewOf(facing: number, step: number): number;
export function createAtlas(data: SpriteData): SpriteAtlas;
export function createPose(): SpritePose;
export function findAnimation(atlas: SpriteAtlas, name: string): SpriteAnimation;
export function poseAt(
  atlas: SpriteAtlas,
  animation: SpriteAnimation,
  view: number,
  ticks: number,
  out?: SpritePose,
  loop?: boolean,
  water?: boolean,
): SpritePose;
export function drawPose(
  context: CanvasRenderingContext2D,
  image: CanvasImageSource,
  pose: SpritePose,
  x: number,
  y: number,
  scale?: number,
): void;
export function handPoint(
  atlas: SpriteAtlas,
  pose: SpritePose,
  arm?: 'first' | 'last',
  out?: [number, number],
): [number, number] | null;
