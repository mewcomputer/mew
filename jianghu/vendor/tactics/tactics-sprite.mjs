//NOTE(@jimmylee)
//The portable sprite runtime: no dependencies, no renderer, nothing from this repository. It reads
//tactics-sprite-data.json and answers one question: which tiles does this unit draw right now? Canvas,
//CSS, Three.js or any other renderer draws the answer. tests/sprite-kit.test.js holds it to the engine's
//own rig on every sequence, view and frame, so a pose here is what the main route draws.
//
//Units are sprite pixels and ticks. A tick is 1/60 s, the unit TYPE1.SEQ delays are written in. A tile's
//(x, y) is its top left from the stance point: the unit origin column at the idle stance's center row, 20
//pixels above the soles. Nothing here allocates per frame once a pose or a player exists.

export const TICKS_PER_SECOND = 60;
export const STANCE_HALF_H = 20;

//NOTE(@jimmylee)
//Where a unit faces, as an offset onto the camera step: (facing + step) % 8 is the view. These four are
//the facings of a tactical RPG grid. The world vectors are the same index: 0 = E, 1 = SE, 2 = S, 3 = SW and on around.
export const FACING = { NE: 7, SE: 1, SW: 3, NW: 5 };
export const FACING_X = [Math.SQRT1_2, 1, Math.SQRT1_2, 0, -Math.SQRT1_2, -1, -Math.SQRT1_2, 0];
export const FACING_Z = [-Math.SQRT1_2, 0, Math.SQRT1_2, 1, Math.SQRT1_2, 0, -Math.SQRT1_2, -1];

//NOTE(@jimmylee)
//Eight views, four patterns: A/B draw the front sequence, C/D the back one, and A/D mirror about the
//unit origin.
const VIEW_PATTERN = [0, 1, 1, 2, 2, 3, 3, 0];
const DEFAULT_AZIMUTH = Math.atan2(0.5, -0.5);
const STEP_SIZE = Math.PI / 4;
const STEP_LEAD = Math.PI / 36;

//NOTE(@jimmylee)
//The camera step, 0-7, from a horizontal vector pointing from the scene toward the camera. Step 0 is the
//game's initial view. STEP_LEAD turns the step a little before the 45-degree line, as the engine does.
export function cameraStep(towardCameraX, towardCameraZ) {
  const raw = Math.round((Math.atan2(towardCameraX, towardCameraZ) - DEFAULT_AZIMUTH + STEP_LEAD) / STEP_SIZE);
  return ((raw % 8) + 8) % 8;
}

export const viewOf = (facing, step) => (facing + step) % 8;
export const isBackView = (view) => VIEW_PATTERN[view] >= 2;
export const isMirroredView = (view) => VIEW_PATTERN[view] === 0 || VIEW_PATTERN[view] === 3;

//NOTE(@jimmylee)
//Reads tactics-sprite-data.json once: cells by id, each sequence's length, each frame's body tile and
//every animation by name in lower case. Idle is not a sequence pair: it has one SHP frame per view.
export function createAtlas(data) {
  if (data?.format !== 'tactics-sprite-data') throw new Error('createAtlas: expected tactics-sprite-data.json');
  const cells = {};
  for (const cell of data.cells) cells[cell.id] = cell;
  const animations = new Map([['idle', { name: 'Idle', idle: true, loop: false, water: false, hitTick: null }]]);
  for (const animation of data.animations) if (!animations.has(animation.name.toLowerCase())) animations.set(animation.name.toLowerCase(), animation);
  return {
    data,
    cells,
    animations,
    sequences: data.sequences.map((entry) => ({ ...entry, totalTicks: entry.ticks.reduce((a, b) => a + b, 0) })),
    bodyTile: data.frames.map((tiles) => {
      let body = 0;
      let area = -1;
      for (let i = 0; i < tiles.length; i++) {
        const { w, h } = cells[tiles[i][0]];
        if (w * h > area) {
          area = w * h;
          body = i;
        }
      }
      return body;
    }),
  };
}

export function findAnimation(atlas, name) {
  const animation = atlas.animations.get(name.toLowerCase());
  if (!animation) throw new Error(`Unknown animation "${name}". GET /api/v1/animations or read data.animations for the names.`);
  return animation;
}

