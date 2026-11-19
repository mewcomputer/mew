# Jianghu

Waterworks is the default view, with the diorama, animated water, and
its four sprites. Drag to orbit. Press `Q` or `E` to rotate, `+` or `-` to
zoom, and `R` to reset the view.

The previous prototypes have separate entry points. `/agents.html` retains
the agent map and its local approval loop. `/workbench.html` opens the voxel
editor, water controls, and dusk lighting.

[/city.html](http://127.0.0.1:4173/city.html) opens Baichuan, an isometric
market district and lower canal ward set inside a larger hillside city.
Stairs and four crossings connect the dam gallery, market, quays, and
Rainbell Square. A courier uses the new downstream bridge and a steward
lights the bronze bell to signal the square.
The upper city wraps around the reservoir, with the same building and
street details on both sides of the dam.
Baichuan's characters use the tactics sprite kit with city-specific
costumes, walking routes, and work and practice poses.
Market passersby can pause their routines for short conversations when
they pass near one another on the same level.
Drag or use the arrow keys to orbit, Shift-drag to pan, scroll or press
`+` and `-` to zoom, and press `R` to reset. The waterfall and spray are
shader effects. The basin and canal sample the rendered scene for reflections
and depth-based refraction.

Select **Test movement** in Baichuan to focus Rainbell Square. Click a tile
to place a test character and see the tiles within five movement points.
Hover a reachable tile to preview its route, then click to walk there.
The movement range refreshes on arrival. Use **WASD** to pan the camera,
**New start** to choose a different origin, or **Escape** to exit.
`src/rainbell-world.ts` is the first world-as-code example. Its named
terraces, bridge, stairs, and prop placements feed both the scene and the
movement grid.

Use Node.js 22.18 or later. Start esbuild's development server and open
[the local sandbox](http://127.0.0.1:4173).

```sh
cd jianghu
npm ci
npm start
```

Jianghu uses strict TypeScript and a locally bundled Three.js dependency.
Waterworks and the Workshop show a district illustration when WebGL 2 is
unavailable. Terrain edits and undo history live in the current session.

## Interface

The agent and Workshop shells use Shanshui Ink by default. Set `data-theme="light"` on
`#jianghu` to preview the Paper palette. Both palettes are defined in
`tokens.css`. Terrain materials and scene lighting belong to the engine.

## Workshop controls

- Drag the district to orbit, or use the rotate and zoom buttons. **Spillway**
  moves the camera to the waterfall.
- Choose **Place** and a material, then select a block face. **Remove** digs
  out the selected voxel. **Undo** reverses the most recent edit. **Edit at +**
  works with the center reticle and camera buttons.
- Close the sluice to raise the upstream level and reduce downstream
  discharge. The water controls pause the simulation or let it run. Terrain
  edits also change the bed the water flows over.

## First playable slice

The prototype audit kept plain Three.js, the camera framing, Waterworks
terrain, and the water renderer. The map advances the water during sluice
changes. Sprites are depth-tested objects in the scene and show eight
views from the unit direction and camera orbit. Idle, walk, and per-role
work poses use the [Internet Development sprite kit](https://github.com/internet-development/skill-create-tactics-threejs-sprite).
Jianghu recolors the kit's custom example sheet and draws role-specific
headgear and instruments. See the [kit notice](vendor/tactics/NOTICE.md).

Walking and work frames respect reduced motion. Live adapters and multiple
islands are future work. The list can route to the same detail panel when
WebGL is unavailable.

## Engine

See the [runtime map and comparison](RUNTIME.md) for the current boundaries,
the comparison with the 3d.internet.dev sprite kit, and the first extraction
to try with Rainbell Square.

- `src/world.ts` stores materials in a voxel grid, tracks affected chunks
  after edits, and builds exposed faces with per-corner ambient occlusion.
- `src/fluid.ts` advances surface flow at a fixed time step. Edge transfers
  conserve volume, with a separate inlet and spillway. The solver follows the
  top surface of each voxel column.
- `src/district.ts` defines the editable terrain, landmarks, waterway, and
  architectural footprints. The rendered geometry and agent routes share
  those footprints. Traffic builds its navigation grid from the district
  at startup.
- `src/terrain-scene.ts`, `src/district-scene.ts`, and `src/water-scene.ts`
  draw the model with Three.js. Spillway ribbons and spray follow the
  simulator's measured discharge. The water material samples opaque-scene
  color and depth for shallow refraction and screen-space reflections.
- `src/scene-view.ts` owns the renderer, orbit camera, frame scheduling,
  and resource cleanup for all four views. `src/district-view.ts` connects
  the Waterworks district and water solver to that shared host.
- `src/water-clock.ts` advances the solver at a fixed step and drops wall
  time spent paused. `src/sandbox.ts` supplies the Workshop controls and
  lighting to the shared view.

Use `npm run typecheck` for the source, tests, and build tools. Run the
headless tests with `npm test`. The production build runs type checking
and writes all four pages and their assets to `dist/` with `npm run build`.

Run `npm run preview -- /path/to/jianghu-preview.html` to bundle the
Waterworks preview. Pass `--map` for the agent example, `--workshop` for
the editor, or `--city` for Baichuan.
