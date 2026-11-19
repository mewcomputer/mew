#!/usr/bin/env bash
set -euo pipefail

# Generate a Homebrew formula for mew from a GitHub release.
# Usage:
#   scripts/generate-homebrew-formula.sh v0.2.0 [owner/repository] [SHA256SUMS]
#
# The script fetches the SHA256SUMS asset from the release and prints the
# formula to stdout. Copy the output into your tap repo, e.g.:
#   scripts/generate-homebrew-formula.sh v0.2.0 > ../homebrew-tap/Formula/mew.rb

release_tag="${1:-}"
version="${release_tag#nightly-}"
repo="${2:-mewcomputer/mew}"
checksums_path="${3:-}"
if [[ ! "$version" =~ ^v[0-9][0-9A-Za-z.+-]*$ || ! "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ || $# -gt 3 ]]; then
    echo "Usage: $0 <release-tag> [owner/repository] [SHA256SUMS]" >&2
    echo "Example: $0 v0.2.0" >&2
    exit 1
fi

base_url="https://github.com/${repo}/releases/download/${release_tag}"
if [[ -z "$checksums_path" ]]; then
    tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t mew-homebrew)"
    trap 'rm -rf "$tmp_dir"' EXIT
    checksums_path="$tmp_dir/SHA256SUMS"
    curl --fail --show-error --silent --location --proto '=https' --proto-redir '=https' --tlsv1.2 \
        --output "$checksums_path" "$base_url/SHA256SUMS"
fi

sha_for() {
    local file="$1"
    local checksum
    checksum="$(awk -v rel_path="$file" '$2 == rel_path { print $1 }' "$checksums_path")"
    if [[ ! "$checksum" =~ ^[0-9a-f]{64}$ ]]; then
        echo "Expected one SHA-256 checksum for $file" >&2
        return 1
    fi
    printf '%s' "$checksum"
}

mac_arm="$(sha_for "mew-${version}-aarch64-apple-darwin.tar.gz")"
mac_intel="$(sha_for "mew-${version}-x86_64-apple-darwin.tar.gz")"
linux_arm="$(sha_for "mew-${version}-aarch64-unknown-linux-gnu.tar.gz")"
linux_intel="$(sha_for "mew-${version}-x86_64-unknown-linux-gnu.tar.gz")"

cat <<EOF
class Mew < Formula
  desc "Terminal AI coding assistant"
  homepage "https://mew.computer"
  version "${version#v}"
  license "MIT"

  on_macos do
    on_arm do
      url "${base_url}/mew-${version}-aarch64-apple-darwin.tar.gz"
      sha256 "${mac_arm}"
    end
    on_intel do
      url "${base_url}/mew-${version}-x86_64-apple-darwin.tar.gz"
      sha256 "${mac_intel}"
    end
  end

  on_linux do
    on_arm do
      url "${base_url}/mew-${version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "${linux_arm}"
    end
    on_intel do
      url "${base_url}/mew-${version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "${linux_intel}"
    end
  end

  def install
    bin.install "bin/mew"
  end

  test do
    system "#{bin}/mew", "--version"
  end
end
EOF
