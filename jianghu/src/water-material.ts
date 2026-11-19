import type * as Three from 'three';

export const waterLayer = 1;

export function createWaterMaterial(T: typeof Three, renderer: Three.WebGLRenderer) {
  const opaque = new T.WebGLRenderTarget(1, 1, {
    type: renderer.extensions.has('EXT_color_buffer_float') ? T.HalfFloatType : T.UnsignedByteType,
    depthTexture: new T.DepthTexture(1, 1, T.UnsignedIntType),
  });
  const uniforms = {
    sceneColor: { value: opaque.texture },
    sceneDepth: { value: opaque.depthTexture },
    resolution: { value: new T.Vector2() },
    projectionScale: { value: new T.Vector2() },
    cameraRange: { value: new T.Vector2() },
    sunDirection: { value: new T.Vector3() },
    sunColor: { value: new T.Color() },
    sunStrength: { value: 0 },
    dusk: { value: 0 },
    time: { value: 0 },
  };
  let dirty = true;

  function capture(scene: Three.Scene, camera: Three.OrthographicCamera) {
    if (!dirty) return;
    const renderSize = renderer.getDrawingBufferSize(uniforms.resolution.value);
    const scale = Math.min(1, 1280 / Math.max(renderSize.x, renderSize.y));
    opaque.setSize(
      Math.max(1, Math.round(renderSize.x * scale)),
      Math.max(1, Math.round(renderSize.y * scale)),
    );
    uniforms.projectionScale.value.set(
      camera.projectionMatrix.elements[0],
      camera.projectionMatrix.elements[5],
    );
    uniforms.cameraRange.value.set(camera.near, camera.far);

    // Capture the opaque world once per camera or scene change. Waves sample this image.
    const previousTarget = renderer.getRenderTarget();
    const previousLayers = camera.layers.mask;
    try {
      camera.layers.disable(waterLayer);
      renderer.setRenderTarget(opaque);
      renderer.render(scene, camera);
    } finally {
      renderer.setRenderTarget(previousTarget);
      camera.layers.mask = previousLayers;
    }
    dirty = false;
  }

  function material(spillway: boolean) {
    return new T.ShaderMaterial({
      uniforms,
      defines: spillway ? { SPILLWAY: 1 } : {},
      side: T.DoubleSide,
      transparent: spillway,
      depthWrite: !spillway,
      vertexShader: `
        attribute vec2 hydraulics;
        varying vec2 vUv;
        varying vec2 vHydraulics;
        varying vec3 vView;
        varying vec3 vWorld;
        void main() {
          vUv = uv;
          vHydraulics = hydraulics;
          vWorld = (modelMatrix * vec4(position, 1.)).xyz;
          vView = (modelViewMatrix * vec4(position, 1.)).xyz;
          gl_Position = projectionMatrix * vec4(vView, 1.);
        }`,
      fragmentShader: `
        uniform sampler2D sceneColor;
        uniform sampler2D sceneDepth;
        uniform vec2 resolution;
        uniform vec2 projectionScale;
        uniform vec2 cameraRange;
        uniform vec3 sunDirection;
        uniform vec3 sunColor;
        uniform float sunStrength;
        uniform float dusk;
        uniform float time;
        varying vec2 vUv;
        varying vec2 vHydraulics;
        varying vec3 vView;
        varying vec3 vWorld;

        const vec3 toEye = vec3(0., 0., 1.);

        float hash(vec2 p) {
          return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
        }
        float noise(vec2 p) {
          vec2 i = floor(p);
          vec2 f = fract(p);
          f = f * f * (3. - 2. * f);
          return mix(mix(hash(i), hash(i + vec2(1., 0.)), f.x),
            mix(hash(i + vec2(0., 1.)), hash(i + 1.), f.x), f.y);
        }
        float cameraDepth(vec2 uv) {
          return mix(cameraRange.x, cameraRange.y, texture2D(sceneDepth, uv).r);
        }
        float edgeFade(vec2 uv) {
          vec2 edge = min(uv, 1. - uv);
          return smoothstep(.0, .045, min(edge.x, edge.y));
        }
        vec3 sky(vec3 direction) {
          float height = clamp(direction.y * .7 + .25, 0., 1.);
          vec3 horizon = mix(vec3(.45, .55, .57), vec3(.12, .18, .28), dusk);
          vec3 zenith = mix(vec3(.20, .33, .44), vec3(.025, .048, .10), dusk);
          return mix(horizon, zenith, height);
        }
        float fresnel(vec3 normal) {
          return .0204 + .9796 * pow(1. - clamp(dot(normal, toEye), 0., 1.), 5.);
        }
        vec3 sunGlint(vec3 normal, float roughness) {
          vec3 light = normalize(mat3(viewMatrix) * sunDirection);
          vec3 halfVector = normalize(light + toEye);
          float nDotH = max(dot(normal, halfVector), 0.);
          float nDotL = max(dot(normal, light), 0.);
          float alpha = roughness * roughness;
          float denominator = (alpha - 1.) * nDotH * nDotH + 1.;
          float lobe = alpha / (3.14159 * denominator * denominator);
          return sunColor * sunStrength * min(lobe * nDotL * .038, 1.4);
        }
        vec4 traceReflection(vec3 normal) {
          vec3 direction = reflect(-toEye, normal);
          float previous = .12;
          for (int i = 0; i < 11; i++) {
            float stepIndex = float(i);
            vec3 probe = vView + normal * .045 + direction * (.13 + .038 * stepIndex * (stepIndex + 1.));
            vec2 uv = probe.xy * projectionScale * .5 + .5;
            float fade = edgeFade(uv);
            if (fade <= 0.) break;
            float gap = cameraDepth(uv) + probe.z;
            if (gap <= 0. && gap > -.29 && previous > 0.) {
              vec4 reflected = texture2D(sceneColor, uv);
              return vec4(reflected.rgb, reflected.a * fade * (1. - stepIndex / 16.));
            }
            previous = gap;
          }
          return vec4(0.);
        }
        vec3 surfaceNormal(vec3 geometryNormal, vec2 world, float current) {
          vec2 a = normalize(vec2(-.42, .91));
          vec2 b = normalize(vec2(.84, .54));
          vec2 c = normalize(vec2(-.94, .35));
          float chop = mix(.56, 1.18, current);
          vec2 ripple = a * cos(dot(world, a) * 18. - time * 8.) * .12;
          ripple += b * cos(dot(world, b) * 31. - time * 12.) * .047;
          ripple += c * cos(dot(world, c) * 52. - time * 15.) * .026;
          vec3 detail = mat3(viewMatrix) * vec3(ripple.x, 0., ripple.y) * chop;
          return normalize(geometryNormal + detail);
        }

        void main() {
          vec3 geometricNormal = normalize(cross(dFdx(vView), dFdy(vView)));
          #ifdef SPILLWAY
            float travel = sqrt(max(vUv.y, .001));
            vec2 sheet = vec2(vUv.x * 70., travel * 14. - time * 8.);
            float turbulence = noise(sheet) * .62 + noise(sheet * 2.7) * .38;
            float threads = noise(vec2(vUv.x * 180., travel * 6. - time * 4.));
            vec3 normal = normalize(geometricNormal + vec3((threads - .5) * .32, (turbulence - .5) * .35, 0.));
            float aeration = smoothstep(.05, .91, vUv.y);
            float entrainedAir = smoothstep(.49, .81, turbulence + .22 * aeration + .085 * threads);
            vec3 reflectedSky = sky(reflect(-toEye, normal) * mat3(viewMatrix));
            vec3 glass = mix(vec3(.055, .27, .24), reflectedSky, fresnel(normal) * .6 + .24);
            vec3 foam = mix(vec3(.82, .91, .88), vec3(.25, .35, .42), dusk);
            vec3 color = mix(glass * mix(1., .55, dusk), foam, entrainedAir);
            color += sunGlint(normal, .29) * .38;
            float separation = smoothstep(.76, 1., vUv.y) * (1. - smoothstep(.29, .68, turbulence));
            float opacity = .29 + entrainedAir * .59 + fresnel(normal) * .43;
            gl_FragColor = vec4(color, clamp(opacity * (1. - separation * .7), 0., .96));
          #else
            float current = clamp(vHydraulics.x / 1.7, 0., 1.);
            vec3 normal = surfaceNormal(geometricNormal, vWorld.xz, current);
            vec2 uv = gl_FragCoord.xy / resolution;
            float thickness = max(cameraDepth(uv) + vView.z, 0.);
            vec2 offset = (normal.xy - geometricNormal.xy) * clamp(thickness, .06, .9) * .052;
            vec2 bent = clamp(uv + offset, vec2(.001), vec2(.999));
            // A foreground bank should never be pulled through the water by refraction.
            if (cameraDepth(bent) + vView.z <= .006) bent = uv;
            vec4 bed = texture2D(sceneColor, bent);
            float opticalDepth = min(thickness, vHydraulics.y * 2.4 + .11);
            vec3 absorption = exp(-vec3(1.75, .78, .51) * opticalDepth);
            vec3 body = mix(vec3(.034, .16, .14), vec3(.013, .046, .071), dusk);
            vec3 transmitted = mix(body, bed.rgb, bed.a * absorption);
            vec3 reflectedSky = sky(reflect(-toEye, normal) * mat3(viewMatrix));
            vec4 reflection = traceReflection(normal);
            vec3 reflected = mix(reflectedSky, reflection.rgb, reflection.a);
            vec3 color = mix(transmitted, reflected, fresnel(normal));
            color += sunGlint(normal, mix(.18, .29, current));
            vec2 foamPattern = vec2(vWorld.x * 25., (vWorld.z - time * .52) * 19.);
            float foamNoise = noise(foamPattern) * .63 + noise(foamPattern * 2.3) * .37;
            float bank = 1. - smoothstep(.028, .17, thickness);
            float shoreFoam = bank * smoothstep(.48, .77, foamNoise) * current;
            float crestFoam = smoothstep(.86, .98, foamNoise) * current * .66;
            vec3 foam = mix(vec3(.69, .81, .75), vec3(.19, .29, .35), dusk);
            gl_FragColor = vec4(mix(color, foam, max(shoreFoam, crestFoam)), 1.);
          #endif
          #include <tonemapping_fragment>
          #include <colorspace_fragment>
        }
        `,
    });
  }

  return {
    capture,
    dispose: () => opaque.dispose(),
    material,
    invalidate() {
      dirty = true;
    },
    setTime(time: number) {
      uniforms.time.value = time;
    },
    setLighting(dusk: boolean, sun: Three.DirectionalLight) {
      uniforms.dusk.value = Number(dusk);
      uniforms.sunDirection.value.copy(sun.position).sub(sun.target.position).normalize();
      uniforms.sunColor.value.copy(sun.color);
      uniforms.sunStrength.value = sun.intensity;
    },
  };
}
