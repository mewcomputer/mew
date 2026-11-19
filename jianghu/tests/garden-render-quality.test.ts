import assert from 'node:assert/strict';
import test from 'node:test';
import { createGardenRenderQuality } from '../src/garden-render-quality.ts';

test('normal garden pacing at 30 Hz and isolated hitches keep full quality', () => {
  const quality = createGardenRenderQuality();
  for (let frame = 0; frame < 120; frame++) {
    assert.equal(quality.recordFrame(frame % 12 === 0 ? 80 : 67), undefined);
  }
  assert.deepEqual(quality.current, { pixelRatio: 1, waterCaptureSize: 512 });
});

test('sustained slow frames lower quality by one step at a time', () => {
  const quality = createGardenRenderQuality();
  for (const tier of [{ pixelRatio: 0.85, waterCaptureSize: 384 }, { pixelRatio: 0.7, waterCaptureSize: 256 }]) {
    for (let frame = 0; frame < 11; frame++) assert.equal(quality.recordFrame(80), undefined);
    assert.deepEqual(quality.recordFrame(80), tier);
  }
  for (let frame = 0; frame < 36; frame++) assert.equal(quality.recordFrame(80), undefined);
  assert.equal(quality.current.pixelRatio, 0.7);
});

test('resuming discards partial measurements and recovery needs steady rendering', () => {
  const quality = createGardenRenderQuality();
  for (let frame = 0; frame < 11; frame++) quality.recordFrame(80);
  quality.resetTiming();
  assert.equal(quality.recordFrame(0), undefined);
  for (let frame = 0; frame < 11; frame++) assert.equal(quality.recordFrame(80), undefined);
  assert.equal(quality.recordFrame(80)?.pixelRatio, 0.85);

  for (let frame = 0; frame < 119; frame++) assert.equal(quality.recordFrame(50), undefined);
  assert.equal(quality.recordFrame(80), undefined);
  for (let frame = 0; frame < 119; frame++) assert.equal(quality.recordFrame(50), undefined);
  assert.equal(quality.recordFrame(50)?.pixelRatio, 1);
});

test('camera motion lowers quality when active frames miss the display cadence', () => {
  const quality = createGardenRenderQuality();
  for (let frame = 0; frame < 11; frame++) assert.equal(quality.recordFrame(34, true), undefined);
  assert.equal(quality.recordFrame(34, true)?.pixelRatio, 0.85);
  for (let frame = 0; frame < 352; frame++) assert.equal(quality.recordFrame(17, true), undefined);
  assert.equal(quality.recordFrame(17, true)?.pixelRatio, 1);
});

test('the still is reserved for sustained stalls after trying every live quality tier', () => {
  const quality = createGardenRenderQuality();
  for (const pixelRatio of [0.85, 0.7]) {
    quality.recordFrame(180, true);
    quality.recordFrame(180, true);
    assert.equal(quality.stalled, false);
    assert.equal(quality.recordFrame(180, true)?.pixelRatio, pixelRatio);
  }
  quality.recordFrame(180, true);
  quality.recordFrame(180, true);
  quality.recordFrame(17, true);
  assert.equal(quality.stalled, false);
  quality.recordFrame(180, true);
  quality.recordFrame(180, true);
  quality.recordFrame(180, true);
  assert.equal(quality.stalled, true);
  quality.resetTiming();
  assert.equal(quality.stalled, false);
});
