import type * as Three from 'three';
import { cityLighting, defaultCityTime, sampleCityTime, type CityTime } from './city-lighting.ts';
import {
  canalEnd,
  canalHalfWidth,
  feederProfile,
  reservoirProfile,
  spillwayCrest as crestLevel,
} from './city-scene.ts';

const basinLevel = 0.09;
const crestZ = -10.18;
const impactZ = -4.38;

const noise = `
  float hash(vec2 p) {
    return fract(sin(dot(p, vec2(32.17, 151.9))) * 43758.5453);
  }
  float noise(vec2 p) {
    vec2 id = floor(p);
    vec2 f = smoothstep(0., 1., fract(p));
    return mix(mix(hash(id), hash(id + vec2(1., 0.)), f.x),
      mix(hash(id + vec2(0., 1.)), hash(id + 1.), f.x), f.y);
  }
`;

const vertexShader = `
  varying vec2 vUv;
  varying vec3 vWorld;
  #include <fog_pars_vertex>
  void main() {
    vUv = uv;
    vWorld = (modelMatrix * vec4(position, 1.)).xyz;
    vec4 mvPosition = modelViewMatrix * vec4(position, 1.);
    gl_Position = projectionMatrix * mvPosition;
    #include <fog_vertex>
  }
`;

const illumination = `
  uniform float nightLight;
  vec3 waterLight(vec3 color) {
    return color * mix(vec3(1.), vec3(.15, .22, .37), nightLight);
  }
`;
const sky = `
  ${illumination}
  uniform vec3 keyDirection;
  uniform vec3 keyColor;
  vec3 sky(vec3 ray) {
    float elevation = max(ray.y, .025);
    vec3 color = mix(vec3(.53, .64, .67), vec3(.15, .28, .41), smoothstep(.02, .89, elevation));
    vec2 skyPlane = ray.xz / (.22 + elevation);
    float veil = noise(skyPlane * vec2(2.8, 3.6) + vec2(7.1, -3.2));
    return mix(mix(color, vec3(.69, .76, .76), smoothstep(.48, .85, veil) * .3),
      mix(vec3(.037, .056, .108), vec3(.012, .023, .058), elevation), nightLight);
  }
  float waterFresnel(vec3 normal, vec3 eye) {
    return .0204 + .9796 * pow(1. - clamp(dot(normal, eye), 0., 1.), 5.);
  }
  vec3 keyGlint(vec3 normal, vec3 eye, float roughness) {
    float nDotH = max(dot(normal, normalize(keyDirection + eye)), 0.);
    float alpha = roughness * roughness;
    float d = 1. + (alpha - 1.) * nDotH * nDotH;
    return keyColor * min(.018 * alpha * max(dot(normal, keyDirection), 0.) / (d * d), 1.);
  }
`;

