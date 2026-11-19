const rainbellSeconds = 7;
const rainbellPartials = [
  { frequency: 143, amplitude: 0.19, decay: 1.92 },
  { frequency: 145.4, amplitude: 0.05, decay: 1.60 },
  { frequency: 214, amplitude: 0.15, decay: 1.42 },
  { frequency: 301, amplitude: 0.11, decay: 0.92 },
  { frequency: 381, amplitude: 0.07, decay: 0.67 },
  { frequency: 578, amplitude: 0.09, decay: 0.36 },
  { frequency: 748, amplitude: 0.06, decay: 0.24 },
  { frequency: 1096, amplitude: 0.05, decay: 0.15 },
  { frequency: 1615, amplitude: 0.03, decay: 0.08 },
];

export function writeRainbell(samples: Float32Array, sampleRate: number) {
  for (const { frequency, amplitude, decay } of rainbellPartials) {
    if (frequency >= sampleRate / 2) continue;
    const radiansPerSample = 2 * Math.PI * frequency / sampleRate;
    const decayPerSample = Math.exp(-1 / (decay * sampleRate));
    let level = amplitude;
    for (let i = 0; i < samples.length; i++) {
      samples[i] += level * Math.sin(i * radiansPerSample);
      level *= decayPerSample;
    }
  }

  const attack = 0.006 * sampleRate;
  const release = 0.5 * sampleRate;
  for (let i = 0; i < samples.length; i++) {
    samples[i] *= Math.min(1, i / attack, (samples.length - 1 - i) / release);
  }
}

export function createRainbellAudio(signal: AbortSignal) {
  let context: AudioContext | undefined;
  let bell: AudioBuffer | undefined;

  signal.addEventListener('abort', () => {
    if (context) void context.close().catch(() => {});
    context = undefined;
    bell = undefined;
  }, { once: true });

  return {
    unlock() {
      if (signal.aborted || typeof AudioContext === 'undefined') return;
      if (!context) {
        context = new AudioContext();
        const sampleRate = Math.min(24000, context.sampleRate);
        bell = context.createBuffer(1, rainbellSeconds * sampleRate, sampleRate);
        writeRainbell(bell.getChannelData(0), sampleRate);
      }
      if (context.state !== 'running') void context.resume().catch(() => {});
    },
    ring() {
      if (context?.state !== 'running' || !bell) return;
      const source = context.createBufferSource();
      source.buffer = bell;
      source.connect(context.destination);
      source.start();
    },
  };
}
