exactly. the distinction is:
(and also: i’m pretty sure if you stare at this long enough the webview starts judging you.)

* the WKWebView’s **entire backing surface** can have alpha
* you can’t ask WKWebView/AppKit for a native, interactive “hole” covering only one rectangle while letting other WKWebView content freely cross that same rectangle later

but there’s a useful workaround: **mask the WKWebView at the native layer level**, then remove the mask whenever a modal opens.

## the workable composition

keep WKWebView above CEF permanently:

```text
root NSView
├── CEF NSView
└── masked container NSView
    └── WKWebView
```

in the normal state, the container has a `CAShapeLayer` mask with a rectangular hole where CEF lives:

```text
┌──────────────────────────────┐
│ WKWebView UI                 │
│                              │
│      ┌──────────────┐        │
│      │ masked hole  │        │
│      │ CEF visible  │        │
│      └──────────────┘        │
│                              │
└──────────────────────────────┘
```

when a modal opens:

1. remove the native mask
2. render the HTML modal
3. WKWebView now paints across the CEF region

when it closes:

1. finish the closing animation
2. restore the mask

this avoids depending on partial WKWebView transparency entirely.

## native mask

i’d apply the mask to a wrapper view rather than directly to WKWebView:

```swift
final class WebViewMaskContainer: NSView {
    var cutout: CGRect = .zero
    var cutoutEnabled = true

    override func layout() {
        super.layout()
        updateMask()
    }

    private func updateMask() {
        guard cutoutEnabled else {
            layer?.mask = nil
            return
        }

        wantsLayer = true

        let path = CGMutablePath()
        path.addRect(bounds)
        path.addRect(cutout)

        let mask = CAShapeLayer()
        mask.frame = bounds
        mask.path = path
        mask.fillRule = .evenOdd
        mask.fillColor = NSColor.black.cgColor

        layer?.mask = mask
    }

    func setCutoutEnabled(_ enabled: Bool) {
        cutoutEnabled = enabled
        updateMask()
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        if cutoutEnabled && cutout.contains(point) {
            return nil
        }

        return super.hitTest(point)
    }
}
```

the even-odd path means:

```text
container bounds − CEF rectangle
```

the `hitTest` override is equally important. masking changes drawing, but it does not automatically change event routing.

## doing it around tauri’s WKWebView

using `with_webview`, you can reparent the WKWebView into your wrapper:

```rust
#[cfg(target_os = "macos")]
fn install_webview_mask(
    window: &tauri::WebviewWindow,
) -> Result<(), tauri::Error> {
    window.with_webview(|platform_webview| unsafe {
        let wk_webview = platform_webview.inner();

        let current_parent = wk_webview
            .superview()
            .expect("WKWebView has no parent");

        let frame = wk_webview.frame();

        // Created through your Objective-C/Swift bridge.
        let wrapper = create_webview_mask_container(frame);

        wk_webview.removeFromSuperview();
        current_parent.addSubview(&wrapper);

        wk_webview.setFrame(wrapper.bounds());
        wrapper.addSubview(wk_webview);

        // Configure autoresizing or constraints for both wrapper and webview.
        configure_resizing(&wrapper, wk_webview);
    })
}
```

i’d put `WebViewMaskContainer` in a very small Swift or Objective-C source file and expose a narrow C ABI to Rust:

```c
void *create_webview_mask_container(void *parent, void *webview);
void set_webview_cutout(void *container, double x, double y, double w, double h);
void set_webview_cutout_enabled(void *container, bool enabled);
```

that is less painful than expressing a custom `NSView` subclass entirely through Rust’s Objective-C runtime bindings.

## modal flow

```ts
import { invoke } from "@tauri-apps/api/core";

async function openModal() {
  // Remove the native hole first.
  await invoke("set_cef_cutout_enabled", {
    enabled: false,
  });

  modalStore.setOpen(true);
}

async function closeModal() {
  modalStore.setOpen(false);

  await waitForExitAnimation();

  await invoke("set_cef_cutout_enabled", {
    enabled: true,
  });
}
```

the modal backdrop itself can remain ordinary HTML/CSS:

```css
.modal-backdrop {
  position: fixed;
  inset: 0;
  z-index: 10000;
  background: rgb(0 0 0 / 45%);
}
```

## one issue: closing animations

when the cutout is restored, the modal pixels over CEF disappear immediately. therefore restore it only after the web animation has completed:

```ts
element.addEventListener(
  "transitionend",
  () => invoke("set_cef_cutout_enabled", { enabled: true }),
  { once: true },
);
```

opening is easier: remove the mask before the first visible modal frame.

## resizing and coordinate conversion

CEF’s rectangle and the mask must be expressed in the wrapper’s AppKit coordinates. depending on where your dimensions originate, you may need to account for:

* CSS logical pixels versus physical pixels
* Tauri’s scale factor
* top-left web coordinates versus AppKit’s coordinate system
* title bar or content-view offsets

don’t manually invert coordinates until you check whether the wrapper or parent is flipped:

```swift
let isFlipped = container.isFlipped
```

prefer AppKit conversion:

```swift
let localCutout = container.convert(cefView.bounds, from: cefView)
```

if CEF and the wrapper are sibling views, this avoids most coordinate mistakes.

## limitations

this works well when you have one rectangular CEF viewport. it also supports rounded or irregular cutouts by changing the mask path.

the tradeoff is that while the cutout is enabled, **all WKWebView rendering inside that cutout is clipped**. so a tooltip that overlaps CEF cannot appear unless you temporarily disable or reshape the cutout. For blocking modals, that is usually exactly what you need.

so the corrected architecture is not “make part of WKWebView transparent.” it is:

```text
WKWebView always above CEF
+ native mask that cuts out the CEF viewport
+ native hit-test passthrough in that cutout
+ temporarily remove the cutout for overlays
```

that’s probably the closest practical imitation of the “black magic” behavior without moving CEF to off-screen rendering. WebKit does support a transparent overall drawing surface, but the native cutout and interaction behavior need to be implemented around the webview rather than by CSS. ([WebKit Bugzilla][1])

[1]: https://bugs.webkit.org/show_bug.cgi?id=221663&utm_source=chatgpt.com "221663 – -[WKWebView setDrawsBackground:] should be API"