export function createReflectiveWater(
  T: typeof Three, scene: Three.Scene, renderer: Three.WebGLRenderer, level: number,
) {
  const time = { value: 0 };
  const keyDirection = { value: new T.Vector3(...sampleCityTime(defaultCityTime).direction).normalize() };
  const keyColor = { value: new T.Color(cityLighting.color) };
  const nightLight = { value: 0 };
  const resolution = { value: new T.Vector2() };
  const reflectionMatrix = { value: new T.Matrix4() };
  const depthRange = { value: 0 };
  const water = new T.Group();
  scene.add(water);
  const type = renderer.extensions.has('EXT_color_buffer_float') ? T.HalfFloatType : T.UnsignedByteType;
  const opaque = new T.WebGLRenderTarget(1, 1, {
    type,
    depthTexture: new T.DepthTexture(1, 1, T.UnsignedIntType),
  });
  const reflection = new T.WebGLRenderTarget(1, 1, { type });
  const mirrorCamera = new T.OrthographicCamera();
  const reflectionClip = [new T.Plane(new T.Vector3(0, 1, 0), -level + 0.025)];
  const previousViewProjection = new T.Matrix4();
  const viewProjection = new T.Matrix4();
  const drawSize = new T.Vector2();
  const sight = new T.Vector3();
  const up = new T.Vector3();
  let dirty = true;
  let capturedLimit = 0;

  function capture(camera: Three.OrthographicCamera, maxCaptureSize = 1024) {
    camera.updateMatrixWorld();
    viewProjection.multiplyMatrices(camera.projectionMatrix, camera.matrixWorldInverse);
    renderer.getDrawingBufferSize(drawSize);
    if (!dirty && maxCaptureSize === capturedLimit && previousViewProjection.equals(viewProjection) &&
        resolution.value.equals(drawSize)) return;
    previousViewProjection.copy(viewProjection);
    resolution.value.copy(drawSize);
    const { x: width, y: height } = drawSize;
    const scale = Math.min(1, maxCaptureSize / Math.max(width, height));
    for (const target of [opaque, reflection]) {
      target.setSize(Math.max(1, Math.round(width * scale)), Math.max(1, Math.round(height * scale)));
    }
    depthRange.value = camera.far - camera.near;
    mirrorCamera.copy(camera);
    mirrorCamera.position.y = 2 * level - camera.position.y;
    camera.getWorldDirection(sight);
    sight.y = -sight.y;
    up.set(0, 1, 0).applyQuaternion(camera.quaternion);
    up.y = -up.y;
    mirrorCamera.up.copy(up);
    mirrorCamera.lookAt(sight.add(mirrorCamera.position));
    mirrorCamera.updateMatrixWorld();
    reflectionMatrix.value.multiplyMatrices(mirrorCamera.projectionMatrix, mirrorCamera.matrixWorldInverse);

    const previousTarget = renderer.getRenderTarget();
    const previousClipping = renderer.clippingPlanes;
    water.visible = false;
    try {
      renderer.setRenderTarget(opaque);
      renderer.render(scene, camera);
      renderer.clippingPlanes = reflectionClip;
      renderer.setRenderTarget(reflection);
      renderer.render(scene, mirrorCamera);
    } finally {
      renderer.setRenderTarget(previousTarget);
      renderer.clippingPlanes = previousClipping;
      water.visible = true;
    }
    capturedLimit = maxCaptureSize;
    dirty = false;
  }

  function surface(
    geometry: Three.BufferGeometry, fragmentShader: string,
    uniforms: Record<string, Three.IUniform<unknown>> = {}, transparent = false,
  ) {
    const material = new T.ShaderMaterial({
      uniforms: { ...T.UniformsUtils.clone(T.UniformsLib.fog), time, keyDirection, keyColor, nightLight, ...uniforms },
      vertexShader,
      fragmentShader: `#include <fog_pars_fragment>\n${fragmentShader}`,
      side: T.DoubleSide,
      fog: true,
      transparent,
      depthWrite: !transparent,
    });
    const mesh = new T.Mesh(geometry, material);
    water.add(mesh);
    return mesh;
  }

  function channel(geometry: Three.BufferGeometry, impact?: number) {
    const wake = impact === undefined ? '' : `
      float radius = length(vec2(xz.x * .36, xz.y - (${impact})));
      height += sin(radius * 7.3 - time * 7.1 + drift * 2.) * .053 * exp(-radius * .5);`;
    const aeration = impact === undefined ? '0.' : `
      exp(-pow((p.y - (${impact} + .72)) * .52, 2.)) * (1. - smoothstep(7.6, 10.1, abs(p.x)))`;
    return surface(geometry, `
    uniform float time;
    uniform sampler2D opaqueColor;
    uniform sampler2D opaqueDepth;
    uniform sampler2D mirroredColor;
    uniform mat4 reflectionMatrix;
    uniform vec2 resolution;
    uniform float depthRange;
    varying vec2 vUv;
    varying vec3 vWorld;
    ${noise}
    ${sky}

    float heightAt(vec2 xz, float flow) {
      vec2 travel = xz - vec2(.045, flow) * time;
      float drift = noise(travel * vec2(.46, .31)) - .5;
      float chop = noise(travel * vec2(2.6, 2.2)) - .5;
      float swell = sin(dot(xz, vec2(2.4, 3.7)) - time * 1.25 + drift * 4.) * .025;
      float ripples = sin(dot(travel, vec2(-6.3, 6.1)) + drift * 8. + chop * 1.35) * .008;
      float capillary = sin(dot(travel, vec2(14.1, -9.7)) + chop * 8.3 + time * 1.7) * .0045;
      float height = swell + ripples + capillary + chop * .007;
      ${wake}
      return height;
    }

    void main() {
      vec2 p = vWorld.xz;
      float flow = ${impact === undefined ? '.44' : 'mix(.32, 1.08, clamp(min(vUv.x, 1. - vUv.x) * 6., 0., 1.))'};
      float h = heightAt(p, flow);
      vec3 normal = normalize(vec3(h - heightAt(p + vec2(.052, 0.), flow), .052,
        h - heightAt(p + vec2(0., .052), flow)));
      vec3 eye = vec3(0., 0., 1.) * mat3(viewMatrix);
      vec3 ray = reflect(-eye, normal);
      vec2 perturbation = (mat3(viewMatrix) * vec3(normal.x, 0., normal.z)).xy;
      vec2 uv = gl_FragCoord.xy / resolution;
      float thickness = max((texture2D(opaqueDepth, uv).r - gl_FragCoord.z) * depthRange, 0.);
      vec2 refractedUv = clamp(uv + perturbation * min(.011, thickness * .016), .001, .999);
      float refractedDepth = (texture2D(opaqueDepth, refractedUv).r - gl_FragCoord.z) * depthRange;
      if (refractedDepth < .018) refractedUv = uv;
      else thickness = mix(thickness, refractedDepth, .52);

      vec4 mirrorPosition = reflectionMatrix * vec4(vWorld, 1.);
      vec2 reflectedUv = mirrorPosition.xy / mirrorPosition.w * .5 + .5;
      reflectedUv += perturbation * .018;
      vec2 edge = min(reflectedUv, 1. - reflectedUv);
      float coverage = smoothstep(0., .06, min(edge.x, edge.y)) * smoothstep(-.02, .17, ray.y);
      vec3 reflected = mix(sky(ray), texture2D(mirroredColor, reflectedUv).rgb, coverage * .94);
      vec3 body = waterLight(vec3(.018, .103, .108));
      vec3 absorption = exp(-vec3(.88, .36, .27) * thickness);
      vec3 bed = texture2D(opaqueColor, refractedUv).rgb;
      vec2 refraction = p * 6. + normal.xz * 2.4;
      float caustic = abs(sin(refraction.x + sin(refraction.y * .81)) * sin(refraction.y * 1.43 - time * .88));
      bed *= .94 + pow(caustic, 5.) * exp(-thickness * .79) * .25;

      vec3 color = mix(mix(body, bed, absorption), reflected, waterFresnel(normal, eye));
      color += keyGlint(normal, eye, .28) * .72;
      vec2 drift = p - vec2(0., time * flow);
      float eddies = noise(drift * vec2(1.3, 1.95));
      float bubbles = noise(drift * vec2(5.8, 4.) + eddies * 1.7);
      float impact = ${aeration};
      float swash = 1. - smoothstep(.025, .37, thickness);
      float foam = impact * smoothstep(.35, .73, eddies * .56 + bubbles * .44);
      foam += swash * smoothstep(.51, .80, eddies * .63 + bubbles * .37) * .57;
      gl_FragColor = vec4(mix(color, waterLight(vec3(.73, .85, .81)), clamp(foam, 0., 1.)), 1.);
      #include <tonemapping_fragment>
      #include <colorspace_fragment>
      #include <fog_fragment>
    }
    `, {
      opaqueColor: { value: opaque.texture },
      opaqueDepth: { value: opaque.depthTexture },
      mirroredColor: { value: reflection.texture },
      reflectionMatrix,
      resolution,
      depthRange,
    });
  }

  function fallingSheet(geometry: Three.BufferGeometry, fadeOut = false) {
    return surface(geometry, `
    uniform float time;
    varying vec2 vUv;
    varying vec3 vWorld;
    ${noise}
    ${sky}
    void main() {
      float travel = 1. - vUv.y;
      float current = travel * 10.3 - time * 6.;
      float folds = noise(vec2(vUv.x * 27., current * .33));
      float spread = smoothstep(.06, .94, travel);
      float x = vUv.x + (folds - .5) * mix(.010, .036, spread);
      float pockets = noise(vec2(x * 17. + folds, current * .56));
      float ribbons = noise(vec2(x * 72., current * 1.13 + pockets * 2.));
      float bubbles = noise(vec2(x * 161., current * 2.7 + ribbons * 1.63));
      float filaments = noise(vec2(x * 266., current * .92));
      float mixture = pockets * .30 + ribbons * .40 + bubbles * .20 + filaments * .10;
      float crestFoam = smoothstep(.35, .70, mixture) * smoothstep(.04, .28, travel);
      float plungeFoam = smoothstep(.37, .69, mixture + spread * .26);
      float entrained = mix(crestFoam, plungeFoam, smoothstep(.12, .69, travel));
      vec3 normal = normalize(cross(dFdx(vWorld), dFdy(vWorld)));
      vec3 eye = vec3(0., 0., 1.) * mat3(viewMatrix);
      normal = normalize(normal + vec3((ribbons - .5) * .38, (pockets - .5) * .22, 0.));
      float fresnel = waterFresnel(normal, eye);
      vec3 sheetColor = waterLight(mix(vec3(.057, .22, .20), vec3(.19, .38, .34), ribbons));
      sheetColor = mix(sheetColor, sky(reflect(-eye, normal)), .10 + fresnel * .37);
      vec3 foam = waterLight(mix(vec3(.59, .73, .69), vec3(.92, .97, .94), bubbles * .48 + ribbons * .52));
      vec3 color = mix(sheetColor, foam, entrained);
      color += keyGlint(normal, eye, .31) * .56;
      float sides = min(vUv.x, 1. - vUv.x);
      float fades = smoothstep(.0, .011 + spread * .014 + folds * .006, sides);
      float opacity = mix(.65 + ribbons * .14, .99, entrained) * fades;
      ${fadeOut ? 'opacity *= smoothstep(0., .28, vUv.y);' : ''}
      gl_FragColor = vec4(color, opacity);
      #include <tonemapping_fragment>
      #include <colorspace_fragment>
      #include <fog_fragment>
    }
    `, {}, true);
  }

  function sprayAt(origin: readonly [number, number, number], width: number) {
    const scale = width / 16;
    const seeds = new Float32Array(Math.round(820 * scale) * 3);
    for (let i = 0; i < seeds.length; i++) seeds[i] = (Math.sin((i + 1) * 73.19) * 21452.31) % 1 + 1;
    const spray = new T.BufferGeometry();
    spray.setAttribute('position', new T.BufferAttribute(seeds, 3));
    const mist = new T.Points(spray, new T.ShaderMaterial({
      uniforms: {
        ...T.UniformsUtils.clone(T.UniformsLib.fog), time, resolution, nightLight,
        origin: { value: new T.Vector3(...origin) }, scale: { value: scale },
      },
      fog: true,
      transparent: true,
      depthWrite: false,
      vertexShader: `
        uniform float time;
        uniform vec2 resolution;
        uniform vec3 origin;
        uniform float scale;
        varying float alpha;
        varying float detail;
        #include <fog_pars_vertex>
        void main() {
          vec3 seed = fract(position);
          float life = fract(seed.z + time * (.25 + .24 * seed.x));
          float arc = 4. * life * (1. - life);
          detail = seed.y;
          vec3 point = origin + scale * vec3((seed.x - .5) * (16. + life * 4.),
             (.3 + seed.y * 3.5) * arc,
             life * (2.2 + 3.6 * seed.y));
          vec4 mvPosition = modelViewMatrix * vec4(point, 1.);
          gl_Position = projectionMatrix * mvPosition;
          #include <fog_vertex>
          float size = mix(.028, 1.55 * life + .25, smoothstep(.37, .9, seed.y));
          gl_PointSize = max(1., scale * size * projectionMatrix[1][1] * resolution.y);
          alpha = smoothstep(0., .13, life) * pow(1. - life, 1.4) * mix(.65, .20, seed.y);
        }
      `,
      fragmentShader: `
        varying float alpha;
        varying float detail;
        #include <fog_pars_fragment>
        ${noise}
        ${illumination}
        void main() {
          float radial = length(gl_PointCoord - .5);
          float cloud = 1. - smoothstep(.0, .49, radial);
          float density = mix(1., .45 + noise(gl_PointCoord * 6. + detail * 100.) * .55, detail);
          gl_FragColor = vec4(waterLight(vec3(.91, .98, .95)), cloud * density * alpha);
          #include <tonemapping_fragment>
          #include <colorspace_fragment>
          #include <fog_fragment>
        }
      `,
    }));
    mist.frustumCulled = false;
    mist.renderOrder = 2;
    water.add(mist);
  }

  return {
    time, surface, channel, fallingSheet, sprayAt, opaque, resolution, depthRange,
    capture,
    setTime({ night, direction, intensity }: CityTime, color: Three.Color) {
      keyDirection.value.fromArray(direction).normalize();
      keyColor.value.copy(color).multiplyScalar(intensity / cityLighting.intensity);
      nightLight.value = night;
      dirty = true;
    },
    invalidate() {
      dirty = true;
    },
    dispose() {
      opaque.dispose();
      reflection.dispose();
    },
  };
}

