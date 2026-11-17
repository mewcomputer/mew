# browser composition experiment

This is an isolated host comparison. It does not replace the packaged mew UI,
change the daemon, or alter the generated theme system.

Both hosts load the same React/shadcn shell, theme tokens and interactive browser
document. The document refuses iframe embedding. The Electron guest is a real
`webview`; the CEF guest is a real offscreen browser painted into a DOM canvas.

## observed results, 2026-10-27

The existing windowed CEF app blanks its browser when an overlapping app menu
opens. The Electron prototype keeps the page beneath the overlay, including
translucency, rounded clipping and opacity changes.

| check in the packaged window | Electron | CEF canvas |
| --- | --- | --- |
| app and browser document load | passed | passed |
| translucent app popover over live browser | passed | passed |
| app dropdown intercepts clicks over guest controls | passed | passed |
| resize and fade beneath an open overlay | passed | passed |
| close/reopen retains browser notes, counter and chat draft | passed | passed |
| browser page scroll, nested scroll and outer workbench scroll | passed | passed |
| guest text, Unicode paste, Enter, Backspace and arrow keys | passed | passed after Enter correction |
| browser select popup | passed | passed, painted inside the canvas |
| Escape returns focus to the app overlay trigger | passed | initially observed; inconsistent in later native checks |
| keyboard traversal into and out of guest | passed | exit unimplemented; focus stays on the canvas |
| browser-to-chat copy/paste | passed | incomplete app editing-shortcut route |
| guest HTML controls exposed through macOS accessibility | observed | unimplemented for the canvas |
| native IME candidate positioning | untested | unimplemented |
| light and dark app themes with browser overlay | passed | passed |
| native window close | passed | passed, exit 0 |

The CEF window checks resumed after unlocking the Mac. Both approaches provide
actual DOM composition. The CEF canvas remains an incomplete host-input
implementation: guest keyboard exit, reliable focus handoff, app editing
shortcuts, native IME geometry and guest accessibility need more work.

CEF startup exposed prototype bugs in query-handler registration, helper
framework lookup, AppKit run-loop handling and browser-process keychain policy.
The corrected app loads both documents and produces OSR pixels. Additional
regressions caught accepted sockets retaining macOS nonblocking mode, premature
connection closure, an incorrect OSR close result and Enter missing its separate
CEF character event. The keychain policy matches the existing production host;
a real CEF command-line regression covers default and explicit system-keychain
settings. Reusing the GPUI-specific shortcut registration did not fix this
standalone app and removed its AX tree, so that experiment was removed.

Electron 45.0.0 reported Chromium 156.0.8078.48 and Node 24.22.0. The CEF SDK is
150.0.14 / Chromium 150.0.7871.129. Versions differ, and the CEF executable is a
debug build. This is a composition and integration experiment, not a controlled
runtime benchmark.

## resource evidence

One macOS `ps` spot sample of the five Electron processes with the animated
fixture visible summed to 32.1% CPU and 488,816 KiB RSS. With the fixture paused,
a later sample summed to 0% CPU and 508,272 KiB RSS. RSS sums include shared pages;
they are not private application memory or a steady-state budget. These samples
do not establish a general CEF versus Electron performance result.

The CEF debug CPU/HTTP spike's seven processes sampled at 70.4% CPU / 901,728 KiB
RSS with the animated fixture visible, and 36.9% CPU / 787,984 KiB RSS after
pausing it. The idle polling and a worker thread per HTTP connection are costs of
this deliberately basic transport. These numbers are not accelerated CEF results.

At a fixed 691 × 758 physical pixels, an 88.021-second interval transferred
11,026,574,456 BGRA body bytes in 5,263 responses: about 125.27 MB/s and 59.8
consumed frames/s. That is transport throughput, not a frame-pacing or latency
measurement. During a later paused interval of 37.967 seconds, native frame and
successful HTTP body counters remained unchanged. Conditional responses avoid
pixel copying while idle, although request polling still consumes CPU.

CEF reports published BGRA bytes, successful HTTP frame-body bytes, physical
frame dimensions and frame counts. The canvas requests one frame at a time,
reuses its pixel buffer, skips unchanged frames using ETags, and stops fetching
while the pane is inert. The browser remains retained. There is still a CPU
copy, HTTP transport, BGRA conversion and canvas upload for each consumed frame.
That path is deliberately simple and must not be treated as a production
rendering strategy.

## what this supports

Recommend Electron for the desktop shell, retaining the Rust daemon, shared
protocol and theme system. The tested composition behavior and native browser
input are available without a custom pixel/input bridge. The CPU-canvas spike
does not offer a resource advantage that would justify its remaining host-input
work for the requested UI. This is a host recommendation, not a repository rewrite.

Stock Electron warns that `webview` has architectural stability risks. This
experiment does not reproduce Codex's customized Chromium runtime. A production
Electron decision still needs an explicit guest-embedding policy, crash handling
and retained-page resource management. `WebContentsView` is another option, but
its native bounds and stacking need separate evaluation.

CEF supports accelerated macOS OSR through IOSurface. This experiment does not
import those surfaces into the React renderer, so its CPU path cannot rule out a
better CEF implementation. Accessibility, IME geometry, keyboard traversal and
native popup behavior remain host responsibilities for a canvas approach.

## run the shared UI and Electron host

From the repository root, with the existing web dependencies installed:

