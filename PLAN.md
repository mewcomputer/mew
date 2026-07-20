# Plan: Put CEF below WKWebView (full layering inversion) and delete the AppKit menus

## Goal

Today the embedded CEF browser is a native child view that paints **above** the
Tauri WKWebView. Any UI that must appear over the browser (the workbench "+"
surface picker, the browser-tools menu) has to be an AppKit `NSMenu`, and any
HTML overlay (settings dialog, command palette, browser-tools popover, session
switcher) that overlaps the CEF rect is clipped/covered by it.

Invert the default order following the approach in `temp.md`:

- CEF sits **below** the WKWebView permanently.
- The WKWebView becomes transparent, and the React app leaves a transparent
  hole exactly where the CEF viewport lives, so the browser shows through.
- All HTML overlays render above the browser natively — no dynamic reordering
  commands needed for the blocking-modal cases the app actually has.
- The two AppKit menu paths (`native_workbench_menu.rs`) are deleted; the
  existing HTML fallbacks (shadcn `CommandDialog` surface picker, browser-tools
  popover) become the only implementation on every host.

This is the "full inversion + all at once" variant confirmed with the user.

## Why full inversion instead of dynamic reordering

The app already has opaque HTML overlays that can overlap the CEF rect today
and are silently broken on desktop: the shadcn `Dialog`-based settings window,
`CommandDialog` surfaces (cmd/ctrl+k palette, the surface picker), the
browser-tools popover (when it falls back to HTML), dropdowns/selects, sheets,
and toasts. Dynamic reordering (pull WKWebView above only while a modal is
open) would fix blocking modals but still requires tracking every overlay's
open state in the frontend and passing it to Rust. Full inversion fixes all of
them at once with a single static ordering, at the cost of two real
constraints:

1. The React app must never paint anything over the transparent CEF hole
   (otherwise that content silently covers the browser). The hole already has
   a dedicated owner: the `viewportRef` div in `browser-panel.tsx`.
2. The window background behind the webview must be opaque and match the app
   background, or the transparent webview shows the desktop through it.

Both are manageable and worth it.

## Relevant current state (verified by reading the code)

- `mew-web-ui/src-tauri/src/lib.rs`: `initialize_cef` passes
  `window.ns_view()` (Tauri content view) as the CEF parent and registers 6
  `native_*_menu_*` commands alongside 5 `cef_browser_*` commands.
- `native/cef-host/src/embed.rs`: `set_as_child(parent_view, …)` in
  `MewBrowserProcessHandler::on_context_initialized`; `native_view` handle
  captured in `on_after_created`; rect/visibility applied through
  `CefEmbedController`.
- `mew-web-ui/src-tauri/src/native_workbench_menu.rs`: AppKit `NSMenu`
  implementation for the workbench "+" menu and the browser-tools menu, with
  `native-workbench-menu-event` / `native-browser-tools-menu-event` Tauri
  events.
- `mew-web-ui/src-tauri/tauri.conf.json`: no `transparent`, no
  `macOSPrivateApi` yet.
- `mew-web-ui/src-tauri/Cargo.toml`: already has `objc2` 0.6 + `objc2-app-kit`
  0.3 (NSView/NSMenu/NSResponder features) — no new native deps needed.
- `mew-web-ui/src/components/right-rail.tsx`: HTML fallback for the "+" menu is
  the existing `CommandDialog`-based `surfacePickerOpen` picker (complete,
  searchable, tested). Native path is `nativeAddMenuAvailable` +
  `showNativeWorkbenchMenu`.
- `mew-web-ui/src/components/browser-panel.tsx`: HTML fallback for browser
  tools is the existing `toolsOpen` popover (complete). Native path is
  `nativeToolsAvailable` + `showNativeBrowserToolsMenu`. The CEF viewport is
  the `viewportRef` div with `bg-muted/20` (line ~469).
- `mew-web-ui/src/lib/host.ts`: TS wrappers for all native menu + CEF commands.
- `mew-web-ui/src/__tests__/host.test.ts`: covers the web-host no-op menu
  listeners.
