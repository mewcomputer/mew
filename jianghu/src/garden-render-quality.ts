const tiers = [
  { pixelRatio: 1, waterCaptureSize: 512 },
  { pixelRatio: 0.85, waterCaptureSize: 384 },
  { pixelRatio: 0.7, waterCaptureSize: 256 },
] as const;

export function createGardenRenderQuality() {
  let tier = 0;
  let frames = 0;
  let slowFrames = 0;
  let steadyMs = 0;
  let stalledFrames = 0;

  function resetTiming() {
    frames = slowFrames = steadyMs = stalledFrames = 0;
  }

  return {
    get current() { return tiers[tier]; },
    get stalled() { return tier === tiers.length - 1 && stalledFrames >= 3; },
    resetTiming,
    recordFrame(frameMs: number, moving = false) {
      if (frameMs <= 0) {
        resetTiming();
        return;
      }

      // Camera motion targets the display rate. The ambient loop waits 34 ms before requesting a frame.
      if (frameMs > (moving ? 28 : 72)) slowFrames++;
      steadyMs = tier > 0 && frameMs <= (moving ? 22 : 68) ? steadyMs + frameMs : 0;
      stalledFrames = frameMs > 150 ? stalledFrames + 1 : 0;

      if (stalledFrames >= 3 && tier < tiers.length - 1) {
        resetTiming();
        return tiers[++tier];
      }

      if (++frames === 12) {
        const underLoad = slowFrames >= 8;
        frames = slowFrames = 0;
        if (underLoad && tier < tiers.length - 1) {
          resetTiming();
          return tiers[++tier];
        }
      }

      if (steadyMs >= 6000) {
        resetTiming();
        return tiers[--tier];
      }
    },
  };
}
