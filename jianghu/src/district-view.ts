import type * as Three from 'three';
import { materials, type District } from './district.ts';
import { createDistrictScene } from './district-scene.ts';
import { createTerrainScene } from './terrain-scene.ts';
import { createWaterSimulation } from './fluid.ts';
import { createWaterScene } from './water-scene.ts';
import { waterLayer } from './water-material.ts';
import { createWaterClock } from './water-clock.ts';
import { createSceneView, type SceneHooks, type SceneViewOptions } from './scene-view.ts';

type DistrictViewOptions = Omit<SceneViewOptions, 'orbit'> & {
  orbit: Omit<SceneViewOptions['orbit'], 'height'>;
};

type DistrictHooks = Pick<SceneHooks, 'afterRender' | 'onContextChange' | 'onHover' | 'onTap' | 'onDrag' | 'onLeave'> & {
  beforeRender?: (now: number) => void;
};

export function createDistrictView(
  T: typeof Three, canvas: HTMLCanvasElement, district: District, options: DistrictViewOptions,
) {
  const view = createSceneView(T, canvas, {
    ...options,
    orbit: { ...options.orbit, height: 12.5 },
  });
  const { scene, renderer, camera } = view;
  camera.layers.enable(waterLayer);
  const { world, hydraulics } = district;
  const terrain = createTerrainScene(T, scene, world, materials);
  const { architecture, landmarks } = createDistrictScene(T, scene, district);
  const water = createWaterSimulation(world, hydraulics);
  const waterScene = createWaterScene(T, scene, world, water, renderer);
  for (let i = 0; i < 810; i++) water.step();
  waterScene.prime();
  const clock = createWaterClock(water);

  return {
    ...view,
    get yaw() { return view.yaw; },
    get targetZ() { return view.targetZ; },
    terrain, architecture, landmarks, water, waterScene,
    start(hooks: DistrictHooks) {
      view.start({
        ...hooks,
        beforeRender(now) {
          hooks.beforeRender?.(now);
          waterScene.capture(camera);
        },
        onInvalidate: waterScene.invalidate,
        onSuspend: clock.reset,
        onDispose() {
          water.dispose();
          waterScene.dispose();
        },
      });
    },
    get waterTime() { return clock.simulated; },
    resetClock: clock.reset,
    advanceWater(now: number, active: boolean) {
      if (clock.advance(now, active)) waterScene.update(clock.elapsed, clock.simulated);
    },
  };
}
