import assert from 'node:assert/strict';
import test from 'node:test';
import { defaultInstallPlatform } from '../src/lib/install-platform.mts';

test('desktop Linux visitors start with Linux', () => {
  assert.equal(defaultInstallPlatform('Linux x86_64', 'Mozilla/5.0 (X11; Linux x86_64)'), 'linux');
  assert.equal(defaultInstallPlatform('Linux aarch64', 'Mozilla/5.0 (X11; Linux aarch64)'), 'linux');
});

test('macOS and unsupported or unavailable platforms start with macOS', () => {
  for (const [platform, userAgent] of [
    ['MacIntel', 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)'],
    ['Win32', 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'],
    ['Linux armv8l', 'Mozilla/5.0 (Linux; Android 14; Pixel 8)'],
    ['Linux x86_64', 'Mozilla/5.0 (X11; CrOS x86_64 14541.0.0)'],
    ['', ''],
  ]) {
    assert.equal(defaultInstallPlatform(platform, userAgent), 'macos', userAgent);
  }
});