export function buildCityWater(T: typeof Three, scene: Three.Scene, renderer: Three.WebGLRenderer) {
  const water = createReflectiveWater(T, scene, renderer, basinLevel);
  const { surface, opaque, resolution, depthRange } = water;
  const riverbed = new T.PlaneGeometry(1, 1, 1, 96);
  const bankVertices = riverbed.getAttribute('position');
  for (let i = 0; i < bankVertices.count; i++) {
    const z = impactZ + (0.5 - bankVertices.getY(i)) * (canalEnd - impactZ);
    bankVertices.setXYZ(i, bankVertices.getX(i) * 2 * canalHalfWidth(z), basinLevel, z);
  }
  water.channel(riverbed, impactZ);

  const lake = new T.PlaneGeometry(1, 1, 1, reservoirProfile.length - 1);
  const lakeVertices = lake.getAttribute('position');
  reservoirProfile.forEach(([z, halfWidth], row) => {
    lakeVertices.setXYZ(row * 2, -halfWidth, crestLevel + 0.02, z);
    lakeVertices.setXYZ(row * 2 + 1, halfWidth, crestLevel + 0.02, z);
  });
  const [mouthZ, mouthX] = feederProfile[0];
  surface(lake, `
    uniform float time;
    uniform sampler2D opaqueColor;
    uniform sampler2D opaqueDepth;
    uniform vec2 resolution;
    uniform float depthRange;
    varying vec2 vUv;
    varying vec3 vWorld;
    ${noise}
    ${sky}
    void main() {
      vec2 p = vWorld.xz;
      float patches = noise(p * .16 - vec2(.018, .032) * time);
      vec2 drift = p - vec2(.11, .19) * time;
      float crossWind = sin(dot(p, vec2(1.7, 2.4)) - time * .74 + patches * 5.);
      float windStreak = sin(dot(drift, vec2(-1.2, 3.1)) + patches * 3.);
      float ripples = noise(drift * vec2(1.7, 2.4)) - .5;
      float capillary = cos(dot(drift, vec2(5.9, -7.3)) + patches * 7.) * patches;
      vec3 normal = normalize(vec3(crossWind * .054 + ripples * .037 + capillary * .025,
        1., windStreak * .052 + crossWind * .029 + ripples * .042 - capillary * .03));
      vec3 eye = vec3(0., 0., 1.) * mat3(viewMatrix);
      float fresnel = waterFresnel(normal, eye);

      // The authored lake has a flat support mesh. Expose that stone at the retaining walls.
      vec2 uv = gl_FragCoord.xy / resolution;
      float thickness = max((texture2D(opaqueDepth, uv).r - gl_FragCoord.z) * depthRange, 0.);
      float bank = 1. - smoothstep(.025, .15, min(vUv.x, 1. - vUv.x));
      vec3 deep = waterLight(mix(vec3(.018, .093, .106), vec3(.026, .135, .133), patches));
      vec3 stone = texture2D(opaqueColor, uv).rgb;
      vec3 bed = mix(deep, stone, exp(-vec3(1.1, .72, .63) * thickness));
      vec3 color = mix(mix(deep, bed, bank * .72), sky(reflect(-eye, normal)), fresnel);
      float inflow = exp(-length((p - vec2(${mouthX.toFixed(2)}, ${mouthZ.toFixed(2)})) * vec2(.34, .19)));
      float foam = smoothstep(.56, .83, noise(drift * vec2(1.8, 2.7))) * inflow;
      gl_FragColor = vec4(mix(color, waterLight(vec3(.51, .69, .62)), foam * .54) + keyGlint(normal, eye, .31) * .72, 1.);
      #include <tonemapping_fragment>
      #include <colorspace_fragment>
      #include <fog_fragment>
    }
  `, {
    opaqueColor: { value: opaque.texture },
    opaqueDepth: { value: opaque.depthTexture },
    resolution,
    depthRange,
  });
  const feeder = new T.PlaneGeometry(1, 1, 1, feederProfile.length - 1);
  const feederVertices = feeder.getAttribute('position');
  feederProfile.forEach(([z, center, y, width], row) => {
    feederVertices.setXYZ(row * 2, center - width, y, z);
    feederVertices.setXYZ(row * 2 + 1, center + width, y, z);
  });
  surface(feeder, `
    uniform float time;
    varying vec3 vWorld;
    varying vec2 vUv;
    ${noise}
    ${illumination}
    void main() {
      vec2 flow = vec2(vUv.x * 3.7, vWorld.z - time * 2.7);
      float current = noise(flow * vec2(2.15, 1.6));
      float turbulence = noise(flow * vec2(5.3, 3.1) + current);
      float edge = smoothstep(.31, .50, abs(vUv.x - .5));
      float wash = smoothstep(.52, .85, current * .61 + turbulence * .39 + edge * .20);
      vec3 color = mix(vec3(.027, .15, .13), vec3(.34, .52, .45), wash);
      gl_FragColor = vec4(waterLight(color), 1.);
      #include <tonemapping_fragment>
      #include <colorspace_fragment>
      #include <fog_fragment>
    }
  `);

  const sheet = new T.PlaneGeometry(1, 1, 100, 84);
  const points = sheet.getAttribute('position');
  const uv = sheet.getAttribute('uv');
  for (let i = 0; i < points.count; i++) {
    const across = uv.getX(i) - 0.5;
    const travel = 1 - uv.getY(i);
    const x = across * (16.15 + 1.8 * travel);
    const y = crestLevel - (crestLevel - basinLevel) * travel ** 2;
    const z = crestZ + travel * (impactZ - crestZ) + Math.sin(across * Math.PI * 29) * 0.15 * travel;
    points.setXYZ(i, x, y, z);
  }
  water.fallingSheet(sheet);

  water.sprayAt([0, 0.17, impactZ], 16);

  return water;
}
