import assert from 'node:assert/strict';
import test from 'node:test';
import { createRainbellAudio, writeRainbell } from '../src/city-audio.ts';

test('the bell has a quick strike and a long, quiet decay without clipping', () => {
  for (const sampleRate of [24000, 48000]) {
    const samples = new Float32Array(7 * sampleRate);
    writeRainbell(samples, sampleRate);

    const rms = (start: number, end: number) => {
      const window = samples.subarray(start * sampleRate, end * sampleRate);
      return Math.sqrt(window.reduce((energy, sample) => energy + sample * sample, 0) / window.length);
    };

    assert.equal(samples[0], 0);
    assert(samples.at(-1) === 0);
    assert(samples.every((sample) => Math.abs(sample) <= 1));
    assert(rms(0.01, 0.12) > 0.1);
    assert(rms(2, 2.25) > 0.01);
    assert(rms(5, 5.25) < rms(2, 2.25) / 3);
  }
});

test('bell audio can be used without a browser audio device', () => {
  const controller = new AbortController();
  const audio = createRainbellAudio(controller.signal);
  audio.ring();
  audio.unlock();
  controller.abort();
  audio.unlock();
  audio.ring();
});
