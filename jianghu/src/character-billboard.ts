import type * as Three from 'three';
import { citySpritePixelSize, spriteCanvasSize, spriteGround } from './agent-sprites.ts';

export function createCharacterBillboards(T: typeof Three) {
  const size = spriteCanvasSize * citySpritePixelSize;
  const geometry = new T.PlaneGeometry(size, size).translate(0, (spriteGround - 0.5) * size, 0);
  // An upward lighting normal keeps the pixel art from relighting as the camera orbits.
  const lightLoop = T.ShaderChunk.lights_fragment_begin.replace(
    'vec3 geometryNormal = normal;',
    'vec3 geometryNormal = normalize(mat3(viewMatrix) * vec3(0.0, 1.0, 0.0));',
  );
  const diffuseLight = T.ShaderChunk.lights_lambert_pars_fragment.replace(
    'float dotNL = saturate( dot( geometryNormal, directLight.direction ) );',
    'float dotNL = 0.35 + 0.65 * saturate(dot(geometryNormal, directLight.direction));',
  );
  return (map: Three.Texture) => {
    const material = new T.MeshLambertMaterial({
      map, alphaTest: 0.48, side: T.DoubleSide,
    });
    material.onBeforeCompile = (shader) => {
      shader.fragmentShader = shader.fragmentShader
        .replace('#include <lights_lambert_pars_fragment>', diffuseLight)
        .replace('#include <lights_fragment_begin>', lightLoop)
        .replace('vec3 outgoingLight = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse + totalEmissiveRadiance;',
          'vec3 outgoingLight = reflectedLight.directDiffuse + reflectedLight.indirectDiffuse + diffuseColor.rgb * 0.22;');
    };
    const sprite = new T.Mesh(geometry, material);
    sprite.receiveShadow = true;
    return sprite;
  };
}
