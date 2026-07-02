#!/usr/bin/env bash
set -euo pipefail

# Generate a Homebrew formula for mew from a GitHub release.
# Usage:
#   scripts/generate-homebrew-formula.sh v0.2.0
#
# The script fetches the SHA256SUMS asset from the release and prints the
# formula to stdout. Copy the output into your tap repo, e.g.:
#   scripts/generate-homebrew-formula.sh v0.2.0 > ../homebrew-mew/Formula/mew.rb

version="${1:-}"
if [[ -z "$version" ]]; then
    echo "Usage: $0 <version>" >&2
    echo "Example: $0 v0.2.0" >&2
    exit 1
fi

repo="mewcomputer/mew"
base_url="https://github.com/${repo}/releases/download/${version}"
checksums_url="${base_url}/SHA256SUMS"

tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t mew-homebrew)"
trap 'rm -rf "$tmp_dir"' EXIT

checksums_path="$tmp_dir/SHA256SUMS"
curl --fail --show-error --silent --proto '=https' --tlsv1.2 \
    --output "$checksums_path" "$checksums_url"

sha_for() {
    local file="$1"
    awk -v rel_path="$file" '$2 == rel_path { print $1 }' "$checksums_path"
}

cat <<EOF
class Mew < Formula
  desc "Terminal AI coding assistant"
  homepage "https://mew.computer"
  version "${version#v}"
  license "MIT"

  on_macos do
    on_arm do
      url "${base_url}/mew-${version}-aarch64-apple-darwin.tar.gz"
      sha256 "$(sha_for "mew-${version}-aarch64-apple-darwin.tar.gz")"
    end
    on_intel do
      url "${base_url}/mew-${version}-x86_64-apple-darwin.tar.gz"
      sha256 "$(sha_for "mew-${version}-x86_64-apple-darwin.tar.gz")"
    end
  end

  on_linux do
    on_arm do
      url "${base_url}/mew-${version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "$(sha_for "mew-${version}-aarch64-unknown-linux-gnu.tar.gz")"
    end
    on_intel do
      url "${base_url}/mew-${version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "$(sha_for "mew-${version}-x86_64-unknown-linux-gnu.tar.gz")"
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
