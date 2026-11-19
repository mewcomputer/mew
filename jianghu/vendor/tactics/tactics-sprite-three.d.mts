import type * as Three from 'three';
import type { createAtlas, SpritePose } from './tactics-sprite.mjs';

interface TacticsSprite {
  readonly group: Three.Group;
  readonly player: { facing: number };
  readonly standHeight: number;
  itemLayer: number | null;
  update(seconds: number, camera: Three.Camera): SpritePose;
  hand(arm: 'first' | 'last', depth: number, out: Three.Vector3): Three.Vector3 | null;
}

export function prepareSheet(texture: Three.Texture): Three.Texture;
export function createTacticsSprite(
  atlas: ReturnType<typeof createAtlas>,
  texture: Three.Texture,
  options: { animation: string; scaleX: number; scaleY: number },
): TacticsSprite;
export const flattenOntoSprite: Three.Material['onBeforeRender'];