- Existing native z-order touchpoint: none today — CEF relies on being added
  last/above by CEF's own `addSubview`.

## Steps

### 1. Native: make CEF the bottom sibling of WKWebView (mew-desktop, macOS)

**File: `mew-web-ui/src-tauri/src/lib.rs`**

- In `initialize_cef`, after the CEF controller is created (CEF adds its view
  to the content view during `on_context_initialized` asynchronously, so this
  cannot happen synchronously at setup), expose a new command
  `cef_browser_order_below_webview` — but rather than a separate command,
  fold the ordering into the existing owner-claim path: when
  `cef_browser_set_rect` runs with `visible: true` (the moment React claims
  the browser for a tab), also ensure CEF is ordered **below** the WKWebView.
  Implementation:
  - Add a small macOS-only helper module (new file
    `mew-web-ui/src-tauri/src/native_layering.rs`) with one function:
    ```rust
    pub fn order_cef_below_webview(wk_content_view: usize, cef_view: usize)
    ```
    It resolves both `NSView`s, asserts they share a superview, and calls
    `parent.addSubview_positioned_relativeTo(cef_view, NSWindowOrderingMode::Below, Some(wk_webview))`.
    To find the WKWebView: the Tauri content view's subviews include the
    `WKWebView`; CEF's view is also a subview. The helper walks the content
    view's subviews, identifies the CEF view by pointer equality with the
    handle passed in, and treats the other top-level subview as the webview.
    Safer alternative: use `window.with_webview(|wv| …)` to get the exact
    `WKWebView` pointer from Tauri instead of guessing from the subview list.
    Prefer `with_webview` — it is the documented API and immune to Tauri
    adding extra container views. (`WebviewWindow::with_webview` runs the
    closure on the main thread and yields the platform webview whose
    `.inner()` is the `WKWebView`.)
  - The `CefEmbedController` needs to expose the native view handle so the
    Tauri layer can reorder it. Add to `native/cef-host/src/embed.rs`:
    ```rust
    pub fn native_view_handle(&self) -> usize {
        self.state.native_view.load(Ordering::Acquire)
    }
    ```
    (mirrors the existing internal loads; no behavior change).
  - Ordering must be re-applied when CEF recreates its view. Today the view
    is created once per process (`on_after_created` runs once), but the
    lifecycle hardening notes say close/reopen paths exist. Make the ordering
    idempotent and apply it at two points:
    1. Right after `initialize_cef` completes successfully, on the main
       thread via `run_on_main_thread` (covers the steady state).
    2. Inside `cef_browser_set_rect` when `visible` is set, guarded by a
       cheap "already ordered" `AtomicBool` so the AppKit call happens once
       per CEF view creation, not per rect update. Reset the flag if the
       native view handle changes (compare-and-store the last seen handle).
  - Keep the owner-gating semantics exactly as they are; ordering is a
    no-op when CEF is unavailable.

**Tests (Rust, `mew-desktop`)**: unit-test the "order only once per handle"
guard logic with the guard factored into a small pure struct
(`NativeLayeringGuard { last_ordered_handle: … }`) so it is testable without
AppKit. AppKit calls themselves stay thin and untested, same as
`native_workbench_menu.rs` today.

### 2. Native: make the WKWebView transparent

**Files: `mew-web-ui/src-tauri/tauri.conf.json`, `mew-web-ui/src-tauri/src/lib.rs`**

- `tauri.conf.json`: add `"transparent": true` to the `main` window and
  `"macOSPrivateApi": true` under `app`. (Tauri documents macOS webview
  transparency as requiring the private-API flag; direct distribution makes
  the App Store constraint irrelevant here.)
- The window behind the webview must be opaque or the desktop shows through
  the transparent hole margins. Tauri paints the `NSWindow` background from
  the webview by default; with transparency enabled the app background comes
  from the page. So: the React root keeps its existing `bg-background`
  coverage everywhere **except** the CEF hole. Verify no ancestor of the
  viewport hole sets a background (the hole's own div currently has
  `bg-muted/20` — changed in step 4).
