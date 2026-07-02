# mew release + install distribution plan

Goal: ship semver-versioned releases of mew with easy install paths: Homebrew, a `curl | bash` install script, and updated docs. Use GitHub Releases as the single source of truth for artifacts and metadata instead of building a custom download API.

Status: planning / ready to hand off.

---

## Current state

- Workspace version is pinned to `0.1.0` in `Cargo.toml`.
- A release workflow exists at `.github/workflows/release.yml` and already triggers on `v*` tags.
- It builds three targets: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu`.
- It creates a GitHub release and uploads tarballs.
- It does **not** generate checksums, does not build `linux-arm64`, does not produce a Homebrew formula, and does not serve an install script.
- `docs/getting-started/installation.md` only documents `cargo install` and building from source.

---

## What "ready for semver" means

Before promoting install methods, the release artifacts need to be trustworthy and discoverable:

1. A real version number (not `0.1.0` forever).
2. A `CHANGELOG.md` following [Keep a Changelog](https://keepachangelog.com/) conventions.
3. Per-platform tarballs with attached SHA256 checksums.
4. A GitHub Release with human-readable notes.

---

## Phase 1: release pipeline hardening

### 1.1 Version and changelog

- Pick the first public version. Options:
  - `0.2.0` — signals pre-1.0 but semver-stable.
  - `1.0.0` — signals "ready" but locks major-version semantics.
  - Recommendation: start at `0.2.0` (or `0.1.0` if you consider the current state the first release) and reserve `1.0.0` for API-stable daemon/protocol contracts.
- Add `CHANGELOG.md` at repo root.
- Bump `workspace.version` in root `Cargo.toml`.
- Commit the version bump and changelog update before tagging.

### 1.2 Expand release matrix

Add to `.github/workflows/release.yml`:

- `aarch64-unknown-linux-gnu` (Linux ARM64, useful for cloud VMs and Asahi).
- Optional: a `x86_64-unknown-linux-musl` target for more portable Linux binaries.
- Optional: a Windows target if demand exists (currently untested; defer unless requested).

### 1.3 Generate and attach checksums

In the release job, after all artifacts are downloaded:

- Generate `SHA256SUMS` containing all `mew-vX.Y.Z-TARGET.tar.gz` hashes.
- Upload `SHA256SUMS` as a release asset.
- The install script and Homebrew formula will consume this file.

### 1.4 Release notes

Keep `softprops/action-gh-release@v2` with `generate_release_notes: true`, but also prepend the relevant section from `CHANGELOG.md` if present. Options:

- Hand-curate the release notes after the draft is created.
- Or use a small action step to extract the `## [X.Y.Z]` block from `CHANGELOG.md` and pass it as `body`.

### 1.5 Optional: version metadata in binary

Ensure `mew --version` prints the exact version from `Cargo.toml`. If it already does, no work. If not, wire `env!("CARGO_PKG_VERSION")` into the version flag.

---

## Phase 2: Homebrew tap

### Tap structure

Create a separate repo (recommended name): `github.com/mewcomputer/homebrew-mew`.

Homebrew taps are discovered by name: `brew tap mewcomputer/mew`.

### Formula: `mew.rb`

A multi-platform formula that downloads the GitHub release tarball per platform:

```ruby
class Mew < Formula
  desc "Terminal AI coding assistant"
  homepage "https://mew.computer"
  version "0.2.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/mewcomputer/mew/releases/download/v0.2.0/mew-v0.2.0-aarch64-apple-darwin.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/mewcomputer/mew/releases/download/v0.2.0/mew-v0.2.0-x86_64-apple-darwin.tar.gz"
      sha256 "..."
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/mewcomputer/mew/releases/download/v0.2.0/mew-v0.2.0-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "..."
    end
    on_intel do
      url "https://github.com/mewcomputer/mew/releases/download/v0.2.0/mew-v0.2.0-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "..."
    end
  end

  def install
    bin.install "bin/mew"
  end

  test do
    system "#{bin}/mew", "--version"
  end
end
```

### Updating the formula

Options:

- **Manual**: after each release, a maintainer opens a PR in `homebrew-mew` updating the version and four sha256 values.
- **Automated**: add a step to `.github/workflows/release.yml` that uses a GitHub App token or `secrets.HOMEBREW_TAP_TOKEN` to commit the updated formula to the tap repo.
- Recommendation: manual for the first release, then automate if the cadence demands it.

### sha256 source

Read the values from the `SHA256SUMS` file attached to the GitHub release.

---

## Phase 3: install script (`get.mew.computer` / `mew.computer/get`)

### Goal

