#!/usr/bin/env bash
set -euo pipefail

generator="$(dirname "$0")/../generate-homebrew-formula.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

version=v0.2.0-pre.20261118.142
tag="nightly-$version"
checksum=8e5eeada42c3072eda69b0f95f0ec4a4a0f20e9b33cfe7c02a786aacce457fe7
targets=(aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-gnu x86_64-unknown-linux-gnu)
for target in "${targets[@]}"; do
    printf '%s  mew-%s-%s.tar.gz\n' "$checksum" "$version" "$target"
done > "$work/SHA256SUMS"

bash "$generator" "$tag" mewcomputer/mew "$work/SHA256SUMS" > "$work/mew.rb"
grep -Fq "version \"${version#v}\"" "$work/mew.rb"
for target in "${targets[@]}"; do
    grep -Fq "https://github.com/mewcomputer/mew/releases/download/$tag/mew-$version-$target.tar.gz" "$work/mew.rb"
done
test "$(grep -Fc "sha256 \"$checksum\"" "$work/mew.rb")" -eq 4
ruby -c "$work/mew.rb"

head -n 3 "$work/SHA256SUMS" > "$work/incomplete"
if bash "$generator" "$tag" mewcomputer/mew "$work/incomplete" > "$work/bad.rb" 2> "$work/error"; then
    echo 'Expected a missing checksum to fail' >&2
    exit 1
fi
test ! -s "$work/bad.rb"
grep -Fq "Expected one SHA-256 checksum for mew-$version-x86_64-unknown-linux-gnu.tar.gz" "$work/error"
