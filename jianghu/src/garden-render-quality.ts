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

  function resetTiming() {
    frames = slowFrames = steadyMs = 0;
  }

  return {
    get current() { return tiers[tier]; },
    resetTiming,
    recordFrame(frameMs: number) {
      if (frameMs <= 0) {
        resetTiming();
        return;
      }

      // The ambient loop waits 34 ms and then aligns with the next display frame.
      if (frameMs > 72) slowFrames++;
      steadyMs = tier > 0 && frameMs <= 68 ? steadyMs + frameMs : 0;

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
