# mew CEF host

This is a macOS-first proof of concept for the authoritative browser engine.
It creates the visible Chromium window with `cef-rs` and enables a loopback
Chrome DevTools Protocol endpoint. `agent-browser` can attach to that same
visible browser with `--cdp`, so the agent and the user operate on one session.

The host is deliberately separate from the GPUI shell. CEF has its own
browser, render, GPU, and helper processes and requires a specific macOS app
bundle layout.

## Development

Install CEF once using the `cef-rs` helper, install the bundle command, then
run it from this package directory:

```sh
# optional: point at an already exported CEF distribution
export CEF_PATH="$HOME/.local/share/cef"
export DYLD_FALLBACK_LIBRARY_PATH="$CEF_PATH:$CEF_PATH/Chromium Embedded Framework.framework/Libraries"

cargo install cef --features build-util --bin bundle-cef-app
cd native/cef-host
bundle-cef-app mew-cef-host -o target/cef-bundle
open target/cef-bundle/mew-cef-host.app
```

If `CEF_PATH` is not already populated, the `cef-dll-sys` build step downloads
the matching CEF distribution automatically.

The CEF bundle command is installed from the `cef` crate because it is a
dependency of this package, not a binary target owned by this package.

The app prints its CDP port. In another terminal:

```sh
agent-browser --cdp 9223 snapshot
agent-browser --cdp 9223 click @e1
```

The port defaults to `9223` and can be changed with `MEW_CEF_DEBUG_PORT`. The
initial URL defaults to `https://example.com` and can be changed with
`MEW_CEF_URL`.

The host uses Chromium's mock keychain by default so an unsigned development
bundle does not trigger repeated macOS Keychain prompts. Set
`MEW_CEF_USE_SYSTEM_KEYCHAIN=1` to exercise real Keychain-backed browser
storage. This switch only affects Chromium browser data; mew's provider
credential keyring is unchanged.

## Native GPUI integration

The GPUI desktop target consumes this package as a native sibling. GPUI owns
the layout and sends the browser viewport bounds through `mew-browser-host`;
CEF creates a child `NSView` over that rectangle. The browser surface remains
opaque to the GPUI tree, and `agent-browser` can attach to CEF's CDP port so
automation and the user operate on the same visible page.

`just desktop-build` builds the GPUI client, the daemon, and the CEF helper,
then packages the framework and helper app bundles. Set
`MEW_CEF_FRAMEWORK_SOURCE` or `CEF_PATH` when the CEF distribution is outside
`~/.local/share/cef`; set `MEW_CEF_HELPER_PATH` to override the helper binary.

Chromium anchors its Mach-port rendezvous names to the main bundle identifier,
and every helper process resolves the same name to find the browser's
rendezvous server. The packaged native app provides the real bundle identity;
development can override it with `MEW_CEF_MAIN_BUNDLE_PATH` when needed.

The embedded browser currently runs without Chromium's macOS sandbox bootstrap
and requests software rendering by default. `MEW_CEF_ENABLE_GPU=1` opts into
GPU rendering for experiments. A production sandbox/helper layout is still a
separate hardening step.

CEF is distributed under its own license. The final app bundle must include
the CEF license and credits.
