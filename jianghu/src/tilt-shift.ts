import type * as Three from 'three';

export function tiltShiftAperture(zoom: number) {
  const closeup = Math.min(1, Math.max(0, (zoom - 0.75) / 3.45));
  return 0.18 + 0.22 * closeup;
}

export function createTiltShift(T: typeof Three, renderer: Three.WebGLRenderer) {
  const gl = renderer.getContext();
  if (!('getInternalformatParameter' in gl) || !renderer.extensions.has('EXT_color_buffer_float')) return null;
  const samples = gl.getInternalformatParameter(gl.RENDERBUFFER, gl.RGBA16F, gl.SAMPLES) as Int32Array;
  const target = new T.WebGLRenderTarget(1, 1, {
    type: T.HalfFloatType,
    depthTexture: new T.DepthTexture(1, 1, T.UnsignedIntType),
    samples: samples.includes(2) ? 2 : samples.includes(4) ? 4 : 0,
  });
  const size = new T.Vector2();
  const uniforms = {
    sceneColor: { value: target.texture },
    sceneDepth: { value: target.depthTexture },
    texel: { value: new T.Vector2() },
    viewHeight: { value: 0 },
    depthRange: { value: 0 },
    orbitDepth: { value: 0 },
    aperture: { value: 0 },
    maxRadius: { value: 0 },
  };

  const material = new T.ShaderMaterial({
    uniforms,
    depthTest: false,
    depthWrite: false,
    vertexShader: `
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = vec4(position.xy, 0., 1.);
      }
    `,
    fragmentShader: `
      varying vec2 vUv;
      uniform sampler2D sceneColor;
      uniform sampler2D sceneDepth;
      uniform vec2 texel;
      uniform float viewHeight, depthRange, orbitDepth, aperture, maxRadius;

      void main() {
        vec4 color = texture2D(sceneColor, vUv);
        float depth = texture2D(sceneDepth, vUv).r;
        float centerDepth = texture2D(sceneDepth, vec2(.5)).r;
        float focus = centerDepth < 1. ? centerDepth : orbitDepth;
        // Orthographic depth is linear. The tilted plane recedes with image height.
        float offset = (depth - focus) * depthRange - (vUv.y - .5) * viewHeight * .68;
        float radius = maxRadius * smoothstep(1., 6., abs(offset) * aperture);
        if (depth < 1. && radius > .35) {
          float weight = 1.;
          for (int tap = 0; tap < 16; tap++) {
            float angle = float(tap) * 2.39996323;
            float ring = sqrt((float(tap) + .5) / 16.);
            vec2 uv = vUv + vec2(cos(angle), sin(angle)) * ring * radius * texel;
            float separation = abs(texture2D(sceneDepth, uv).r - depth) * depthRange;
            float coverage = mix(1., .18, smoothstep(.75, 6., separation));
            color += texture2D(sceneColor, uv) * coverage;
            weight += coverage;
          }
          color /= weight;
        }
        gl_FragColor = color;
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }
    `,
  });
  const geometry = new T.PlaneGeometry(2, 2);
  const screen = new T.Scene();
  const screenCamera = new T.Camera();
  screen.add(new T.Mesh(geometry, material));

  return {
    render(scene: Three.Scene, camera: Three.OrthographicCamera, zoom: number, orbitRadius: number) {
      renderer.getDrawingBufferSize(size);
      target.setSize(size.x, size.y);
      uniforms.texel.value.set(1 / size.x, 1 / size.y);
      uniforms.viewHeight.value = camera.top - camera.bottom;
      uniforms.depthRange.value = camera.far - camera.near;
      uniforms.orbitDepth.value = (orbitRadius - camera.near) / uniforms.depthRange.value;
      const aperture = tiltShiftAperture(zoom);
      uniforms.aperture.value = aperture;
      uniforms.maxRadius.value = 9 * aperture * renderer.getPixelRatio();

      renderer.setRenderTarget(target);
      try {
        renderer.render(scene, camera);
      } finally {
        renderer.setRenderTarget(null);
      }
      renderer.render(screen, screenCamera);
    },
    dispose() {
      target.dispose();
      geometry.dispose();
      material.dispose();
    },
  };
}
