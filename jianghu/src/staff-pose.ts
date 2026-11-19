import data from '../vendor/tactics/tactics-sprite-data.json' with { type: 'json' };
import { createAtlas } from '../vendor/tactics/tactics-sprite.mjs';

function createStaffAtlas() {
  const walk = data.animations.find(({ name }) => name === 'Walk');
  if (!walk) throw new Error('The staff pose requires the Walk animation.');

  const frames = data.frames.slice();
  const sequences = data.sequences.slice();
  for (const index of [walk.front, walk.back]) {
    const sequence = sequences[index];
    const [, [, torsoX, torsoY], [hand, handX, handY, reversed]] = frames[sequence.frames[0]];
    const heldFrames = new Map<number, number>();
    const frameIds = sequence.frames.map((frame) => {
      const held = heldFrames.get(frame);
      if (held !== undefined) return held;

      // Keep the first frame's closed grip aligned to each frame's moving torso.
      const [farArm, torso] = frames[frame];
      const grip = [hand, Number(handX) + Number(torso[1]) - Number(torsoX),
        Number(handY) + Number(torso[2]) - Number(torsoY)];
      if (reversed !== undefined) grip.push(reversed);
      const id = frames.push([farArm, torso, grip]) - 1;
      heldFrames.set(frame, id);
      return id;
    });
    sequences[index] = { ...sequence, frames: frameIds };
  }

  return createAtlas({ ...data, frames, sequences });
}

export const staffAtlas = createStaffAtlas();
