import type * as Three from 'three';

export const defaultCityTime = 16 * 60;

export const cityLighting = {
  exposure: 1.04,
  color: '#ffe0b5',
  intensity: 3.1,
  background: '#778d93',
  fog: '#9dabad',
  sky: '#c9dbf5',
  ground: '#566256',
  fill: 0.82,
  environment: 0.38,
};

const cityNightLighting = {
  direction: [0.62, 0.69, 0.38] as const,
  color: '#acc7ff',
  intensity: 0.64,
  background: '#121d35',
  fog: '#303f5b',
  sky: '#9cbaff',
  ground: '#344460',
  fill: 0.36,
  environment: 0.07,
};

function smoothstep(start: number, end: number, value: number) {
  const t = Math.max(0, Math.min(1, (value - start) / (end - start)));
  return t * t * (3 - 2 * t);
}

export function sampleCityTime(minutes: number) {
  const minute = ((minutes % 1440) + 1440) % 1440;
  const daylight = smoothstep(330, 450, minute) * (1 - smoothstep(1050, 1170, minute));
  const night = 1 - daylight;
  const twilight = 4 * daylight * night;
  const solarArc = Math.PI * Math.max(0, Math.min(1, (minute - 360) / 720));
  const [mx, my, mz] = cityNightLighting.direction;
  return {
    night,
    twilight,
    direction: [
      mx * night + Math.cos(solarArc) * 0.843 * daylight,
      my * night + (0.21 + Math.sin(solarArc) * 0.8) * daylight,
      mz * night + 0.35 * daylight,
    ] as const,
    intensity: (cityLighting.intensity * daylight + cityNightLighting.intensity * night) * (1 - twilight * 0.63),
  };
}

export type CityTime = ReturnType<typeof sampleCityTime>;

export function mountCityLighting(T: typeof Three, scene: Three.Scene) {
  const background = new T.Color();
  const fog = new T.Fog(cityLighting.fog, 127, 246);
  scene.background = background;
  scene.fog = fog;

  const afternoon = sampleCityTime(defaultCityTime);
  const direction = new T.Vector3(...afternoon.direction).normalize();
  const key = new T.DirectionalLight();
  key.target.position.set(0, 6, -18);
  key.castShadow = true;
  Object.assign(key.shadow.camera, { left: -83, right: 83, top: 68, bottom: -76, far: 221 });
  key.shadow.camera.updateProjectionMatrix();
  key.shadow.mapSize.set(3072, 3072);
  key.shadow.radius = 1.9;
  key.shadow.normalBias = 0.11;
  key.shadow.bias = -0.00022;
  const fill = new T.HemisphereLight();
  scene.add(key, key.target, fill);

  const sky = createSky(T, direction);
  scene.environment = sky;
  const twilightKey = new T.Color('#ffae76');
  const twilightSky = new T.Color('#7d637c');
  const twilightFog = new T.Color('#ac8186');
  const nightColor = new T.Color();
  function setTime({ night, twilight, direction: lightDirection, intensity }: CityTime) {
    direction.fromArray(lightDirection).normalize();
    key.position.copy(key.target.position).addScaledVector(direction, 112);
    key.color.set(cityLighting.color).lerp(nightColor.set(cityNightLighting.color), night).lerp(twilightKey, twilight * 0.82);
    key.intensity = intensity;
    fill.color.set(cityLighting.sky).lerp(nightColor.set(cityNightLighting.sky), night);
    fill.groundColor.set(cityLighting.ground).lerp(nightColor.set(cityNightLighting.ground), night);
    fill.intensity = T.MathUtils.lerp(cityLighting.fill, cityNightLighting.fill, night);
    background.set(cityLighting.background).lerp(nightColor.set(cityNightLighting.background), night).lerp(twilightSky, twilight * 0.79);
    fog.color.set(cityLighting.fog).lerp(nightColor.set(cityNightLighting.fog), night).lerp(twilightFog, twilight * 0.74);
    scene.environmentIntensity = T.MathUtils.lerp(cityLighting.environment, cityNightLighting.environment, night);
  }
  setTime(afternoon);
  return {
    setTime,
    keyColor: key.color,
    dispose() {
      scene.environment = null;
      sky.dispose();
    },
  };
}

function createSky(T: typeof Three, sun: Three.Vector3) {
  const width = 128;
  const height = 64;
  const data = new Uint8Array(width * height * 4);
  const ground = new T.Color(0.08, 0.10, 0.10);
  const horizon = new T.Color(0.52, 0.55, 0.59);
  const zenith = new T.Color(0.23, 0.39, 0.67);
  const glow = new T.Color(0.38, 0.22, 0.09);
  const direction = new T.Vector3();
  const color = new T.Color();

  for (let row = 0; row < height; row++) {
    const latitude = ((row + 0.5) / height - 0.5) * Math.PI;
    const elevation = Math.sin(latitude);
    const band = Math.cos(latitude);
    const base = elevation < 0 ? ground : zenith;
    const heightMix = Math.sqrt(Math.abs(elevation));
    for (let column = 0; column < width; column++) {
      const longitude = ((column + 0.5) / width - 0.5) * 2 * Math.PI;
      direction.set(band * Math.cos(longitude), elevation, band * Math.sin(longitude));
      const sunGlow = Math.max(0, direction.dot(sun)) ** 8;
      color.copy(horizon).lerp(base, heightMix);
      color.r += glow.r * sunGlow;
      color.g += glow.g * sunGlow;
      color.b += glow.b * sunGlow;
      const offset = (row * width + column) * 4;
      data[offset] = Math.round(color.r * 255);
      data[offset + 1] = Math.round(color.g * 255);
      data[offset + 2] = Math.round(color.b * 255);
      data[offset + 3] = 255;
    }
  }

  const sky = new T.DataTexture(data, width, height, T.RGBAFormat);
  sky.mapping = T.EquirectangularReflectionMapping;
  sky.colorSpace = T.LinearSRGBColorSpace;
  sky.minFilter = sky.magFilter = T.LinearFilter;
  sky.needsUpdate = true;
  return sky;
}