//NOTE(@jimmylee)
//The frame index of a sequence at `ticks`. A last delay of 0 holds that frame until the game moves on.
function frameIndexAt(entry, ticks, loop) {
  const elapsed = loop && entry.totalTicks > 0 ? ticks % entry.totalTicks : ticks;
  let t = 0;
  for (let i = 0; i < entry.ticks.length; i++) {
    t += entry.ticks[i];
    if (elapsed < t) return i;
  }
  return entry.ticks.length - 1;
}

//NOTE(@jimmylee)
//A pose owns its tile objects, so a renderer can ask for one every frame and allocate nothing. TYPE1
//draws at most 6 tiles in a frame. `tiles[0 .. count]` run back to front.
export function createPose() {
  const tiles = [];
  for (let i = 0; i < 6; i++) tiles.push({ id: '', sx: 0, sy: 0, w: 0, h: 0, x: 0, y: 0, flipX: false, order: 0, body: false });
  return { tiles, count: 0, shp: 0, frame: 0, done: false, back: false, mirrored: false, fb: 0, du: 0, moveX: 0, moveY: 0 };
}

function writeTiles(atlas, shp, mirrored, out) {
  const tiles = atlas.data.frames[shp];
  const body = atlas.bodyTile[shp];
  out.shp = shp;
  out.count = tiles.length;
  for (let i = 0; i < tiles.length; i++) {
    const [id, u, v, reversed] = tiles[i];
    const cell = atlas.cells[id];
    const tile = out.tiles[i];
    tile.id = id;
    tile.sx = cell.x;
    tile.sy = cell.y;
    tile.w = cell.w;
    tile.h = cell.h;
    tile.x = (mirrored ? -u : u) - cell.w / 2;
    tile.y = v - cell.h / 2;
    tile.flipX = mirrored !== (reversed === 1);
    tile.order = i - body;
    tile.body = i === body;
  }
}

//NOTE(@jimmylee)
//The pose of `animation` on `view` (0-7, viewOf) at `ticks` since it started, written to `out`. `moveX` and
//`moveY` are the SEQ unit motion on a flat 2D stage, already mirrored: add them to every tile. `fb` and `du`
//are the same motion before that, pixels forward along the facing and down, for a renderer with a world.
export function poseAt(atlas, animation, view, ticks, out = createPose(), loop = animation.loop, water = animation.water) {
  const back = isBackView(view);
  const mirrored = isMirroredView(view);
  out.back = back;
  if (animation.idle) {
    //NOTE(@jimmylee)
    //The idle frames carry their own mirror (F4-F6), so the pattern adds none.
    out.mirrored = false;
    out.frame = 0;
    out.done = true;
    out.fb = out.du = out.moveX = out.moveY = 0;
    writeTiles(atlas, atlas.data.idleFrames[view], false, out);
    return out;
  }
  const entry = atlas.sequences[back ? animation.back : animation.front];
  const frame = frameIndexAt(entry, ticks, loop);
  out.mirrored = mirrored;
  out.frame = frame;
  out.done = !loop && ticks >= entry.totalTicks;
  out.fb = entry.fb ? entry.fb[frame] : 0;
  out.du = entry.du ? entry.du[frame] : 0;
  out.moveX = out.fb * (mirrored ? 1 : -1);
  out.moveY = out.du;
  writeTiles(atlas, entry.frames[frame] + (water ? atlas.data.waterOffset : 0), mirrored, out);
  return out;
}

//NOTE(@jimmylee)
//Which events an animation crossed going from tick `from` to tick `to`, written to `out` as labels. An
//event at tick t fires when from < t <= to, so start from -1 to catch tick 0. PostGenericAttack is the tick
//a hit lands, PlaySound the tick a sound or a shot goes.
export function eventsBetween(entry, from, to, loop, out = []) {
  out.length = 0;
  if (!entry.events) return out;
  for (const [tick, label] of entry.events) {
    if (loop && entry.totalTicks > 0) {
      if (Math.floor((to - tick) / entry.totalTicks) > Math.floor((from - tick) / entry.totalTicks)) out.push(label);
    } else if (from < tick && tick <= to) out.push(label);
  }
  return out;
}

