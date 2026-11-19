import type * as Three from 'three';

export function createGardenWaterfall(T: typeof Three, width: number, height: number) {
  const fall = new T.Mesh(new T.PlaneGeometry(width, height, 1, 16), new T.ShaderMaterial({
    transparent: true, depthWrite: false, side: T.DoubleSide,
    uniforms: { time: { value: 0 }, tint: { value: new T.Color() } },
    vertexShader: `
      varying vec2 vUv;
      void main() {
        vUv = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.);
      }
    `,
    fragmentShader: `
      varying vec2 vUv;
      uniform float time;
      uniform vec3 tint;
      void main() {
        float edge = smoothstep(0., .15, min(vUv.x, 1. - vUv.x));
        float thread = .62 + .19 * sin(vUv.x * 51. + 3. * sin(vUv.y * 8. + time * 2.));
        float streak = .88 + .12 * sin(vUv.x * 100. + vUv.y * 21. + time * 8.);
        float fade = smoothstep(0., .22, vUv.y);
        gl_FragColor = vec4(tint * thread * streak, edge * fade * .83);
        #include <tonemapping_fragment>
        #include <colorspace_fragment>
      }
    `,
  }));
  return fall;
}

export function gardenWaterGeometry(T: typeof Three, source: Three.Mesh) {
  const pool = source.geometry.clone().applyMatrix4(source.matrixWorld);
  pool.computeBoundingBox();
  const lip = pool.boundingBox!.max;
  const columns = 32;
  const spillway = new T.PlaneGeometry(1, 1, columns, 48);
  const points = spillway.getAttribute('position');
  const uv = spillway.getAttribute('uv');
  for (let i = 0; i < points.count; i++) {
    const across = uv.getX(i) - 0.5;
    const travel = 1 - uv.getY(i);
    points.setXYZ(i,
      -10 + across * (3.98 + 0.24 * travel),
      T.MathUtils.lerp(lip.y, -4.9, travel ** 2),
      lip.z - 0.065 + travel * (0.62 + Math.sin(91 * across) * 0.065),
    );
  }
  const end = points.count - 1 - columns / 2;
  const runoff = [points.getX(end), points.getY(end), points.getZ(end)] as const;
  source.visible = false;
  return { pool, spillway, level: lip.y, runoff };
}
