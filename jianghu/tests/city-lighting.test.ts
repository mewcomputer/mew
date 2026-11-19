import assert from 'node:assert/strict';
import test from 'node:test';
import * as T from 'three';
import { cityLighting, defaultCityTime, mountCityLighting, sampleCityTime } from '../src/city-lighting.ts';

test('the direct sun follows the same direction and color as the water', () => {
  const scene = new T.Scene();
  const lighting = mountCityLighting(T, scene);
  const sun = scene.children.find((child) => child instanceof T.DirectionalLight)!;
  assert.ok(sun.castShadow);
  assert.ok(scene.children.includes(sun.target));
  const direction = sun.position.clone().sub(sun.target.position).normalize();
  assert.ok(direction.distanceTo(new T.Vector3(...sampleCityTime(defaultCityTime).direction).normalize()) < 1e-6);
  assert.ok(sun.color.equals(new T.Color(cityLighting.color)));
  lighting.dispose();
});

test('the sky lights materials from above and releases its texture on teardown', () => {
  const scene = new T.Scene();
  const lighting = mountCityLighting(T, scene);
  const sky = scene.environment;
  assert.ok(sky instanceof T.DataTexture);
  assert.equal(sky.mapping, T.EquirectangularReflectionMapping);
  const { data, width, height } = sky.image;
  const nadir = 0;
  const zenith = (height - 1) * width * 4;
  assert.ok(data[zenith + 2] > data[nadir + 2]);
  assert.ok(data[zenith + 2] > data[zenith]);
  let disposed = false;
  sky.addEventListener('dispose', () => { disposed = true; });
  lighting.dispose();
  assert.equal(scene.environment, null);
  assert.ok(disposed);
});

test('night lowers the sky and key light and daylight can be restored', () => {
  const scene = new T.Scene();
  const lighting = mountCityLighting(T, scene);
  const key = scene.children.find((child) => child instanceof T.DirectionalLight)!;
  const day = { color: key.color.clone(), intensity: key.intensity, environment: scene.environmentIntensity, position: key.position.clone() };
  const dayFog = scene.fog!.color.clone();

  lighting.setTime(sampleCityTime(0));
  assert.ok(key.intensity < day.intensity / 2);
  assert.ok(key.color.b > key.color.r);
  assert.ok(scene.environmentIntensity < day.environment);
  assert.ok(scene.fog!.color.b > scene.fog!.color.r);
  assert.ok(!key.position.equals(day.position));

  lighting.setTime(sampleCityTime(defaultCityTime));
  assert.equal(key.intensity, day.intensity);
  assert.equal(scene.environmentIntensity, day.environment);
  assert.ok(key.color.equals(day.color));
  assert.ok(key.position.equals(day.position));
  assert.ok(scene.fog!.color.equals(dayFog));
  lighting.dispose();
});