//NOTE(@jimmylee)
//One unit's clock: play by name, ask for the pose each frame. `seconds` is whatever clock the game runs on,
//the same one for every call. `then` names the animation to go to when this one is done, which is how an
//attack comes back to idle, and `duration` ends a loop or a held frame after that many seconds, which is how
//a shiver stops. `events` holds what the last update crossed, including on the update that moved on.
//A held last frame counts as done, so with no `duration` a `then` leaves it at once.
export function createPlayer(atlas, { facing = 'SE', animation = 'Idle', seconds = 0 } = {}) {
  const pose = createPose();
  const start = (name, at, { loop, water, then = null, duration = Infinity } = {}) => {
    player.animation = findAnimation(atlas, name);
    player.loop = loop ?? player.animation.loop;
    player.water = water ?? player.animation.water;
    player.then = then;
    player.duration = duration;
    player.startedAt = at;
    player.lastTicks = -1;
  };
  const player = {
    facing: typeof facing === 'number' ? facing : FACING[facing],
    animation: null,
    loop: false,
    water: false,
    then: null,
    duration: Infinity,
    startedAt: seconds,
    lastTicks: -1,
    events: [],
    pose,
    play(name, at, options) {
      start(name, at, options);
      return player;
    },
    update(at, step = 0) {
      const view = viewOf(player.facing, step);
      const ticks = Math.max(0, (at - player.startedAt) * TICKS_PER_SECOND);
      player.events.length = 0;
      poseAt(atlas, player.animation, view, ticks, pose, player.loop, player.water);
      const entry = player.animation.idle ? null : atlas.sequences[pose.back ? player.animation.back : player.animation.front];
      if (entry) {
        eventsBetween(entry, player.lastTicks, ticks, player.loop, player.events);
        player.lastTicks = ticks;
      }
      const length = player.duration !== Infinity ? player.duration : entry ? entry.totalTicks / TICKS_PER_SECOND : 0;
      if (player.then && (player.duration !== Infinity ? at - player.startedAt >= length : pose.done)) {
        start(player.then, player.startedAt + length);
        poseAt(atlas, player.animation, view, Math.max(0, (at - player.startedAt) * TICKS_PER_SECOND), pose, player.loop, player.water);
      }
      return pose;
    },
  };
  start(animation, seconds);
  return player;
}

//NOTE(@jimmylee)
//Draws a pose on a 2D canvas with its stance point at (x, y), `scale` screen pixels per sprite pixel. The
//soles are at y + 20 * scale. Tiles draw back to front, so the order is the depth.
export function drawPose(context, image, pose, x, y, scale = 1) {
  context.imageSmoothingEnabled = false;
  for (let i = 0; i < pose.count; i++) {
    const tile = pose.tiles[i];
    const left = x + (tile.x + pose.moveX) * scale;
    const top = y + (tile.y + pose.moveY) * scale;
    const w = tile.w * scale;
    const h = tile.h * scale;
    if (tile.flipX) {
      context.save();
      context.translate(left + w, top);
      context.scale(-1, 1);
      context.drawImage(image, tile.sx, tile.sy, tile.w, tile.h, 0, 0, w, h);
      context.restore();
    } else {
      context.drawImage(image, tile.sx, tile.sy, tile.w, tile.h, left, top, w, h);
    }
  }
}

//NOTE(@jimmylee)
//The center of the hand a tile draws, [x, y] from the stance point with the pose's motion added, for a
//prop held on a 2D stage: `arm` 'first' reads the back tile, 'last' the front one. Null when that tile
//has no grip, or when the data has none: a grip belongs to one sheet's art, so data.hands is a1.png's
//and a kit built for another sheet leaves it out. Never reuse a grip across sheets.
export function handPoint(atlas, pose, arm = 'last', out = [0, 0]) {
  const tile = pose.tiles[arm === 'first' ? 0 : pose.count - 1];
  const grip = atlas.data.hands?.[tile.id];
  if (!grip) return null;
  out[0] = tile.x + (tile.flipX ? tile.w - 1 - grip[0] : grip[0]) + 0.5 + pose.moveX;
  out[1] = tile.y + grip[1] + 0.5 + pose.moveY;
  return out;
}

//NOTE(@jimmylee)
//The same hand read from an SHP frame, before any mirror or motion, for a renderer that places those itself.
export function frameHand(atlas, shp, arm = 'last', out = [0, 0]) {
  const tiles = atlas.data.frames[shp];
  const [id, u, v, reversed] = tiles[arm === 'first' ? 0 : tiles.length - 1];
  const grip = atlas.data.hands?.[id];
  if (!grip) return null;
  const cell = atlas.cells[id];
  out[0] = u - cell.w / 2 + (reversed ? cell.w - 1 - grip[0] : grip[0]) + 0.5;
  out[1] = v - cell.h / 2 + grip[1] + 0.5;
  return out;
}