A one-liner install that works on macOS and Linux:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://get.mew.computer | sh
```

Or, if the URL is a path rather than a subdomain:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh
```

### Script behavior

The script should be a single static bash file served by the Astro site (or a plain static file on a CDN).

It calls the GitHub API for metadata:

```sh
LATEST_URL="https://api.github.com/repos/mewcomputer/mew/releases/latest"
RELEASE_JSON=$(curl -sSL "$LATEST_URL")
VERSION=$(echo "$RELEASE_JSON" | grep -o '"tag_name": "v[^"]*"' | cut -d'"' -f4)
```

Then it:

1. Detects OS/arch (`uname -s`, `uname -m`).
2. Maps to the matching tarball name.
3. Downloads the tarball from `https://github.com/mewcomputer/mew/releases/download/${VERSION}/mew-${VERSION}-${TARGET}.tar.gz`.
4. Downloads `SHA256SUMS` from the same release.
5. Verifies the tarball's hash.
6. Extracts the `bin/mew` binary.
7. Installs it to `~/.local/bin` or `~/bin` (whichever is on `PATH`, or override via `MEW_INSTALL_DIR`).
8. Prints instructions if `PATH` needs updating.

### Environment variables

Mirror the polytoken pattern where useful:

- `MEW_VERSION` — install a specific version.
- `MEW_INSTALL_DIR` — override install location.
- `MEW_DRY_RUN` — print what would happen.
- `MEW_VERBOSE` — extra logging.

### Security notes

- Require `https://` for all downloads.
- Verify SHA256 before installing.
- Do not run as root; fail if the chosen install dir is not writable.
- Print the exact URL being downloaded so users can audit.

### Hosting

Option A: serve as a static file from the existing Astro site.

- Add `site/public/get.sh`.
- It will be available at `https://mew.computer/get.sh`.
- Add a redirect or alias from `https://mew.computer/get` if desired.

Option B: dedicated subdomain.

- `get.mew.computer` pointing to the same site or a CDN.
- More vanity, same file.

Recommendation: start with `mew.computer/get.sh` (Option A), since the Astro site already exists. Add a `get.mew.computer` redirect later.

### Script update cadence

The script itself rarely changes. The GitHub API call always resolves the latest release, so no per-release edits are needed unless the tarball naming convention changes.

---

## Phase 4: documentation updates

Update `docs/getting-started/installation.md` to lead with the easy paths:

1. **Homebrew** (macOS / Linux with Homebrew):
   ```sh
   brew tap mewcomputer/mew
   brew install mew
   ```
2. **Install script** (macOS / Linux):
   ```sh
   curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh
   ```
3. **Manual download**: link to GitHub Releases.
4. **Cargo** (for contributors / bleeding edge):
   ```sh
   cargo install --git https://github.com/mewcomputer/mew mew
   ```
5. **Build from source**: keep the existing instructions.

Add an **Upgrading** section:

- Homebrew: `brew upgrade mew`
- Script: re-run the install script.
- Cargo: `cargo install --force --git ...`

---

## Phase 5: optional future package managers

Out of scope for the first pass, but worth noting:

- **Nix**: `mew` in nixpkgs or a flake in the repo.
- **Scoop** / **Chocolatey**: Windows install paths.
- **AUR**: Arch User Repository package.
- **apt/deb/rpm**: if we want distro-native packages later.

---

## Implementation order

1. Pick the first version and write `CHANGELOG.md`.
2. Update `Cargo.toml` workspace version.
3. Expand release matrix and add checksum generation in `.github/workflows/release.yml`.
4. Create the `homebrew-mew` repo and initial formula.
5. Add `site/public/get.sh` install script.
6. Update `docs/getting-started/installation.md`.
7. Cut the first `v0.2.0` release and verify Homebrew + install script end-to-end.

---

## Done when

A new user on macOS or Linux can run one of:

```sh
brew tap mewcomputer/mew && brew install mew
curl --proto '=https' --tlsv1.2 -sSf https://mew.computer/get.sh | sh
```

and end up with a working `mew --version` that matches the latest GitHub release.

---

## Open questions

1. What should the first public version be? (`0.2.0`? `0.1.0`? `1.0.0`?)
2. Homebrew tap name: `mewcomputer/homebrew-mew` or `mewcomputer/homebrew-tap`?
3. Install script URL: `mew.computer/get.sh`, `get.mew.computer`, or both?
4. Do we want the release workflow to auto-update the Homebrew formula, or manual PRs for now?
5. Should we add `aarch64-unknown-linux-musl` or stick with `gnu` for the first release?
