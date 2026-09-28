# mew desktop distribution plan

Goal: make the native GPUI client a usable macOS daily-driver install while
keeping the CLI/TUI distribution independent.

## Decisions

- CLI/TUI remain the existing release products: platform tarballs, checksums,
  Homebrew formula, and the shell installer.
- The native client ships as a separate macOS app artifact. It includes the
  GPUI executable, the daemon sidecar, the CEF framework, and every CEF helper
  app required by the bundle layout.
- Build arm64 and Intel artifacts separately. The embedded CEF distribution is
  architecture-specific, so a universal app is not a first-release goal.
- Publish a `.dmg` for convenient installation and retain a `.zip` or raw
  `.app` archive for scripted/manual use.
- Do not require Apple Developer credentials. The first distribution is
  unsigned or ad-hoc signed, with clear first-launch Gatekeeper guidance.
  Notarization, stapling, and trusted update delivery are explicitly deferred.
- GitHub Releases remain the source of truth for desktop artifacts and
  SHA256SUMS. There is no custom download service or updater in this pass.

## Current local workflow

From a checkout on macOS:

```sh
just desktop-install
open -a /Applications/mew.app
```

`desktop-install` builds the release GPUI client, daemon, and CEF helper,
packages `target/release/bundle/macos/mew.app`, and copies it to
`/Applications/mew.app`. Re-running it replaces the installed app but leaves
configuration and sessions under `~/.config/mew` intact.

`just desktop-dev` remains the debug/HMR-style path and is not a distribution
artifact. The package script currently expects a matching CEF distribution in
`~/.local/share/cef`, `CEF_PATH`, or `MEW_CEF_FRAMEWORK_SOURCE`.

## Workstreams

### 1. Release identity

- Use the workspace semver for `CFBundleShortVersionString`,
  `CFBundleVersion`, release names, and `mew --version`.
- Preserve the git revision as separate build metadata for daemon/client
  diagnostics, including a dirty marker for local builds.
- Stamp the app and helper `Info.plist` files from the package version during
  packaging so the shipped bundles do not depend on a checked-in fallback.

### 2. Deterministic app packaging

- Keep `scripts/package-desktop-native.sh` responsible for assembling the app
  bundle and validate that all required executables and framework paths exist.
- Add a release packaging wrapper that stages the app, adds the project license
  and CEF's generated `CREDITS.html` notices, creates the `.dmg` and archive,
  and writes checksums. Refuse to publish when the CEF notices are missing.
- Keep the daemon sidecar adjacent to `mew-desktop`; the supervisor should
  continue resolving it from the installed app bundle without environment
  variables.
- Add a smoke check for bundle metadata, executable permissions, CEF helper
  names, and a launch/daemon health check on macOS.

### 3. Release automation

- Add a desktop job to the tag-triggered release workflow for
  `aarch64-apple-darwin` and `x86_64-apple-darwin`.
- Make the CEF version and source reproducible in CI, with caching where
  possible. A release must not depend on a developer's home directory.
- Upload desktop artifacts and their hashes alongside, but separately from,
  the CLI/TUI tarballs.
- Add a matching nightly desktop workflow only after the stable packaging path
  is repeatable.

### 4. Install surfaces

- Update desktop docs with DMG drag-to-Applications instructions and the
  unsigned first-launch path.
- Keep `brew install mew` and `mew.computer/get.sh` explicitly CLI/TUI-only.
- Add a separate `mew-desktop` Homebrew Cask only if the unsigned artifact is
  practical for Homebrew's quarantine behavior. It must not replace the CLI
  formula.
- Document `just desktop-install` as the source-checkout daily workflow.

### 5. Verification

- Build from a clean checkout on both macOS architectures.
- Inspect the produced bundle and archive contents before publishing.
- Mount the DMG, copy the app to `/Applications`, launch it, confirm the
  packaged daemon connects, and exercise a basic model-picker interaction.
- Verify every published artifact against `SHA256SUMS`.
- Run focused desktop tests, `just desktop-build`, and the release packaging
  smoke checks before cutting a tag.

## Explicitly out of scope

- Apple Developer signing, notarization, and stapling.
- Sparkle or another automatic updater.
- A universal macOS binary.
- Windows or Linux GPUI packaging.
- Merging the desktop app into the CLI/TUI Homebrew formula or shell installer.

## Done when

On a clean supported Mac, a user can download the matching desktop DMG from a
GitHub Release, install `mew.app`, follow the documented first-launch warning,
and use the app with its bundled daemon and browser runtime. CLI/TUI users can
continue installing `mew` through the existing independent release channels.