- If visual inspection shows the opaque-window concern is real (e.g. during
  resize, before React paints), set the `NSWindow` background color to the
  app background in `initialize_cef`/setup via `objc2-app-kit`
  (`NSWindow::setBackgroundColor` with the theme background). Only add this
  if the smoke test shows a flash of desktop through the window; otherwise
  skip (YAGNI).

### 3. Frontend: carve the transparent hole

**File: `mew-web-ui/src/index.css` (and/or the global stylesheet)**

- Add `html, body, #root { background: transparent; }` **only when running
  in the desktop host** — the browser build must keep its normal background
  or every page becomes transparent over nothing. Mechanism: `isDesktopHost()`
  already exists; add a `data-host="desktop"` attribute on
  `document.documentElement` during `initializeHost()` in
  `mew-web-ui/src/lib/host.ts`, and gate the CSS:
  ```css
  [data-host="desktop"] body { background: transparent; }
  ```
  Keep the app's own surfaces (chat column, workbench chrome, sidebars)
  painting their existing `bg-background` — only the layers *above* the CEF
  hole must be transparent.

**File: `mew-web-ui/src/components/browser-panel.tsx`**

- The CEF viewport div (`viewportRef`, currently `bg-muted/20`) must be
  transparent **when CEF is available** so the native view shows through.
  When CEF is unavailable (`nativeAvailable === false`) the div is just a
  placeholder and should keep its muted background. Since `nativeAvailable`
  starts as `null` on desktop, default the div to transparent only after
  `nativeAvailable === true`; render `bg-muted/20` otherwise.
