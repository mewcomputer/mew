# Jianghu runtime

[Rainbell Square](src/rainbell-world.ts) is the first world-as-code example.
Its terrain, bridge, stairs, and prop footprints feed both the Baichuan scene
and the tactics map. The playtest puts a character on that map, shows the
movement budget, and walks the route selected by the player.

## Where the code lives today

| Area | Owns | Start here |
|---|---|---|
| Browser view | Renderer, camera, input routing, frame scheduling, suspension, WebGL context recovery, scene cleanup | [scene-view.ts](src/scene-view.ts) |
| Tactics movement | Surface tiles, blockers, stair and bridge connections, budgeted reachability, route geometry, and walk interpolation | [tactics-movement.ts](src/tactics-movement.ts), [tactics-walk.ts](src/tactics-walk.ts), [surface-grid.ts](src/surface-grid.ts) |
| Character drawing | Vendored atlas and pose evaluation, Jianghu costumes, Canvas sprite assembly, and layered staff rendering in the movement playtest | [vendor/tactics](vendor/tactics), [agent-sprites.ts](src/agent-sprites.ts), [staff-character.ts](src/staff-character.ts) |
| Baichuan | Authored architecture and cast, street-life encounters, bell audio, city water passes, and the movement playtest UI | [city-main.ts](src/city-main.ts), [city-scene.ts](src/city-scene.ts), [city-characters.ts](src/city-characters.ts), [rainbell-playtest.ts](src/rainbell-playtest.ts) |
| Waterworks | Editable voxel terrain and fluid simulation, connected to the shared browser view by the district adapter | [world.ts](src/world.ts), [fluid.ts](src/fluid.ts), [water-clock.ts](src/water-clock.ts), [district-view.ts](src/district-view.ts) |
| Examples and delivery | Waterworks player, Workshop, agent map, Baichuan, and standalone preview bundling | [tools/pages.ts](tools/pages.ts), [tools/build-preview.ts](tools/build-preview.ts) |

The tactics graph and walk interpolation run under Node without Three.js.
The walk module still takes its direction tuple from the city character
types. Baichuan and the agent map each construct their own Three.js sprite
objects around the shared painter. The agent map also has its own navigation
for the Waterworks district. [City characters](src/city-characters.ts) ·
[Agent map](src/map-scene.ts) · [Agent traffic](src/agent-traffic.ts)

## Comparison with Jimmy Lee’s runtime

The public kit has a renderer-independent sprite runtime and a separate
Three.js adapter. Jianghu’s vendored `tactics-sprite.mjs` and sprite JSON
matched the published files byte for byte when checked for this comparison.
Jianghu uses core functions such as `poseAt` and `drawPose`; its callers
choose the mode and advance the pose themselves. The public core also offers
`createPlayer` for timed playback, transitions, and animation events.
[Runtime index](https://3d.internet.dev/llms.txt) ·
[Sprite core](https://3d.internet.dev/assets/tactics-sprite.mjs) ·
[Published data](https://3d.internet.dev/api/v1/data)

| Question | Jimmy’s published runtime | Jianghu today |
|---|---|---|
| How does a unit draw? | Several camera-facing atlas planes. Units can share one prepared texture and its materials. The body writes depth and the overlay planes test depth. | Residents and agents use a nearest-filtered Canvas texture per billboard. The Rainbell test character uses the layered adapter and its tinted costume atlas. |
| Who owns the clock and motion? | The host passes seconds and a camera to `update`. The player handles animation playback and reports events. The host positions the unit and decides where it walks. | The city and map decide animation mode and ticks. Rainbell separately computes a walkable route and samples its world position, including stair height. |
| How do held items work? | The adapter exposes world-space hand points and supports mesh attachment, release, and ordering meshes among sprite layers. | The painter draws pixel props onto the canvas. The Rainbell staff uses a world-space grip point and a depth layer between the torso and the near arm. |
| How do you embed it? | The documented entry point creates one unit in the caller’s scene, with a per-unit disposer and caller-owned sheet lifetime. The guide includes a minimal example and integration tests across camera and rendering setups. | A shared view owns a canvas and scene lifetime for several entry points. Character construction and update are reachable through scene-specific code. |

[Three.js adapter source](https://3d.internet.dev/assets/tactics-sprite-three.mjs) ·
[Three.js integration guide](https://3d.internet.dev/references/threejs.md) ·
[Jianghu painter](src/agent-sprites.ts) ·
[Jianghu view](src/scene-view.ts)

The site’s demo engine has systems outside the public sprite adapter. Its
physics notes describe deriving a camera-independent body from each sprite
frame for water interactions and a body debug view. Rainbell navigation
instead uses authored floor surfaces and footprints, with a route providing
the character’s ground position. Those solve different spatial problems.
[Demo physics notes](https://3d.internet.dev/three-isometric-engine/physics/AGENTS.md) ·
[Rainbell world](src/rainbell-world.ts)

## Where to draw the runtime boundary

The useful pattern to borrow is the **small embedding contract**. A developer
can load a sheet, create a unit, put it on the ground, advance it with the
host’s clock, and remove it. Jianghu can apply the same idea at its own
level. A developer should be able to load a small authored world, ask for a
route, and show an actor following it from an ordinary Three.js scene.

1. **Start with the two existing character consumers.** Give Baichuan and the
   agent map the same Canvas-backed sprite adapter. The caller provides
   appearance and ground pose. Keep the city’s resident schedules, the map’s
   status rules, and the pointer UI with their respective examples. Move the
   direction type out of the city cast when the shared adapter needs it.
2. **Prove the world entry point with a tiny independent scene.** Mount a
   terrace, a stair, and a character using the Rainbell movement modules and
   the shared camera. The test is whether the example can render, walk, and
   clean up without importing Baichuan’s entry point or cast. That gives a
   concrete reason to move surface rendering out of `buildCity` and shows
   which APIs are actually public.
3. **Use the Rainbell staff as the layered rendering example.** It exercises
   a custom gripping walk in the vendored Three.js adapter. Pixel comparisons
   verify the staff overlaps the torso and the grip covers the staff at four
   camera angles. Before using the adapter for more of the cast, measure
   texture uploads and draw calls and check the city’s costume accessories,
   shadows, and reflection passes. The upstream guide reports its browser
   checks against Three.js r187. Jianghu’s staff example runs on r180.
   [Upstream verification](https://3d.internet.dev/references/threejs.md#verified-here) ·
   [Jianghu package](package.json)