```sh
pnpm --filter mew-web-ui exec vite build --config prototypes/composition/vite.config.ts
npm --prefix prototypes/browser-composition ci
npm --prefix prototypes/browser-composition start
```

To make a disposable macOS app with its own bundle identity and profile:

```sh
npm --prefix prototypes/browser-composition run package
```

The command prints the app path, bundle ID and executable. It refuses to replace
an existing destination, preserves framework symlinks and entitlements, and
verifies the ad-hoc signature. The guest is restricted to the local fixture,
with Node disabled, sandboxing enabled and permissions/popups/downloads denied.

The CEF example is `native/cef-host/examples/composition.rs`. It uses the existing
bundled helper and framework. `MEW_COMPOSITION_ASSETS` must point to the absolute
`target/composition-ui` directory. `MEW_CEF_FRAMEWORK_PATH` must also be passed to
the helper processes; relying on their default framework lookup is insufficient
for the current app-bundle layout. Use `MEW_CEF_ENABLE_GPU=1` consistently for this
comparison. This fixture-only CEF harness uses the existing unsandboxed embedding
configuration and has no daemon or user configuration.

Build the example with `cargo build --manifest-path native/cef-host/Cargo.toml
--example composition`. To reproduce the validated macOS packaging procedure,
set `source_app` below to an existing mew app bundle containing the matching CEF
framework and helper apps, then run this from the repository root:

```python
import json, pathlib, plistlib, shutil, subprocess, tempfile

source_app = pathlib.Path('target/debug/bundle/macos/mew.app').resolve()
binary = pathlib.Path('target/debug/examples/composition').resolve()
assets = pathlib.Path('target/composition-ui').resolve()
assert source_app.is_dir() and binary.is_file() and (assets / 'index.html').is_file()
root = pathlib.Path(tempfile.mkdtemp(prefix='mew-composition-cef-'))
app = root / 'mew-composition-cef.app'
subprocess.run(['/usr/bin/ditto', str(source_app), str(app)], check=True)
identifier = 'com.mew.composition.cefqa.' + root.name.removeprefix('mew-composition-cef-')
info = app / 'Contents/Info.plist'
data = plistlib.loads(info.read_bytes())
shutil.copy2(binary, app / 'Contents/MacOS' / data['CFBundleExecutable'])
data.update(CFBundleIdentifier=identifier, CFBundleName='mew composition CEF',
            CFBundleDisplayName='mew composition CEF')
info.write_bytes(plistlib.dumps(data))
for index, helper in enumerate(sorted((app / 'Contents/Frameworks').glob('*.app')), 1):
    info = helper / 'Contents/Info.plist'
    data = plistlib.loads(info.read_bytes())
    data['CFBundleIdentifier'] = f'{identifier}.helper{index}'
    info.write_bytes(plistlib.dumps(data))
subprocess.run(['/usr/bin/codesign', '--force', '--deep', '--sign', '-', str(app)], check=True)
subprocess.run(['/usr/bin/codesign', '--verify', '--deep', '--strict', str(app)], check=True)
framework = app / 'Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework'
env = {'MEW_COMPOSITION_ASSETS': str(assets), 'MEW_CEF_FRAMEWORK_PATH': str(framework),
       'MEW_CEF_ENABLE_GPU': '1'}
(root / 'launch-env.json').write_text(json.dumps(env, indent=2) + '\n')
print(app)
print(json.dumps(env, indent=2))
```

Preserve executable/helper names. Launch the printed app's
`Contents/MacOS/mew-desktop` with the printed environment. Clear inherited
`MEW_CEF_HELPER_PATH`, `MEW_CEF_MAIN_BUNDLE_PATH` and
`MEW_CEF_USE_SYSTEM_KEYCHAIN` overrides for this fixture run.

## verification

```sh
pnpm --filter mew-web-ui exec vitest run --config prototypes/composition/vite.config.ts
pnpm --filter mew-web-ui exec tsc --noEmit -p prototypes/composition/tsconfig.json
npm --prefix prototypes/browser-composition test
CEF_PATH=/Users/natalie/.local/share/cef/150.0.14 cargo test --manifest-path native/cef-host/Cargo.toml --example composition
```

The ordinary suites pass 9 UI, 5 Electron and 10 CEF tests. UI tests cover retained
DOM content and overlay focus return; they do not simulate browser acceptance.
Electron tests include actual packaging/signature verification and a real HTTP
server. CEF tests exercise ownership, bounds, conversion inputs, popup paint
callbacks, key-event translation, real HTTP frame responses and idle/partial
preconnections. The separately run native keychain
regression requires `MEW_CEF_FRAMEWORK_PATH` and `--ignored`.

Photos from the packaged windows are in this task's visualization directory:
`composition-01-windowed-live.jpg` through `composition-17-cef-composed.jpg`.

## primary references

- [Electron embedding options](https://www.electronjs.org/docs/latest/tutorial/web-embeds)
- [Electron webview behavior and stability warning](https://www.electronjs.org/docs/latest/api/webview-tag)
- [Electron security guidance](https://www.electronjs.org/docs/latest/tutorial/security)
- [Pinned CEF accelerated OSR delivery](https://github.com/chromiumembedded/cef/blob/7c1aa68455db1f1fad159c2b83070ad318212b3d/libcef/browser/osr/video_consumer_osr.cc)
- [Pinned CEF rendering contract](https://github.com/chromiumembedded/cef/blob/7c1aa68455db1f1fad159c2b83070ad318212b3d/include/cef_render_handler.h)