- The empty-state overlay ("the browser surface is closed…") sits inside the
  viewport div. With a transparent hole, that overlay needs its own
  background when shown (it currently relies on the parent's `bg-muted/20`).
  Give the empty-state container `bg-muted/20` (or `bg-background`) directly.
- Audit: no other element should paint over the viewport rect. The
  browser-tools popover and the surface picker *intentionally* overlap it —
  that is now fine and is the point.

### 4. Frontend: delete the native menu paths

**Files: `mew-web-ui/src/components/right-rail.tsx`, `mew-web-ui/src/components/browser-panel.tsx`, `mew-web-ui/src/lib/host.ts`, `mew-web-ui/src-tauri/src/lib.rs`, `mew-web-ui/src-tauri/src/native_workbench_menu.rs`, `mew-web-ui/src/__tests__/host.test.ts`**

- `right-rail.tsx`: remove `nativeAddMenuAvailable`, the
  `listenNativeWorkbenchMenuEvents` effect, `showNativeWorkbenchMenu` /
  `hideNativeWorkbenchMenu` calls, and the `addMenuOpen` state.
  `openAddWorkbenchMenu` becomes `setSurfacePickerOpen(true)` (plus toggle
  close behavior). The `CommandDialog` picker is already the full-featured
  version (searchable, icons, descriptions, shortcuts) — the native menu was
  the reduced one.
- `browser-panel.tsx`: remove `nativeToolsAvailable`/`nativeToolsOpen`, the
  native tools menu listener effect, `showNativeBrowserToolsMenu` /
  `hideNativeBrowserToolsMenu`, and the `toolsButtonRef` bounds computation.
  The tools button always toggles the HTML popover (`setToolsOpen`).
- `host.ts`: delete `nativeWorkbenchMenuAvailable`,
  `showNativeWorkbenchMenu`, `hideNativeWorkbenchMenu`,
  `listenNativeWorkbenchMenuEvents`, `nativeBrowserToolsMenuAvailable`,
  `showNativeBrowserToolsMenu`, `hideNativeBrowserToolsMenu`,
  `listenNativeBrowserToolsMenuEvents`, and the
  `NativeWorkbenchMenuEvent`/`NativeBrowserToolsMenuEvent` types.
- `src-tauri/src/lib.rs`: delete the 6 `native_*_menu_*` commands, their
  `*_impl` shims, and the `mod native_workbench_menu;` declaration; remove
  the commands from `invoke_handler`.
- Delete `mew-web-ui/src-tauri/src/native_workbench_menu.rs`.
- `src-tauri/Cargo.toml`: check whether `objc2-app-kit` features `NSMenu` /
  `NSMenuItem` are still needed after deletion (the layering helper needs
  `NSView`; keep `NSResponder` only if something else uses it). Trim unused
  features.
- `__tests__/host.test.ts`: update to drop the deleted-listener no-op tests;
  keep/extend the no-op tests for the remaining CEF listeners on the web
  host.
- Check `mew-web-ui/src/components/__tests__` for right-rail/browser-panel
  tests referencing the native menus and update them (CURRENT.md notes a
  "web-host no-op listener regression test" and RightRail interaction tests).

### 5. Verify

- `cargo test --manifest-path mew-web-ui/src-tauri/Cargo.toml` — layering
  guard unit tests + existing 12 host tests.
- `cargo clippy --manifest-path mew-web-ui/src-tauri/Cargo.toml --all-targets -- -D warnings`
  and same for `native/cef-host`; `cargo fmt --all -- --check`.
- `pnpm --filter mew-web-ui test`, `pnpm --filter mew-web-ui build`,
  TypeScript check.
- **Manual desktop smoke (this change is mostly visual; must be done live):**
  1. `just desktop-dev`; open a Browser workbench tab, navigate to
     example.com — page renders and is interactive (proves CEF is below but
     still receiving input: WKWebView hit-tests through transparent regions
     by default, so clicks in the hole reach CEF — confirm this; if WKWebView
     swallows them, that's the known risk below).
  2. With the browser visible, open: the "+" surface picker, the browser
     tools popover, cmd/ctrl+k palette, the settings dialog — each should
     render **over** the live browser page with correct dimming.
  3. Move/resize the window and the workbench divider — the CEF view tracks
     the hole (existing rect plumbing), no ghosting or offset.
  4. Switch workbench tabs away from Browser and back — CEF hides and
     returns without ordering glitches.
  5. Check window edges during resize for desktop bleed-through (the
     opaque-window concern from step 2).
  6. `pnpm desktop:verify:cef` — all 7 checks still pass against the
     reordered view.
- Update `CURRENT.md` with a dated entry.

## Risks and mitigations

1. **Hit-testing through the transparent hole.** WKWebView normally ignores
   clicks on fully transparent pixels, which is what lets CEF receive input
   under the hole. This is well-established behavior for transparent
   WKWebViews, but it is private-API-adjacent; if clicks land on the webview
   instead of CEF, the fallback is `setIgnoresMouseEvents` is too blunt
   (kills all input) — the real fallback is reverting to dynamic ordering
   for the browser region. Smoke step 5.1 is the gate for this.
2. **Content painting over the hole silently covers the browser.** Any future
   React work that adds a background to an ancestor of the viewport will
   break the browser view without an error. Mitigation: a short comment on
   the viewport div in `browser-panel.tsx` stating it must stay transparent
   because CEF renders beneath it.
3. **CEF view recreation.** If CEF recreates its native view (close/reopen
   flows), the new view is added above again until the guard re-orders it.
   The per-handle guard in step 1 handles this; verify in smoke step 5.4.
4. **`macOSPrivateApi`** is a hard requirement for transparent windows on
   macOS Tauri and blocks Mac App Store distribution. mew is distributed
   directly, so this is acceptable; note it in the CURRENT.md entry.
5. **Deleting native menus removes the only above-CEF UI until transparency
   works.** Do not merge step 4 separately from steps 1–3; land the whole
   change together, and keep the smoke test before deleting
   `native_workbench_menu.rs` locally if step 5.1 fails.

## Out of scope

- Non-blocking partial-overlap UI that must coexist with CEF interaction
  (hit-test passthrough wrapper) — temp.md's "when this stops being enough"
  case. Not needed by any current surface.
- Off-screen rendering, Windows/Linux CEF embedding.
- Dynamic reordering commands (`set_cef_covered` style) — unnecessary once
  the inversion holds; the temp.md modal dance is the fallback plan if
  hit-testing fails, not part of this change.
